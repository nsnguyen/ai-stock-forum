use std::sync::{Arc, RwLock, RwLockReadGuard};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    agents::{
        AgentProfileDraft, AgentProfileVersion, AgentReadiness, ProfileEditPreview,
        ProfileReviewRegistry, ReviewReservationError,
        candidate_digest, diff_profile, normalize_profile_name_key, profile_template_from_provenance,
        review_digest,
    },
    app::{
        AgentProfileCreatedView, AgentProfileHistoryEntry, AgentProfileHistoryView,
        AgentProfileSummary, AgentProfileVersionActivatedView, AgentProfileView, AgentProfilesView,
        AppError, ApplicationCommand, ApplicationEvent, AuditTailView, CommandEnvelope,
        CommandOutcome, CommandView, EVENT_SCHEMA_VERSION, HelpView, InputRejectedView,
        PendingEvent, SetupStatusView, ShutdownDisposition, ShutdownReason, ShutdownView, StatusView,
    },
    audit::AuditEntry,
    config::{AppPaths, StartupError},
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CausationId, Clock, CommandId,
        CorrelationId, EventId, IdGenerator, InstallationId, MemoryNamespaceId, ProfileReviewToken,
        SessionId, Sha256Digest, canonical_json_bytes, sha256,
    },
    persistence::{
        CommandReceiptRecord, CommandReceiptRepository, Database, EventRepository,
        ImmediateTransaction, PersistenceError, ProjectionRepository, RecoveryError,
        insert_expected_version,
    },
    policy::{Capability, Effect, PolicyDecision, PolicyRule, evaluate},
    recovery::{BootstrapState, ProjectionState, RecoveryCoordinator, reduce},
    setup::SetupStatus,
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
    rules: [PolicyRule; 8],
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
                PolicyRule::new(Effect::Grant, Capability::AgentProfileEdit),
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
        Ok(IndependentApplicationService { executor })
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
        let mut database = Database::open(paths)?;
        let state =
            RecoveryCoordinator::bootstrap(&mut database, clock.as_ref(), ids.as_ref(), &[])?;
        let lifecycle = Arc::new(SharedLifecycle {
            session_id: state.session_id(),
            phase: RwLock::new(LifecyclePhase::Open),
        });
        let reviews = Arc::new(ProfileReviewRegistry::default());
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

    pub fn preview_agent_profile_edit(
        &self,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate: AgentProfileDraft,
    ) -> Result<ProfileEditPreview, AppError> {
        self.executor.preview_agent_profile_edit(
            profile_id,
            expected_active_version_id,
            candidate,
        )
    }

    pub fn cancel_agent_profile_edit(&self) {
        self.executor.reviews.cancel();
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
        let projected_sequence = u64::try_from(projected_sequence)
            .map_err(|_| PersistenceError::InvalidEventRecord)?;
        let projection = ProjectionRepository::load_at(
            self.executor.database.connection(),
            projected_sequence,
        )?;
        let events = EventRepository::tail_through(
            self.executor.database.connection(),
            limit,
            projection.last_sequence,
        )?;
        let active_profiles = projection.agent_profiles.active_profiles();
        let selected_agent_profile = active_profiles.first().map(profile_view);
        let selected_agent_profile_history = active_profiles
            .first()
            .and_then(|profile| profile_history_view(&projection, profile.profile_id()));

        Ok(PresentationSnapshot {
            installation_id: self.state.installation_id(),
            session_id: self.state.session_id(),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: projection.setup_status.clone(),
            recent_audit: events.iter().map(AuditEntry::from_event).collect(),
            agent_profiles: AgentProfilesView {
                profiles: active_profiles.iter().map(profile_summary).collect(),
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
        self.executor.preview_agent_profile_edit(
            profile_id,
            expected_active_version_id,
            candidate,
        )
    }
}

impl ApplicationWorker {
    pub fn execute_user(
        &mut self,
        command: ApplicationCommand,
    ) -> Result<CommandOutcome, AppError> {
        self.executor.execute_user(command)
    }

    pub fn execute(&mut self, envelope: CommandEnvelope) -> Result<CommandOutcome, AppError> {
        self.executor.execute(envelope)
    }
}

impl CommandExecutor {
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
        authorize_passive(self.policy.as_ref(), Capability::AgentProfileEdit)?;
        validate_draft(&candidate)?;
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
        envelope: CommandEnvelope,
        session_id: SessionId,
        _phase: RwLockReadGuard<'_, LifecyclePhase>,
    ) -> Result<CommandOutcome, AppError> {
        let request = CommandRequest::from(&envelope);
        let request_json = encode_canonical(&request)?;
        let command_fingerprint = sha256(request_json.as_bytes());
        let review_operation = if matches!(
            &request.command,
            ApplicationCommand::ActivateAgentProfileVersion { .. }
        ) {
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
            Some(self.reviews.operation())
        } else {
            None
        };
        if matches!(
            &request.command,
            ApplicationCommand::CreateAgentProfile { .. }
                | ApplicationCommand::ActivateAgentProfileVersion { .. }
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
            self.hook
                .after_receipt_store(transaction.transaction())?;
            self.hook.before_commit(transaction.transaction())?;
            transaction.commit()?;
            return stored.into_result();
        }

        let mut reserved = false;
        let precommit = (|| -> Result<(StoredExecution, Vec<crate::app::EventEnvelope>), AppError> {
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
                    review_operation
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
                    reserved = true;
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
                    ensure_name_available(&projection, Some(*profile_id), &candidate.display_name)?;
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
                occurred_at_ms: event_occurred_at(&event).unwrap_or_else(|| self.clock.now_millis()),
                correlation_id: request.correlation_id,
                causation_id: Some(CausationId::from_uuid(envelope.command_id.as_uuid())),
                object: None,
                event,
            };
            let committed = EventRepository::append(&transaction, pending)?;
            self.hook
                .after_event_append(transaction.transaction())?;
            reduce(&mut projection, &committed)?;
            if let ApplicationEvent::AgentProfileCreated { profile }
            | ApplicationEvent::AgentProfileVersionActivated { profile, .. } = &committed.event
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
            let outcome = materialize_success(
                &transaction,
                envelope.command_id,
                &request,
                &events,
                &projection,
            )?;
            self.hook
                .after_audit_append(transaction.transaction())?;
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
            self.hook
                .after_receipt_store(transaction.transaction())?;
            self.hook.before_commit(transaction.transaction())?;
            Ok((stored, events))
        })();

        let (stored, _) = match precommit {
            Ok(result) => result,
            Err(error) => {
                if reserved {
                    review_operation
                        .as_ref()
                        .expect("reserved review has an operation owner")
                        .release(envelope.command_id);
                }
                return Err(error);
            }
        };
        if let Err(error) = transaction.commit() {
            if reserved {
                review_operation
                    .as_ref()
                    .expect("reserved review has an operation owner")
                    .release(envelope.command_id);
            }
            return Err(error.into());
        }
        if reserved {
            review_operation
                .as_ref()
                .expect("reserved review has an operation owner")
                .consume(envelope.command_id);
        }
        if matches!(request.command, ApplicationCommand::RequestShutdown) {
            self.reviews.cancel();
        }
        stored.into_result()
    }
}

