use crate::{
    app::{
        AppError, ApplicationEvent, EVENT_SCHEMA_VERSION, EventEnvelope, PendingEvent,
        ShutdownReason,
    },
    config::{ProcessGuard, StartupError},
    domain::{Actor, Clock, CorrelationId, EventId, IdGenerator, InstallationId, SessionId},
    persistence::{
        Database, EventRepository, ImmediateTransaction, PersistenceError, ProjectionRepository,
        RecoveryError,
    },
};

use super::{ProjectionState, reduce};

pub trait RecoveryHook: Send + Sync {
    fn recover(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        interrupted_session: SessionId,
    ) -> Result<(), RecoveryError>;
}

pub struct NoopRecoveryHook;

impl RecoveryHook for NoopRecoveryHook {
    fn recover(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
        _interrupted_session: SessionId,
    ) -> Result<(), RecoveryError> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct BootstrapState {
    installation_id: InstallationId,
    session_id: SessionId,
    projection: ProjectionState,
    previous_session_interrupted: bool,
    process_guard: ProcessGuard,
}

impl BootstrapState {
    pub const fn installation_id(&self) -> InstallationId {
        self.installation_id
    }

    pub const fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn projection(&self) -> &ProjectionState {
        &self.projection
    }

    pub const fn previous_session_interrupted(&self) -> bool {
        self.previous_session_interrupted
    }

    pub const fn process_guard(&self) -> &ProcessGuard {
        &self.process_guard
    }
}

pub struct RecoveryCoordinator;

impl RecoveryCoordinator {
    pub fn bootstrap(
        database: &mut Database,
        clock: &dyn Clock,
        ids: &dyn IdGenerator,
        hooks: &[Box<dyn RecoveryHook>],
    ) -> Result<BootstrapState, StartupError> {
        let process_guard = database.acquire_process_guard()?;
        EventRepository::verify(database.connection()).map_err(startup_from_recovery)?;
        let events =
            EventRepository::load_all(database.connection()).map_err(startup_from_recovery)?;
        let mut state = match ProjectionRepository::load(database.connection()) {
            Ok(state) => state,
            Err(RecoveryError::InvalidEventRecord) => {
                rebuild_and_record(database, &events, clock, ids)?
            }
            Err(error) => return Err(startup_from_recovery(error)),
        };

        if state.installation.is_none() {
            append_and_commit(
                database,
                &mut state,
                ApplicationEvent::InstallationInitialized {
                    installation_id: InstallationId::from_uuid(ids.next_uuid()),
                },
                clock,
                ids,
            )?;
        }

        let previous_session_interrupted = state
            .sessions
            .values()
            .find(|session| session.ended.is_none())
            .map(|session| session.session_id);
        if let Some(interrupted_session) = previous_session_interrupted {
            let transaction = database
                .immediate_transaction()
                .map_err(startup_from_persistence)?;
            let mut next = state.clone();
            append_and_project(
                &transaction,
                &mut next,
                ApplicationEvent::PreviousSessionInterrupted {
                    session_id: interrupted_session,
                },
                clock,
                ids,
            )
            .map_err(startup_from_persistence)?;
            for hook in hooks {
                hook.recover(transaction.transaction(), interrupted_session)
                    .map_err(startup_from_recovery)?;
            }
            transaction.commit().map_err(startup_from_persistence)?;
            state = next;
        }

        let session_id = SessionId::from_uuid(ids.next_uuid());
        append_and_commit(
            database,
            &mut state,
            ApplicationEvent::ProcessSessionStarted { session_id },
            clock,
            ids,
        )?;
        let installation_id = state
            .installation
            .as_ref()
            .ok_or(StartupError::EventStreamRecovery(
                RecoveryError::InvalidEventRecord,
            ))?
            .installation_id;
        Ok(BootstrapState {
            installation_id,
            session_id,
            projection: state,
            previous_session_interrupted: previous_session_interrupted.is_some(),
            process_guard,
        })
    }

    pub fn finish_session(
        database: &mut Database,
        state: &mut BootstrapState,
        reason: ShutdownReason,
        clock: &dyn Clock,
        ids: &dyn IdGenerator,
    ) -> Result<Vec<EventEnvelope>, AppError> {
        let session_id = state.session_id;
        let transaction = database.immediate_transaction()?;
        let mut next = ProjectionRepository::load_in(&transaction)?;
        let session = next
            .sessions
            .get(&session_id)
            .ok_or(RecoveryError::InvalidEventRecord)?;
        if let Some(end) = &session.ended {
            let existing = EventRepository::load_all(transaction.transaction())?
                .into_iter()
                .find(|event| event.event_id == end.ended_event_id)
                .ok_or(RecoveryError::InvalidEventRecord)?;
            transaction.commit()?;
            state.projection = next;
            return Ok(vec![existing]);
        }

        let event = append_and_project(
            &transaction,
            &mut next,
            ApplicationEvent::ProcessSessionEnded { session_id, reason },
            clock,
            ids,
        )?;
        transaction.commit()?;
        state.projection = next;
        Ok(vec![event])
    }
}

fn rebuild_and_record(
    database: &mut Database,
    events: &[EventEnvelope],
    clock: &dyn Clock,
    ids: &dyn IdGenerator,
) -> Result<ProjectionState, StartupError> {
    let transaction = database
        .immediate_transaction()
        .map_err(startup_from_persistence)?;
    let mut state =
        ProjectionRepository::rebuild_in(&transaction, events).map_err(startup_from_recovery)?;
    let through_sequence = state.last_sequence;
    append_and_project(
        &transaction,
        &mut state,
        ApplicationEvent::ProjectionRebuilt { through_sequence },
        clock,
        ids,
    )
    .map_err(startup_from_persistence)?;
    transaction.commit().map_err(startup_from_persistence)?;
    Ok(state)
}

fn append_and_commit(
    database: &mut Database,
    state: &mut ProjectionState,
    event: ApplicationEvent,
    clock: &dyn Clock,
    ids: &dyn IdGenerator,
) -> Result<EventEnvelope, StartupError> {
    let transaction = database
        .immediate_transaction()
        .map_err(startup_from_persistence)?;
    let mut next = state.clone();
    let envelope = append_and_project(&transaction, &mut next, event, clock, ids)
        .map_err(startup_from_persistence)?;
    transaction.commit().map_err(startup_from_persistence)?;
    *state = next;
    Ok(envelope)
}

fn append_and_project(
    transaction: &ImmediateTransaction<'_>,
    state: &mut ProjectionState,
    event: ApplicationEvent,
    clock: &dyn Clock,
    ids: &dyn IdGenerator,
) -> Result<EventEnvelope, PersistenceError> {
    let envelope = EventRepository::append(
        transaction,
        PendingEvent {
            event_id: EventId::from_uuid(ids.next_uuid()),
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::System,
            occurred_at_ms: clock.now_millis(),
            correlation_id: CorrelationId::from_uuid(ids.next_uuid()),
            causation_id: None,
            object: None,
            event,
        },
    )?;
    reduce(state, &envelope).map_err(|_| PersistenceError::ProjectionStateConflict)?;
    ProjectionRepository::store(transaction, state)?;
    Ok(envelope)
}

fn startup_from_recovery(error: RecoveryError) -> StartupError {
    StartupError::EventStreamRecovery(error)
}

fn startup_from_persistence(error: PersistenceError) -> StartupError {
    match error {
        PersistenceError::ProjectionStateConflict => {
            StartupError::EventStreamRecovery(RecoveryError::InvalidEventRecord)
        }
        other => StartupError::Persistence(other),
    }
}
