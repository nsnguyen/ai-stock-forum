use std::sync::Arc;

use crate::{
    app::{
        AppError, ApplicationCommand, ApplicationEvent, AuditTailView, CommandEnvelope,
        CommandOutcome, CommandView, HelpView, InputRejectedView, PendingEvent, SetupStatusView,
        ShutdownDisposition, ShutdownReason, ShutdownView, StatusView, EVENT_SCHEMA_VERSION,
    },
    audit::AuditEntry,
    config::{AppPaths, StartupError},
    domain::{
        Actor, CausationId, Clock, CommandId, CorrelationId, EventId, IdGenerator,
    },
    persistence::{Database, EventRepository, ProjectionRepository, RecoveryError},
    policy::{evaluate, Capability, Effect, PolicyDecision, PolicyRule},
    recovery::{reduce, BootstrapState, ProjectionState, RecoveryCoordinator},
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

pub struct ApplicationService {
    database: Database,
    state: BootstrapState,
    clock: Arc<dyn Clock>,
    ids: Arc<dyn IdGenerator>,
    policy: Arc<dyn CommandPolicy>,
}

impl ApplicationService {
    pub fn bootstrap(
        paths: &AppPaths,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
    ) -> Result<Self, StartupError> {
        Self::bootstrap_with_policy(paths, clock, ids, Arc::new(PhaseZeroPolicy::default()))
    }

    #[doc(hidden)]
    pub fn bootstrap_with_policy(
        paths: &AppPaths,
        clock: Arc<dyn Clock>,
        ids: Arc<dyn IdGenerator>,
        policy: Arc<dyn CommandPolicy>,
    ) -> Result<Self, StartupError> {
        let mut database = Database::open(paths)?;
        let state = RecoveryCoordinator::bootstrap(
            &mut database,
            clock.as_ref(),
            ids.as_ref(),
            &[],
        )?;
        Ok(Self {
            database,
            state,
            clock,
            ids,
            policy,
        })
    }

    pub fn execute_user(
        &mut self,
        command: ApplicationCommand,
    ) -> Result<CommandOutcome, AppError> {
        self.execute(CommandEnvelope {
            command_id: CommandId::from_uuid(self.ids.next_uuid()),
            correlation_id: CorrelationId::from_uuid(self.ids.next_uuid()),
            actor: Actor::Human,
            command,
        })
    }

    pub fn execute(&mut self, envelope: CommandEnvelope) -> Result<CommandOutcome, AppError> {
        let command_event = event_for_command(&envelope.command);
        let causation_id = CausationId::from_uuid(envelope.command_id.as_uuid());
        let transaction = self.database.immediate_transaction()?;

        if let Some(existing) = EventRepository::load_by_causation_id(&transaction, causation_id)? {
            if !matches_request(&existing, &envelope, &command_event) {
                return Err(AppError::CommandConflict);
            }
            transaction.commit()?;
            return self.outcome_from_event(envelope.command_id, existing);
        }

        let capability = envelope.command.required_capability();
        match self.policy.authorize(capability) {
            AuthorizationDecision::Granted => {}
            AuthorizationDecision::Denied(decision) => {
                return Err(AppError::CapabilityDenied {
                    capability,
                    decision,
                });
            }
            AuthorizationDecision::ApprovalRequired => {
                return Err(AppError::ApprovalRequired { capability });
            }
        }

        let mut projection = ProjectionRepository::load_in(&transaction)?;
        let pending = PendingEvent {
            event_id: EventId::from_uuid(self.ids.next_uuid()),
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: envelope.actor,
            occurred_at_ms: self.clock.now_millis(),
            correlation_id: envelope.correlation_id,
            causation_id: Some(causation_id),
            object: None,
            event: command_event,
        };
        let committed = EventRepository::append(&transaction, pending)?;
        reduce(&mut projection, &committed)?;
        ProjectionRepository::store(&transaction, &projection)?;
        transaction.commit()?;

        self.outcome_from_event(envelope.command_id, committed)
    }

    pub fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        RecoveryCoordinator::finish_session(
            &mut self.database,
            &mut self.state,
            reason,
            self.clock.as_ref(),
            self.ids.as_ref(),
        )?;
        Ok(())
    }

    pub const fn installation_id(&self) -> crate::domain::InstallationId {
        self.state.installation_id()
    }

    pub const fn session_id(&self) -> crate::domain::SessionId {
        self.state.session_id()
    }

    fn outcome_from_event(
        &self,
        command_id: CommandId,
        event: crate::app::EventEnvelope,
    ) -> Result<CommandOutcome, AppError> {
        let projection = ProjectionRepository::load_at(self.database.connection(), event.sequence)?;
        let (view, shutdown) = self.view_from_event(&event, &projection)?;
        Ok(CommandOutcome {
            command_id,
            correlation_id: event.correlation_id,
            committed_events: vec![event],
            view,
            shutdown,
        })
    }

    fn view_from_event(
        &self,
        event: &crate::app::EventEnvelope,
        projection: &ProjectionState,
    ) -> Result<(CommandView, ShutdownDisposition), AppError> {
        let result = match &event.event {
            ApplicationEvent::HelpViewed => (
                CommandView::Help(HelpView),
                ShutdownDisposition::Continue,
            ),
            ApplicationEvent::StatusViewed => {
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
            ApplicationEvent::SetupStatusViewed => (
                CommandView::SetupStatus(SetupStatusView {
                    status: projection.setup_status.clone(),
                }),
                ShutdownDisposition::Continue,
            ),
            ApplicationEvent::AuditTailViewed { limit } => {
                let entries = EventRepository::tail_through(
                    self.database.connection(),
                    *limit,
                    event.sequence,
                )?
                .iter()
                .map(AuditEntry::from_event)
                .collect();
                (
                    CommandView::AuditTail(AuditTailView {
                        limit: *limit,
                        entries,
                    }),
                    ShutdownDisposition::Continue,
                )
            }
            ApplicationEvent::CommandRejected { rejection } => (
                CommandView::InputRejected(InputRejectedView {
                    rejection: rejection.clone(),
                }),
                ShutdownDisposition::Continue,
            ),
            ApplicationEvent::ShutdownRequested => (
                CommandView::Shutdown(ShutdownView {
                    disposition: ShutdownDisposition::Requested,
                }),
                ShutdownDisposition::Requested,
            ),
            _ => return Err(RecoveryError::InvalidEventRecord.into()),
        };
        Ok(result)
    }
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

fn matches_request(
    existing: &crate::app::EventEnvelope,
    request: &CommandEnvelope,
    command_event: &ApplicationEvent,
) -> bool {
    existing.actor == request.actor
        && existing.correlation_id == request.correlation_id
        && existing.causation_id
            == Some(CausationId::from_uuid(request.command_id.as_uuid()))
        && existing.object.is_none()
        && &existing.event == command_event
}