fn authorize_passive(policy: &dyn CommandPolicy, capability: Capability) -> Result<(), AppError> {
    match policy.authorize(capability) {
        AuthorizationDecision::Granted => Ok(()),
        AuthorizationDecision::Denied(decision) => {
            Err(AppError::CapabilityDenied { capability, decision })
        }
        AuthorizationDecision::ApprovalRequired => Err(AppError::ApprovalRequired { capability }),
    }
}

fn validate_draft(draft: &AgentProfileDraft) -> Result<(), AppError> {
    for (field, value) in [
        ("display_name", draft.display_name.as_str()),
        ("description", draft.description.as_str()),
        ("primary_specialty", draft.primary_specialty.as_str()),
        ("personality", draft.personality.as_str()),
        ("instructions", draft.instructions.as_str()),
    ] {
        reject_forbidden_profile_tab(field, value)?;
    }
    for tag in &draft.specialty_tags {
        reject_forbidden_profile_tab("specialty_tag", tag)?;
    }
    if let Some(provider) = &draft.bindings.model_provider {
        reject_forbidden_profile_tab("model_provider", provider)?;
    }
    if let Some(model_name) = &draft.bindings.model_name {
        reject_forbidden_profile_tab("model_name", model_name)?;
    }
    AgentProfileDraft::new(
        draft.display_name.clone(),
        draft.description.clone(),
        draft.role,
        draft.primary_specialty.clone(),
        draft.specialty_tags.clone(),
        draft.personality.clone(),
        draft.instructions.clone(),
        draft.bindings.clone(),
        draft.skill_refs.clone(),
        draft.mcp_refs.clone(),
    )?;
    Ok(())
}

