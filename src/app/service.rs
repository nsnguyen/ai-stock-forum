use std::{collections::BTreeSet, sync::{Arc, RwLock, RwLockReadGuard}};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    agents::{
        AgentBindingCatalogSnapshot, AgentProfileDraft, AgentProfileVersion, AgentReadiness,
        ProfileEditPreview, ProfileReviewRegistry, ProfileTemplate, ReviewReservationError,
        builtin_profile_templates, candidate_digest, diff_profile, normalize_profile_name_key,
        profile_template_from_provenance, review_digest,
    },
    app::{
        AgentProfileCreatedView, AgentProfileHistoryEntry, AgentProfileHistoryView,
        AgentProfileSelector, AgentProfileSummary, AgentProfileVersionActivatedView,
        AgentProfileVersionView, AgentProfileView, AgentProfilesView, AppError, ApplicationCommand,
        ApplicationEvent, AgentSkillAssignmentOperation, AgentSkillAssignmentPreview,
        AgentSkillMutationView, AuditTailView, CommandEnvelope, CommandOutcome, CommandView,
        EVENT_SCHEMA_VERSION, HelpView, InputRejectedView, MAX_AGENT_PROFILE_HISTORY_RESULTS,
        MAX_AGENT_PROFILE_LIST_RESULTS, MAX_SKILL_HISTORY_RESULTS, MAX_SKILL_LIST_RESULTS,
        PendingEvent, SetupStatusView, ShutdownDisposition, ShutdownReason, ShutdownView,
        SkillCreatedView, SkillEventSummary, SkillHistoryEntry, SkillHistoryEventEntry,
        SkillHistoryView, SkillSelector, SkillSummary, SkillVersionActivatedView, SkillView,
        SkillsView, StatusView,
    },
    audit::AuditEntry,
    config::{AppPaths, StartupError},
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CausationId, Clock, CommandId, CorrelationId,
        EventId, IdGenerator, InstallationId, MemoryNamespaceId, ObjectVersion,
        ProfileReviewToken, SessionId, Sha256Digest, SkillId, SkillReviewToken, SkillVersionId,
        canonical_json_bytes, sha256,
    },
    persistence::{
        CommandReceiptRecord, CommandReceiptRepository, Database, EventRepository,
        ImmediateTransaction, PersistenceError, ProjectionRepository, RecoveryError,
        insert_expected_version, insert_skill_version, load_active_skill,
        load_active_skill_by_name, load_all_skill_versions, load_skill_history,
        load_skill_version, set_active_skill,
    },
    policy::{Capability, Effect, PolicyDecision, PolicyRule, evaluate},
    recovery::{BootstrapState, ProjectionState, RecoveryCoordinator, reduce},
    setup::SetupStatus,
    skills::{SkillDraft, SkillEditPreview, SkillProvenance, SkillReviewRegistry, SkillVersion,
        SkillVersionRef},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationDecision {
    Granted,
    Denied(PolicyDecision),
    ApprovalRequired,
}

pub trait CommandPolicy: Send + Sync {
    fn authorize(&self, capability: Capability) -> AuthorizationDecision;
}

pub trait CommandTransactionHook: Send + Sync {
    fn before_profile_mutation_transaction(&self, _command_id: CommandId) {}

    fn before_profile_review_operation(&self, _command_id: CommandId) {}

