use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock, RwLockReadGuard},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    agents::{
        AgentBindingCatalogSnapshot, AgentProfileDraft, AgentProfileVersion,
        AgentProfilesProjection, AgentReadiness, ProfileEditPreview, ProfileReviewRegistry,
        ProfileTemplate, ReviewReservationError, builtin_profile_templates, candidate_digest,
        diff_profile, normalize_profile_name_key, profile_template_from_provenance, review_digest,
    },
    app::{
        AgentProfileCreatedView, AgentProfileHistoryEntry, AgentProfileHistoryView,
        AgentProfileSelector, AgentProfileSummary, AgentProfileVersionActivatedView,
        AgentProfileVersionView, AgentProfileView, AgentProfilesView,
        AgentSkillAssignmentOperation, AgentSkillAssignmentPreview, AgentSkillMutationView,
        AppError, ApplicationCommand, ApplicationEvent, AuditTailView, CommandEnvelope,
        CommandOutcome, CommandView, EVENT_SCHEMA_VERSION, EpisodicSummariesView,
        EpisodicSummaryListItem, EpisodicSummaryView, HelpView, InputRejectedView,
        MAX_AGENT_PROFILE_HISTORY_RESULTS, MAX_AGENT_PROFILE_LIST_RESULTS,
        MAX_SKILL_HISTORY_RESULTS, MAX_SKILL_LIST_RESULTS, MemoryEntriesView,
        MemoryEntryHistorySummary, MemoryEntryHistoryView, MemoryEntryMutationView,
        MemoryEntrySummary, MemoryEntryVersionView, MemoryEntryView, MemoryProfileIdentityView,
        MemoryProposalCreatedView, MemoryProposalResolutionView, MemoryProposalStatusRef,
        MemoryProposalSummary, MemoryProposalView, MemoryProposalsView, MemorySnapshotView,
        PendingEvent, SetupStatusView, ShutdownDisposition, ShutdownReason, ShutdownView,
        SkillCreatedView, SkillEventSummary, SkillHistoryEntry, SkillHistoryEventEntry,
        SkillHistoryView, SkillSelector, SkillSummary, SkillVersionActivatedView, SkillView,
        SkillsView, StatusView,
    },
    audit::AuditEntry,
    config::{AppPaths, StartupError},
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CausationId, Clock, CommandId,
        CorrelationId, Digest, EventId, IdGenerator, InstallationId, MemoryEntryId,
        MemoryEntryVersionId, MemoryNamespaceId, MemoryProposalId, MemoryReviewToken, ObjectRef,
        ObjectVersion, ProfileReviewToken, SessionId, Sha256Digest, SkillId, SkillReviewToken,
        SkillVersionId, canonical_json_bytes, sha256,
    },
    memory::{
        EpisodicContextItem, EpisodicQualification, ExpectedMemoryEntryState,
        MemoryEditPreviewOutcome, MemoryEditReview, MemoryEditReviewBinding, MemoryEntryDraft,
        MemoryEntryState, MemoryEntryVersion, MemoryKvContextItem, MemoryMutationKind,
        MemoryNoChange, MemoryPlaintextAcknowledgement, MemoryProposal, MemoryProposalOperation,
        MemoryProposalOperationKind, MemoryProposalRef, MemoryProposalResolution,
        MemoryProposalStatus, MemoryResolutionAction, MemoryResolutionReviewBinding,
        MemoryRetrievalRequest, MemoryReviewRegistry, MemorySnapshot, NormalizedMemoryKey,
        ReservedMemoryReview, prepare_direct_memory_edit, prepare_memory_resolution_review,
    },
    persistence::{
        CommandReceiptRecord, CommandReceiptRepository, Database, EventRepository,
        ImmediateTransaction, MemoryRepository, PersistenceError, ProjectionRepository,
        RecoveryError, insert_expected_version, insert_skill_version, load_active_skill,
        load_active_skill_by_name, load_all_skill_versions, load_skill_history, load_skill_version,
        set_active_skill,
    },
    policy::{
        ApprovalAction, ApprovalRecord, ApprovalStatus, Capability, Effect, PolicyDecision,
        PolicyRule, evaluate,
    },
    recovery::{BootstrapState, ProjectionState, RecoveryCoordinator, reduce},
    setup::SetupStatus,
    skills::{
        SkillDraft, SkillEditPreview, SkillProvenance, SkillReviewRegistry, SkillVersion,
        SkillVersionRef,
    },
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
    rules: [PolicyRule; 19],
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
                PolicyRule::new(Effect::Grant, Capability::MemoryRead),
                PolicyRule::new(Effect::Grant, Capability::MemoryPreview),
                PolicyRule::new(Effect::Grant, Capability::MemoryMutate),
                PolicyRule::new(Effect::Grant, Capability::MemoryPropose),
                PolicyRule::new(Effect::Grant, Capability::MemoryResolve),
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

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)] // Contract variants intentionally retain exact typed payloads.
pub enum MemoryEditPreview {
    NoChange(MemoryNoChange),
    Review(MemoryEditReview),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryProposalResolutionReview {
    pub action: MemoryResolutionAction,
    pub proposal: MemoryProposal,
    pub approval_id: ApprovalId,
    pub expected_approval_status: ApprovalStatus,
    pub expected_entry: ExpectedMemoryEntryState,
    pub proposer_is_historical: bool,
    pub proposer_identity: MemoryProfileIdentityView,
    pub namespace_owner_identity: MemoryProfileIdentityView,
    pub plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
    pub review_token: MemoryReviewToken,
    pub review_digest: Digest,
}

pub struct ApplicationWorker {
    executor: CommandExecutor,
}

struct CommandExecutor {
    paths: AppPaths,
    database: Database,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    policy: Arc<dyn CommandPolicy>,
    hook: Arc<dyn CommandTransactionHook>,
    lifecycle: Arc<SharedLifecycle>,
    reviews: Arc<ProfileReviewRegistry>,
    skill_reviews: Arc<SkillReviewRegistry>,
    memory_reviews: Arc<MemoryReviewRegistry>,
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
        executor.memory_reviews = Arc::new(MemoryReviewRegistry::default());
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
        let memory_reviews = Arc::new(MemoryReviewRegistry::default());
        Ok(Self {
            paths: paths.clone(),
            state,
            executor: CommandExecutor {
                paths: paths.clone(),
                database,
                clock,
                ids,
                policy,
                hook,
                lifecycle,
                reviews,
                skill_reviews,
                memory_reviews,
                binding_catalog: Arc::new(binding_catalog),
            },
        })
    }

