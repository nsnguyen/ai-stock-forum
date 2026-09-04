use std::sync::{Arc, RwLock, RwLockReadGuard};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    app::{
        AppError, ApplicationCommand, ApplicationEvent, AuditTailView, CommandEnvelope,
        CommandOutcome, CommandView, EVENT_SCHEMA_VERSION, HelpView, InputRejectedView,
        PendingEvent, SetupStatusView, ShutdownDisposition, ShutdownReason, ShutdownView,
        StatusView,
    },
    audit::AuditEntry,
    config::{AppPaths, StartupError},
    domain::{
        Actor, CausationId, Clock, CommandId, CorrelationId, EventId, IdGenerator, InstallationId,
        SessionId, Sha256Digest, canonical_json_bytes, sha256,
    },
    persistence::{
        CommandReceiptRecord, CommandReceiptRepository, Database, EventRepository,
        ImmediateTransaction, PersistenceError, ProjectionRepository, RecoveryError,
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
    rules: [PolicyRule; 5],
}

impl Default for PhaseZeroPolicy {
    fn default() -> Self {
        Self {
            rules: [
                PolicyRule::new(Effect::Grant, Capability::HelpRead),
                PolicyRule::new(Effect::Grant, Capability::StatusRead),
                PolicyRule::new(Effect::Grant, Capability::SetupStatusRead),
                PolicyRule::new(Effect::Grant, Capability::AuditRead),
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
        let projection = self.state.projection();
        let events = EventRepository::tail_through(
            self.executor.database.connection(),
            limit,
            projection.last_sequence,
        )?;

        Ok(PresentationSnapshot {
            installation_id: self.state.installation_id(),
            session_id: self.state.session_id(),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: projection.setup_status.clone(),
            recent_audit: events.iter().map(AuditEntry::from_event).collect(),
        })
    }

    pub const fn previous_session_interrupted(&self) -> bool {
        self.state.previous_session_interrupted()
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
        let transaction = self.database.immediate_transaction()?;
        ensure_authoritative_session_open(&transaction, session_id)?;
        let mut projection = ProjectionRepository::load_in(&transaction)?;
        match projection.sessions.get(&session_id) {
            Some(session) if session.ended.is_none() => {}
            _ => return Err(AppError::LifecycleFinished),
        }

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

        let capability = request.command.required_capability();
        let (policy_decision, stored, events) = match self.policy.authorize(capability) {
            AuthorizationDecision::Granted => {
                let pending = PendingEvent {
                    event_id: EventId::from_uuid(self.ids.next_uuid()),
                    event_schema_version: EVENT_SCHEMA_VERSION,
                    actor: request.actor.clone(),
                    occurred_at_ms: self.clock.now_millis(),
                    correlation_id: request.correlation_id,
                    causation_id: Some(CausationId::from_uuid(envelope.command_id.as_uuid())),
                    object: None,
                    event: event_for_command(&request.command),
                };
                let committed = EventRepository::append(&transaction, pending)?;
                reduce(&mut projection, &committed)?;
                ProjectionRepository::store(&transaction, &projection)?;
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
                (
                    StoredPolicyDecision::Granted,
                    StoredExecution::Success { outcome },
                    events,
                )
            }
            AuthorizationDecision::Denied(PolicyDecision::Denied) => (
                StoredPolicyDecision::Denied,
                StoredExecution::CapabilityDenied {
                    capability,
                    decision: PolicyDecision::Denied,
                },
                Vec::new(),
            ),
            AuthorizationDecision::Denied(PolicyDecision::DeniedByDefault) => (
                StoredPolicyDecision::DeniedByDefault,
                StoredExecution::CapabilityDenied {
                    capability,
                    decision: PolicyDecision::DeniedByDefault,
                },
                Vec::new(),
            ),
            AuthorizationDecision::Denied(PolicyDecision::Granted) => {
                return Err(invalid_receipt());
            }
            AuthorizationDecision::ApprovalRequired => (
                StoredPolicyDecision::ApprovalRequired,
                StoredExecution::ApprovalRequired { capability },
                Vec::new(),
            ),
        };
        if events.is_empty() {
            self.hook
                .before_outcome_materialization(transaction.transaction())?;
        }
        let outcome_json = encode_canonical(&stored)?;
        self.hook.before_receipt_write(transaction.transaction())?;
        CommandReceiptRepository::insert(
            &transaction,
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
        transaction.commit()?;
        stored.into_result()
    }
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

fn event_for_command(command: &ApplicationCommand) -> ApplicationEvent {
    match command {
        ApplicationCommand::ShowHelp => ApplicationEvent::HelpViewed,
        ApplicationCommand::ShowStatus => ApplicationEvent::StatusViewed,
        ApplicationCommand::ShowSetupStatus => ApplicationEvent::SetupStatusViewed,
        ApplicationCommand::ShowAuditTail { limit } => {
            ApplicationEvent::AuditTailViewed { limit: *limit }
        }
        ApplicationCommand::RejectInput(rejection) => ApplicationEvent::CommandRejected {
            rejection: rejection.clone(),
        },
        ApplicationCommand::RequestShutdown => ApplicationEvent::ShutdownRequested,
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