    fn after_profile_name_precheck(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
        _command_id: CommandId,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_profile_review_reservation(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
        _command_id: CommandId,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn before_user_lifecycle_read(&self) {}

    fn after_user_lifecycle_read(&self) {}

    fn before_finish_lifecycle_write(&self) {}

    fn before_outcome_materialization(
        &self,
        transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError>;

    fn before_receipt_write(
        &self,
        transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError>;

    fn after_event_append(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_profile_mirror_insert(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_active_pointer_update(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_projection_store(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_audit_append(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn after_receipt_store(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn before_commit(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }
}

pub struct NoopCommandTransactionHook;

impl CommandTransactionHook for NoopCommandTransactionHook {
    fn before_outcome_materialization(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn before_receipt_write(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }
}

struct PhaseZeroPolicy {
    rules: [PolicyRule; 14],
}

impl Default for PhaseZeroPolicy {
    fn default() -> Self {
        Self {
            rules: [
                PolicyRule::new(Effect::Grant, Capability::HelpRead),
                PolicyRule::new(Effect::Grant, Capability::StatusRead),
                PolicyRule::new(Effect::Grant, Capability::SetupStatusRead),
                PolicyRule::new(Effect::Grant, Capability::AuditRead),
                PolicyRule::new(Effect::Grant, Capability::AgentProfileRead),
                PolicyRule::new(Effect::Grant, Capability::AgentProfileCreate),
                PolicyRule::new(Effect::Grant, Capability::AgentProfilePreview),
                PolicyRule::new(Effect::Grant, Capability::AgentProfileActivate),
                PolicyRule::new(Effect::Grant, Capability::SkillRead),
                PolicyRule::new(Effect::Grant, Capability::SkillCreate),
                PolicyRule::new(Effect::Grant, Capability::SkillVersion),
                PolicyRule::new(Effect::Grant, Capability::AgentSkillAssign),
                PolicyRule::new(Effect::Grant, Capability::AgentSkillUnassign),
                PolicyRule::new(Effect::Grant, Capability::Shutdown),
            ],
        }
    }
}

impl CommandPolicy for PhaseZeroPolicy {
    fn authorize(&self, capability: Capability) -> AuthorizationDecision {
        match evaluate(capability, &self.rules) {
            PolicyDecision::Granted => AuthorizationDecision::Granted,
            decision @ (PolicyDecision::Denied | PolicyDecision::DeniedByDefault) => {
                AuthorizationDecision::Denied(decision)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandRequest {
    correlation_id: CorrelationId,
    actor: Actor,
    command: ApplicationCommand,
}

impl From<&CommandEnvelope> for CommandRequest {
    fn from(envelope: &CommandEnvelope) -> Self {
        Self {
            correlation_id: envelope.correlation_id,
            actor: envelope.actor.clone(),
            command: envelope.command.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoredPolicyDecision {
    Granted,
    Denied,
    DeniedByDefault,
    ApprovalRequired,
}

impl StoredPolicyDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::DeniedByDefault => "denied_by_default",
            Self::ApprovalRequired => "approval_required",
        }
    }

    fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "granted" => Ok(Self::Granted),
            "denied" => Ok(Self::Denied),
            "denied_by_default" => Ok(Self::DeniedByDefault),
            "approval_required" => Ok(Self::ApprovalRequired),
            _ => Err(invalid_receipt()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
#[expect(
    clippy::large_enum_variant,
    reason = "stored exact outcomes retain value semantics through receipt materialization"
)]
enum StoredExecution {
    Success {
        outcome: CommandOutcome,
    },
    CapabilityDenied {
        capability: Capability,
        decision: PolicyDecision,
    },
    ApprovalRequired {
        capability: Capability,
    },
}

impl StoredExecution {
    fn into_result(self) -> Result<CommandOutcome, AppError> {
        match self {
            Self::Success { outcome } => Ok(outcome),
            Self::CapabilityDenied {
                capability,
                decision,
            } => Err(AppError::CapabilityDenied {
                capability,
                decision,
            }),
            Self::ApprovalRequired { capability } => Err(AppError::ApprovalRequired { capability }),
        }
    }
}

pub struct ApplicationService {
    paths: AppPaths,
    state: BootstrapState,
    executor: CommandExecutor,
}

#[doc(hidden)]
pub struct IndependentApplicationService {
    executor: CommandExecutor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseReadiness {
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessGuardOwnership {
    Held,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationSnapshot {
    pub installation_id: InstallationId,
    pub session_id: SessionId,
    pub database_readiness: DatabaseReadiness,
    pub process_guard_ownership: ProcessGuardOwnership,
    pub setup_status: SetupStatus,
    pub recent_audit: Vec<AuditEntry>,
    pub agent_profiles: AgentProfilesView,
    pub selected_agent_profile: Option<AgentProfileView>,
    pub selected_agent_profile_history: Option<AgentProfileHistoryView>,
}

pub struct ApplicationWorker {
    executor: CommandExecutor,
}

struct CommandExecutor {
    database: Database,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    policy: Arc<dyn CommandPolicy>,
    hook: Arc<dyn CommandTransactionHook>,
    lifecycle: Arc<SharedLifecycle>,
    reviews: Arc<ProfileReviewRegistry>,
    skill_reviews: Arc<SkillReviewRegistry>,
    binding_catalog: Arc<AgentBindingCatalogSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecyclePhase {
    Open,
    Closed,
}

#[derive(Debug)]
struct SharedLifecycle {
    session_id: SessionId,
    phase: RwLock<LifecyclePhase>,
}

impl ApplicationService {
    #[doc(hidden)]
    pub fn independent_profile_instance(
        &self,
    ) -> Result<IndependentApplicationService, StartupError> {
        let worker = self.worker()?;
        let mut executor = worker.executor;
        executor.reviews = Arc::new(ProfileReviewRegistry::default());
        executor.skill_reviews = Arc::new(SkillReviewRegistry::default());
        Ok(IndependentApplicationService { executor })
    }

    #[doc(hidden)]
    pub fn independent_skill_instance(
        &self,
    ) -> Result<IndependentApplicationService, StartupError> {
        self.independent_profile_instance()
    }

    pub fn bootstrap(
        paths: &AppPaths,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
    ) -> Result<Self, StartupError> {
        Self::bootstrap_with_dependencies(
            paths,
            clock,
            ids,
            Arc::new(PhaseZeroPolicy::default()),
            Arc::new(NoopCommandTransactionHook),
        )
    }

    #[doc(hidden)]
    pub fn bootstrap_with_policy(
        paths: &AppPaths,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
        policy: Arc<dyn CommandPolicy>,
    ) -> Result<Self, StartupError> {
        Self::bootstrap_with_dependencies(
            paths,
            clock,
            ids,
            policy,
            Arc::new(NoopCommandTransactionHook),
        )
    }

    #[doc(hidden)]
    pub fn bootstrap_with_dependencies(
        paths: &AppPaths,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
        policy: Arc<dyn CommandPolicy>,
        hook: Arc<dyn CommandTransactionHook>,
    ) -> Result<Self, StartupError> {
        Self::bootstrap_with_dependencies_and_binding_catalog(
            paths,
            clock,
            ids,
            policy,
            hook,
            AgentBindingCatalogSnapshot::default(),
        )
    }

    #[doc(hidden)]
    pub fn bootstrap_with_dependencies_and_binding_catalog(
        paths: &AppPaths,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
        policy: Arc<dyn CommandPolicy>,
        hook: Arc<dyn CommandTransactionHook>,
        binding_catalog: AgentBindingCatalogSnapshot,
    ) -> Result<Self, StartupError> {
        let mut database = Database::open(paths)?;
        let state = RecoveryCoordinator::bootstrap_after_database_ready(
            &mut database,
            clock.as_ref(),
            ids.as_ref(),
            &[],
        )?;
        let lifecycle = Arc::new(SharedLifecycle {
            session_id: state.session_id(),
            phase: RwLock::new(LifecyclePhase::Open),
        });
        let reviews = Arc::new(ProfileReviewRegistry::default());
        let skill_reviews = Arc::new(SkillReviewRegistry::default());
        Ok(Self {
            paths: paths.clone(),
            state,
            executor: CommandExecutor {
                database,
                clock,
                ids,
                policy,
                hook,
                lifecycle,
                reviews,
                skill_reviews,
                binding_catalog: Arc::new(binding_catalog),
            },
        })
    }

    pub fn worker(&self) -> Result<ApplicationWorker, StartupError> {
        Ok(ApplicationWorker {
            executor: CommandExecutor {
                database: Database::open(&self.paths)?,
                clock: self.executor.clock.clone(),
                ids: self.executor.ids.clone(),
                policy: self.executor.policy.clone(),
                hook: self.executor.hook.clone(),
                lifecycle: self.executor.lifecycle.clone(),
                reviews: self.executor.reviews.clone(),
                skill_reviews: self.executor.skill_reviews.clone(),
                binding_catalog: self.executor.binding_catalog.clone(),
            },
        })
    }

    pub fn execute_user(
        &mut self,
        command: ApplicationCommand,
    ) -> Result<CommandOutcome, AppError> {
        self.executor.execute_user(command)
    }

    pub fn execute(&mut self, envelope: CommandEnvelope) -> Result<CommandOutcome, AppError> {
        self.executor.execute(envelope)
    }

    pub fn agent_profile_templates(&self) -> Result<Vec<ProfileTemplate>, AppError> {
        self.executor.agent_profile_templates()
    }

    pub fn agent_binding_catalog(&self) -> Result<AgentBindingCatalogSnapshot, AppError> {
        self.executor.agent_binding_catalog()
    }

    pub fn preview_agent_profile_edit(
        &self,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate: AgentProfileDraft,
    ) -> Result<ProfileEditPreview, AppError> {
        self.executor
            .preview_agent_profile_edit(profile_id, expected_active_version_id, candidate)
    }

    pub fn cancel_agent_profile_edit(&self) -> Result<(), AppError> {
        self.executor.reviews.cancel();
        Ok(())
    }

    pub fn preview_skill_creation(&self, candidate: SkillDraft) -> Result<SkillEditPreview, AppError> {
        self.executor.preview_skill_creation(candidate)
    }

    pub fn preview_skill_version(
        &self,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.executor.preview_skill_version(skill_id, expected_active_version_id, candidate)
    }

    pub fn preview_agent_skill_assignment(
        &self,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        skill: SkillVersionRef,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        self.executor.preview_agent_skill_operation(
            profile_id,
            expected_active_profile_version_id,
            AgentSkillAssignmentOperation::Assign { skill },
        )
    }

    pub fn preview_agent_skill_upgrade(
        &self,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        expected: SkillVersionRef,
        replacement: SkillVersionRef,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        self.executor.preview_agent_skill_operation(
            profile_id,
            expected_active_profile_version_id,
            AgentSkillAssignmentOperation::Upgrade { expected, replacement },
        )
    }

    pub fn preview_agent_skill_unassignment(
        &self,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        expected: SkillVersionRef,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        self.executor.preview_agent_skill_operation(
            profile_id,
            expected_active_profile_version_id,
            AgentSkillAssignmentOperation::Unassign { expected },
        )
    }

    pub fn cancel_skill_review(&self) -> Result<(), AppError> {
        self.executor.skill_reviews.cancel();
        Ok(())
    }

    pub fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        self.executor.hook.before_finish_lifecycle_write();
        let lifecycle = self.executor.lifecycle.clone();
        let mut phase = lifecycle
            .phase
            .write()
            .map_err(|_| AppError::LifecycleFinished)?;
        if *phase == LifecyclePhase::Closed {
            return Ok(());
        }
        self.executor.reviews.cancel();
        self.executor.skill_reviews.cancel();
        RecoveryCoordinator::finish_session(
            &mut self.executor.database,
            &mut self.state,
            reason,
            self.executor.clock.as_ref(),
            self.executor.ids.as_ref(),
        )?;
        *phase = LifecyclePhase::Closed;
        Ok(())
    }

    pub fn installation_id(&self) -> crate::domain::InstallationId {
        self.state.installation_id()
    }

    pub fn session_id(&self) -> crate::domain::SessionId {
        self.state.session_id()
    }

    pub fn presentation_snapshot(
        &self,
        limit: crate::app::AuditLimit,
    ) -> Result<PresentationSnapshot, AppError> {
        let projected_sequence = self
            .executor
            .database
            .connection()
            .query_row(
                "SELECT last_event_sequence FROM projection_metadata WHERE singleton = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| PersistenceError::QueryFailed)?;
        let projected_sequence =
            u64::try_from(projected_sequence).map_err(|_| PersistenceError::InvalidEventRecord)?;
        let projection =
            ProjectionRepository::load_at(self.executor.database.connection(), projected_sequence)?;
        let events = EventRepository::tail_through(
            self.executor.database.connection(),
            limit,
            projection.last_sequence,
        )?;
        let total_count = projection.agent_profiles.active_profile_count();
        let active_profiles = projection
            .agent_profiles
            .active_profiles_bounded(MAX_AGENT_PROFILE_LIST_RESULTS);
        let selected_agent_profile = active_profiles
            .first()
            .map(|profile| profile_view(profile, self.executor.binding_catalog.as_ref()));
        let selected_agent_profile_history = active_profiles.first().and_then(|profile| {
            profile_history_view(
                &projection,
                profile.profile_id(),
                self.executor.binding_catalog.as_ref(),
            )
        });

        Ok(PresentationSnapshot {
            installation_id: self.state.installation_id(),
            session_id: self.state.session_id(),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: projection.setup_status.clone(),
            recent_audit: events.iter().map(AuditEntry::from_event).collect(),
            agent_profiles: AgentProfilesView {
                profiles: active_profiles
                    .iter()
                    .map(|profile| profile_summary(profile, self.executor.binding_catalog.as_ref()))
                    .collect(),
                total_count: u32::try_from(total_count)
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                returned_count: u32::try_from(active_profiles.len())
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                truncated: total_count > active_profiles.len(),
            },
            selected_agent_profile,
            selected_agent_profile_history,
        })
    }

    pub const fn previous_session_interrupted(&self) -> bool {
        self.state.previous_session_interrupted()
    }
}

impl IndependentApplicationService {
    pub fn execute(&mut self, envelope: CommandEnvelope) -> Result<CommandOutcome, AppError> {
        self.executor.execute(envelope)
    }

    pub fn preview_agent_profile_edit(
        &self,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate: AgentProfileDraft,
    ) -> Result<ProfileEditPreview, AppError> {
        self.executor
            .preview_agent_profile_edit(profile_id, expected_active_version_id, candidate)
    }

    pub fn preview_skill_creation(&self, candidate: SkillDraft) -> Result<SkillEditPreview, AppError> {
        self.executor.preview_skill_creation(candidate)
    }

    pub fn preview_skill_version(
        &self,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.executor.preview_skill_version(skill_id, expected_active_version_id, candidate)
    }

    pub fn preview_agent_skill_assignment(
        &self,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        skill: SkillVersionRef,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        self.executor.preview_agent_skill_operation(
            profile_id,
            expected_active_profile_version_id,
            AgentSkillAssignmentOperation::Assign { skill },
        )
    }

    pub fn preview_agent_skill_upgrade(
        &self,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        expected: SkillVersionRef,
        replacement: SkillVersionRef,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        self.executor.preview_agent_skill_operation(
            profile_id,
            expected_active_profile_version_id,
            AgentSkillAssignmentOperation::Upgrade {
                expected,
                replacement,
            },
        )
    }

    pub fn preview_agent_skill_unassignment(
        &self,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        expected: SkillVersionRef,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        self.executor.preview_agent_skill_operation(
            profile_id,
            expected_active_profile_version_id,
            AgentSkillAssignmentOperation::Unassign { expected },
        )
    }
}

impl ApplicationWorker {
    pub fn agent_profile_templates(&self) -> Result<Vec<ProfileTemplate>, AppError> {
        self.executor.agent_profile_templates()
    }

    pub fn agent_binding_catalog(&self) -> Result<AgentBindingCatalogSnapshot, AppError> {
        self.executor.agent_binding_catalog()
    }

    pub fn execute_user(
        &mut self,
        command: ApplicationCommand,
    ) -> Result<CommandOutcome, AppError> {
        self.executor.execute_user(command)
    }

    pub fn execute(&mut self, envelope: CommandEnvelope) -> Result<CommandOutcome, AppError> {
        self.executor.execute(envelope)
    }

    pub fn preview_skill_creation(&self, candidate: SkillDraft) -> Result<SkillEditPreview, AppError> {
        self.executor.preview_skill_creation(candidate)
    }

    pub fn preview_skill_version(
        &self,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.executor.preview_skill_version(skill_id, expected_active_version_id, candidate)
    }
}

impl CommandExecutor {
    fn preview_skill_creation(&self, candidate: SkillDraft) -> Result<SkillEditPreview, AppError> {
        self.ensure_passive_open(Capability::SkillCreate)?;
        let candidate = candidate.canonicalized()?;
        let normalized = candidate.normalized_name()?;
        if load_active_skill_by_name(self.database.connection(), &normalized)?.is_some() {
            return Err(AppError::DuplicateSkillName);
        }
        self.skill_reviews.operation().issue_edit(
            SkillReviewToken::from_uuid(self.ids.next_uuid()),
            SkillId::from_uuid(self.ids.next_uuid()),
            None,
            &candidate,
        ).map_err(AppError::from)
    }

    fn preview_skill_version(
        &self,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.ensure_passive_open(Capability::SkillVersion)?;
        let candidate = candidate.canonicalized()?;
        let current = load_active_skill(self.database.connection(), skill_id)?
            .ok_or(AppError::SkillNotFound)?;
        if current.skill_version_id() != expected_active_version_id {
            return Err(AppError::StaleSkillVersion);
        }
        if current.content() == &candidate {
            return Err(crate::domain::DomainError::SkillUnchanged.into());
        }
        ensure_skill_name_available(self.database.connection(), Some(skill_id), &candidate)?;
        self.skill_reviews.operation().issue_edit(
            SkillReviewToken::from_uuid(self.ids.next_uuid()),
            skill_id,
            Some(expected_active_version_id),
            &candidate,
        ).map_err(AppError::from)
    }

    fn preview_agent_skill_operation(
        &self,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        operation: AgentSkillAssignmentOperation,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        let capability = match operation {
            AgentSkillAssignmentOperation::Unassign { .. } => Capability::AgentSkillUnassign,
            AgentSkillAssignmentOperation::Assign { .. }
            | AgentSkillAssignmentOperation::Upgrade { .. } => Capability::AgentSkillAssign,
        };
        self.ensure_passive_open(capability)?;
        let projection = ProjectionRepository::load(self.database.connection())?;
        let current = projection.agent_profiles.active_profile(profile_id)
            .ok_or(AppError::AgentProfileNotFound)?;
        if current.profile_version_id() != expected_active_profile_version_id {
            return Err(AppError::StaleAgentProfileVersion);
        }
        validate_assignment_operation(self.database.connection(), current, &operation)?;
        self.skill_reviews.operation().issue_assignment(
            SkillReviewToken::from_uuid(self.ids.next_uuid()),
            profile_id,
            expected_active_profile_version_id,
            operation,
        ).map_err(AppError::from)
    }

    fn ensure_passive_open(&self, capability: Capability) -> Result<(), AppError> {
        let phase = self.lifecycle.phase.read().map_err(|_| AppError::LifecycleFinished)?;
        if *phase == LifecyclePhase::Closed {
            return Err(AppError::LifecycleFinished);
        }
        authorize_passive(self.policy.as_ref(), capability)
    }
    fn agent_binding_catalog(&self) -> Result<AgentBindingCatalogSnapshot, AppError> {
        let phase = self
            .lifecycle
            .phase
            .read()
            .map_err(|_| AppError::LifecycleFinished)?;
        if *phase == LifecyclePhase::Closed {
            return Err(AppError::LifecycleFinished);
        }
        authorize_passive(self.policy.as_ref(), Capability::AgentProfileRead)?;
        Ok(self.binding_catalog.as_ref().clone())
    }

    fn agent_profile_templates(&self) -> Result<Vec<ProfileTemplate>, AppError> {
        let phase = self
            .lifecycle
            .phase
            .read()
            .map_err(|_| AppError::LifecycleFinished)?;
        if *phase == LifecyclePhase::Closed {
            return Err(AppError::LifecycleFinished);
        }
        authorize_passive(self.policy.as_ref(), Capability::AgentProfileRead)?;
        Ok(builtin_profile_templates().to_vec())
    }

    fn preview_agent_profile_edit(
        &self,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate: AgentProfileDraft,
    ) -> Result<ProfileEditPreview, AppError> {
        let phase = self
            .lifecycle
            .phase
            .read()
            .map_err(|_| AppError::LifecycleFinished)?;
        if *phase == LifecyclePhase::Closed {
            return Err(AppError::LifecycleFinished);
        }
        authorize_passive(self.policy.as_ref(), Capability::AgentProfilePreview)?;
        let candidate = candidate.canonicalized()?;
        let review_operation = self.reviews.operation();
        let projection = ProjectionRepository::load(self.database.connection())?;
        let current = projection
            .agent_profiles
            .active_profile(profile_id)
            .ok_or(AppError::AgentProfileNotFound)?;
        if current.profile_version_id() != expected_active_version_id {
            return Err(AppError::StaleAgentProfileVersion);
        }
        ensure_name_available(&projection, Some(profile_id), &candidate.display_name)?;
        let diffs = diff_profile(current, &candidate)?;
        let candidate_digest = candidate_digest(&candidate)?;
        let review_digest = review_digest(
            profile_id,
            expected_active_version_id,
            &candidate_digest,
            &diffs,
        )?;
        let review_token = ProfileReviewToken::from_uuid(self.ids.next_uuid());
        review_operation.replace(
            review_token,
            profile_id,
            expected_active_version_id,
            candidate_digest,
            review_digest.clone(),
        );
        Ok(ProfileEditPreview {
            profile_id,
            expected_active_version_id,
            diffs,
            review_token,
            review_digest,
        })
    }

    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        self.hook.before_user_lifecycle_read();
        let lifecycle = self.lifecycle.clone();
        let phase = lifecycle
            .phase
            .read()
            .map_err(|_| AppError::LifecycleFinished)?;
        if *phase == LifecyclePhase::Closed {
            return Err(AppError::LifecycleFinished);
        }
        self.hook.after_user_lifecycle_read();
        let envelope = CommandEnvelope {
            command_id: CommandId::from_uuid(self.ids.next_uuid()),
            correlation_id: CorrelationId::from_uuid(self.ids.next_uuid()),
            actor: Actor::Human,
            command,
        };
        self.execute_locked(envelope, lifecycle.session_id, phase)
    }

    fn execute(&mut self, envelope: CommandEnvelope) -> Result<CommandOutcome, AppError> {
        let lifecycle = self.lifecycle.clone();
        let phase = lifecycle
            .phase
            .read()
            .map_err(|_| AppError::LifecycleFinished)?;
        if *phase == LifecyclePhase::Closed {
            return Err(AppError::LifecycleFinished);
        }
        self.execute_locked(envelope, lifecycle.session_id, phase)
    }

    fn execute_locked(
        &mut self,
        mut envelope: CommandEnvelope,
        session_id: SessionId,
        _phase: RwLockReadGuard<'_, LifecyclePhase>,
    ) -> Result<CommandOutcome, AppError> {
        envelope.command.canonicalize_profile_payloads()?;
        let request = CommandRequest::from(&envelope);
        let request_json = encode_canonical(&request)?;
        let command_fingerprint = sha256(request_json.as_bytes());
        let reviewed_mutation = matches!(
            &request.command,
            ApplicationCommand::ActivateAgentProfileVersion { .. }
                | ApplicationCommand::CreateSkill { .. }
                | ApplicationCommand::ActivateSkillVersion { .. }
                | ApplicationCommand::AssignAgentSkill { .. }
                | ApplicationCommand::UpgradeAgentSkill { .. }
                | ApplicationCommand::UnassignAgentSkill { .. }
        );
        if reviewed_mutation {
            let replay_transaction = self.database.immediate_transaction()?;
            if let Some(receipt) =
                CommandReceiptRepository::load(&replay_transaction, envelope.command_id)?
            {
                let stored = validate_receipt(
                    &replay_transaction,
                    &receipt,
                    &request,
                    &request_json,
                    &command_fingerprint,
                )?;
                replay_transaction.commit()?;
                return stored.into_result();
            }
            replay_transaction.commit()?;
            self.hook.before_profile_review_operation(envelope.command_id);
        }
        let profile_review_operation = matches!(
            &request.command,
            ApplicationCommand::ActivateAgentProfileVersion { .. }
        )
        .then(|| self.reviews.operation());
        let skill_review_operation = matches!(
            &request.command,
            ApplicationCommand::CreateSkill { .. }
                | ApplicationCommand::ActivateSkillVersion { .. }
                | ApplicationCommand::AssignAgentSkill { .. }
                | ApplicationCommand::UpgradeAgentSkill { .. }
                | ApplicationCommand::UnassignAgentSkill { .. }
        )
        .then(|| self.skill_reviews.operation());
        if matches!(
            &request.command,
            ApplicationCommand::CreateAgentProfile { .. }
                | ApplicationCommand::ActivateAgentProfileVersion { .. }
                | ApplicationCommand::CreateSkill { .. }
                | ApplicationCommand::ActivateSkillVersion { .. }
                | ApplicationCommand::AssignAgentSkill { .. }
                | ApplicationCommand::UpgradeAgentSkill { .. }
                | ApplicationCommand::UnassignAgentSkill { .. }
        ) {
            self.hook
                .before_profile_mutation_transaction(envelope.command_id);
        }
        let transaction = self.database.immediate_transaction()?;
        if let Some(receipt) = CommandReceiptRepository::load(&transaction, envelope.command_id)? {
            let stored = validate_receipt(
                &transaction,
                &receipt,
                &request,
                &request_json,
                &command_fingerprint,
            )?;
            transaction.commit()?;
            return stored.into_result();
        }

        ensure_authoritative_session_open(&transaction, session_id)?;
        let mut projection = ProjectionRepository::load_in(&transaction)?;
        match projection.sessions.get(&session_id) {
            Some(session) if session.ended.is_none() => {}
            _ => return Err(AppError::LifecycleFinished),
        }

        let capability = request.command.required_capability();
        let denied = match self.policy.authorize(capability) {
            AuthorizationDecision::Granted => None,
            AuthorizationDecision::Denied(PolicyDecision::Denied) => Some((
                StoredPolicyDecision::Denied,
                StoredExecution::CapabilityDenied {
                    capability,
                    decision: PolicyDecision::Denied,
                },
            )),
            AuthorizationDecision::Denied(PolicyDecision::DeniedByDefault) => Some((
                StoredPolicyDecision::DeniedByDefault,
                StoredExecution::CapabilityDenied {
                    capability,
                    decision: PolicyDecision::DeniedByDefault,
                },
            )),
            AuthorizationDecision::Denied(PolicyDecision::Granted) => return Err(invalid_receipt()),
            AuthorizationDecision::ApprovalRequired => Some((
                StoredPolicyDecision::ApprovalRequired,
                StoredExecution::ApprovalRequired { capability },
            )),
        };
        if let Some((policy_decision, stored)) = denied {
            self.hook
                .before_outcome_materialization(transaction.transaction())?;
            let outcome_json = encode_canonical(&stored)?;
            self.hook.before_receipt_write(transaction.transaction())?;
            insert_receipt(
                &transaction,
                &envelope,
                command_fingerprint,
                request_json,
                capability,
                policy_decision,
                outcome_json,
                &[],
            )?;
            self.hook.after_receipt_store(transaction.transaction())?;
            self.hook.before_commit(transaction.transaction())?;
            transaction.commit()?;
            return stored.into_result();
        }

        let mut profile_reserved = false;
        let mut skill_reserved = false;
        let precommit =
            (|| -> Result<(StoredExecution, Vec<crate::app::EventEnvelope>), AppError> {
                let event = match &request.command {
                    ApplicationCommand::ActivateAgentProfileVersion {
                        profile_id,
                        expected_active_version_id,
                        candidate,
                        review_token,
                        review_digest: supplied_review_digest,
                    } => {
                        validate_draft(candidate)?;
                        let base = projection
                            .agent_profiles
                            .version(*expected_active_version_id)
                            .ok_or(AppError::StaleAgentProfileVersion)?;
                        let diffs = diff_profile(base, candidate)?;
                        let computed_candidate_digest = candidate_digest(candidate)?;
                        let computed_review_digest = review_digest(
                            *profile_id,
                            *expected_active_version_id,
                            &computed_candidate_digest,
                            &diffs,
                        )?;
                        if &computed_review_digest != supplied_review_digest {
                            return Err(AppError::ReviewDigestMismatch);
                        }
                        profile_review_operation
                            .as_ref()
                            .expect("activation owns the profile review operation")
                            .reserve(
                                envelope.command_id,
                                *review_token,
                                *profile_id,
                                *expected_active_version_id,
                                &computed_candidate_digest,
                                &computed_review_digest,
                            )
                            .map_err(|error| match error {
                                ReviewReservationError::Unavailable => {
                                    AppError::ProfileReviewUnavailable
                                }
                                ReviewReservationError::Mismatch => AppError::ProfileReviewMismatch,
                            })?;
                        profile_reserved = true;
                        self.hook.after_profile_review_reservation(
                            transaction.transaction(),
                            envelope.command_id,
                        )?;
                        let current = projection
                            .agent_profiles
                            .active_profile(*profile_id)
                            .ok_or(AppError::AgentProfileNotFound)?;
                        if current.profile_version_id() != *expected_active_version_id {
                            return Err(AppError::StaleAgentProfileVersion);
                        }
                        ensure_name_available(
                            &projection,
                            Some(*profile_id),
                            &candidate.display_name,
                        )?;
                        self.hook.after_profile_name_precheck(
                            transaction.transaction(),
                            envelope.command_id,
                        )?;
                        let profile = AgentProfileVersion::next_version(
                            current,
                            AgentProfileVersionId::from_uuid(self.ids.next_uuid()),
                            self.clock.now_millis(),
                            candidate.clone(),
                        )?;
                        ApplicationEvent::AgentProfileVersionActivated {
                            profile,
                            previous_version_id: *expected_active_version_id,
                        }
                    }
                    ApplicationCommand::CreateSkill {
                        skill_id,
                        candidate,
                        review_token,
                        review_digest,
                    } => {
                        skill_review_operation
                            .as_ref()
                            .expect("skill creation owns the skill review operation")
                            .reserve_edit(
                                envelope.command_id,
                                *review_token,
                                *skill_id,
                                None,
                                candidate,
                                review_digest,
                            )?;
                        skill_reserved = true;
                        if !load_skill_history(transaction.transaction(), *skill_id)?.is_empty() {
                            return Err(AppError::StaleSkillVersion);
                        }
                        ensure_skill_name_available(transaction.transaction(), None, candidate)?;
                        let skill = SkillVersion::create(
                            *skill_id,
                            SkillVersionId::from_uuid(self.ids.next_uuid()),
                            self.clock.now_millis(),
                            SkillProvenance::User,
                            candidate.clone(),
                        )?;
                        insert_skill_version(transaction.transaction(), &skill)?;
                        set_active_skill(transaction.transaction(), &skill)
                            .map_err(map_skill_write_error)?;
                        ApplicationEvent::SkillCreated {
                            skill: skill.reference(),
                            display_name: skill.content().display_name.clone(),
                            provenance: skill.provenance().clone(),
                        }
                    }
                    ApplicationCommand::ActivateSkillVersion {
                        skill_id,
                        expected_active_version_id,
                        candidate,
                        review_token,
                        review_digest,
                    } => {
                        skill_review_operation
                            .as_ref()
                            .expect("skill versioning owns the skill review operation")
                            .reserve_edit(
                                envelope.command_id,
                                *review_token,
                                *skill_id,
                                Some(*expected_active_version_id),
                                candidate,
                                review_digest,
                            )?;
                        skill_reserved = true;
                        let current = load_active_skill(transaction.transaction(), *skill_id)?
                            .ok_or(AppError::SkillNotFound)?;
                        if current.skill_version_id() != *expected_active_version_id {
                            return Err(AppError::StaleSkillVersion);
                        }
                        ensure_skill_name_available(
                            transaction.transaction(),
                            Some(*skill_id),
                            candidate,
                        )?;
                        let skill = SkillVersion::next_version(
                            &current,
                            SkillVersionId::from_uuid(self.ids.next_uuid()),
                            self.clock.now_millis(),
                            candidate.clone(),
                        )?;
                        insert_skill_version(transaction.transaction(), &skill)?;
                        set_active_skill(transaction.transaction(), &skill)
                            .map_err(map_skill_write_error)?;
                        ApplicationEvent::SkillVersionActivated {
                            skill: skill.reference(),
                            previous_version_id: current.skill_version_id(),
                            display_name: skill.content().display_name.clone(),
                            provenance: skill.provenance().clone(),
                        }
                    }
                    ApplicationCommand::AssignAgentSkill {
                        profile_id,
                        expected_active_profile_version_id,
                        skill,
                        review_token,
                        review_digest,
                    } => {
                        let operation = AgentSkillAssignmentOperation::Assign {
                            skill: skill.clone(),
                        };
                        skill_review_operation
                            .as_ref()
                            .expect("assignment owns the skill review operation")
                            .reserve_assignment(
                                envelope.command_id,
                                *review_token,
                                *profile_id,
                                *expected_active_profile_version_id,
                                &operation,
                                review_digest,
                            )?;
                        skill_reserved = true;
                        prepare_agent_skill_event(
                            &projection,
                            transaction.transaction(),
                            self.ids.as_ref(),
                            self.clock.as_ref(),
                            *profile_id,
                            *expected_active_profile_version_id,
                            operation,
                        )?
                    }
                    ApplicationCommand::UpgradeAgentSkill {
                        profile_id,
                        expected_active_profile_version_id,
                        expected,
                        replacement,
                        review_token,
                        review_digest,
                    } => {
                        let operation = AgentSkillAssignmentOperation::Upgrade {
                            expected: expected.clone(),
                            replacement: replacement.clone(),
                        };
                        skill_review_operation
                            .as_ref()
                            .expect("upgrade owns the skill review operation")
                            .reserve_assignment(
                                envelope.command_id,
                                *review_token,
                                *profile_id,
                                *expected_active_profile_version_id,
                                &operation,
                                review_digest,
                            )?;
                        skill_reserved = true;
                        prepare_agent_skill_event(
                            &projection,
                            transaction.transaction(),
                            self.ids.as_ref(),
                            self.clock.as_ref(),
                            *profile_id,
                            *expected_active_profile_version_id,
                            operation,
                        )?
                    }
                    ApplicationCommand::UnassignAgentSkill {
                        profile_id,
                        expected_active_profile_version_id,
                        expected,
                        review_token,
                        review_digest,
                    } => {
                        let operation = AgentSkillAssignmentOperation::Unassign {
                            expected: expected.clone(),
                        };
                        skill_review_operation
                            .as_ref()
                            .expect("unassignment owns the skill review operation")
                            .reserve_assignment(
                                envelope.command_id,
                                *review_token,
                                *profile_id,
                                *expected_active_profile_version_id,
                                &operation,
                                review_digest,
                            )?;
                        skill_reserved = true;
                        prepare_agent_skill_event(
                            &projection,
                            transaction.transaction(),
                            self.ids.as_ref(),
                            self.clock.as_ref(),
                            *profile_id,
                            *expected_active_profile_version_id,
                            operation,
                        )?
                    }
                    command => prepare_event(
                        command,
                        &projection,
                        self.clock.as_ref(),
                        self.ids.as_ref(),
                        self.hook.as_ref(),
                        transaction.transaction(),
                        envelope.command_id,
                    )?,
                };
                let pending = PendingEvent {
                    event_id: EventId::from_uuid(self.ids.next_uuid()),
                    event_schema_version: EVENT_SCHEMA_VERSION,
                    actor: request.actor.clone(),
                    occurred_at_ms: event_occurred_at(&event)
                        .unwrap_or_else(|| self.clock.now_millis()),
                    correlation_id: request.correlation_id,
                    causation_id: Some(CausationId::from_uuid(envelope.command_id.as_uuid())),
                    object: None,
                    event,
                };
                let committed = EventRepository::append(&transaction, pending)?;
                self.hook.after_event_append(transaction.transaction())?;
                reduce(&mut projection, &committed)?;
                if let ApplicationEvent::AgentProfileCreated { profile }
                | ApplicationEvent::AgentProfileVersionActivated { profile, .. } =
                    &committed.event
                {
                    insert_expected_version(
                        transaction.transaction(),
                        i64::try_from(committed.sequence)
                            .map_err(|_| PersistenceError::InvalidAgentProfilePayload)?,
                        profile,
                    )?;
                    self.hook
                        .after_profile_mirror_insert(transaction.transaction())?;
                }
                if let ApplicationEvent::AgentSkillAssigned { profile, .. }
                | ApplicationEvent::AgentSkillUpgraded { profile, .. }
                | ApplicationEvent::AgentSkillUnassigned { profile, .. } = &committed.event
                {
                    insert_expected_version(
                        transaction.transaction(),
                        i64::try_from(committed.sequence)
                            .map_err(|_| PersistenceError::InvalidAgentProfilePayload)?,
                        profile,
                    )?;
                    self.hook.after_profile_mirror_insert(transaction.transaction())?;
                }
                ProjectionRepository::store_with_after_active_profiles(
                    &transaction,
                    &projection,
                    |transaction| self.hook.after_active_pointer_update(transaction),
                )
                .map_err(map_profile_projection_write_error)?;
                self.hook
                    .after_projection_store(transaction.transaction())?;
                self.hook
                    .before_outcome_materialization(transaction.transaction())?;
                let events = vec![committed];
                let outcome = materialize_success(
                    &transaction,
                    envelope.command_id,
                    &request,
                    &events,
                    &projection,
                    self.binding_catalog.as_ref(),
                )?;
                self.hook.after_audit_append(transaction.transaction())?;
                let stored = StoredExecution::Success { outcome };
                let outcome_json = encode_canonical(&stored)?;
                self.hook.before_receipt_write(transaction.transaction())?;
                insert_receipt(
                    &transaction,
                    &envelope,
                    command_fingerprint.clone(),
                    request_json.clone(),
                    capability,
                    StoredPolicyDecision::Granted,
                    outcome_json,
                    &events,
                )?;
                self.hook.after_receipt_store(transaction.transaction())?;
                self.hook.before_commit(transaction.transaction())?;
                Ok((stored, events))
            })();

        let (stored, _) = match precommit {
            Ok(result) => result,
            Err(error) => {
                if profile_reserved {
                    profile_review_operation
                        .as_ref()
                        .expect("reserved review has an operation owner")
                        .release(envelope.command_id);
                }
                if skill_reserved {
                    skill_review_operation
                        .as_ref()
                        .expect("reserved skill review has an operation owner")
                        .release(envelope.command_id);
                }
                return Err(error);
            }
        };
        if let Err(error) = transaction.commit() {
            if profile_reserved {
                profile_review_operation
                    .as_ref()
                    .expect("reserved review has an operation owner")
                    .release(envelope.command_id);
            }
            if skill_reserved {
                skill_review_operation
                    .as_ref()
                    .expect("reserved skill review has an operation owner")
                    .release(envelope.command_id);
            }
            return Err(error.into());
        }
        if profile_reserved {
            profile_review_operation
                .as_ref()
                .expect("reserved review has an operation owner")
                .consume(envelope.command_id);
        }
        if skill_reserved {
            skill_review_operation
                .as_ref()
                .expect("reserved skill review has an operation owner")
                .consume_reserved(envelope.command_id);
        }
        if matches!(request.command, ApplicationCommand::RequestShutdown) {
            self.reviews.cancel();
            self.skill_reviews.cancel();
        }
        stored.into_result()
    }
}

fn authorize_passive(policy: &dyn CommandPolicy, capability: Capability) -> Result<(), AppError> {
    match policy.authorize(capability) {
        AuthorizationDecision::Granted => Ok(()),
        AuthorizationDecision::Denied(decision) => Err(AppError::CapabilityDenied {
            capability,
            decision,
        }),
        AuthorizationDecision::ApprovalRequired => Err(AppError::ApprovalRequired { capability }),
    }
}

fn validate_draft(draft: &AgentProfileDraft) -> Result<(), AppError> {
    draft.canonicalized()?;
    Ok(())
}

fn map_profile_projection_write_error(error: PersistenceError) -> AppError {
    match error {
        PersistenceError::AgentProfileHistoryMismatch => AppError::AgentProfileHistoryMismatch,
        PersistenceError::DuplicateAgentProfileName => AppError::DuplicateProfileName,
        error => AppError::Persistence(error),
    }
}

fn ensure_name_available(
    projection: &ProjectionState,
    excluded_profile_id: Option<AgentProfileId>,
    display_name: &str,
) -> Result<(), AppError> {
    let normalized = normalize_profile_name_key(display_name)?;
    if projection
        .agent_profiles
        .active_profiles()
        .iter()
        .any(|profile| {
            Some(profile.profile_id()) != excluded_profile_id
                && profile.normalized_name() == &normalized
        })
    {
        Err(AppError::DuplicateProfileName)
    } else {
        Ok(())
    }
}

fn prepare_event(
    command: &ApplicationCommand,
    projection: &ProjectionState,
    clock: &dyn Clock,
    ids: &dyn IdGenerator,
    hook: &dyn CommandTransactionHook,
    transaction: &rusqlite::Transaction<'_>,
    command_id: CommandId,
) -> Result<ApplicationEvent, AppError> {
    match command {
        ApplicationCommand::CreateAgentProfile {
            draft,
            template_provenance,
        } => {
            validate_draft(draft)?;
            if draft.template_provenance() != template_provenance.as_ref() {
                return Err(crate::domain::DomainError::InvalidProfileTemplateProvenance.into());
            }
            if let Some(provenance) = template_provenance {
                profile_template_from_provenance(provenance)?;
            }
            ensure_name_available(projection, None, &draft.display_name)?;
            hook.after_profile_name_precheck(transaction, command_id)?;
            let profile_id = AgentProfileId::from_uuid(ids.next_uuid());
            let created_at_ms = clock.now_millis();
            let profile = AgentProfileVersion::create(
                profile_id,
                AgentProfileVersionId::from_uuid(ids.next_uuid()),
                MemoryNamespaceId::from_uuid(ids.next_uuid()),
                created_at_ms,
                draft.clone(),
                template_provenance.clone(),
            )?;
            Ok(ApplicationEvent::AgentProfileCreated { profile })
        }
        ApplicationCommand::ListAgentProfiles => {
            let total_count = projection.agent_profiles.active_profile_count();
            let returned_count = total_count.min(MAX_AGENT_PROFILE_LIST_RESULTS);
            Ok(ApplicationEvent::AgentProfilesListed {
                total_count: u32::try_from(total_count)
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                returned_count: u32::try_from(returned_count)
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                truncated: returned_count < total_count,
            })
        }
        ApplicationCommand::ShowAgentProfile { selector } => {
            let profile = resolve_active_profile(projection, selector)
                .ok_or(AppError::AgentProfileNotFound)?;
            Ok(ApplicationEvent::AgentProfileViewed {
                profile_id: profile.profile_id(),
                active_version_id: profile.profile_version_id(),
            })
        }
        ApplicationCommand::ShowAgentProfileHistory { selector } => {
            let profile = resolve_active_profile(projection, selector)
                .ok_or(AppError::AgentProfileNotFound)?;
            let total_count = projection
                .agent_profiles
                .history_count(profile.profile_id());
            let returned_count = total_count.min(MAX_AGENT_PROFILE_HISTORY_RESULTS);
            Ok(ApplicationEvent::AgentProfileHistoryViewed {
                profile_id: profile.profile_id(),
                total_count: u32::try_from(total_count)
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                returned_count: u32::try_from(returned_count)
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                truncated: returned_count < total_count,
                active_version_id: profile.profile_version_id(),
            })
        }
        ApplicationCommand::ShowAgentProfileVersion { selector, version } => {
            let active = resolve_active_profile(projection, selector)
                .ok_or(AppError::AgentProfileNotFound)?;
            let profile = projection
                .agent_profiles
                .profile_version(active.profile_id(), *version)
                .ok_or(AppError::AgentProfileNotFound)?;
            Ok(ApplicationEvent::AgentProfileVersionViewed {
                profile_id: profile.profile_id(),
                profile_version_id: profile.profile_version_id(),
                version: profile.version(),
                predecessor_version_id: profile.supersedes(),
            })
        }
        ApplicationCommand::ListSkills => {
            let skills = active_skills(transaction)?;
            let total_count = skills.len();
            let returned = skills.into_iter().take(MAX_SKILL_LIST_RESULTS).collect::<Vec<_>>();
            Ok(ApplicationEvent::SkillsListed {
                skills: returned.iter().map(skill_event_summary).collect(),
                total_count: u32::try_from(total_count)
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                returned_count: u32::try_from(returned.len())
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                truncated: returned.len() < total_count,
            })
        }
        ApplicationCommand::ShowSkill { selector } => {
            let skill = resolve_active_skill(transaction, selector)?;
            Ok(ApplicationEvent::SkillViewed {
                skill: skill.reference(),
                display_name: skill.content().display_name.clone(),
                provenance: skill.provenance().clone(),
            })
        }
        ApplicationCommand::ShowSkillHistory { selector } => {
            let active = resolve_active_skill(transaction, selector)?;
            let history = load_skill_history(transaction, active.skill_id())?;
            let total_count = history.len();
            let versions = history
                .into_iter()
                .rev()
                .take(MAX_SKILL_HISTORY_RESULTS)
                .map(|skill| SkillHistoryEventEntry {
                    skill: skill.reference(),
                    created_at_ms: skill.created_at_ms(),
                    predecessor_version_id: skill.predecessor(),
                })
                .collect::<Vec<_>>();
            Ok(ApplicationEvent::SkillHistoryViewed {
                skill_id: active.skill_id(),
                active: active.reference(),
                versions: versions.clone(),
                total_count: u32::try_from(total_count)
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                returned_count: u32::try_from(versions.len())
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                truncated: versions.len() < total_count,
            })
        }
        ApplicationCommand::ShowSkillVersion { selector, version } => {
            let active = resolve_active_skill(transaction, selector)?;
            let skill = load_skill_history(transaction, active.skill_id())?
                .into_iter()
                .find(|skill| skill.version() == *version)
                .ok_or(AppError::SkillNotFound)?;
            Ok(ApplicationEvent::SkillVersionViewed {
                skill: skill.reference(),
                display_name: skill.content().display_name.clone(),
                provenance: skill.provenance().clone(),
                predecessor_version_id: skill.predecessor(),
            })
        }
        ApplicationCommand::ShowHelp => Ok(ApplicationEvent::HelpViewed),
        ApplicationCommand::ShowStatus => Ok(ApplicationEvent::StatusViewed),
        ApplicationCommand::ShowSetupStatus => Ok(ApplicationEvent::SetupStatusViewed),
        ApplicationCommand::ShowAuditTail { limit } => {
            Ok(ApplicationEvent::AuditTailViewed { limit: *limit })
        }
        ApplicationCommand::RejectInput(rejection) => Ok(ApplicationEvent::CommandRejected {
            rejection: rejection.clone(),
        }),
        ApplicationCommand::RequestShutdown => Ok(ApplicationEvent::ShutdownRequested),
        ApplicationCommand::ActivateAgentProfileVersion { .. }
        | ApplicationCommand::CreateSkill { .. }
        | ApplicationCommand::ActivateSkillVersion { .. }
        | ApplicationCommand::AssignAgentSkill { .. }
        | ApplicationCommand::UpgradeAgentSkill { .. }
        | ApplicationCommand::UnassignAgentSkill { .. } => Err(invalid_receipt()),
    }
}

fn event_occurred_at(event: &ApplicationEvent) -> Option<i64> {
    match event {
        ApplicationEvent::AgentProfileCreated { profile }
        | ApplicationEvent::AgentProfileVersionActivated { profile, .. } => {
            Some(profile.created_at_ms())
        }
        ApplicationEvent::AgentSkillAssigned { profile, .. }
        | ApplicationEvent::AgentSkillUpgraded { profile, .. }
        | ApplicationEvent::AgentSkillUnassigned { profile, .. } => Some(profile.created_at_ms()),
        _ => None,
    }
}

fn ensure_skill_name_available(
    connection: &rusqlite::Connection,
    excluded_skill_id: Option<SkillId>,
    candidate: &SkillDraft,
) -> Result<(), AppError> {
    let normalized = candidate.normalized_name()?;
    match load_active_skill_by_name(connection, &normalized)? {
        Some(existing) if Some(existing.skill_id()) != excluded_skill_id => {
            Err(AppError::DuplicateSkillName)
        }
        _ => Ok(()),
    }
}

fn active_skills(connection: &rusqlite::Connection) -> Result<Vec<SkillVersion>, AppError> {
    let ids = load_all_skill_versions(connection)?
        .into_iter()
        .map(|skill| skill.skill_id())
        .collect::<BTreeSet<_>>();
    let mut active = Vec::with_capacity(ids.len());
    for skill_id in ids {
        if let Some(skill) = load_active_skill(connection, skill_id)? {
            active.push((skill.normalized_name()?.as_str().to_owned(), skill));
        }
    }
    active.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(active.into_iter().map(|(_, skill)| skill).collect())
}

fn resolve_active_skill(
    connection: &rusqlite::Connection,
    selector: &SkillSelector,
) -> Result<SkillVersion, AppError> {
    let skill = match selector {
        SkillSelector::Id(skill_id) => load_active_skill(connection, *skill_id)?,
        SkillSelector::Name(_) => match selector.normalized_name() {
            Some(name) => load_active_skill_by_name(connection, &name)?,
            None => None,
        },
    };
    skill.ok_or(AppError::SkillNotFound)
}

fn skill_event_summary(skill: &SkillVersion) -> SkillEventSummary {
    SkillEventSummary {
        skill: skill.reference(),
        display_name: skill.content().display_name.clone(),
        provenance: skill.provenance().clone(),
    }
}

fn validate_assignment_operation(
    connection: &rusqlite::Connection,
    current: &AgentProfileVersion,
    operation: &AgentSkillAssignmentOperation,
) -> Result<AgentProfileDraft, AppError> {
    match operation {
        AgentSkillAssignmentOperation::Assign { skill } => {
            load_skill_version(connection, skill)?.ok_or(AppError::SkillNotFound)?;
            if current
                .skill_refs()
                .iter()
                .any(|assigned| assigned.skill_id() == skill.skill_id())
            {
                return Err(AppError::SkillAlreadyAssigned);
            }
            if current.skill_refs().len() >= 16 {
                return Err(AppError::AgentSkillLimitExceeded);
            }
            current.assign_skill(skill.clone()).map_err(AppError::from)
        }
        AgentSkillAssignmentOperation::Upgrade { expected, replacement } => {
            load_skill_version(connection, expected)?.ok_or(AppError::SkillNotFound)?;
            load_skill_version(connection, replacement)?.ok_or(AppError::SkillNotFound)?;
            if !current.skill_refs().contains(expected) {
                return Err(AppError::SkillNotAssigned);
            }
            current
                .upgrade_skill(expected.clone(), replacement.clone())
                .map_err(AppError::from)
        }
        AgentSkillAssignmentOperation::Unassign { expected } => {
            load_skill_version(connection, expected)?.ok_or(AppError::SkillNotFound)?;
            if !current.skill_refs().contains(expected) {
                return Err(AppError::SkillNotAssigned);
            }
            current.unassign_skill(expected.clone()).map_err(AppError::from)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn prepare_agent_skill_event(
    projection: &ProjectionState,
    connection: &rusqlite::Connection,
    ids: &dyn IdGenerator,
    clock: &dyn Clock,
    profile_id: AgentProfileId,
    expected_active_profile_version_id: AgentProfileVersionId,
    operation: AgentSkillAssignmentOperation,
) -> Result<ApplicationEvent, AppError> {
    let current = projection
        .agent_profiles
        .active_profile(profile_id)
        .ok_or(AppError::AgentProfileNotFound)?;
    if current.profile_version_id() != expected_active_profile_version_id {
        return Err(AppError::StaleAgentProfileVersion);
    }
    let candidate = validate_assignment_operation(connection, current, &operation)?;
    let profile = AgentProfileVersion::next_version(
        current,
        AgentProfileVersionId::from_uuid(ids.next_uuid()),
        clock.now_millis(),
        candidate,
    )?;
    Ok(match operation {
        AgentSkillAssignmentOperation::Assign { skill } => ApplicationEvent::AgentSkillAssigned {
            profile,
            previous_profile_version_id: expected_active_profile_version_id,
            skill,
        },
        AgentSkillAssignmentOperation::Upgrade { expected, replacement } => {
            ApplicationEvent::AgentSkillUpgraded {
                profile,
                previous_profile_version_id: expected_active_profile_version_id,
                expected,
                replacement,
            }
        }
        AgentSkillAssignmentOperation::Unassign { expected } => {
            ApplicationEvent::AgentSkillUnassigned {
                profile,
                previous_profile_version_id: expected_active_profile_version_id,
                expected,
            }
        }
    })
}

fn map_skill_write_error(error: PersistenceError) -> AppError {
    match error {
        PersistenceError::DuplicateSkillName => AppError::DuplicateSkillName,
        error => AppError::Persistence(error),
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_receipt(
    transaction: &ImmediateTransaction<'_>,
    envelope: &CommandEnvelope,
    command_fingerprint: Sha256Digest,
    request_json: String,
    capability: Capability,
    policy_decision: StoredPolicyDecision,
    outcome_json: String,
    events: &[crate::app::EventEnvelope],
) -> Result<(), AppError> {
    CommandReceiptRepository::insert(
        transaction,
        &CommandReceiptRecord {
            command_id: envelope.command_id,
            command_fingerprint,
            request_json,
            capability: capability_name(capability).to_owned(),
            policy_decision: policy_decision.as_str().to_owned(),
            outcome_json,
            event_ids: events.iter().map(|event| event.event_id).collect(),
        },
    )?;
    Ok(())
}

fn ensure_authoritative_session_open(
    transaction: &ImmediateTransaction<'_>,
    session_id: SessionId,
) -> Result<(), AppError> {
    let is_open = transaction
        .transaction()
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM process_session_projection
                WHERE session_id = ?1 AND ended_event_id IS NULL
            )",
            [session_id.to_string()],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| PersistenceError::QueryFailed)?;
    if is_open {
        Ok(())
    } else {
        Err(AppError::LifecycleFinished)
    }
}

fn validate_receipt(
    transaction: &ImmediateTransaction<'_>,
    receipt: &CommandReceiptRecord,
    request: &CommandRequest,
    request_json: &str,
    command_fingerprint: &Sha256Digest,
) -> Result<StoredExecution, AppError> {
    let stored_request: CommandRequest = decode_canonical(&receipt.request_json)?;
    if receipt.command_fingerprint != sha256(receipt.request_json.as_bytes()) {
        return Err(invalid_receipt());
    }
    if &stored_request != request
        || receipt.request_json != request_json
        || &receipt.command_fingerprint != command_fingerprint
    {
        return Err(AppError::CommandConflict);
    }
    let capability = parse_capability(&receipt.capability)?;
    if capability != stored_request.command.required_capability() {
        return Err(invalid_receipt());
    }
    let policy_decision = StoredPolicyDecision::parse(&receipt.policy_decision)?;
    let stored: StoredExecution = decode_canonical(&receipt.outcome_json)?;
    match (&stored, policy_decision) {
        (StoredExecution::Success { outcome }, StoredPolicyDecision::Granted) => {
            let events = receipt
                .event_ids
                .iter()
                .map(|event_id| {
                    EventRepository::load_by_event_id(transaction, *event_id)?
                        .ok_or_else(invalid_receipt)
                })
                .collect::<Result<Vec<_>, AppError>>()?;
            let last = events.last().ok_or_else(invalid_receipt)?;
            let projection =
                ProjectionRepository::load_at(transaction.transaction(), last.sequence)?;
            let validation_catalog = AgentBindingCatalogSnapshot::default();
            let expected = materialize_success(
                transaction,
                receipt.command_id,
                &stored_request,
                &events,
                &projection,
                &validation_catalog,
            )?;
            if !receipt_outcomes_match(outcome, &expected) {
                return Err(invalid_receipt());
            }
        }
        (
            StoredExecution::CapabilityDenied {
                capability: outcome_capability,
                decision,
            },
            StoredPolicyDecision::Denied,
        ) if receipt.event_ids.is_empty()
            && *outcome_capability == capability
            && *decision == PolicyDecision::Denied => {}
        (
            StoredExecution::CapabilityDenied {
                capability: outcome_capability,
                decision,
            },
            StoredPolicyDecision::DeniedByDefault,
        ) if receipt.event_ids.is_empty()
            && *outcome_capability == capability
            && *decision == PolicyDecision::DeniedByDefault => {}
        (
            StoredExecution::ApprovalRequired {
                capability: outcome_capability,
            },
            StoredPolicyDecision::ApprovalRequired,
        ) if receipt.event_ids.is_empty() && *outcome_capability == capability => {}
        _ => return Err(invalid_receipt()),
    }
    Ok(stored)
}

fn receipt_outcomes_match(stored: &CommandOutcome, expected: &CommandOutcome) -> bool {
    let mut stored = stored.clone();
    let mut expected = expected.clone();
    normalize_catalog_readiness(&mut stored.view);
    normalize_catalog_readiness(&mut expected.view);
    stored == expected
}

fn normalize_catalog_readiness(view: &mut CommandView) {
    match view {
        CommandView::AgentProfileCreated(created) => created.readiness = AgentReadiness::Unbound,
        CommandView::AgentProfileVersionActivated(activated) => {
            activated.readiness = AgentReadiness::Unbound;
        }
        CommandView::AgentProfiles(profiles) => {
            for profile in &mut profiles.profiles {
                profile.readiness = AgentReadiness::Unbound;
            }
        }
        CommandView::AgentProfile(profile) => profile.readiness = AgentReadiness::Unbound,
        CommandView::AgentProfileHistory(history) => {
            for version in &mut history.versions {
                version.readiness = AgentReadiness::Unbound;
            }
        }
        CommandView::AgentProfileVersion(version) => version.readiness = AgentReadiness::Unbound,
        CommandView::Help(_)
        | CommandView::Status(_)
        | CommandView::SetupStatus(_)
        | CommandView::AuditTail(_)
        | CommandView::SkillCreated(_)
        | CommandView::SkillVersionActivated(_)
        | CommandView::Skills(_)
        | CommandView::Skill(_)
        | CommandView::SkillHistory(_)
        | CommandView::SkillVersion(_)
        | CommandView::AgentSkillAssigned(_)
        | CommandView::AgentSkillUpgraded(_)
        | CommandView::AgentSkillUnassigned(_)
        | CommandView::InputRejected(_)
        | CommandView::Shutdown(_) => {}
    }
}

fn materialize_success(
    transaction: &ImmediateTransaction<'_>,
    command_id: CommandId,
    request: &CommandRequest,
    events: &[crate::app::EventEnvelope],
    projection: &ProjectionState,
    binding_catalog: &AgentBindingCatalogSnapshot,
) -> Result<CommandOutcome, AppError> {
    let [event] = events else {
        return Err(invalid_receipt());
    };
    if event.actor != request.actor
        || event.correlation_id != request.correlation_id
        || event.causation_id != Some(CausationId::from_uuid(command_id.as_uuid()))
        || event.object.is_some()
    {
        return Err(invalid_receipt());
    }
    let (view, shutdown) = match (&request.command, &event.event) {
        (ApplicationCommand::ShowHelp, ApplicationEvent::HelpViewed) => {
            (CommandView::Help(HelpView), ShutdownDisposition::Continue)
        }
        (ApplicationCommand::ShowStatus, ApplicationEvent::StatusViewed) => {
            let installation_id = projection
                .installation
                .as_ref()
                .ok_or(RecoveryError::InvalidEventRecord)?
                .installation_id;
            let session_id = projection
                .sessions
                .values()
                .find(|session| session.ended.is_none())
                .ok_or(RecoveryError::InvalidEventRecord)?
                .session_id;
            (
                CommandView::Status(StatusView {
                    installation_id,
                    session_id,
                }),
                ShutdownDisposition::Continue,
            )
        }
        (ApplicationCommand::ShowSetupStatus, ApplicationEvent::SetupStatusViewed) => (
            CommandView::SetupStatus(SetupStatusView {
                status: projection.setup_status.clone(),
            }),
            ShutdownDisposition::Continue,
        ),
        (
            ApplicationCommand::ShowAuditTail { limit: requested },
            ApplicationEvent::AuditTailViewed { limit: committed },
        ) if requested == committed => {
            let entries = EventRepository::tail_through(
                transaction.transaction(),
                *requested,
                event.sequence,
            )?
            .iter()
            .map(AuditEntry::from_event)
            .collect();
            (
                CommandView::AuditTail(AuditTailView {
                    limit: *requested,
                    entries,
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::RejectInput(requested),
            ApplicationEvent::CommandRejected {
                rejection: committed,
            },
        ) if requested == committed => (
            CommandView::InputRejected(InputRejectedView {
                rejection: committed.clone(),
            }),
            ShutdownDisposition::Continue,
        ),
        (ApplicationCommand::RequestShutdown, ApplicationEvent::ShutdownRequested) => (
            CommandView::Shutdown(ShutdownView {
                disposition: ShutdownDisposition::Requested,
            }),
            ShutdownDisposition::Requested,
        ),
        (
            ApplicationCommand::CreateAgentProfile {
                draft,
                template_provenance,
            },
            ApplicationEvent::AgentProfileCreated { profile },
        ) => {
            validate_draft(draft)?;
            let expected = AgentProfileVersion::create(
                profile.profile_id(),
                profile.profile_version_id(),
                profile.memory_namespace_id(),
                profile.created_at_ms(),
                draft.clone(),
                template_provenance.clone(),
            )?;
            if &expected != profile
                || projection
                    .agent_profiles
                    .active_profile(profile.profile_id())
                    != Some(profile)
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::AgentProfileCreated(AgentProfileCreatedView {
                    profile_id: profile.profile_id(),
                    profile_version_id: profile.profile_version_id(),
                    version: profile.version(),
                    readiness: profile_readiness(profile, binding_catalog),
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ActivateAgentProfileVersion {
                profile_id,
                expected_active_version_id,
                candidate,
                review_digest: supplied_review_digest,
                ..
            },
            ApplicationEvent::AgentProfileVersionActivated {
                profile,
                previous_version_id,
            },
        ) => {
            let previous = projection
                .agent_profiles
                .version(*expected_active_version_id)
                .ok_or_else(invalid_receipt)?;
            let diffs = diff_profile(previous, candidate).map_err(|_| invalid_receipt())?;
            let digest = candidate_digest(candidate).map_err(|_| invalid_receipt())?;
            if review_digest(*profile_id, *expected_active_version_id, &digest, &diffs)
                .map_err(|_| invalid_receipt())?
                != *supplied_review_digest
                || previous_version_id != expected_active_version_id
                || profile.profile_id() != *profile_id
            {
                return Err(invalid_receipt());
            }
            let expected = AgentProfileVersion::next_version(
                previous,
                profile.profile_version_id(),
                profile.created_at_ms(),
                candidate.clone(),
            )?;
            if &expected != profile
                || projection.agent_profiles.active_profile(*profile_id) != Some(profile)
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::AgentProfileVersionActivated(AgentProfileVersionActivatedView {
                    profile_id: *profile_id,
                    profile_version_id: profile.profile_version_id(),
                    previous_version_id: *previous_version_id,
                    version: profile.version(),
                    readiness: profile_readiness(profile, binding_catalog),
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ListAgentProfiles,
            ApplicationEvent::AgentProfilesListed {
                total_count,
                returned_count,
                truncated,
            },
        ) => {
            let actual_total = projection.agent_profiles.active_profile_count();
            let profiles = projection
                .agent_profiles
                .active_profiles_bounded(MAX_AGENT_PROFILE_LIST_RESULTS);
            if usize::try_from(*total_count).ok() != Some(actual_total)
                || usize::try_from(*returned_count).ok() != Some(profiles.len())
                || *truncated != (profiles.len() < actual_total)
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::AgentProfiles(AgentProfilesView {
                    profiles: profiles
                        .iter()
                        .map(|profile| profile_summary(profile, binding_catalog))
                        .collect(),
                    total_count: *total_count,
                    returned_count: *returned_count,
                    truncated: *truncated,
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ShowAgentProfile { selector },
            ApplicationEvent::AgentProfileViewed {
                profile_id: committed,
                active_version_id,
            },
        ) => {
            let profile =
                resolve_active_profile(projection, selector).ok_or_else(invalid_receipt)?;
            if profile.profile_id() != *committed
                || profile.profile_version_id() != *active_version_id
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::AgentProfile(AgentProfileView {
                    profile: profile.clone(),
                    readiness: profile_readiness(profile, binding_catalog),
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ShowAgentProfileHistory { selector },
            ApplicationEvent::AgentProfileHistoryViewed {
                profile_id: committed,
                total_count,
                returned_count,
                truncated,
                active_version_id,
            },
        ) => {
            let active =
                resolve_active_profile(projection, selector).ok_or_else(invalid_receipt)?;
            let versions = projection
                .agent_profiles
                .history_bounded_desc(active.profile_id(), MAX_AGENT_PROFILE_HISTORY_RESULTS)
                .into_iter()
                .map(|profile| AgentProfileHistoryEntry {
                    profile_version_id: profile.profile_version_id(),
                    version: profile.version(),
                    supersedes: profile.supersedes(),
                    created_at_ms: profile.created_at_ms(),
                    readiness: profile_readiness(&profile, binding_catalog),
                    content_digest: profile.content_digest().clone(),
                })
                .collect::<Vec<_>>();
            let actual_total = projection.agent_profiles.history_count(active.profile_id());
            if active.profile_id() != *committed
                || active.profile_version_id() != *active_version_id
                || usize::try_from(*total_count).ok() != Some(actual_total)
                || usize::try_from(*returned_count).ok() != Some(versions.len())
                || *truncated != (versions.len() < actual_total)
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::AgentProfileHistory(AgentProfileHistoryView {
                    profile_id: *committed,
                    active_version_id: *active_version_id,
                    versions,
                    total_count: *total_count,
                    returned_count: *returned_count,
                    truncated: *truncated,
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ShowAgentProfileVersion { selector, version },
            ApplicationEvent::AgentProfileVersionViewed {
                profile_id,
                profile_version_id,
                version: committed_version,
                predecessor_version_id,
            },
        ) => {
            let active =
                resolve_active_profile(projection, selector).ok_or_else(invalid_receipt)?;
            let profile = projection
                .agent_profiles
                .profile_version(active.profile_id(), *version)
                .ok_or_else(invalid_receipt)?;
            if profile.profile_id() != *profile_id
                || profile.profile_version_id() != *profile_version_id
                || profile.version() != *committed_version
                || profile.supersedes() != *predecessor_version_id
            {
                return Err(invalid_receipt());
            }
            let predecessor_diff = match predecessor_version_id {
                Some(predecessor_id) => {
                    let predecessor = projection
                        .agent_profiles
                        .version(*predecessor_id)
                        .ok_or_else(invalid_receipt)?;
                    diff_profile(predecessor, &profile.to_draft()).map_err(|_| invalid_receipt())?
                }
                None => Vec::new(),
            };
            (
                CommandView::AgentProfileVersion(AgentProfileVersionView {
                    profile: profile.clone(),
                    readiness: profile_readiness(profile, binding_catalog),
                    predecessor_diff,
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::CreateSkill {
                skill_id, candidate, ..
            },
            ApplicationEvent::SkillCreated {
                skill,
                display_name,
                provenance,
            },
        ) => {
            let accepted = load_skill_version(transaction.transaction(), skill)?
                .ok_or_else(invalid_receipt)?;
            if accepted.skill_id() != *skill_id
                || accepted.version() != ObjectVersion::new(1)?
                || accepted.predecessor().is_some()
                || accepted.content() != candidate
                || accepted.content().display_name != *display_name
                || accepted.provenance() != provenance
                || provenance != &SkillProvenance::User
                || load_active_skill(transaction.transaction(), *skill_id)?.as_ref()
                    != Some(&accepted)
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::SkillCreated(SkillCreatedView {
                    skill_id: accepted.skill_id(),
                    skill_version_id: accepted.skill_version_id(),
                    version: accepted.version(),
                    content_digest: accepted.content_digest().clone(),
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ActivateSkillVersion {
                skill_id,
                expected_active_version_id,
                candidate,
                ..
            },
            ApplicationEvent::SkillVersionActivated {
                skill,
                previous_version_id,
                display_name,
                provenance,
            },
        ) => {
            let accepted = load_skill_version(transaction.transaction(), skill)?
                .ok_or_else(invalid_receipt)?;
            let previous = load_skill_history(transaction.transaction(), *skill_id)?
                .into_iter()
                .find(|version| version.skill_version_id() == *expected_active_version_id)
                .ok_or_else(invalid_receipt)?;
            let expected = SkillVersion::next_version(
                &previous,
                accepted.skill_version_id(),
                accepted.created_at_ms(),
                candidate.clone(),
            )?;
            if previous_version_id != expected_active_version_id
                || accepted.skill_id() != *skill_id
                || accepted != expected
                || accepted.content().display_name != *display_name
                || accepted.provenance() != provenance
                || load_active_skill(transaction.transaction(), *skill_id)?.as_ref()
                    != Some(&accepted)
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::SkillVersionActivated(SkillVersionActivatedView {
                    skill_id: accepted.skill_id(),
                    skill_version_id: accepted.skill_version_id(),
                    previous_version_id: *previous_version_id,
                    version: accepted.version(),
                    content_digest: accepted.content_digest().clone(),
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ListSkills,
            ApplicationEvent::SkillsListed {
                skills,
                total_count,
                returned_count,
                truncated,
            },
        ) => {
            if usize::try_from(*returned_count).ok() != Some(skills.len())
                || *returned_count > *total_count
                || *truncated != (*returned_count < *total_count)
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::Skills(SkillsView {
                    skills: skills
                        .iter()
                        .map(|skill| SkillSummary {
                            skill_ref: skill.skill.clone(),
                            display_name: skill.display_name.clone(),
                            provenance: skill.provenance.clone(),
                        })
                        .collect(),
                    total_count: *total_count,
                    returned_count: *returned_count,
                    truncated: *truncated,
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ShowSkill { selector },
            ApplicationEvent::SkillViewed {
                skill,
                display_name,
                provenance,
            },
        ) => {
            let active = resolve_active_skill(transaction.transaction(), selector)?;
            let accepted = load_skill_version(transaction.transaction(), skill)?
                .ok_or_else(invalid_receipt)?;
            if active.reference() != *skill
                || accepted.content().display_name != *display_name
                || accepted.provenance() != provenance
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::Skill(SkillView {
                    skill_ref: skill.clone(),
                    content: accepted.content().clone(),
                    created_at_ms: accepted.created_at_ms(),
                    provenance: provenance.clone(),
                    predecessor_version_id: accepted.predecessor(),
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ShowSkillVersion { selector, version },
            ApplicationEvent::SkillVersionViewed {
                skill,
                display_name,
                provenance,
                predecessor_version_id,
            },
        ) => {
            let active = resolve_active_skill(transaction.transaction(), selector)?;
            let accepted = load_skill_version(transaction.transaction(), skill)?
                .ok_or_else(invalid_receipt)?;
            if active.skill_id() != accepted.skill_id()
                || accepted.version() != *version
                || accepted.predecessor() != *predecessor_version_id
                || accepted.content().display_name != *display_name
                || accepted.provenance() != provenance
            {
                return Err(invalid_receipt());
            }
            let view = SkillView {
                skill_ref: skill.clone(),
                content: accepted.content().clone(),
                created_at_ms: accepted.created_at_ms(),
                provenance: provenance.clone(),
                predecessor_version_id: accepted.predecessor(),
            };
            (CommandView::SkillVersion(view), ShutdownDisposition::Continue)
        }
        (
            ApplicationCommand::ShowSkillHistory { selector },
            ApplicationEvent::SkillHistoryViewed {
                skill_id,
                active,
                versions,
                total_count,
                returned_count,
                truncated,
            },
        ) => {
            let selected = resolve_active_skill(transaction.transaction(), selector)?;
            if selected.reference() != *active
                || active.skill_id() != *skill_id
                || usize::try_from(*returned_count).ok() != Some(versions.len())
                || *returned_count > *total_count
                || *truncated != (*returned_count < *total_count)
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::SkillHistory(SkillHistoryView {
                    skill_id: *skill_id,
                    active_version_id: active.skill_version_id(),
                    versions: versions
                        .iter()
                        .map(|version| SkillHistoryEntry {
                            skill_ref: version.skill.clone(),
                            created_at_ms: version.created_at_ms,
                            predecessor_version_id: version.predecessor_version_id,
                        })
                        .collect(),
                    total_count: *total_count,
                    returned_count: *returned_count,
                    truncated: *truncated,
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::AssignAgentSkill {
                profile_id,
                expected_active_profile_version_id,
                skill,
                ..
            },
            ApplicationEvent::AgentSkillAssigned {
                profile,
                previous_profile_version_id,
                skill: committed_skill,
            },
        ) if skill == committed_skill
            && expected_active_profile_version_id == previous_profile_version_id => materialize_agent_skill_mutation(
            projection,
            *profile_id,
            *previous_profile_version_id,
            profile,
            CommandView::AgentSkillAssigned,
        )?,
        (
            ApplicationCommand::UpgradeAgentSkill {
                profile_id,
                expected_active_profile_version_id,
                expected,
                replacement,
                ..
            },
            ApplicationEvent::AgentSkillUpgraded {
                profile,
                previous_profile_version_id,
                expected: committed_expected,
                replacement: committed_replacement,
            },
        ) if expected == committed_expected
            && replacement == committed_replacement
            && expected_active_profile_version_id == previous_profile_version_id => {
            materialize_agent_skill_mutation(
                projection,
                *profile_id,
                *previous_profile_version_id,
                profile,
                CommandView::AgentSkillUpgraded,
            )?
        }
        (
            ApplicationCommand::UnassignAgentSkill {
                profile_id,
                expected_active_profile_version_id,
                expected,
                ..
            },
            ApplicationEvent::AgentSkillUnassigned {
                profile,
                previous_profile_version_id,
                expected: committed_expected,
            },
        ) if expected == committed_expected
            && expected_active_profile_version_id == previous_profile_version_id => materialize_agent_skill_mutation(
            projection,
            *profile_id,
            *previous_profile_version_id,
            profile,
            CommandView::AgentSkillUnassigned,
        )?,
        _ => return Err(invalid_receipt()),
    };
    Ok(CommandOutcome {
        command_id,
        correlation_id: request.correlation_id,
        committed_events: events.to_vec(),
        view,
        shutdown,
    })
}

fn materialize_agent_skill_mutation(
    projection: &ProjectionState,
    profile_id: AgentProfileId,
    previous_profile_version_id: AgentProfileVersionId,
    profile: &AgentProfileVersion,
    wrap: fn(AgentSkillMutationView) -> CommandView,
) -> Result<(CommandView, ShutdownDisposition), AppError> {
    if profile.profile_id() != profile_id
        || profile.supersedes() != Some(previous_profile_version_id)
        || projection.agent_profiles.active_profile(profile_id) != Some(profile)
    {
        return Err(invalid_receipt());
    }
    Ok((
        wrap(AgentSkillMutationView {
            profile_id,
            profile_version_id: profile.profile_version_id(),
            previous_profile_version_id,
            version: profile.version(),
            profile_content_digest: profile.content_digest().clone(),
        }),
        ShutdownDisposition::Continue,
    ))
}

fn profile_readiness(
    profile: &AgentProfileVersion,
    binding_catalog: &AgentBindingCatalogSnapshot,
) -> AgentReadiness {
    profile.readiness_with_catalog(binding_catalog)
}

fn resolve_active_profile<'a>(
    projection: &'a ProjectionState,
    selector: &AgentProfileSelector,
) -> Option<&'a AgentProfileVersion> {
    match selector {
        AgentProfileSelector::Id(profile_id) => {
            projection.agent_profiles.active_profile(*profile_id)
        }
        AgentProfileSelector::Name(_) => selector
            .normalized_name()
            .as_ref()
            .and_then(|name| projection.agent_profiles.active_profile_by_name(name)),
    }
}

fn profile_view(
    profile: &AgentProfileVersion,
    binding_catalog: &AgentBindingCatalogSnapshot,
) -> AgentProfileView {
    AgentProfileView {
        profile: profile.clone(),
        readiness: profile_readiness(profile, binding_catalog),
    }
}

fn profile_history_view(
    projection: &ProjectionState,
    profile_id: AgentProfileId,
    binding_catalog: &AgentBindingCatalogSnapshot,
) -> Option<AgentProfileHistoryView> {
    let active = projection.agent_profiles.active_profile(profile_id)?;
    let versions: Vec<AgentProfileHistoryEntry> = projection
        .agent_profiles
        .history_bounded_desc(profile_id, MAX_AGENT_PROFILE_HISTORY_RESULTS)
        .into_iter()
        .map(|profile| AgentProfileHistoryEntry {
            profile_version_id: profile.profile_version_id(),
            version: profile.version(),
            supersedes: profile.supersedes(),
            created_at_ms: profile.created_at_ms(),
            readiness: profile_readiness(&profile, binding_catalog),
            content_digest: profile.content_digest().clone(),
        })
        .collect();
    let total_count = projection.agent_profiles.history_count(profile_id);
    let returned_count = versions.len();
    Some(AgentProfileHistoryView {
        profile_id,
        active_version_id: active.profile_version_id(),
        versions,
        total_count: u32::try_from(total_count).ok()?,
        returned_count: u32::try_from(returned_count).ok()?,
        truncated: returned_count < total_count,
    })
}

fn profile_summary(
    profile: &AgentProfileVersion,
    binding_catalog: &AgentBindingCatalogSnapshot,
) -> AgentProfileSummary {
    AgentProfileSummary {
        profile_id: profile.profile_id(),
        profile_version_id: profile.profile_version_id(),
        version: profile.version(),
        display_name: profile.display_name().to_owned(),
        role: profile.role(),
        primary_specialty: profile.primary_specialty().to_owned(),
        readiness: profile_readiness(profile, binding_catalog),
        content_digest: profile.content_digest().clone(),
    }
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<String, AppError> {
    String::from_utf8(
        canonical_json_bytes(value).map_err(|_| PersistenceError::InvalidEventRecord)?,
    )
    .map_err(|_| invalid_receipt())
}

fn decode_canonical<T>(json: &str) -> Result<T, AppError>
where
    T: DeserializeOwned,
{
    let value: serde_json::Value = serde_json::from_str(json).map_err(|_| invalid_receipt())?;
    let canonical = String::from_utf8(
        canonical_json_bytes(&value).map_err(|_| PersistenceError::InvalidEventRecord)?,
    )
    .map_err(|_| invalid_receipt())?;
    if canonical != json {
        return Err(invalid_receipt());
    }
    serde_json::from_value(value).map_err(|_| invalid_receipt())
}

fn capability_name(capability: Capability) -> &'static str {
    match capability {
        Capability::HelpRead => "help_read",
        Capability::StatusRead => "status_read",
        Capability::SetupStatusRead => "setup_status_read",
        Capability::AuditRead => "audit_read",
        Capability::AgentProfileRead => "agent_profile_read",
        Capability::AgentProfileCreate => "agent_profile_create",
        Capability::AgentProfilePreview => "agent_profile_preview",
        Capability::AgentProfileActivate => "agent_profile_activate",
        Capability::SkillRead => "skill_read",
        Capability::SkillCreate => "skill_create",
        Capability::SkillVersion => "skill_version",
        Capability::AgentSkillAssign => "skill_assign",
        Capability::AgentSkillUnassign => "skill_unassign",
        Capability::Shutdown => "shutdown",
        Capability::DiscussionRun => "discussion_run",
        Capability::McpUse => "mcp_use",
        Capability::EngineeringJobRun => "engineering_job_run",
        Capability::GitMerge => "git_merge",
        Capability::GitPush => "git_push",
        Capability::FinanceRecommendation => "finance_recommendation",
    }
}

fn parse_capability(value: &str) -> Result<Capability, AppError> {
    match value {
        "help_read" => Ok(Capability::HelpRead),
        "status_read" => Ok(Capability::StatusRead),
        "setup_status_read" => Ok(Capability::SetupStatusRead),
        "audit_read" => Ok(Capability::AuditRead),
        "agent_profile_read" => Ok(Capability::AgentProfileRead),
        "agent_profile_create" => Ok(Capability::AgentProfileCreate),
        "agent_profile_preview" => Ok(Capability::AgentProfilePreview),
        "agent_profile_activate" => Ok(Capability::AgentProfileActivate),
        "skill_read" => Ok(Capability::SkillRead),
        "skill_create" => Ok(Capability::SkillCreate),
        "skill_version" => Ok(Capability::SkillVersion),
        "skill_assign" => Ok(Capability::AgentSkillAssign),
        "skill_unassign" => Ok(Capability::AgentSkillUnassign),
        "shutdown" => Ok(Capability::Shutdown),
        "discussion_run" => Ok(Capability::DiscussionRun),
        "mcp_use" => Ok(Capability::McpUse),
        "engineering_job_run" => Ok(Capability::EngineeringJobRun),
        "git_merge" => Ok(Capability::GitMerge),
        "git_push" => Ok(Capability::GitPush),
        "finance_recommendation" => Ok(Capability::FinanceRecommendation),
        _ => Err(invalid_receipt()),
    }
}

fn invalid_receipt() -> AppError {
    AppError::Persistence(PersistenceError::InvalidEventRecord)
}