    pub fn worker(&self) -> Result<ApplicationWorker, StartupError> {
        Ok(ApplicationWorker {
            executor: CommandExecutor {
                paths: self.paths.clone(),
                database: Database::open(&self.paths)?,
                clock: self.executor.clock.clone(),
                ids: self.executor.ids.clone(),
                policy: self.executor.policy.clone(),
                hook: self.executor.hook.clone(),
                lifecycle: self.executor.lifecycle.clone(),
                reviews: self.executor.reviews.clone(),
                skill_reviews: self.executor.skill_reviews.clone(),
                memory_reviews: self.executor.memory_reviews.clone(),
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

    pub fn preview_skill_creation(
        &self,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.executor.preview_skill_creation(candidate)
    }

    pub fn preview_skill_version(
        &self,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.executor
            .preview_skill_version(skill_id, expected_active_version_id, candidate)
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

    pub fn cancel_skill_review(&self) -> Result<(), AppError> {
        self.executor.skill_reviews.cancel();
        Ok(())
    }

    pub fn preview_memory_set(
        &self,
        selector: AgentProfileSelector,
        candidate: MemoryEntryDraft,
    ) -> Result<MemoryEditPreview, AppError> {
        self.executor.preview_memory_set(selector, candidate)
    }

    pub fn preview_memory_delete(
        &self,
        selector: AgentProfileSelector,
        display_key: String,
    ) -> Result<MemoryEditPreview, AppError> {
        self.executor.preview_memory_delete(selector, display_key)
    }

    pub fn preview_memory_proposal_approval(
        &self,
        proposal: MemoryProposalRef,
    ) -> Result<MemoryProposalResolutionReview, AppError> {
        self.executor
            .preview_memory_proposal_resolution(proposal, MemoryResolutionAction::Approve)
    }

    pub fn preview_memory_proposal_rejection(
        &self,
        proposal: MemoryProposalRef,
    ) -> Result<MemoryProposalResolutionReview, AppError> {
        self.executor
            .preview_memory_proposal_resolution(proposal, MemoryResolutionAction::Reject)
    }

    pub fn cancel_memory_review(&self) -> Result<(), AppError> {
        self.executor
            .memory_reviews
            .cancel()
            .map_err(AppError::from)
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
        self.executor.memory_reviews.finish();
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

    pub fn preview_skill_creation(
        &self,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.executor.preview_skill_creation(candidate)
    }

    pub fn preview_skill_version(
        &self,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.executor
            .preview_skill_version(skill_id, expected_active_version_id, candidate)
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

    pub fn preview_memory_set(
        &self,
        selector: AgentProfileSelector,
        candidate: MemoryEntryDraft,
    ) -> Result<MemoryEditPreview, AppError> {
        self.executor.preview_memory_set(selector, candidate)
    }

    pub fn preview_memory_delete(
        &self,
        selector: AgentProfileSelector,
        display_key: String,
    ) -> Result<MemoryEditPreview, AppError> {
        self.executor.preview_memory_delete(selector, display_key)
    }

    pub fn preview_memory_proposal_approval(
        &self,
        proposal: MemoryProposalRef,
    ) -> Result<MemoryProposalResolutionReview, AppError> {
        self.executor
            .preview_memory_proposal_resolution(proposal, MemoryResolutionAction::Approve)
    }

    pub fn preview_memory_proposal_rejection(
        &self,
        proposal: MemoryProposalRef,
    ) -> Result<MemoryProposalResolutionReview, AppError> {
        self.executor
            .preview_memory_proposal_resolution(proposal, MemoryResolutionAction::Reject)
    }

    pub fn cancel_memory_review(&self) -> Result<(), AppError> {
        self.executor
            .memory_reviews
            .cancel()
            .map_err(AppError::from)
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

    pub fn preview_skill_creation(
        &self,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.executor.preview_skill_creation(candidate)
    }

    pub fn preview_skill_version(
        &self,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.executor
            .preview_skill_version(skill_id, expected_active_version_id, candidate)
    }
}

impl CommandExecutor {
    fn preview_memory_set(
        &self,
        selector: AgentProfileSelector,
        candidate: MemoryEntryDraft,
    ) -> Result<MemoryEditPreview, AppError> {
        self.preview_memory_edit(selector, candidate.normalized_key(), Some(candidate))
    }

    fn preview_memory_delete(
        &self,
        selector: AgentProfileSelector,
        display_key: String,
    ) -> Result<MemoryEditPreview, AppError> {
        self.preview_memory_edit(selector, NormalizedMemoryKey::new(&display_key)?, None)
    }

    fn preview_memory_edit(
        &self,
        selector: AgentProfileSelector,
        key: NormalizedMemoryKey,
        candidate: Option<MemoryEntryDraft>,
    ) -> Result<MemoryEditPreview, AppError> {
        self.ensure_passive_open(Capability::MemoryPreview)?;
        let mut database =
            Database::open(&self.paths).map_err(|_| PersistenceError::QueryFailed)?;
        let transaction = database.immediate_transaction()?;
        let projection = ProjectionRepository::load_in(&transaction)?;
        let profile = resolve_memory_profile(&projection.agent_profiles, &selector)?;
        let current = MemoryRepository::load_current_entry(
            &transaction,
            profile.memory_namespace_id(),
            &key,
        )?;
        let expected = match current.as_ref() {
            None => ExpectedMemoryEntryState::Absent,
            Some(entry) if entry.reference().state() == MemoryEntryState::Present => {
                ExpectedMemoryEntryState::Present(entry.reference())
            }
            Some(entry) => ExpectedMemoryEntryState::Deleted(entry.reference()),
        };
        let operation = if candidate.is_some() {
            MemoryMutationKind::Set
        } else {
            MemoryMutationKind::Delete
        };
        let prepared = prepare_direct_memory_edit(
            Actor::Human,
            profile.reference(),
            profile.memory_namespace_id(),
            current.as_ref(),
            expected,
            operation,
            candidate,
            MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        )?;
        transaction.commit()?;
        match prepared {
            MemoryEditPreviewOutcome::NoChange(no_change) => {
                Ok(MemoryEditPreview::NoChange(no_change))
            }
            MemoryEditPreviewOutcome::Prepared(prepared) => {
                let review_token =
                    crate::domain::MemoryReviewToken::from_uuid(self.ids.next_uuid());
                self.memory_reviews
                    .replace_direct(review_token, prepared.binding.clone());
                Ok(MemoryEditPreview::Review(
                    prepared.into_review(review_token),
                ))
            }
        }
    }

    fn preview_memory_proposal_resolution(
        &self,
        proposal_ref: MemoryProposalRef,
        action: MemoryResolutionAction,
    ) -> Result<MemoryProposalResolutionReview, AppError> {
        self.ensure_passive_open(Capability::MemoryPreview)?;
        let mut database =
            Database::open(&self.paths).map_err(|_| PersistenceError::QueryFailed)?;
        let transaction = database.immediate_transaction()?;
        let projection = ProjectionRepository::load_in(&transaction)?;
        let (proposal, status, resolution) =
            MemoryRepository::load_proposal(&transaction, proposal_ref.proposal_id())?
                .ok_or(AppError::MemoryProposalNotFound)?;
        if proposal.reference() != proposal_ref
            || status != MemoryProposalStatus::Pending
            || resolution.is_some()
        {
            return Err(crate::domain::DomainError::MemoryProposalReviewUnavailable.into());
        }
        let approval =
            MemoryRepository::load_memory_approval(&transaction, proposal.approval_id())?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
        if approval.action() != ApprovalAction::MemoryMutation
            || approval.object() != &proposal.object_ref()?
            || approval.actor() != &Actor::Agent(proposal.proposer().profile_id())
            || approval.status() != ApprovalStatus::Pending
            || approval.expires_at_millis().is_some()
            || approval.resolution().is_some()
        {
            return Err(PersistenceError::MemoryRowMismatch.into());
        }
        let proposer = projection
            .agent_profiles
            .resolve_reference(proposal.proposer())
            .map_err(|_| PersistenceError::MemoryRowMismatch)?;
        if proposer.memory_namespace_id() != proposal.namespace_id() {
            return Err(PersistenceError::MemoryRowMismatch.into());
        }
        let namespace_owner = projection
            .agent_profiles
            .active_profiles()
            .into_iter()
            .find(|profile| profile.memory_namespace_id() == proposal.namespace_id())
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        let expected_entry = current_expected_memory_entry(
            &transaction,
            proposal.namespace_id(),
            proposal.normalized_key(),
        )?;
        let acknowledgement = MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1;
        let (review_digest, binding) = prepare_memory_resolution_review(
            action,
            proposal.reference(),
            proposal.approval_id(),
            expected_entry.clone(),
        )?;
        let proposer_is_historical = namespace_owner.reference() != *proposal.proposer();
        transaction.commit()?;
        let review_token = MemoryReviewToken::from_uuid(self.ids.next_uuid());
        self.memory_reviews
            .replace_resolution(review_token, binding);
        Ok(MemoryProposalResolutionReview {
            action,
            proposal,
            approval_id: approval.approval_id(),
            expected_approval_status: approval.status(),
            expected_entry,
            proposer_is_historical,
            proposer_identity: memory_profile_identity(proposer),
            namespace_owner_identity: memory_profile_identity(&namespace_owner),
            plaintext_acknowledgement: acknowledgement,
            review_token,
            review_digest,
        })
    }

    fn preview_skill_creation(&self, candidate: SkillDraft) -> Result<SkillEditPreview, AppError> {
        self.ensure_passive_open(Capability::SkillCreate)?;
        let candidate = candidate.canonicalized()?;
        let normalized = candidate.normalized_name()?;
        if load_active_skill_by_name(self.database.connection(), &normalized)?.is_some() {
            return Err(AppError::DuplicateSkillName);
        }
        self.skill_reviews
            .operation()
            .issue_edit(
                SkillReviewToken::from_uuid(self.ids.next_uuid()),
                Actor::Human,
                SkillId::from_uuid(self.ids.next_uuid()),
                None,
                &candidate,
            )
            .map_err(AppError::from)
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
        self.skill_reviews
            .operation()
            .issue_edit(
                SkillReviewToken::from_uuid(self.ids.next_uuid()),
                Actor::Human,
                skill_id,
                Some(expected_active_version_id),
                &candidate,
            )
            .map_err(AppError::from)
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
        let current = projection
            .agent_profiles
            .active_profile(profile_id)
            .ok_or(AppError::AgentProfileNotFound)?;
        if current.profile_version_id() != expected_active_profile_version_id {
            return Err(AppError::StaleAgentProfileVersion);
        }
        validate_assignment_operation(self.database.connection(), current, &operation)?;
        self.skill_reviews
            .operation()
            .issue_assignment(
                SkillReviewToken::from_uuid(self.ids.next_uuid()),
                Actor::Human,
                profile_id,
                expected_active_profile_version_id,
                operation,
            )
            .map_err(AppError::from)
    }

    fn ensure_passive_open(&self, capability: Capability) -> Result<(), AppError> {
        let phase = self
            .lifecycle
            .phase
            .read()
            .map_err(|_| AppError::LifecycleFinished)?;
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
        let reviewed_memory_mutation = is_reviewed_memory_mutation(&envelope.command);
        let result = self.execute_locked(envelope, lifecycle.session_id, phase);
        self.invalidate_memory_review_after_terminal_failure(reviewed_memory_mutation, &result);
        result
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
        let reviewed_memory_mutation = is_reviewed_memory_mutation(&envelope.command);
        let result = self.execute_locked(envelope, lifecycle.session_id, phase);
        self.invalidate_memory_review_after_terminal_failure(reviewed_memory_mutation, &result);
        result
    }

    fn invalidate_memory_review_after_terminal_failure(
        &self,
        reviewed_memory_mutation: bool,
        result: &Result<CommandOutcome, AppError>,
    ) {
        if reviewed_memory_mutation
            && result.as_ref().is_err_and(|error| {
                memory_review_failure_disposition(error)
                    == MemoryReviewFailureDisposition::Invalidate
            })
        {
            self.memory_reviews.finish();
        }
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
                | ApplicationCommand::SetMemoryEntry { .. }
                | ApplicationCommand::DeleteMemoryEntry { .. }
                | ApplicationCommand::ApproveMemoryProposal { .. }
                | ApplicationCommand::RejectMemoryProposal { .. }
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
            self.hook
                .before_profile_review_operation(envelope.command_id);
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
        let memory_reviews = self.memory_reviews.clone();
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

        if is_memory_command(&request.command) {
            enforce_memory_actor_command(&request.actor, &request.command)?;
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
        let mut memory_reserved = None;
        let precommit =
            (|| -> Result<(StoredExecution, Vec<crate::app::EventEnvelope>), AppError> {
                let mut skill_event_object = None;
                let mut prepared_memory_view = None;
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
                                &request.actor,
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
                        skill_event_object = Some(skill_version_object(&skill)?);
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
                                &request.actor,
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
                        skill_event_object = Some(skill_version_object(&skill)?);
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
                                &request.actor,
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
                                &request.actor,
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
                                &request.actor,
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
                    command @ (ApplicationCommand::ListMemoryEntries { .. }
                    | ApplicationCommand::ShowMemoryEntry { .. }
                    | ApplicationCommand::ShowMemoryEntryHistory { .. }
                    | ApplicationCommand::ShowMemoryEntryVersion { .. }
                    | ApplicationCommand::ListMemoryProposals { .. }
                    | ApplicationCommand::ShowMemoryProposal { .. }
                    | ApplicationCommand::ListEpisodicSummaries { .. }
                    | ApplicationCommand::ShowEpisodicSummary { .. }) => {
                        let (event, view) =
                            prepare_memory_read(&transaction, &projection.agent_profiles, command)?;
                        prepared_memory_view = Some(view);
                        event
                    }
                    ApplicationCommand::BuildMemorySnapshot { request } => {
                        let (event, view) = prepare_memory_snapshot(
                            &transaction,
                            &projection.agent_profiles,
                            request,
                        )?;
                        prepared_memory_view = Some(view);
                        event
                    }
                    command @ (ApplicationCommand::SetMemoryEntry { .. }
                    | ApplicationCommand::DeleteMemoryEntry { .. }) => {
                        let (review_token, binding) =
                            direct_memory_review_binding(&projection, &request.actor, command)?;
                        let reserved = memory_reviews
                            .reserve_direct(envelope.command_id, review_token, &binding)
                            .map_err(AppError::from)?;
                        memory_reserved = Some(reserved);
                        prepare_direct_memory_mutation(
                            &transaction,
                            &projection,
                            self.ids.as_ref(),
                            self.clock.as_ref(),
                            &request.actor,
                            envelope.command_id,
                            command,
                            memory_reserved
                                .as_ref()
                                .expect("direct mutation owns its memory review"),
                        )?
                    }
                    command @ ApplicationCommand::ProposeMemoryMutation { .. } => {
                        prepare_memory_proposal_creation(
                            &transaction,
                            &projection,
                            self.ids.as_ref(),
                            self.clock.as_ref(),
                            &request.actor,
                            command,
                        )?
                    }
                    command @ (ApplicationCommand::ApproveMemoryProposal { .. }
                    | ApplicationCommand::RejectMemoryProposal { .. }) => {
                        let (review_token, binding) =
                            memory_resolution_review_binding(&request.actor, command)?;
                        let reserved = memory_reviews
                            .reserve_resolution(envelope.command_id, review_token, &binding)
                            .map_err(AppError::from)?;
                        memory_reserved = Some(reserved);
                        prepare_memory_resolution(
                            &transaction,
                            &projection,
                            self.ids.as_ref(),
                            self.clock.as_ref(),
                            &request.actor,
                            envelope.command_id,
                            command,
                            memory_reserved
                                .as_ref()
                                .expect("proposal resolution owns its memory review"),
                        )?
                    }
                    command @ (ApplicationCommand::CreateAgentProfile { .. }
                    | ApplicationCommand::ListAgentProfiles
                    | ApplicationCommand::ShowAgentProfile { .. }
                    | ApplicationCommand::ShowAgentProfileHistory { .. }
                    | ApplicationCommand::ShowAgentProfileVersion { .. }
                    | ApplicationCommand::ListSkills
                    | ApplicationCommand::ShowSkill { .. }
                    | ApplicationCommand::ShowSkillHistory { .. }
                    | ApplicationCommand::ShowSkillVersion { .. }
                    | ApplicationCommand::ShowHelp
                    | ApplicationCommand::ShowStatus
                    | ApplicationCommand::ShowSetupStatus
                    | ApplicationCommand::ShowAuditTail { .. }
                    | ApplicationCommand::RejectInput(_)
                    | ApplicationCommand::RequestShutdown) => prepare_event(
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
                    event_id: memory_mutation_event_id(&event)
                        .unwrap_or_else(|| EventId::from_uuid(self.ids.next_uuid())),
                    event_schema_version: EVENT_SCHEMA_VERSION,
                    actor: request.actor.clone(),
                    occurred_at_ms: event_occurred_at(&event)
                        .unwrap_or_else(|| self.clock.now_millis()),
                    correlation_id: request.correlation_id,
                    causation_id: Some(CausationId::from_uuid(envelope.command_id.as_uuid())),
                    object: memory_mutation_event_object(&event)?.or(skill_event_object),
                    event,
                };
                let committed = EventRepository::append(&transaction, pending)?;
                self.hook.after_event_append(transaction.transaction())?;
                persist_memory_mutation_event(&transaction, &committed)?;
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
                    self.hook
                        .after_profile_mirror_insert(transaction.transaction())?;
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
                let outcome = match prepared_memory_view {
                    Some(view) => CommandOutcome {
                        command_id: envelope.command_id,
                        correlation_id: request.correlation_id,
                        committed_events: events.clone(),
                        view,
                        shutdown: ShutdownDisposition::Continue,
                    },
                    None => materialize_success(
                        &transaction,
                        envelope.command_id,
                        &request,
                        &events,
                        &projection,
                        self.binding_catalog.as_ref(),
                    )?,
                };
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
                if let Some(reserved) = memory_reserved.take() {
                    finish_failed_memory_review(reserved, &error);
                }
                return Err(error);
            }
        };
        if let Err(error) = transaction.commit() {
            let app_error = AppError::Persistence(error);
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
            if let Some(reserved) = memory_reserved.take() {
                finish_failed_memory_review(reserved, &app_error);
            }
            return Err(app_error);
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
        if let Some(reserved) = memory_reserved.take() {
            reserved.consume().map_err(AppError::from)?;
        }
        if matches!(request.command, ApplicationCommand::RequestShutdown) {
            self.reviews.cancel();
            self.skill_reviews.cancel();
            self.memory_reviews.finish();
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

fn is_memory_command(command: &ApplicationCommand) -> bool {
    matches!(
        command,
        ApplicationCommand::SetMemoryEntry { .. }
            | ApplicationCommand::DeleteMemoryEntry { .. }
            | ApplicationCommand::ProposeMemoryMutation { .. }
            | ApplicationCommand::ApproveMemoryProposal { .. }
            | ApplicationCommand::RejectMemoryProposal { .. }
            | ApplicationCommand::ListMemoryEntries { .. }
            | ApplicationCommand::ShowMemoryEntry { .. }
            | ApplicationCommand::ShowMemoryEntryHistory { .. }
            | ApplicationCommand::ShowMemoryEntryVersion { .. }
            | ApplicationCommand::ListMemoryProposals { .. }
            | ApplicationCommand::ShowMemoryProposal { .. }
            | ApplicationCommand::ListEpisodicSummaries { .. }
            | ApplicationCommand::ShowEpisodicSummary { .. }
            | ApplicationCommand::BuildMemorySnapshot { .. }
    )
}

fn is_reviewed_memory_mutation(command: &ApplicationCommand) -> bool {
    matches!(
        command,
        ApplicationCommand::SetMemoryEntry { .. }
            | ApplicationCommand::DeleteMemoryEntry { .. }
            | ApplicationCommand::ApproveMemoryProposal { .. }
            | ApplicationCommand::RejectMemoryProposal { .. }
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MemoryReviewFailureDisposition {
    Release,
    Invalidate,
}

fn memory_review_failure_disposition(error: &AppError) -> MemoryReviewFailureDisposition {
    match error {
        AppError::Persistence(
            PersistenceError::Contention
            | PersistenceError::Capacity
            | PersistenceError::QueryFailed,
        ) => MemoryReviewFailureDisposition::Release,
        _ => MemoryReviewFailureDisposition::Invalidate,
    }
}

fn finish_failed_memory_review(reserved: ReservedMemoryReview<'_>, error: &AppError) {
    match memory_review_failure_disposition(error) {
        MemoryReviewFailureDisposition::Release => {
            let _ = reserved.release();
        }
        MemoryReviewFailureDisposition::Invalidate => {
            let _ = reserved.invalidate();
        }
    }
}

fn direct_memory_review_binding(
    projection: &ProjectionState,
    actor: &Actor,
    command: &ApplicationCommand,
) -> Result<(MemoryReviewToken, MemoryEditReviewBinding), AppError> {
    let acknowledgement = MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1;
    match command {
        ApplicationCommand::SetMemoryEntry {
            profile,
            expected,
            candidate,
            review_token,
            review_digest,
        } => {
            let exact = projection.agent_profiles.resolve_reference(profile)?;
            let candidate_digest = sha256(&canonical_json_bytes(candidate)?);
            Ok((
                *review_token,
                MemoryEditReviewBinding::new(
                    actor.clone(),
                    exact.reference(),
                    exact.memory_namespace_id(),
                    expected.clone(),
                    MemoryMutationKind::Set,
                    Some(candidate_digest),
                    acknowledgement,
                    review_digest.clone(),
                )?,
            ))
        }
        ApplicationCommand::DeleteMemoryEntry {
            profile,
            expected,
            review_token,
            review_digest,
        } => {
            let exact = projection.agent_profiles.resolve_reference(profile)?;
            Ok((
                *review_token,
                MemoryEditReviewBinding::new(
                    actor.clone(),
                    exact.reference(),
                    exact.memory_namespace_id(),
                    ExpectedMemoryEntryState::Present(expected.clone()),
                    MemoryMutationKind::Delete,
                    None,
                    acknowledgement,
                    review_digest.clone(),
                )?,
            ))
        }
        _ => unreachable!("direct memory review commands are dispatched exhaustively"),
    }
}

fn memory_resolution_review_binding(
    actor: &Actor,
    command: &ApplicationCommand,
) -> Result<(MemoryReviewToken, MemoryResolutionReviewBinding), AppError> {
    let (
        action,
        proposal,
        approval_id,
        expected_approval_status,
        expected_entry,
        review_token,
        review_digest,
    ) = match command {
        ApplicationCommand::ApproveMemoryProposal {
            proposal,
            approval_id,
            expected_approval_status,
            expected_entry,
            review_token,
            review_digest,
        } => (
            MemoryResolutionAction::Approve,
            proposal,
            *approval_id,
            *expected_approval_status,
            expected_entry,
            *review_token,
            review_digest,
        ),
        ApplicationCommand::RejectMemoryProposal {
            proposal,
            approval_id,
            expected_approval_status,
            expected_entry,
            review_token,
            review_digest,
        } => (
            MemoryResolutionAction::Reject,
            proposal,
            *approval_id,
            *expected_approval_status,
            expected_entry,
            *review_token,
            review_digest,
        ),
        _ => unreachable!("resolution review commands are dispatched exhaustively"),
    };
    Ok((
        review_token,
        MemoryResolutionReviewBinding::new(
            actor.clone(),
            action,
            proposal.clone(),
            approval_id,
            expected_approval_status,
            expected_entry.clone(),
            MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            review_digest.clone(),
        )?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn prepare_direct_memory_mutation(
    tx: &ImmediateTransaction<'_>,
    projection: &ProjectionState,
    ids: &dyn IdGenerator,
    clock: &dyn Clock,
    actor: &Actor,
    command_id: CommandId,
    command: &ApplicationCommand,
    reserved: &ReservedMemoryReview<'_>,
) -> Result<ApplicationEvent, AppError> {
    if actor != &Actor::Human || reserved.command_id() != command_id {
        return Err(crate::domain::DomainError::MemoryReviewUnavailable.into());
    }
    let _next_sequence = projection
        .last_sequence
        .checked_add(1)
        .ok_or(PersistenceError::InvalidEventRecord)?;
    let (profile_ref, expected, candidate, token) = match command {
        ApplicationCommand::SetMemoryEntry {
            profile,
            expected,
            candidate,
            review_token,
            ..
        } => (profile, expected.clone(), Some(candidate), *review_token),
        ApplicationCommand::DeleteMemoryEntry {
            profile,
            expected,
            review_token,
            ..
        } => (
            profile,
            ExpectedMemoryEntryState::Present(expected.clone()),
            None,
            *review_token,
        ),
        _ => unreachable!("direct memory mutation commands are dispatched exhaustively"),
    };
    if reserved.token() != token {
        return Err(crate::domain::DomainError::MemoryReviewUnavailable.into());
    }
    let profile = projection.agent_profiles.resolve_reference(profile_ref)?;
    let namespace_id = profile.memory_namespace_id();
    let key = match candidate {
        Some(candidate) => candidate.normalized_key(),
        None => match &expected {
            ExpectedMemoryEntryState::Present(reference) => reference.normalized_key().clone(),
            _ => return Err(crate::domain::DomainError::MemoryExpectedStateMismatch.into()),
        },
    };
    let current = MemoryRepository::load_current_entry(tx, namespace_id, &key)?;
    let operation = if candidate.is_some() {
        MemoryMutationKind::Set
    } else {
        MemoryMutationKind::Delete
    };
    match prepare_direct_memory_edit(
        actor.clone(),
        profile.reference(),
        namespace_id,
        current.as_ref(),
        expected.clone(),
        operation,
        candidate.cloned(),
        MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
    )? {
        MemoryEditPreviewOutcome::NoChange(_) => {
            return Err(crate::domain::DomainError::MemoryExpectedStateMismatch.into());
        }
        MemoryEditPreviewOutcome::Prepared(_) => {}
    }
    let active_count = MemoryRepository::count_active_entries(tx, namespace_id)?;
    let creates_active_key = candidate.is_some()
        && current
            .as_ref()
            .is_none_or(|entry| entry.reference().state() == MemoryEntryState::Deleted);
    if creates_active_key && active_count >= 1_024 {
        return Err(PersistenceError::Capacity.into());
    }

    let entry = match (current.as_ref(), candidate) {
        (None, Some(candidate)) => {
            let entry_id = MemoryEntryId::from_uuid(ids.next_uuid());
            let version_id = MemoryEntryVersionId::from_uuid(ids.next_uuid());
            let event_id = EventId::from_uuid(ids.next_uuid());
            let occurred_at_ms = clock.now_millis();
            MemoryEntryVersion::create_present(
                namespace_id,
                entry_id,
                version_id,
                candidate.clone(),
                Actor::Human,
                occurred_at_ms,
                None,
                event_id,
            )?
        }
        (Some(current), Some(candidate)) => {
            let version_id = MemoryEntryVersionId::from_uuid(ids.next_uuid());
            let event_id = EventId::from_uuid(ids.next_uuid());
            let occurred_at_ms = clock.now_millis();
            current.next_present(
                version_id,
                candidate.clone(),
                Actor::Human,
                occurred_at_ms,
                None,
                event_id,
            )?
        }
        (Some(current), None) => {
            let version_id = MemoryEntryVersionId::from_uuid(ids.next_uuid());
            let event_id = EventId::from_uuid(ids.next_uuid());
            let occurred_at_ms = clock.now_millis();
            current.next_deleted(version_id, Actor::Human, occurred_at_ms, None, event_id)?
        }
        (None, None) => {
            return Err(crate::domain::DomainError::MemoryExpectedStateMismatch.into());
        }
    };
    let new_expected = match entry.reference().state() {
        MemoryEntryState::Present => ExpectedMemoryEntryState::Present(entry.reference()),
        MemoryEntryState::Deleted => ExpectedMemoryEntryState::Deleted(entry.reference()),
    };
    let mut expired_proposals = MemoryRepository::load_pending_proposals_for_key(
        tx,
        namespace_id,
        entry.reference().normalized_key(),
    )?
    .into_iter()
    .filter(|proposal| proposal.expected() != &new_expected)
    .map(|proposal| {
        MemoryProposalResolution::new(
            proposal.reference(),
            MemoryProposalStatus::Expired,
            proposal.approval_id(),
            Actor::Human,
            entry.created_at_ms(),
            entry.creation_event_id(),
        )
    })
    .collect::<Result<Vec<_>, _>>()?;
    expired_proposals.sort_by_key(|resolution| resolution.proposal().proposal_id());
    Ok(match entry.reference().state() {
        MemoryEntryState::Present => ApplicationEvent::MemoryEntrySet {
            entry,
            expired_proposals,
        },
        MemoryEntryState::Deleted => ApplicationEvent::MemoryEntryDeleted {
            entry,
            expired_proposals,
        },
    })
}

fn prepare_memory_proposal_creation(
    tx: &ImmediateTransaction<'_>,
    projection: &ProjectionState,
    ids: &dyn IdGenerator,
    clock: &dyn Clock,
    actor: &Actor,
    command: &ApplicationCommand,
) -> Result<ApplicationEvent, AppError> {
    let ApplicationCommand::ProposeMemoryMutation {
        proposer,
        expected,
        operation,
        rationale,
    } = command
    else {
        unreachable!("proposal creation commands are dispatched exhaustively")
    };
    let proposer = projection.agent_profiles.resolve_reference(proposer)?;
    if actor != &Actor::Agent(proposer.profile_id()) {
        return Err(crate::domain::DomainError::MemoryProposalActorMismatch.into());
    }
    let (key, candidate) = match operation {
        MemoryProposalOperation::Set { candidate } => {
            (candidate.normalized_key(), Some(candidate.clone()))
        }
        MemoryProposalOperation::Delete => match expected {
            ExpectedMemoryEntryState::Present(reference) => {
                (reference.normalized_key().clone(), None)
            }
            _ => return Err(crate::domain::DomainError::InvalidMemoryProposal.into()),
        },
    };
    let current = MemoryRepository::load_current_entry(tx, proposer.memory_namespace_id(), &key)?;
    let mutation_kind = if candidate.is_some() {
        MemoryMutationKind::Set
    } else {
        MemoryMutationKind::Delete
    };
    match prepare_direct_memory_edit(
        Actor::Human,
        proposer.reference(),
        proposer.memory_namespace_id(),
        current.as_ref(),
        expected.clone(),
        mutation_kind,
        candidate,
        MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
    )? {
        MemoryEditPreviewOutcome::Prepared(_) => {}
        MemoryEditPreviewOutcome::NoChange(_) => {
            return Err(crate::domain::DomainError::InvalidMemoryProposal.into());
        }
    }
    if MemoryRepository::count_pending_proposals(tx, proposer.memory_namespace_id())? >= 256 {
        return Err(PersistenceError::Capacity.into());
    }
    let display_key = match operation {
        MemoryProposalOperation::Set { candidate } => candidate.display_key().to_owned(),
        MemoryProposalOperation::Delete => current
            .as_ref()
            .ok_or(crate::domain::DomainError::MemoryExpectedStateMismatch)?
            .display_key()
            .to_owned(),
    };
    let proposal_id = MemoryProposalId::from_uuid(ids.next_uuid());
    let approval_id = ApprovalId::from_uuid(ids.next_uuid());
    let event_id = EventId::from_uuid(ids.next_uuid());
    let occurred_at_ms = clock.now_millis();
    let proposal = MemoryProposal::new(
        proposal_id,
        proposer,
        actor,
        operation.clone(),
        display_key,
        expected.clone(),
        rationale.clone(),
        occurred_at_ms,
        event_id,
        approval_id,
    )?;
    let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
        .approval_id(approval_id)
        .object(proposal.object_ref()?)
        .actor(actor.clone())
        .created_at_millis(occurred_at_ms)
        .build()
        .map_err(|_| crate::domain::DomainError::InvalidMemoryProposal)?;
    Ok(ApplicationEvent::MemoryProposalCreated { proposal, approval })
}

#[allow(clippy::too_many_arguments)]
fn prepare_memory_resolution(
    tx: &ImmediateTransaction<'_>,
    projection: &ProjectionState,
    ids: &dyn IdGenerator,
    clock: &dyn Clock,
    actor: &Actor,
    command_id: CommandId,
    command: &ApplicationCommand,
    reserved: &ReservedMemoryReview<'_>,
) -> Result<ApplicationEvent, AppError> {
    if actor != &Actor::Human || reserved.command_id() != command_id {
        return Err(crate::domain::DomainError::MemoryProposalReviewUnavailable.into());
    }
    let (action, proposal_ref, approval_id, expected_status, expected_entry, token) = match command
    {
        ApplicationCommand::ApproveMemoryProposal {
            proposal,
            approval_id,
            expected_approval_status,
            expected_entry,
            review_token,
            ..
        } => (
            MemoryResolutionAction::Approve,
            proposal,
            *approval_id,
            *expected_approval_status,
            expected_entry,
            *review_token,
        ),
        ApplicationCommand::RejectMemoryProposal {
            proposal,
            approval_id,
            expected_approval_status,
            expected_entry,
            review_token,
            ..
        } => (
            MemoryResolutionAction::Reject,
            proposal,
            *approval_id,
            *expected_approval_status,
            expected_entry,
            *review_token,
        ),
        _ => unreachable!("proposal resolution commands are dispatched exhaustively"),
    };
    if reserved.token() != token || expected_status != ApprovalStatus::Pending {
        return Err(crate::domain::DomainError::MemoryProposalReviewUnavailable.into());
    }
    let (proposal, status, existing_resolution) =
        MemoryRepository::load_proposal(tx, proposal_ref.proposal_id())?
            .ok_or(AppError::MemoryProposalNotFound)?;
    if proposal.reference() != *proposal_ref
        || status != MemoryProposalStatus::Pending
        || existing_resolution.is_some()
        || proposal.approval_id() != approval_id
    {
        return Err(crate::domain::DomainError::MemoryProposalReviewUnavailable.into());
    }
    let projected = projection
        .memory
        .proposals()
        .find(|(id, _)| **id == proposal_ref.proposal_id())
        .map(|(_, projected)| projected)
        .ok_or(PersistenceError::MemoryRowMismatch)?;
    if projected.proposal() != proposal_ref
        || projected.namespace_id() != proposal.namespace_id()
        || projected.normalized_key() != proposal.normalized_key()
        || projected.expected() != proposal.expected()
        || projected.approval_id() != approval_id
        || projected.status() != MemoryProposalStatus::Pending
        || projected.resolution_event_id().is_some()
    {
        return Err(PersistenceError::MemoryRowMismatch.into());
    }
    let approval = MemoryRepository::load_memory_approval(tx, approval_id)?
        .ok_or(PersistenceError::MemoryRowMismatch)?;
    if approval.approval_id() != approval_id
        || approval.action() != ApprovalAction::MemoryMutation
        || approval.object() != &proposal.object_ref()?
        || approval.actor() != &Actor::Agent(proposal.proposer().profile_id())
        || approval.status() != expected_status
        || approval.expires_at_millis().is_some()
        || approval.resolution().is_some()
    {
        return Err(crate::domain::DomainError::MemoryProposalReviewUnavailable.into());
    }
    let current = MemoryRepository::load_current_entry(
        tx,
        proposal.namespace_id(),
        proposal.normalized_key(),
    )?;
    let current_expected = match current.as_ref() {
        None => ExpectedMemoryEntryState::Absent,
        Some(entry) if entry.reference().state() == MemoryEntryState::Present => {
            ExpectedMemoryEntryState::Present(entry.reference())
        }
        Some(entry) => ExpectedMemoryEntryState::Deleted(entry.reference()),
    };
    if &current_expected != expected_entry || &current_expected != proposal.expected() {
        return Err(crate::domain::DomainError::MemoryExpectedStateMismatch.into());
    }
    let pending_for_key = if action == MemoryResolutionAction::Approve {
        MemoryRepository::load_pending_proposals_for_key(
            tx,
            proposal.namespace_id(),
            proposal.normalized_key(),
        )?
    } else {
        Vec::new()
    };
    if action == MemoryResolutionAction::Approve {
        let creates_active_key =
            matches!(proposal.operation(), MemoryProposalOperation::Set { .. })
                && current
                    .as_ref()
                    .is_none_or(|entry| entry.reference().state() == MemoryEntryState::Deleted);
        if creates_active_key
            && MemoryRepository::count_active_entries(tx, proposal.namespace_id())? >= 1_024
        {
            return Err(PersistenceError::Capacity.into());
        }
    }
    let _next_sequence = projection
        .last_sequence
        .checked_add(1)
        .ok_or(PersistenceError::InvalidEventRecord)?;

    if action == MemoryResolutionAction::Reject {
        let event_id = EventId::from_uuid(ids.next_uuid());
        let occurred_at_ms = clock.now_millis();
        let resolution = MemoryProposalResolution::new(
            proposal.reference(),
            MemoryProposalStatus::Rejected,
            approval_id,
            Actor::Human,
            occurred_at_ms,
            event_id,
        )?;
        return Ok(ApplicationEvent::MemoryProposalRejected { resolution });
    }

    let entry = match (current.as_ref(), proposal.operation()) {
        (None, MemoryProposalOperation::Set { candidate }) => {
            let entry_id = MemoryEntryId::from_uuid(ids.next_uuid());
            let version_id = MemoryEntryVersionId::from_uuid(ids.next_uuid());
            let event_id = EventId::from_uuid(ids.next_uuid());
            let occurred_at_ms = clock.now_millis();
            MemoryEntryVersion::create_present(
                proposal.namespace_id(),
                entry_id,
                version_id,
                candidate.clone(),
                Actor::Human,
                occurred_at_ms,
                Some(proposal.reference()),
                event_id,
            )?
        }
        (Some(current), MemoryProposalOperation::Set { candidate }) => {
            let version_id = MemoryEntryVersionId::from_uuid(ids.next_uuid());
            let event_id = EventId::from_uuid(ids.next_uuid());
            let occurred_at_ms = clock.now_millis();
            current.next_present(
                version_id,
                candidate.clone(),
                Actor::Human,
                occurred_at_ms,
                Some(proposal.reference()),
                event_id,
            )?
        }
        (Some(current), MemoryProposalOperation::Delete) => {
            let version_id = MemoryEntryVersionId::from_uuid(ids.next_uuid());
            let event_id = EventId::from_uuid(ids.next_uuid());
            let occurred_at_ms = clock.now_millis();
            current.next_deleted(
                version_id,
                Actor::Human,
                occurred_at_ms,
                Some(proposal.reference()),
                event_id,
            )?
        }
        (None, MemoryProposalOperation::Delete) => {
            return Err(crate::domain::DomainError::MemoryExpectedStateMismatch.into());
        }
    };
    let resolution = MemoryProposalResolution::new(
        proposal.reference(),
        MemoryProposalStatus::Accepted,
        approval_id,
        Actor::Human,
        entry.created_at_ms(),
        entry.creation_event_id(),
    )?;
    let new_expected = match entry.reference().state() {
        MemoryEntryState::Present => ExpectedMemoryEntryState::Present(entry.reference()),
        MemoryEntryState::Deleted => ExpectedMemoryEntryState::Deleted(entry.reference()),
    };
    let mut expired_proposals = pending_for_key
        .into_iter()
        .filter(|sibling| sibling.reference() != proposal.reference())
        .filter(|sibling| sibling.expected() != &new_expected)
        .map(|sibling| {
            MemoryProposalResolution::new(
                sibling.reference(),
                MemoryProposalStatus::Expired,
                sibling.approval_id(),
                Actor::Human,
                entry.created_at_ms(),
                entry.creation_event_id(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    expired_proposals.sort_by_key(|resolution| resolution.proposal().proposal_id());
    Ok(ApplicationEvent::MemoryProposalAccepted {
        resolution,
        entry,
        expired_proposals,
    })
}

fn memory_mutation_event_id(event: &ApplicationEvent) -> Option<EventId> {
    match event {
        ApplicationEvent::MemoryEntrySet { entry, .. }
        | ApplicationEvent::MemoryEntryDeleted { entry, .. } => Some(entry.creation_event_id()),
        ApplicationEvent::MemoryProposalCreated { proposal, .. } => {
            Some(proposal.creation_event_id())
        }
        ApplicationEvent::MemoryProposalAccepted { resolution, .. }
        | ApplicationEvent::MemoryProposalRejected { resolution } => {
            Some(resolution.resolution_event_id())
        }
        _ => None,
    }
}

fn memory_mutation_event_object(event: &ApplicationEvent) -> Result<Option<ObjectRef>, AppError> {
    match event {
        ApplicationEvent::MemoryEntrySet { entry, .. }
        | ApplicationEvent::MemoryEntryDeleted { entry, .. } => {
            let reference = entry.reference();
            Ok(Some(ObjectRef::new(
                "memory_entry_version",
                reference.entry_version_id().to_string(),
                reference.version(),
                reference.content_digest().clone(),
            )?))
        }
        ApplicationEvent::MemoryProposalCreated { proposal, .. } => {
            Ok(Some(proposal.object_ref()?))
        }
        ApplicationEvent::MemoryProposalAccepted { resolution, .. }
        | ApplicationEvent::MemoryProposalRejected { resolution } => Ok(Some(ObjectRef::new(
            "memory_proposal",
            resolution.proposal().proposal_id().to_string(),
            resolution.proposal().version(),
            resolution.proposal().content_digest().clone(),
        )?)),
        _ => Ok(None),
    }
}

fn persist_memory_mutation_event(
    tx: &ImmediateTransaction<'_>,
    committed: &crate::app::EventEnvelope,
) -> Result<(), AppError> {
    if let ApplicationEvent::MemoryProposalCreated { proposal, approval } = &committed.event {
        MemoryRepository::insert_proposal_with_approval(
            tx,
            committed.sequence,
            proposal,
            approval,
        )?;
        return Ok(());
    }
    if let ApplicationEvent::MemoryProposalRejected { resolution } = &committed.event {
        let approval = MemoryRepository::load_memory_approval(tx, resolution.approval_id())?
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        let resolved = approval
            .resolve(
                ApprovalStatus::Rejected,
                Actor::Human,
                resolution.resolved_at_ms(),
            )
            .map_err(|_| PersistenceError::MemoryRowMismatch)?;
        MemoryRepository::resolve_proposal(tx, committed.sequence, resolution, &resolved)?;
        return Ok(());
    }
    if let ApplicationEvent::MemoryProposalAccepted {
        resolution,
        entry,
        expired_proposals,
    } = &committed.event
    {
        let approval = MemoryRepository::load_memory_approval(tx, resolution.approval_id())?
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        let resolved = approval
            .resolve(
                ApprovalStatus::Accepted,
                Actor::Human,
                resolution.resolved_at_ms(),
            )
            .map_err(|_| PersistenceError::MemoryRowMismatch)?;
        MemoryRepository::resolve_proposal(tx, committed.sequence, resolution, &resolved)?;
        MemoryRepository::insert_entry_version(tx, committed.sequence, entry)?;
        MemoryRepository::replace_current_entry(tx, entry)?;
        for sibling in expired_proposals {
            let approval = MemoryRepository::load_memory_approval(tx, sibling.approval_id())?
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            let expired = approval
                .resolve(
                    ApprovalStatus::Expired,
                    Actor::Human,
                    sibling.resolved_at_ms(),
                )
                .map_err(|_| PersistenceError::MemoryRowMismatch)?;
            MemoryRepository::resolve_proposal(tx, committed.sequence, sibling, &expired)?;
        }
        return Ok(());
    }
    let (entry, expired_proposals) = match &committed.event {
        ApplicationEvent::MemoryEntrySet {
            entry,
            expired_proposals,
        }
        | ApplicationEvent::MemoryEntryDeleted {
            entry,
            expired_proposals,
        } => (entry, expired_proposals),
        _ => return Ok(()),
    };
    MemoryRepository::insert_entry_version(tx, committed.sequence, entry)?;
    MemoryRepository::replace_current_entry(tx, entry)?;
    for resolution in expired_proposals {
        let approval = MemoryRepository::load_memory_approval(tx, resolution.approval_id())?
            .ok_or(PersistenceError::MemoryRowMismatch)?;
        let resolved = approval
            .resolve(
                crate::policy::ApprovalStatus::Expired,
                Actor::Human,
                resolution.resolved_at_ms(),
            )
            .map_err(|_| PersistenceError::MemoryRowMismatch)?;
        MemoryRepository::resolve_proposal(tx, committed.sequence, resolution, &resolved)?;
    }
    Ok(())
}

fn enforce_memory_actor_command(
    actor: &Actor,
    command: &ApplicationCommand,
) -> Result<(), AppError> {
    let allowed = match (actor, command) {
        (
            Actor::Human,
            ApplicationCommand::SetMemoryEntry { .. }
            | ApplicationCommand::DeleteMemoryEntry { .. }
            | ApplicationCommand::ApproveMemoryProposal { .. }
            | ApplicationCommand::RejectMemoryProposal { .. }
            | ApplicationCommand::ListMemoryEntries { .. }
            | ApplicationCommand::ShowMemoryEntry { .. }
            | ApplicationCommand::ShowMemoryEntryHistory { .. }
            | ApplicationCommand::ShowMemoryEntryVersion { .. }
            | ApplicationCommand::ListMemoryProposals { .. }
            | ApplicationCommand::ShowMemoryProposal { .. }
            | ApplicationCommand::ListEpisodicSummaries { .. }
            | ApplicationCommand::ShowEpisodicSummary { .. }
            | ApplicationCommand::BuildMemorySnapshot { .. },
        ) => true,
        (Actor::Agent(actor_id), ApplicationCommand::ProposeMemoryMutation { proposer, .. }) => {
            *actor_id == proposer.profile_id()
        }
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(AppError::CapabilityDenied {
            capability: command.required_capability(),
            decision: PolicyDecision::Denied,
        })
    }
}

fn resolve_memory_profile<'a>(
    profiles: &'a AgentProfilesProjection,
    selector: &AgentProfileSelector,
) -> Result<&'a AgentProfileVersion, AppError> {
    match selector {
        AgentProfileSelector::Id(profile_id) => profiles.active_profile(*profile_id),
        AgentProfileSelector::Name(_) => selector
            .normalized_name()
            .as_ref()
            .and_then(|name| profiles.active_profile_by_name(name)),
    }
    .ok_or(AppError::AgentProfileNotFound)
}

fn memory_profile_identity(profile: &AgentProfileVersion) -> MemoryProfileIdentityView {
    MemoryProfileIdentityView {
        profile: profile.reference(),
        display_name: profile.display_name().to_owned(),
    }
}

fn current_expected_memory_entry(
    tx: &ImmediateTransaction<'_>,
    namespace_id: MemoryNamespaceId,
    key: &NormalizedMemoryKey,
) -> Result<ExpectedMemoryEntryState, AppError> {
    Ok(
        match MemoryRepository::load_current_entry(tx, namespace_id, key)? {
            None => ExpectedMemoryEntryState::Absent,
            Some(entry) if entry.reference().state() == MemoryEntryState::Present => {
                ExpectedMemoryEntryState::Present(entry.reference())
            }
            Some(entry) => ExpectedMemoryEntryState::Deleted(entry.reference()),
        },
    )
}

fn prepare_memory_read(
    tx: &ImmediateTransaction<'_>,
    profiles: &AgentProfilesProjection,
    command: &ApplicationCommand,
) -> Result<(ApplicationEvent, CommandView), AppError> {
    match command {
        ApplicationCommand::ListMemoryEntries { selector } => {
            let profile = resolve_memory_profile(profiles, selector)?;
            let page =
                MemoryRepository::list_current_entries(tx, profile.memory_namespace_id(), 100)?;
            let event = ApplicationEvent::MemoryEntriesListed {
                profile: profile.reference(),
                namespace_id: profile.memory_namespace_id(),
                entries: page
                    .records
                    .iter()
                    .map(|record| record.entry.clone())
                    .collect(),
                total_count: page.total_count,
                returned_count: page.returned_count,
                omitted_count: page.omitted_count,
            };
            let view = CommandView::MemoryEntries(MemoryEntriesView {
                profile: profile.reference(),
                namespace_id: profile.memory_namespace_id(),
                entries: page
                    .records
                    .into_iter()
                    .map(|record| MemoryEntrySummary {
                        entry: record.entry,
                        display_key: record.display_key,
                        purpose_tags: record.purpose_tags,
                        value_bytes: record.value_bytes,
                        created_at_ms: record.created_at_ms,
                    })
                    .collect(),
                total_count: page.total_count,
                returned_count: page.returned_count,
                omitted_count: page.omitted_count,
            });
            Ok((event, view))
        }
        ApplicationCommand::ShowMemoryEntry {
            selector,
            display_key,
        } => {
            let profile = resolve_memory_profile(profiles, selector)?;
            let key = NormalizedMemoryKey::new(display_key)?;
            let entry =
                MemoryRepository::load_current_entry(tx, profile.memory_namespace_id(), &key)?
                    .filter(|entry| entry.reference().state() == MemoryEntryState::Present)
                    .ok_or(AppError::MemoryEntryNotFound)?;
            Ok((
                ApplicationEvent::MemoryEntryShown {
                    profile: profile.reference(),
                    entry: entry.reference(),
                },
                CommandView::MemoryEntry(MemoryEntryView {
                    profile: profile.reference(),
                    entry,
                }),
            ))
        }
        ApplicationCommand::ShowMemoryEntryHistory {
            selector,
            display_key,
        } => {
            let profile = resolve_memory_profile(profiles, selector)?;
            let key = NormalizedMemoryKey::new(display_key)?;
            let current =
                MemoryRepository::load_current_entry(tx, profile.memory_namespace_id(), &key)?
                    .ok_or(AppError::MemoryEntryNotFound)?;
            let page =
                MemoryRepository::load_entry_history(tx, profile.memory_namespace_id(), &key, 100)?;
            let current = current.reference();
            if page
                .versions
                .first()
                .map(|entry| entry.reference())
                .as_ref()
                != Some(&current)
            {
                return Err(PersistenceError::MemoryRowMismatch.into());
            }
            let event = ApplicationEvent::MemoryEntryHistoryShown {
                profile: profile.reference(),
                current: current.clone(),
                versions: page
                    .versions
                    .iter()
                    .map(|entry| entry.reference())
                    .collect(),
                total_count: page.total_count,
                returned_count: page.returned_count,
                omitted_count: page.omitted_count,
            };
            let view = CommandView::MemoryEntryHistory(MemoryEntryHistoryView {
                profile: profile.reference(),
                current,
                versions: page
                    .versions
                    .into_iter()
                    .map(|entry| MemoryEntryHistorySummary {
                        entry: entry.reference(),
                        display_key: entry.display_key().to_owned(),
                        created_at_ms: entry.created_at_ms(),
                        accepted_proposal: entry.accepted_proposal().cloned(),
                    })
                    .collect(),
                total_count: page.total_count,
                returned_count: page.returned_count,
                omitted_count: page.omitted_count,
            });
            Ok((event, view))
        }
        ApplicationCommand::ShowMemoryEntryVersion {
            selector,
            display_key,
            version,
        } => {
            let profile = resolve_memory_profile(profiles, selector)?;
            let key = NormalizedMemoryKey::new(display_key)?;
            let entry = MemoryRepository::load_entry_version(
                tx,
                profile.memory_namespace_id(),
                &key,
                *version,
            )?
            .ok_or(AppError::MemoryEntryNotFound)?;
            Ok((
                ApplicationEvent::MemoryEntryVersionShown {
                    profile: profile.reference(),
                    entry: entry.reference(),
                },
                CommandView::MemoryEntryVersion(MemoryEntryVersionView {
                    profile: profile.reference(),
                    entry,
                }),
            ))
        }
        ApplicationCommand::ListMemoryProposals { selector, filter } => {
            let profile = resolve_memory_profile(profiles, selector)?;
            let page =
                MemoryRepository::list_proposals(tx, profile.memory_namespace_id(), *filter, 100)?;
            let event = ApplicationEvent::MemoryProposalsListed {
                profile: profile.reference(),
                filter: *filter,
                proposals: page
                    .records
                    .iter()
                    .map(|record| MemoryProposalStatusRef {
                        proposal: record.proposal.clone(),
                        status: record.status,
                    })
                    .collect(),
                total_count: page.total_count,
                returned_count: page.returned_count,
                omitted_count: page.omitted_count,
            };
            let view = CommandView::MemoryProposals(MemoryProposalsView {
                profile: profile.reference(),
                namespace_id: profile.memory_namespace_id(),
                filter: *filter,
                proposals: page
                    .records
                    .into_iter()
                    .map(|record| MemoryProposalSummary {
                        proposal: record.proposal,
                        proposer: record.proposer,
                        operation: record.operation,
                        display_key: record.display_key,
                        status: record.status,
                        created_at_ms: record.created_at_ms,
                    })
                    .collect(),
                total_count: page.total_count,
                returned_count: page.returned_count,
                omitted_count: page.omitted_count,
            });
            Ok((event, view))
        }
        ApplicationCommand::ShowMemoryProposal { proposal_id } => {
            let (proposal, status, resolution) = MemoryRepository::load_proposal(tx, *proposal_id)?
                .ok_or(AppError::MemoryProposalNotFound)?;
            let proposer = profiles
                .resolve_reference(proposal.proposer())
                .map_err(|_| PersistenceError::MemoryRowMismatch)?;
            let owner = profiles
                .active_profiles()
                .into_iter()
                .find(|profile| profile.memory_namespace_id() == proposal.namespace_id())
                .ok_or(PersistenceError::MemoryRowMismatch)?;
            let current_entry = current_expected_memory_entry(
                tx,
                proposal.namespace_id(),
                proposal.normalized_key(),
            )?;
            let proposer_is_historical = profiles
                .active_profile(proposer.profile_id())
                .is_none_or(|active| active.reference() != *proposal.proposer());
            let event = ApplicationEvent::MemoryProposalShown {
                proposal: proposal.reference(),
                status,
                resolution: resolution.clone(),
            };
            let view = CommandView::MemoryProposal(MemoryProposalView {
                proposal,
                status,
                resolution,
                current_entry,
                proposer_is_historical,
                proposer_identity: memory_profile_identity(proposer),
                namespace_owner_identity: memory_profile_identity(&owner),
            });
            Ok((event, view))
        }
        ApplicationCommand::ListEpisodicSummaries { selector } => {
            let profile = resolve_memory_profile(profiles, selector)?;
            let page =
                MemoryRepository::list_episodic_summaries(tx, profile.memory_namespace_id(), 100)?;
            let event = ApplicationEvent::EpisodicSummariesListed {
                profile: profile.reference(),
                summaries: page
                    .records
                    .iter()
                    .map(|record| record.summary.clone())
                    .collect(),
                total_count: page.total_count,
                returned_count: page.returned_count,
                omitted_count: page.omitted_count,
            };
            let view = CommandView::EpisodicSummaries(EpisodicSummariesView {
                profile: profile.reference(),
                namespace_id: profile.memory_namespace_id(),
                summaries: page
                    .records
                    .into_iter()
                    .map(|record| EpisodicSummaryListItem {
                        summary: record.summary,
                        label: record.label,
                        purpose_tags: record.purpose_tags,
                        source_count: record.source_count,
                        created_at_ms: record.created_at_ms,
                    })
                    .collect(),
                total_count: page.total_count,
                returned_count: page.returned_count,
                omitted_count: page.omitted_count,
            });
            Ok((event, view))
        }
        ApplicationCommand::ShowEpisodicSummary { summary_id } => {
            let summary = MemoryRepository::load_episodic_summary(tx, *summary_id)?
                .ok_or(AppError::EpisodicSummaryNotFound)?;
            let profile = profiles
                .resolve_reference(summary.reference().profile())
                .map_err(|_| PersistenceError::MemoryRowMismatch)?;
            if profile.memory_namespace_id() != summary.reference().namespace_id() {
                return Err(PersistenceError::MemoryRowMismatch.into());
            }
            Ok((
                ApplicationEvent::EpisodicSummaryShown {
                    summary: summary.reference(),
                },
                CommandView::EpisodicSummary(EpisodicSummaryView {
                    summary,
                    qualification: EpisodicQualification::SummaryVerifySources,
                }),
            ))
        }
        _ => Err(AppError::WrongMemoryCommandDispatcher),
    }
}

fn prepare_memory_snapshot(
    tx: &ImmediateTransaction<'_>,
    profiles: &AgentProfilesProjection,
    request: &MemoryRetrievalRequest,
) -> Result<(ApplicationEvent, CommandView), AppError> {
    let profile = profiles
        .resolve_reference(request.scope().profile())
        .map_err(AppError::from)?;
    request.scope().validate_against(profile)?;
    let snapshot = MemoryRepository::build_snapshot(tx, request)?;
    let metadata = snapshot.metadata();
    Ok((
        ApplicationEvent::MemorySnapshotBuilt { metadata },
        CommandView::MemorySnapshot(MemorySnapshotView { snapshot }),
    ))
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
            let returned = skills
                .into_iter()
                .take(MAX_SKILL_LIST_RESULTS)
                .collect::<Vec<_>>();
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
        ApplicationCommand::SetMemoryEntry { .. }
        | ApplicationCommand::DeleteMemoryEntry { .. }
        | ApplicationCommand::ProposeMemoryMutation { .. }
        | ApplicationCommand::ApproveMemoryProposal { .. }
        | ApplicationCommand::RejectMemoryProposal { .. }
        | ApplicationCommand::ListMemoryEntries { .. }
        | ApplicationCommand::ShowMemoryEntry { .. }
        | ApplicationCommand::ShowMemoryEntryHistory { .. }
        | ApplicationCommand::ShowMemoryEntryVersion { .. }
        | ApplicationCommand::ListMemoryProposals { .. }
        | ApplicationCommand::ShowMemoryProposal { .. }
        | ApplicationCommand::ListEpisodicSummaries { .. }
        | ApplicationCommand::ShowEpisodicSummary { .. }
        | ApplicationCommand::BuildMemorySnapshot { .. } => {
            unreachable!("memory commands are dispatched before prepare_event")
        }
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
        ApplicationEvent::MemoryEntrySet { entry, .. }
        | ApplicationEvent::MemoryEntryDeleted { entry, .. }
        | ApplicationEvent::MemoryProposalAccepted { entry, .. } => Some(entry.created_at_ms()),
        ApplicationEvent::MemoryProposalCreated { proposal, .. } => Some(proposal.created_at_ms()),
        ApplicationEvent::MemoryProposalRejected { resolution } => {
            Some(resolution.resolved_at_ms())
        }
        ApplicationEvent::EpisodicSummaryRecorded { summary } => Some(summary.created_at_ms()),
        _ => None,
    }
}

fn skill_version_object(skill: &SkillVersion) -> Result<crate::domain::ObjectRef, AppError> {
    crate::domain::ObjectRef::new(
        "skill_version",
        skill.skill_version_id().to_string(),
        skill.version(),
        sha256(&canonical_json_bytes(skill)?),
    )
    .map_err(AppError::from)
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

fn resolve_projected_active_skill<'a>(
    projection: &'a ProjectionState,
    selector: &SkillSelector,
) -> Option<&'a SkillVersionRef> {
    match selector {
        SkillSelector::Id(skill_id) => projection.skills.active_skill(*skill_id),
        SkillSelector::Name(_) => {
            let normalized_name = selector.normalized_name()?;
            projection.skills.active_refs().find(|active| {
                projection
                    .skills
                    .version_metadata()
                    .any(|(version, display_name, _, _, _)| {
                        version == *active
                            && SkillSelector::Name(display_name.to_owned())
                                .normalized_name()
                                .as_ref()
                                == Some(&normalized_name)
                    })
            })
        }
    }
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
        AgentSkillAssignmentOperation::Upgrade {
            expected,
            replacement,
        } => {
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
            current
                .unassign_skill(expected.clone())
                .map_err(AppError::from)
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
        AgentSkillAssignmentOperation::Upgrade {
            expected,
            replacement,
        } => ApplicationEvent::AgentSkillUpgraded {
            profile,
            previous_profile_version_id: expected_active_profile_version_id,
            expected,
            replacement,
        },
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
        | CommandView::MemoryEntries(_)
        | CommandView::MemoryEntry(_)
        | CommandView::MemoryEntryHistory(_)
        | CommandView::MemoryEntryVersion(_)
        | CommandView::MemoryProposals(_)
        | CommandView::MemoryProposal(_)
        | CommandView::EpisodicSummaries(_)
        | CommandView::EpisodicSummary(_)
        | CommandView::MemoryEntryMutation(_)
        | CommandView::MemoryProposalCreated(_)
        | CommandView::MemoryProposalResolution(_)
        | CommandView::MemorySnapshot(_)
        | CommandView::InputRejected(_)
        | CommandView::Shutdown(_) => {}
    }
}

fn materialize_memory_view(
    tx: &ImmediateTransaction<'_>,
    command: &ApplicationCommand,
    event: &crate::app::EventEnvelope,
    projection_at_event: &ProjectionState,
) -> Result<CommandView, AppError> {
    fn load_exact_entry(
        tx: &ImmediateTransaction<'_>,
        reference: &crate::memory::MemoryEntryRef,
    ) -> Result<crate::memory::MemoryEntryVersion, AppError> {
        let entry = MemoryRepository::load_entry_version(
            tx,
            reference.namespace_id(),
            reference.normalized_key(),
            reference.version(),
        )?
        .ok_or_else(invalid_receipt)?;
        if entry.reference() != *reference {
            return Err(invalid_receipt());
        }
        Ok(entry)
    }

    fn validate_predecessor(
        tx: &ImmediateTransaction<'_>,
        entry: &crate::memory::MemoryEntryVersion,
        expected: &ExpectedMemoryEntryState,
    ) -> Result<(), AppError> {
        match expected {
            ExpectedMemoryEntryState::Absent
                if entry.reference().version().get() == 1
                    && entry.predecessor_version_id().is_none() => {}
            ExpectedMemoryEntryState::Present(reference)
            | ExpectedMemoryEntryState::Deleted(reference) => {
                let predecessor = load_exact_entry(tx, reference)?;
                if predecessor.reference().entry_id() != entry.reference().entry_id()
                    || predecessor.reference().namespace_id() != entry.reference().namespace_id()
                    || predecessor.reference().normalized_key()
                        != entry.reference().normalized_key()
                    || entry.predecessor_version_id()
                        != Some(predecessor.reference().entry_version_id())
                    || predecessor.reference().version().get().checked_add(1)
                        != Some(entry.reference().version().get())
                {
                    return Err(invalid_receipt());
                }
            }
            _ => return Err(invalid_receipt()),
        }
        Ok(())
    }

    match (command, &event.event) {
        (
            ApplicationCommand::ListMemoryEntries { selector },
            ApplicationEvent::MemoryEntriesListed {
                profile,
                namespace_id,
                entries,
                total_count,
                returned_count,
                omitted_count,
            },
        ) => {
            let selected = resolve_memory_profile(&projection_at_event.agent_profiles, selector)
                .map_err(|_| invalid_receipt())?;
            let exact = projection_at_event
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| invalid_receipt())?;
            if selected != exact
                || exact.memory_namespace_id() != *namespace_id
                || u64::try_from(entries.len()).ok() != Some(*returned_count)
                || returned_count.checked_add(*omitted_count) != Some(*total_count)
            {
                return Err(invalid_receipt());
            }
            let mut summaries = Vec::with_capacity(entries.len());
            for reference in entries {
                let entry = MemoryRepository::load_entry_version(
                    tx,
                    reference.namespace_id(),
                    reference.normalized_key(),
                    reference.version(),
                )?
                .ok_or_else(invalid_receipt)?;
                if entry.reference() != *reference {
                    return Err(invalid_receipt());
                }
                summaries.push(MemoryEntrySummary {
                    entry: reference.clone(),
                    display_key: entry.display_key().to_owned(),
                    purpose_tags: entry.purpose_tags().to_vec(),
                    value_bytes: u64::try_from(entry.value().ok_or_else(invalid_receipt)?.len())
                        .map_err(|_| invalid_receipt())?,
                    created_at_ms: entry.created_at_ms(),
                });
            }
            Ok(CommandView::MemoryEntries(MemoryEntriesView {
                profile: profile.clone(),
                namespace_id: *namespace_id,
                entries: summaries,
                total_count: *total_count,
                returned_count: *returned_count,
                omitted_count: *omitted_count,
            }))
        }
        (
            ApplicationCommand::ShowMemoryEntry {
                selector,
                display_key,
            },
            ApplicationEvent::MemoryEntryShown { profile, entry },
        ) => {
            let selected = resolve_memory_profile(&projection_at_event.agent_profiles, selector)
                .map_err(|_| invalid_receipt())?;
            let exact = projection_at_event
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| invalid_receipt())?;
            let key = NormalizedMemoryKey::new(display_key).map_err(|_| invalid_receipt())?;
            if selected != exact
                || exact.memory_namespace_id() != entry.namespace_id()
                || &key != entry.normalized_key()
                || entry.state() != MemoryEntryState::Present
            {
                return Err(invalid_receipt());
            }
            let stored = MemoryRepository::load_entry_version(
                tx,
                entry.namespace_id(),
                entry.normalized_key(),
                entry.version(),
            )?
            .ok_or_else(invalid_receipt)?;
            if stored.reference() != *entry {
                return Err(invalid_receipt());
            }
            Ok(CommandView::MemoryEntry(MemoryEntryView {
                profile: profile.clone(),
                entry: stored,
            }))
        }
        (
            ApplicationCommand::ShowMemoryEntryHistory {
                selector,
                display_key,
            },
            ApplicationEvent::MemoryEntryHistoryShown {
                profile,
                current,
                versions,
                total_count,
                returned_count,
                omitted_count,
            },
        ) => {
            let selected = resolve_memory_profile(&projection_at_event.agent_profiles, selector)
                .map_err(|_| invalid_receipt())?;
            let exact = projection_at_event
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| invalid_receipt())?;
            let key = NormalizedMemoryKey::new(display_key).map_err(|_| invalid_receipt())?;
            if selected != exact
                || exact.memory_namespace_id() != current.namespace_id()
                || &key != current.normalized_key()
                || versions.first() != Some(current)
                || u64::try_from(versions.len()).ok() != Some(*returned_count)
                || returned_count.checked_add(*omitted_count) != Some(*total_count)
            {
                return Err(invalid_receipt());
            }
            let mut summaries = Vec::with_capacity(versions.len());
            for reference in versions {
                let entry = MemoryRepository::load_entry_version(
                    tx,
                    reference.namespace_id(),
                    reference.normalized_key(),
                    reference.version(),
                )?
                .ok_or_else(invalid_receipt)?;
                if entry.reference() != *reference {
                    return Err(invalid_receipt());
                }
                summaries.push(MemoryEntryHistorySummary {
                    entry: reference.clone(),
                    display_key: entry.display_key().to_owned(),
                    created_at_ms: entry.created_at_ms(),
                    accepted_proposal: entry.accepted_proposal().cloned(),
                });
            }
            Ok(CommandView::MemoryEntryHistory(MemoryEntryHistoryView {
                profile: profile.clone(),
                current: current.clone(),
                versions: summaries,
                total_count: *total_count,
                returned_count: *returned_count,
                omitted_count: *omitted_count,
            }))
        }
        (
            ApplicationCommand::ShowMemoryEntryVersion {
                selector,
                display_key,
                version,
            },
            ApplicationEvent::MemoryEntryVersionShown { profile, entry },
        ) => {
            let selected = resolve_memory_profile(&projection_at_event.agent_profiles, selector)
                .map_err(|_| invalid_receipt())?;
            let exact = projection_at_event
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| invalid_receipt())?;
            let key = NormalizedMemoryKey::new(display_key).map_err(|_| invalid_receipt())?;
            if selected != exact
                || exact.memory_namespace_id() != entry.namespace_id()
                || &key != entry.normalized_key()
                || *version != entry.version()
            {
                return Err(invalid_receipt());
            }
            let stored = MemoryRepository::load_entry_version(
                tx,
                entry.namespace_id(),
                entry.normalized_key(),
                entry.version(),
            )?
            .ok_or_else(invalid_receipt)?;
            if stored.reference() != *entry {
                return Err(invalid_receipt());
            }
            Ok(CommandView::MemoryEntryVersion(MemoryEntryVersionView {
                profile: profile.clone(),
                entry: stored,
            }))
        }
        (
            ApplicationCommand::ListMemoryProposals { selector, filter },
            ApplicationEvent::MemoryProposalsListed {
                profile,
                filter: event_filter,
                proposals,
                total_count,
                returned_count,
                omitted_count,
            },
        ) => {
            let selected = resolve_memory_profile(&projection_at_event.agent_profiles, selector)
                .map_err(|_| invalid_receipt())?;
            let exact = projection_at_event
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| invalid_receipt())?;
            if selected != exact
                || filter != event_filter
                || u64::try_from(proposals.len()).ok() != Some(*returned_count)
                || returned_count.checked_add(*omitted_count) != Some(*total_count)
            {
                return Err(invalid_receipt());
            }
            let mut summaries = Vec::with_capacity(proposals.len());
            for reference in proposals {
                let (proposal, _, _) =
                    MemoryRepository::load_proposal(tx, reference.proposal.proposal_id())?
                        .ok_or_else(invalid_receipt)?;
                if proposal.reference() != reference.proposal
                    || proposal.namespace_id() != exact.memory_namespace_id()
                    || (*event_filter == crate::memory::MemoryProposalFilter::Pending
                        && reference.status != MemoryProposalStatus::Pending)
                {
                    return Err(invalid_receipt());
                }
                summaries.push(MemoryProposalSummary {
                    proposal: proposal.reference(),
                    proposer: proposal.proposer().clone(),
                    operation: match proposal.operation() {
                        MemoryProposalOperation::Set { .. } => MemoryProposalOperationKind::Set,
                        MemoryProposalOperation::Delete => MemoryProposalOperationKind::Delete,
                    },
                    display_key: proposal.display_key().to_owned(),
                    status: reference.status,
                    created_at_ms: proposal.created_at_ms(),
                });
            }
            Ok(CommandView::MemoryProposals(MemoryProposalsView {
                profile: profile.clone(),
                namespace_id: exact.memory_namespace_id(),
                filter: *event_filter,
                proposals: summaries,
                total_count: *total_count,
                returned_count: *returned_count,
                omitted_count: *omitted_count,
            }))
        }
        (
            ApplicationCommand::ShowMemoryProposal { proposal_id },
            ApplicationEvent::MemoryProposalShown {
                proposal: proposal_ref,
                status,
                resolution,
            },
        ) => {
            if *proposal_id != proposal_ref.proposal_id() {
                return Err(invalid_receipt());
            }
            let (proposal, _, _) =
                MemoryRepository::load_proposal(tx, *proposal_id)?.ok_or_else(invalid_receipt)?;
            if proposal.reference() != *proposal_ref {
                return Err(invalid_receipt());
            }
            let proposer = projection_at_event
                .agent_profiles
                .resolve_reference(proposal.proposer())
                .map_err(|_| invalid_receipt())?;
            let owner = projection_at_event
                .agent_profiles
                .active_profiles()
                .into_iter()
                .find(|profile| profile.memory_namespace_id() == proposal.namespace_id())
                .ok_or_else(invalid_receipt)?;
            let current_entry = match projection_at_event
                .memory
                .current_entry(proposal.namespace_id(), proposal.normalized_key())
            {
                None => ExpectedMemoryEntryState::Absent,
                Some(entry) if entry.state() == MemoryEntryState::Present => {
                    ExpectedMemoryEntryState::Present(entry.clone())
                }
                Some(entry) => ExpectedMemoryEntryState::Deleted(entry.clone()),
            };
            let proposer_is_historical = projection_at_event
                .agent_profiles
                .active_profile(proposer.profile_id())
                .is_none_or(|active| active.reference() != *proposal.proposer());
            Ok(CommandView::MemoryProposal(MemoryProposalView {
                proposal,
                status: *status,
                resolution: resolution.clone(),
                current_entry,
                proposer_is_historical,
                proposer_identity: memory_profile_identity(proposer),
                namespace_owner_identity: memory_profile_identity(&owner),
            }))
        }
        (
            ApplicationCommand::ListEpisodicSummaries { selector },
            ApplicationEvent::EpisodicSummariesListed {
                profile,
                summaries,
                total_count,
                returned_count,
                omitted_count,
            },
        ) => {
            let selected = resolve_memory_profile(&projection_at_event.agent_profiles, selector)
                .map_err(|_| invalid_receipt())?;
            let exact = projection_at_event
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| invalid_receipt())?;
            if selected != exact
                || u64::try_from(summaries.len()).ok() != Some(*returned_count)
                || returned_count.checked_add(*omitted_count) != Some(*total_count)
            {
                return Err(invalid_receipt());
            }
            let mut items = Vec::with_capacity(summaries.len());
            for reference in summaries {
                let summary = MemoryRepository::load_episodic_summary(tx, reference.summary_id())?
                    .ok_or_else(invalid_receipt)?;
                if summary.reference() != *reference
                    || reference.profile() != profile
                    || reference.namespace_id() != exact.memory_namespace_id()
                {
                    return Err(invalid_receipt());
                }
                items.push(EpisodicSummaryListItem {
                    summary: reference.clone(),
                    label: summary.label().to_owned(),
                    purpose_tags: summary.purpose_tags().to_vec(),
                    source_count: u64::try_from(summary.sources().len())
                        .map_err(|_| invalid_receipt())?,
                    created_at_ms: summary.created_at_ms(),
                });
            }
            Ok(CommandView::EpisodicSummaries(EpisodicSummariesView {
                profile: profile.clone(),
                namespace_id: exact.memory_namespace_id(),
                summaries: items,
                total_count: *total_count,
                returned_count: *returned_count,
                omitted_count: *omitted_count,
            }))
        }
        (
            ApplicationCommand::ShowEpisodicSummary { summary_id },
            ApplicationEvent::EpisodicSummaryShown { summary: reference },
        ) => {
            if *summary_id != reference.summary_id() {
                return Err(invalid_receipt());
            }
            projection_at_event
                .agent_profiles
                .resolve_reference(reference.profile())
                .map_err(|_| invalid_receipt())?;
            let summary = MemoryRepository::load_episodic_summary(tx, *summary_id)?
                .ok_or_else(invalid_receipt)?;
            if summary.reference() != *reference {
                return Err(invalid_receipt());
            }
            Ok(CommandView::EpisodicSummary(EpisodicSummaryView {
                summary,
                qualification: EpisodicQualification::SummaryVerifySources,
            }))
        }
        (
            ApplicationCommand::BuildMemorySnapshot { request },
            ApplicationEvent::MemorySnapshotBuilt { metadata },
        ) => {
            if request.scope() != metadata.scope() || request.budget() != metadata.budget() {
                return Err(invalid_receipt());
            }
            let profile = projection_at_event
                .agent_profiles
                .resolve_reference(metadata.scope().profile())
                .map_err(|_| invalid_receipt())?;
            metadata
                .scope()
                .validate_against(profile)
                .map_err(|_| invalid_receipt())?;
            let mut entries = Vec::with_capacity(metadata.entry_refs().len());
            for reference in metadata.entry_refs() {
                let entry = MemoryRepository::load_entry_version(
                    tx,
                    reference.namespace_id(),
                    reference.normalized_key(),
                    reference.version(),
                )?
                .ok_or_else(invalid_receipt)?;
                if entry.reference() != *reference {
                    return Err(invalid_receipt());
                }
                entries.push(MemoryKvContextItem::from_entry(&entry)?);
            }
            let mut summaries = Vec::with_capacity(metadata.summary_refs().len());
            for reference in metadata.summary_refs() {
                let summary = MemoryRepository::load_episodic_summary(tx, reference.summary_id())?
                    .ok_or_else(invalid_receipt)?;
                if summary.reference() != *reference {
                    return Err(invalid_receipt());
                }
                summaries.push(EpisodicContextItem::from_summary(&summary)?);
            }
            let snapshot = MemorySnapshot::replay(metadata.clone(), entries, summaries)
                .map_err(|_| invalid_receipt())?;
            Ok(CommandView::MemorySnapshot(MemorySnapshotView { snapshot }))
        }
        (
            ApplicationCommand::ProposeMemoryMutation {
                proposer,
                expected,
                operation,
                rationale,
            },
            ApplicationEvent::MemoryProposalCreated { proposal, approval },
        ) => {
            let exact = projection_at_event
                .agent_profiles
                .resolve_reference(proposer)
                .map_err(|_| invalid_receipt())?;
            if proposal.proposer() != proposer
                || proposal.namespace_id() != exact.memory_namespace_id()
                || proposal.expected() != expected
                || proposal.operation() != operation
                || proposal.rationale() != rationale
                || approval.approval_id() != proposal.approval_id()
                || approval.status() != crate::policy::ApprovalStatus::Pending
            {
                return Err(invalid_receipt());
            }
            let (stored, _, _) =
                MemoryRepository::load_proposal(tx, proposal.reference().proposal_id())?
                    .ok_or_else(invalid_receipt)?;
            if stored != *proposal {
                return Err(invalid_receipt());
            }
            Ok(CommandView::MemoryProposalCreated(
                MemoryProposalCreatedView {
                    proposal: proposal.reference(),
                    approval_id: approval.approval_id(),
                    status: MemoryProposalStatus::Pending,
                },
            ))
        }
        (
            ApplicationCommand::SetMemoryEntry {
                profile,
                expected,
                candidate,
                ..
            },
            ApplicationEvent::MemoryEntrySet {
                entry,
                expired_proposals,
            },
        ) => {
            let exact = projection_at_event
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| invalid_receipt())?;
            if exact.memory_namespace_id() != entry.reference().namespace_id()
                || entry.reference().state() != MemoryEntryState::Present
                || entry.display_key() != candidate.display_key()
                || entry.value() != Some(candidate.value())
                || entry.purpose_tags() != candidate.purpose_tags()
                || entry.reference().normalized_key() != &candidate.normalized_key()
            {
                return Err(invalid_receipt());
            }
            validate_predecessor(tx, entry, expected)?;
            let stored = load_exact_entry(tx, &entry.reference())?;
            if stored != *entry {
                return Err(invalid_receipt());
            }
            Ok(CommandView::MemoryEntryMutation(MemoryEntryMutationView {
                entry: entry.reference(),
                expired_proposals: expired_proposals
                    .iter()
                    .map(|resolution| resolution.proposal().clone())
                    .collect(),
            }))
        }
        (
            ApplicationCommand::DeleteMemoryEntry {
                profile, expected, ..
            },
            ApplicationEvent::MemoryEntryDeleted {
                entry,
                expired_proposals,
            },
        ) => {
            let exact = projection_at_event
                .agent_profiles
                .resolve_reference(profile)
                .map_err(|_| invalid_receipt())?;
            if exact.memory_namespace_id() != entry.reference().namespace_id()
                || entry.reference().state() != MemoryEntryState::Deleted
            {
                return Err(invalid_receipt());
            }
            validate_predecessor(
                tx,
                entry,
                &ExpectedMemoryEntryState::Present(expected.clone()),
            )?;
            let stored = load_exact_entry(tx, &entry.reference())?;
            if stored != *entry {
                return Err(invalid_receipt());
            }
            Ok(CommandView::MemoryEntryMutation(MemoryEntryMutationView {
                entry: entry.reference(),
                expired_proposals: expired_proposals
                    .iter()
                    .map(|resolution| resolution.proposal().clone())
                    .collect(),
            }))
        }
        (
            ApplicationCommand::ApproveMemoryProposal {
                proposal: command_proposal,
                approval_id,
                expected_approval_status,
                expected_entry,
                ..
            },
            ApplicationEvent::MemoryProposalAccepted {
                resolution,
                entry,
                expired_proposals,
            },
        ) => {
            if *expected_approval_status != crate::policy::ApprovalStatus::Pending
                || resolution.proposal() != command_proposal
                || resolution.approval_id() != *approval_id
                || resolution.status() != MemoryProposalStatus::Accepted
                || entry.accepted_proposal() != Some(command_proposal)
            {
                return Err(invalid_receipt());
            }
            let (proposal, status, stored_resolution) =
                MemoryRepository::load_proposal(tx, command_proposal.proposal_id())?
                    .ok_or_else(invalid_receipt)?;
            if proposal.reference() != *command_proposal
                || proposal.approval_id() != *approval_id
                || proposal.expected() != expected_entry
                || status != MemoryProposalStatus::Accepted
                || stored_resolution.as_ref() != Some(resolution)
            {
                return Err(invalid_receipt());
            }
            projection_at_event
                .agent_profiles
                .resolve_reference(proposal.proposer())
                .map_err(|_| invalid_receipt())?;
            validate_predecessor(tx, entry, expected_entry)?;
            match proposal.operation() {
                MemoryProposalOperation::Set { candidate }
                    if entry.reference().state() == MemoryEntryState::Present
                        && entry.display_key() == candidate.display_key()
                        && entry.value() == Some(candidate.value())
                        && entry.purpose_tags() == candidate.purpose_tags() => {}
                MemoryProposalOperation::Delete
                    if entry.reference().state() == MemoryEntryState::Deleted => {}
                _ => return Err(invalid_receipt()),
            }
            let stored_entry = load_exact_entry(tx, &entry.reference())?;
            if stored_entry != *entry {
                return Err(invalid_receipt());
            }
            Ok(CommandView::MemoryProposalResolution(
                MemoryProposalResolutionView {
                    resolution: resolution.clone(),
                    entry: Some(entry.reference()),
                    expired_proposals: expired_proposals
                        .iter()
                        .map(|expired| expired.proposal().clone())
                        .collect(),
                },
            ))
        }
        (
            ApplicationCommand::RejectMemoryProposal {
                proposal: command_proposal,
                approval_id,
                expected_approval_status,
                expected_entry,
                ..
            },
            ApplicationEvent::MemoryProposalRejected { resolution },
        ) => {
            if *expected_approval_status != crate::policy::ApprovalStatus::Pending
                || resolution.proposal() != command_proposal
                || resolution.approval_id() != *approval_id
                || resolution.status() != MemoryProposalStatus::Rejected
            {
                return Err(invalid_receipt());
            }
            let (proposal, status, stored_resolution) =
                MemoryRepository::load_proposal(tx, command_proposal.proposal_id())?
                    .ok_or_else(invalid_receipt)?;
            if proposal.reference() != *command_proposal
                || proposal.approval_id() != *approval_id
                || proposal.expected() != expected_entry
                || status != MemoryProposalStatus::Rejected
                || stored_resolution.as_ref() != Some(resolution)
            {
                return Err(invalid_receipt());
            }
            projection_at_event
                .agent_profiles
                .resolve_reference(proposal.proposer())
                .map_err(|_| invalid_receipt())?;
            Ok(CommandView::MemoryProposalResolution(
                MemoryProposalResolutionView {
                    resolution: resolution.clone(),
                    entry: None,
                    expired_proposals: vec![],
                },
            ))
        }
        _ => Err(invalid_receipt()),
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
    {
        return Err(invalid_receipt());
    }
    if is_memory_command(&request.command) {
        let view = materialize_memory_view(transaction, &request.command, event, projection)?;
        return Ok(CommandOutcome {
            command_id,
            correlation_id: request.correlation_id,
            committed_events: events.to_vec(),
            view,
            shutdown: ShutdownDisposition::Continue,
        });
    }
    match &event.event {
        ApplicationEvent::SkillCreated { skill, .. }
        | ApplicationEvent::SkillVersionActivated { skill, .. } => {
            let accepted = load_skill_version(transaction.transaction(), skill)?
                .ok_or_else(invalid_receipt)?;
            if event.object.as_ref() != Some(&skill_version_object(&accepted)?) {
                return Err(invalid_receipt());
            }
        }
        _ if event.object.is_some() => return Err(invalid_receipt()),
        _ => {}
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
                skill_id,
                candidate,
                ..
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
                || projection.skills.active_skill(*skill_id) != Some(skill)
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
                || projection.skills.active_skill(*skill_id) != Some(skill)
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
            let accepted = load_skill_version(transaction.transaction(), skill)?
                .ok_or_else(invalid_receipt)?;
            let active_matches = match resolve_projected_active_skill(projection, selector) {
                Some(active) => active == skill,
                None => {
                    matches!(provenance, SkillProvenance::BuiltIn { .. })
                        && seeded_skill_selector_matches(selector, skill, display_name)
                }
            };
            if !active_matches
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
            let accepted = load_skill_version(transaction.transaction(), skill)?
                .ok_or_else(invalid_receipt)?;
            let selected_skill_matches = match resolve_projected_active_skill(projection, selector)
            {
                Some(active) => active.skill_id() == accepted.skill_id(),
                None => {
                    matches!(provenance, SkillProvenance::BuiltIn { .. })
                        && seeded_skill_selector_matches(selector, skill, display_name)
                }
            };
            if !selected_skill_matches
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
            (
                CommandView::SkillVersion(view),
                ShutdownDisposition::Continue,
            )
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
            if let Some(selected) = resolve_projected_active_skill(projection, selector) {
                let mut projected_versions = projection
                    .skills
                    .version_metadata()
                    .filter(|(version, _, _, _, _)| version.skill_id() == *skill_id)
                    .map(|(version, _, _, predecessor_version_id, _)| {
                        (version.clone(), predecessor_version_id)
                    })
                    .collect::<Vec<_>>();
                projected_versions.sort_by_key(|(version, _)| version.version());
                let expected_versions = projected_versions
                    .into_iter()
                    .rev()
                    .take(MAX_SKILL_HISTORY_RESULTS)
                    .collect::<Vec<_>>();
                if *selected != *active
                    || active.skill_id() != *skill_id
                    || usize::try_from(*total_count).ok()
                        != Some(
                            projection
                                .skills
                                .version_metadata()
                                .filter(|(version, _, _, _, _)| version.skill_id() == *skill_id)
                                .count(),
                        )
                    || versions
                        .iter()
                        .zip(&expected_versions)
                        .any(|(version, expected)| {
                            version.skill != expected.0
                                || version.predecessor_version_id != expected.1
                        })
                    || usize::try_from(*returned_count).ok() != Some(versions.len())
                    || versions.len() != expected_versions.len()
                    || *returned_count > *total_count
                    || *truncated != (*returned_count < *total_count)
                {
                    return Err(invalid_receipt());
                }
            } else {
                let persisted_active = load_active_skill(transaction.transaction(), *skill_id)?
                    .ok_or_else(invalid_receipt)?;
                let persisted_history = load_skill_history(transaction.transaction(), *skill_id)?;
                let expected_versions = persisted_history
                    .iter()
                    .rev()
                    .take(MAX_SKILL_HISTORY_RESULTS)
                    .collect::<Vec<_>>();
                if !matches!(
                    persisted_active.provenance(),
                    SkillProvenance::BuiltIn { .. }
                ) || persisted_active.reference() != *active
                    || active.skill_id() != *skill_id
                    || !seeded_skill_selector_matches(
                        selector,
                        active,
                        &persisted_active.content().display_name,
                    )
                    || persisted_history
                        .iter()
                        .any(|skill| !matches!(skill.provenance(), SkillProvenance::BuiltIn { .. }))
                    || usize::try_from(*total_count).ok() != Some(persisted_history.len())
                    || versions
                        .iter()
                        .zip(&expected_versions)
                        .any(|(version, expected)| {
                            version.skill != expected.reference()
                                || version.created_at_ms != expected.created_at_ms()
                                || version.predecessor_version_id != expected.predecessor()
                        })
                    || usize::try_from(*returned_count).ok() != Some(versions.len())
                    || versions.len() != expected_versions.len()
                    || *returned_count > *total_count
                    || *truncated != (*returned_count < *total_count)
                {
                    return Err(invalid_receipt());
                }
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
            && expected_active_profile_version_id == previous_profile_version_id =>
        {
            materialize_agent_skill_mutation(
                projection,
                *profile_id,
                *previous_profile_version_id,
                profile,
                CommandView::AgentSkillAssigned,
            )?
        }
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
            && expected_active_profile_version_id == previous_profile_version_id =>
        {
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
            && expected_active_profile_version_id == previous_profile_version_id =>
        {
            materialize_agent_skill_mutation(
                projection,
                *profile_id,
                *previous_profile_version_id,
                profile,
                CommandView::AgentSkillUnassigned,
            )?
        }
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
        Capability::MemoryRead => "memory_read",
        Capability::MemoryPreview => "memory_preview",
        Capability::MemoryMutate => "memory_mutate",
        Capability::MemoryPropose => "memory_propose",
        Capability::MemoryResolve => "memory_resolve",
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
        "memory_read" => Ok(Capability::MemoryRead),
        "memory_preview" => Ok(Capability::MemoryPreview),
        "memory_mutate" => Ok(Capability::MemoryMutate),
        "memory_propose" => Ok(Capability::MemoryPropose),
        "memory_resolve" => Ok(Capability::MemoryResolve),
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

fn seeded_skill_selector_matches(
    selector: &SkillSelector,
    skill: &SkillVersionRef,
    display_name: &str,
) -> bool {
    match selector {
        SkillSelector::Id(skill_id) => *skill_id == skill.skill_id(),
        SkillSelector::Name(name) => {
            SkillSelector::Name(name.clone()).normalized_name()
                == SkillSelector::Name(display_name.to_owned()).normalized_name()
        }
    }
}

fn invalid_receipt() -> AppError {
    AppError::Persistence(PersistenceError::InvalidEventRecord)
}
