use ai_stock_forum::{
    app::{
        ApplicationEvent, EventEnvelope, PendingEvent, ShutdownReason, EVENT_SCHEMA_VERSION,
    },
    config::AppPaths,
    domain::{
        Actor, CorrelationId, EventId, InstallationId, SessionId,
    },
    persistence::{Database, EventRepository, ProjectionRepository, RecoveryError},
    recovery::{reduce, ProjectionState},
    setup::{SetupDraft, SetupPath, SetupStatus},
};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

fn database() -> (TempDir, Database) {
    let temporary_directory = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    (temporary_directory, database)
}

fn pending(event_id: u128, event: ApplicationEvent) -> PendingEvent {
    PendingEvent {
        event_id: EventId::from_uuid(Uuid::from_u128(event_id)),
        event_schema_version: EVENT_SCHEMA_VERSION,
        actor: Actor::System,
        occurred_at_ms: 1_700_000_000_000 + i64::try_from(event_id).unwrap(),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(100 + event_id)),
        causation_id: None,
        object: None,
        event,
    }
}

fn append(database: &mut Database, event_id: u128, event: ApplicationEvent) -> EventEnvelope {
    let transaction = database.immediate_transaction().unwrap();
    let envelope = EventRepository::append(&transaction, pending(event_id, event)).unwrap();
    transaction.commit().unwrap();
    envelope
}

fn installation_id() -> InstallationId {
    InstallationId::from_uuid(Uuid::from_u128(10))
}

fn session_id() -> SessionId {
    SessionId::from_uuid(Uuid::from_u128(20))
}

fn installation_session_events(database: &mut Database) -> Vec<EventEnvelope> {
    vec![
        append(
            database,
            1,
            ApplicationEvent::InstallationInitialized {
                installation_id: installation_id(),
            },
        ),
        append(
            database,
            2,
            ApplicationEvent::ProcessSessionStarted {
                session_id: session_id(),
            },
        ),
        append(
            database,
            3,
            ApplicationEvent::ProcessSessionEnded {
                session_id: session_id(),
                reason: ShutdownReason::UserQuit,
            },
        ),
    ]
}

fn reduce_all(events: &[EventEnvelope]) -> ProjectionState {
    let mut state = ProjectionState::default();
    for event in events {
        reduce(&mut state, event).unwrap();
    }
    state
}

#[test]
fn direct_reduction_rebuilds_installation_and_session_tombstone() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);

    let state = reduce_all(&events);

    assert_eq!(state.installation.as_ref().unwrap().installation_id, installation_id());
    let session = state.sessions.get(&session_id()).unwrap();
    assert_eq!(session.started_event_id, events[1].event_id);
    assert_eq!(session.ended.as_ref().unwrap().reason, ShutdownReason::UserQuit);
    assert_eq!(state.setup_status, SetupStatus::NotStarted);
    assert_eq!(state.last_sequence, 3);
    assert_eq!(state.last_event_digest.as_ref(), Some(&events[2].event_digest));
}

#[test]
fn reducer_rejects_out_of_order_duplicate_illegal_and_future_events() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let mut state = ProjectionState::default();

    assert_eq!(reduce(&mut state, &events[1]), Err(RecoveryError::EventSequenceGap));
    reduce(&mut state, &events[0]).unwrap();
    reduce(&mut state, &events[1]).unwrap();
    assert_eq!(reduce(&mut state, &events[1]), Err(RecoveryError::EventSequenceGap));

    let mut illegal = events[2].clone();
    illegal.event = ApplicationEvent::ProcessSessionEnded {
        session_id: SessionId::from_uuid(Uuid::from_u128(21)),
        reason: ShutdownReason::UserQuit,
    };
    assert_eq!(reduce(&mut state, &illegal), Err(RecoveryError::InvalidEventRecord));

    let mut future = events[2].clone();
    future.event_schema_version = EVENT_SCHEMA_VERSION + 1;
    assert_eq!(reduce(&mut state, &future), Err(RecoveryError::UnsupportedEventSchema));
}

#[test]
fn projections_replay_deterministically_and_store_transactionally_idempotently() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let direct = reduce_all(&events);
    assert_eq!(reduce_all(&events).digest().unwrap(), direct.digest().unwrap());

    let transaction = database.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &direct).unwrap();
    transaction.commit().unwrap();
    let first = ProjectionRepository::load(database.connection()).unwrap();

    let transaction = database.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &direct).unwrap();
    transaction.commit().unwrap();
    let repeated = ProjectionRepository::load(database.connection()).unwrap();
    assert_eq!(repeated.digest().unwrap(), first.digest().unwrap());
    assert_eq!(repeated.digest().unwrap(), direct.digest().unwrap());

    let (installation, created_event, created_at): (String, String, i64) = database
        .connection()
        .query_row(
            "SELECT installation_id, created_event_id, created_at_ms FROM installation_projection",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(installation, installation_id().to_string());
    assert_eq!(created_event, events[0].event_id.to_string());
    assert_eq!(created_at, events[0].occurred_at_ms);

    let (ended_event, ended_at, reason): (Option<String>, Option<i64>, Option<String>) = database
        .connection()
        .query_row(
            "SELECT ended_event_id, ended_at_ms, end_reason FROM process_session_projection WHERE session_id = ?1",
            [session_id().to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(ended_event, Some(events[2].event_id.to_string()));
    assert_eq!(ended_at, Some(events[2].occurred_at_ms));
    assert_eq!(reason.as_deref(), Some("user_quit"));
    let count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM process_session_projection", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn rebuild_writes_the_same_projection_without_changing_event_authority() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let event_digest_before: String = database
        .connection()
        .query_row("SELECT event_digest FROM event_stream WHERE sequence = 3", [], |row| row.get(0))
        .unwrap();

    let rebuilt = ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();

    assert_eq!(rebuilt.digest().unwrap(), reduce_all(&events).digest().unwrap());
    assert_eq!(
        database
            .connection()
            .query_row("SELECT event_digest FROM event_stream WHERE sequence = 3", [], |row| row.get::<_, String>(0))
            .unwrap(),
        event_digest_before
    );
}

#[test]
fn setup_and_projection_deserialization_reject_invalid_states() {
    assert!(serde_json::from_value::<SetupDraft>(json!({
        "draft_id": "00000000-0000-0000-0000-000000000030",
        "schema_version": 0,
        "state": "drafting",
        "path": SetupPath::QuickStart,
        "current_review_digest": null,
        "payload": {},
        "created_at_ms": 1,
        "updated_at_ms": 1
    }))
    .is_err());
    assert!(serde_json::from_value::<ProjectionState>(json!({
        "installation": null,
        "sessions": {},
        "setup_status": "not_started",
        "last_sequence": 0,
        "last_event_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "previous_session_interrupted": false
    }))
    .is_err());
}