fn reject_forbidden_profile_tab(field: &'static str, value: &str) -> Result<(), AppError> {
    if value.contains('\t') {
        Err(crate::domain::DomainError::UnsafeProfileText { field }.into())
    } else {
        Ok(())
    }
}

fn map_profile_projection_write_error(error: PersistenceError) -> AppError {
    match error {
        PersistenceError::AgentProfileHistoryMismatch => AppError::DuplicateProfileName,
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
            let profiles = projection.agent_profiles.active_profiles();
            Ok(ApplicationEvent::AgentProfilesListed {
                result_count: u32::try_from(profiles.len())
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                active_version_ids: profiles
                    .iter()
                    .map(AgentProfileVersion::profile_version_id)
                    .collect(),
            })
        }
        ApplicationCommand::ShowAgentProfile { profile_id } => {
            let profile = projection
                .agent_profiles
                .active_profile(*profile_id)
                .ok_or(AppError::AgentProfileNotFound)?;
            Ok(ApplicationEvent::AgentProfileViewed {
                profile_id: *profile_id,
                active_version_id: profile.profile_version_id(),
            })
        }
        ApplicationCommand::ShowAgentProfileHistory { profile_id } => {
            let profile = projection
                .agent_profiles
                .active_profile(*profile_id)
                .ok_or(AppError::AgentProfileNotFound)?;
            let count = projection.agent_profiles.history(*profile_id).len().min(100);
            Ok(ApplicationEvent::AgentProfileHistoryViewed {
                profile_id: *profile_id,
                result_count: u32::try_from(count)
                    .map_err(|_| PersistenceError::InvalidEventRecord)?,
                active_version_id: profile.profile_version_id(),
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
        ApplicationCommand::ActivateAgentProfileVersion { .. } => Err(invalid_receipt()),
    }
}

fn event_occurred_at(event: &ApplicationEvent) -> Option<i64> {
    match event {
        ApplicationEvent::AgentProfileCreated { profile }
        | ApplicationEvent::AgentProfileVersionActivated { profile, .. } => {
            Some(profile.created_at_ms())
        }
        _ => None,
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
            let expected = materialize_success(
                transaction,
                receipt.command_id,
                &stored_request,
                &events,
                &projection,
            )?;
            if outcome != &expected {
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

fn materialize_success(
    transaction: &ImmediateTransaction<'_>,
    command_id: CommandId,
    request: &CommandRequest,
    events: &[crate::app::EventEnvelope],
    projection: &ProjectionState,
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
                || projection.agent_profiles.active_profile(profile.profile_id()) != Some(profile)
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::AgentProfileCreated(AgentProfileCreatedView {
                    profile_id: profile.profile_id(),
                    profile_version_id: profile.profile_version_id(),
                    version: profile.version(),
                    readiness: profile_readiness(profile),
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
                    readiness: profile_readiness(profile),
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ListAgentProfiles,
            ApplicationEvent::AgentProfilesListed {
                result_count,
                active_version_ids,
            },
        ) => {
            let profiles = projection.agent_profiles.active_profiles();
            let expected_ids = profiles
                .iter()
                .map(AgentProfileVersion::profile_version_id)
                .collect::<Vec<_>>();
            if usize::try_from(*result_count).ok() != Some(profiles.len())
                || &expected_ids != active_version_ids
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::AgentProfiles(AgentProfilesView {
                    profiles: profiles.iter().map(profile_summary).collect(),
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ShowAgentProfile {
                profile_id: requested,
            },
            ApplicationEvent::AgentProfileViewed {
                profile_id: committed,
                active_version_id,
            },
        ) if requested == committed => {
            let profile = projection
                .agent_profiles
                .active_profile(*requested)
                .ok_or_else(invalid_receipt)?;
            if profile.profile_version_id() != *active_version_id {
                return Err(invalid_receipt());
            }
            (
                CommandView::AgentProfile(AgentProfileView {
                    profile: profile.clone(),
                    readiness: profile_readiness(profile),
                }),
                ShutdownDisposition::Continue,
            )
        }
        (
            ApplicationCommand::ShowAgentProfileHistory {
                profile_id: requested,
            },
            ApplicationEvent::AgentProfileHistoryViewed {
                profile_id: committed,
                result_count,
                active_version_id,
            },
        ) if requested == committed => {
            let active = projection
                .agent_profiles
                .active_profile(*requested)
                .ok_or_else(invalid_receipt)?;
            let versions = projection
                .agent_profiles
                .history(*requested)
                .into_iter()
                .rev()
                .take(100)
                .map(|profile| AgentProfileHistoryEntry {
                    profile_version_id: profile.profile_version_id(),
                    version: profile.version(),
                    supersedes: profile.supersedes(),
                    created_at_ms: profile.created_at_ms(),
                    readiness: profile_readiness(&profile),
                    content_digest: profile.content_digest().clone(),
                })
                .collect::<Vec<_>>();
            if active.profile_version_id() != *active_version_id
                || usize::try_from(*result_count).ok() != Some(versions.len())
            {
                return Err(invalid_receipt());
            }
            (
                CommandView::AgentProfileHistory(AgentProfileHistoryView {
                    profile_id: *requested,
                    active_version_id: *active_version_id,
                    versions,
                }),
                ShutdownDisposition::Continue,
            )
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

fn profile_readiness(profile: &AgentProfileVersion) -> AgentReadiness {
    match (&profile.bindings().model_provider, &profile.bindings().model_name) {
        (Some(_), Some(_)) => AgentReadiness::Ready,
        _ => AgentReadiness::NotReady,
    }
}

fn profile_view(profile: &AgentProfileVersion) -> AgentProfileView {
    AgentProfileView {
        profile: profile.clone(),
        readiness: profile_readiness(profile),
    }
}

fn profile_history_view(
    projection: &ProjectionState,
    profile_id: AgentProfileId,
) -> Option<AgentProfileHistoryView> {
    let active = projection.agent_profiles.active_profile(profile_id)?;
    let versions = projection
        .agent_profiles
        .history(profile_id)
        .into_iter()
        .rev()
        .take(100)
        .map(|profile| AgentProfileHistoryEntry {
            profile_version_id: profile.profile_version_id(),
            version: profile.version(),
            supersedes: profile.supersedes(),
            created_at_ms: profile.created_at_ms(),
            readiness: profile_readiness(&profile),
            content_digest: profile.content_digest().clone(),
        })
        .collect();
    Some(AgentProfileHistoryView {
        profile_id,
        active_version_id: active.profile_version_id(),
        versions,
    })
}

fn profile_summary(profile: &AgentProfileVersion) -> AgentProfileSummary {
    AgentProfileSummary {
        profile_id: profile.profile_id(),
        profile_version_id: profile.profile_version_id(),
        version: profile.version(),
        display_name: profile.display_name().to_owned(),
        role: profile.role(),
        primary_specialty: profile.primary_specialty().to_owned(),
        readiness: profile_readiness(profile),
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
        Capability::AgentProfileEdit => "agent_profile_edit",
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
        "agent_profile_edit" => Ok(Capability::AgentProfileEdit),
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
