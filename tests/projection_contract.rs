use ai_stock_forum::{
    app::{
        ApplicationEvent, AuditLimit, EventEnvelope, InputRejection, InputRejectionCategory,
        PendingEvent, SafeToken, ShutdownReason, EVENT_SCHEMA_VERSION,
    },
    config::AppPaths,
    domain::{
        sha256, Actor, ConfigurationVersionId, CorrelationId, EventId, InstallationId,
        ObjectVersion, SessionId, SetupDraftId,
    },
    persistence::{Database, EventRepository, PersistenceError, ProjectionRepository, RecoveryError},
    recovery::{reduce, ProjectionState, ReducerEffect},
    recovery::{InstallationProjection, SessionEndProjection, SessionProjection},
    setup::{
        CapabilityReadiness, CapabilityReadinessStatus, InstallationConfigurationVersion,
        SetupDraft, SetupDraftState, SetupPath, SetupStatus, SetupStepOutcome, SetupStepStatus,
    },
};
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};
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

fn second_session_id() -> SessionId {
    SessionId::from_uuid(Uuid::from_u128(21))
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

fn all_event_variants(database: &mut Database) -> Vec<EventEnvelope> {
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
        append(database, 3, ApplicationEvent::HelpViewed),
        append(database, 4, ApplicationEvent::StatusViewed),
        append(database, 5, ApplicationEvent::SetupStatusViewed),
        append(
            database,
            6,
            ApplicationEvent::AuditTailViewed {
                limit: AuditLimit::new(1).unwrap(),
            },
        ),
        append(
            database,
            7,
            ApplicationEvent::CommandRejected {
                rejection: InputRejection::from_input(
                    InputRejectionCategory::Unknown,
                    Some(SafeToken::new("/unknown").unwrap()),
                    b"/unknown secret",
                ),
            },
        ),
        append(database, 8, ApplicationEvent::ShutdownRequested),
        append(
            database,
            9,
            ApplicationEvent::ProcessSessionEnded {
                session_id: session_id(),
                reason: ShutdownReason::InputClosed,
            },
        ),
        append(
            database,
            10,
            ApplicationEvent::ProcessSessionStarted {
                session_id: second_session_id(),
            },
        ),
        append(
            database,
            11,
            ApplicationEvent::PreviousSessionInterrupted {
                session_id: second_session_id(),
            },
        ),
        append(
            database,
            12,
            ApplicationEvent::ProjectionRebuilt {
                through_sequence: 11,
            },
        ),
    ]
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

#[test]
fn reducer_accepts_every_valid_event_variant_without_startup_local_history() {
    let (_temporary_directory, mut database) = database();
    let events = all_event_variants(&mut database);

    let direct = reduce_all(&events);
    let rebuilt = ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();

    assert_eq!(direct, rebuilt);
    assert_eq!(direct.last_sequence, 12);
    assert_eq!(
        direct.sessions.get(&second_session_id()).unwrap().ended.as_ref().unwrap().reason,
        ShutdownReason::Interrupted
    );
    assert_eq!(ProjectionRepository::load(database.connection()).unwrap(), direct);
}

#[test]
fn reducer_rejects_a_second_open_session_and_leaves_state_unchanged() {
    let (_temporary_directory, mut database) = database();
    let initialized = append(
        &mut database,
        1,
        ApplicationEvent::InstallationInitialized {
            installation_id: installation_id(),
        },
    );
    let started = append(
        &mut database,
        2,
        ApplicationEvent::ProcessSessionStarted {
            session_id: session_id(),
        },
    );
    let second_start = append(
        &mut database,
        3,
        ApplicationEvent::ProcessSessionStarted {
            session_id: second_session_id(),
        },
    );
    let mut state = ProjectionState::default();
    reduce(&mut state, &initialized).unwrap();
    reduce(&mut state, &started).unwrap();
    let before = state.clone();

    assert_eq!(reduce(&mut state, &second_start), Err(RecoveryError::InvalidEventRecord));
    assert_eq!(state, before);
}

#[test]
fn reducer_reports_sequence_overflow_with_a_typed_error() {
    let (_temporary_directory, mut database) = database();
    let mut event = append(&mut database, 1, ApplicationEvent::HelpViewed);
    let marker = sha256(b"overflow-marker");
    event.sequence = u64::MAX;
    event.previous_event_digest = Some(marker.clone());
    let mut state = ProjectionState {
        last_sequence: u64::MAX,
        last_event_digest: Some(marker),
        ..ProjectionState::default()
    };

    let error = reduce(&mut state, &event).unwrap_err();
    assert_eq!(error.code(), "event_sequence_overflow");
}

#[test]
fn store_rejects_stale_fabricated_and_immutable_projection_overwrites() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let direct = reduce_all(&events);
    let transaction = database.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &direct).unwrap();
    transaction.commit().unwrap();

    let stale = reduce_all(&events[..2]);
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(ProjectionRepository::store(&transaction, &stale), Err(PersistenceError::ProjectionStateConflict));
    transaction.rollback().unwrap();

    let mut changed_start = direct.clone();
    changed_start.sessions.get_mut(&session_id()).unwrap().started_at_ms += 1;
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(ProjectionRepository::store(&transaction, &changed_start), Err(PersistenceError::ProjectionStateConflict));
    transaction.rollback().unwrap();

    let mut removed_tombstone = direct.clone();
    removed_tombstone.sessions.get_mut(&session_id()).unwrap().ended = None;
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(ProjectionRepository::store(&transaction, &removed_tombstone), Err(PersistenceError::ProjectionStateConflict));
    transaction.rollback().unwrap();

    let mut rewritten_installation = direct.clone();
    rewritten_installation.installation.as_mut().unwrap().created_at_ms += 1;
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(ProjectionRepository::store(&transaction, &rewritten_installation), Err(PersistenceError::ProjectionStateConflict));
    transaction.rollback().unwrap();

    let mut modified_tombstone = direct.clone();
    modified_tombstone.sessions.get_mut(&session_id()).unwrap().ended.as_mut().unwrap().ended_at_ms += 1;
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(ProjectionRepository::store(&transaction, &modified_tombstone), Err(PersistenceError::ProjectionStateConflict));
    transaction.rollback().unwrap();

    let next_event = append(&mut database, 4, ApplicationEvent::HelpViewed);
    let mut newer = direct.clone();
    reduce(&mut newer, &next_event).unwrap();
    let transaction = database.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &newer).unwrap();
    transaction.commit().unwrap();
    assert_eq!(ProjectionRepository::load(database.connection()).unwrap(), newer);

}

#[test]
fn load_rejects_partial_projection_rows_instead_of_treating_them_as_empty() {
    let (_temporary_directory, mut database) = database();
    let initialized = append(
        &mut database,
        1,
        ApplicationEvent::InstallationInitialized {
            installation_id: installation_id(),
        },
    );
    database
        .connection()
        .execute(
            "INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (1, ?1, ?2, ?3)",
            (installation_id().to_string(), initialized.event_id.to_string(), initialized.occurred_at_ms),
        )
        .unwrap();

    assert_eq!(
        ProjectionRepository::load(database.connection()),
        Err(RecoveryError::InvalidEventRecord)
    );
}

#[test]
fn setup_and_nested_projection_models_reject_unknown_deserialization_fields() {
    let digest = sha256(b"setup-model");
    let draft_id = SetupDraftId::from_uuid(Uuid::from_u128(30));
    let configuration_id = ConfigurationVersionId::from_uuid(Uuid::from_u128(31));
    let event_id = EventId::from_uuid(Uuid::from_u128(32));
    let mut draft = serde_json::to_value(
        SetupDraft::new(
            draft_id,
            1,
            SetupDraftState::Drafting,
            SetupPath::QuickStart,
            None,
            json!({}),
            1,
            1,
        )
        .unwrap(),
    )
    .unwrap();
    draft.as_object_mut().unwrap().insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<SetupDraft>(draft).is_err());

    let mut configuration = serde_json::to_value(InstallationConfigurationVersion::new(
        configuration_id,
        ObjectVersion::new(1).unwrap(),
        draft_id,
        digest.clone(),
        digest.clone(),
        json!({}),
        event_id,
        1,
    ))
    .unwrap();
    configuration.as_object_mut().unwrap().insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<InstallationConfigurationVersion>(configuration).is_err());

    let mut outcome = serde_json::to_value(
        SetupStepOutcome::new(draft_id, "connectivity".into(), 1, SetupStepStatus::Passed, None, 1)
            .unwrap(),
    )
    .unwrap();
    outcome.as_object_mut().unwrap().insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<SetupStepOutcome>(outcome).is_err());

    let mut readiness = serde_json::to_value(
        CapabilityReadiness::new(
            configuration_id,
            "help_read".into(),
            CapabilityReadinessStatus::Ready,
            None,
            1,
            digest,
        )
        .unwrap(),
    )
    .unwrap();
    readiness.as_object_mut().unwrap().insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<CapabilityReadiness>(readiness).is_err());

    let mut installation = serde_json::to_value(InstallationProjection {
        installation_id: installation_id(),
        created_event_id: event_id,
        created_at_ms: 1,
    })
    .unwrap();
    installation.as_object_mut().unwrap().insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<InstallationProjection>(installation).is_err());

    let mut end = serde_json::to_value(SessionEndProjection {
        ended_event_id: event_id,
        ended_at_ms: 2,
        reason: ShutdownReason::UserQuit,
    })
    .unwrap();
    end.as_object_mut().unwrap().insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<SessionEndProjection>(end).is_err());

    let mut session = serde_json::to_value(SessionProjection {
        session_id: session_id(),
        started_event_id: event_id,
        started_at_ms: 1,
        ended: None,
    })
    .unwrap();
    session.as_object_mut().unwrap().insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<SessionProjection>(session).is_err());
}

#[test]
fn event_append_and_projection_store_rollback_atomically() {
    let (_temporary_directory, mut database) = database();
    let transaction = database.immediate_transaction().unwrap();
    let event = EventRepository::append(
        &transaction,
        pending(
            1,
            ApplicationEvent::InstallationInitialized {
                installation_id: installation_id(),
            },
        ),
    )
    .unwrap();
    let mut state = ProjectionState::default();
    reduce(&mut state, &event).unwrap();
    ProjectionRepository::store(&transaction, &state).unwrap();
    transaction.rollback().unwrap();

    let event_count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM event_stream", [], |row| row.get(0))
        .unwrap();
    let projection_count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM projection_metadata", [], |row| row.get(0))
        .unwrap();
    assert_eq!((event_count, projection_count), (0, 0));
}

#[test]
fn rebuild_acquires_an_immediate_snapshot_before_reading_the_stream() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut first = Database::open(&paths).unwrap();
    let mut second = Database::open(&paths).unwrap();
    second.connection().busy_timeout(Duration::ZERO).unwrap();
    let transaction = first.immediate_transaction().unwrap();

    assert_eq!(
        ProjectionRepository::rebuild(second.connection_mut(), &[]),
        Err(RecoveryError::QueryFailed)
    );
    transaction.rollback().unwrap();
}

#[test]
fn projection_state_deserialization_rejects_zero_marker_installations_and_multiple_open_sessions() {
    let event_id = EventId::from_uuid(Uuid::from_u128(40));
    let zero_marker_installation = ProjectionState {
        installation: Some(InstallationProjection {
            installation_id: installation_id(),
            created_event_id: event_id,
            created_at_ms: 1,
        }),
        ..ProjectionState::default()
    };
    assert!(serde_json::from_value::<ProjectionState>(
        serde_json::to_value(zero_marker_installation).unwrap()
    )
    .is_err());

    let mut sessions = BTreeMap::new();
    for id in [session_id(), second_session_id()] {
        sessions.insert(
            id,
            SessionProjection {
                session_id: id,
                started_event_id: event_id,
                started_at_ms: 1,
                ended: None,
            },
        );
    }
    let multiple_open_sessions = ProjectionState {
        installation: Some(InstallationProjection {
            installation_id: installation_id(),
            created_event_id: event_id,
            created_at_ms: 1,
        }),
        sessions,
        setup_status: SetupStatus::NotStarted,
        last_sequence: 1,
        last_event_digest: Some(sha256(b"marker")),
    };
    assert!(serde_json::from_value::<ProjectionState>(
        serde_json::to_value(multiple_open_sessions).unwrap()
    )
    .is_err());
}

#[test]
fn newly_applied_interruption_returns_a_transient_effect_but_replay_does_not_store_it() {
    let (_temporary_directory, mut database) = database();
    let initialized = append(&mut database, 1, ApplicationEvent::InstallationInitialized { installation_id: installation_id() });
    let started = append(&mut database, 2, ApplicationEvent::ProcessSessionStarted { session_id: session_id() });
    let interrupted = append(&mut database, 3, ApplicationEvent::PreviousSessionInterrupted { session_id: session_id() });
    let mut state = ProjectionState::default();
    reduce(&mut state, &initialized).unwrap();
    reduce(&mut state, &started).unwrap();

    assert_eq!(reduce(&mut state, &interrupted).unwrap(), ReducerEffect::PreviousSessionInterrupted { session_id: session_id() });
    let replayed = reduce_all(&[initialized, started, interrupted]);
    assert_eq!(replayed, state);
    assert_eq!(serde_json::to_value(&replayed).unwrap().get("previous_session_interrupted"), None);
}

#[test]
fn projection_state_deserialization_independently_rejects_marker_digest_and_reachability_violations() {
    for value in [
        json!({"installation":null,"sessions":{},"setup_status":"not_started","last_sequence":0,"last_event_digest":sha256(b"x")} ),
        json!({"installation":null,"sessions":{},"setup_status":"not_started","last_sequence":1,"last_event_digest":null}),
        json!({"installation":null,"sessions":{},"setup_status":{"draft_saved":{"draft_id":"00000000-0000-0000-0000-000000000030"}},"last_sequence":1,"last_event_digest":sha256(b"x")}),
    ] {
        assert!(serde_json::from_value::<ProjectionState>(value).is_err());
    }
}

#[test]
fn interruption_rejects_wrong_target_and_already_ended_session_without_mutation() {
    let (_temporary_directory, mut fixture) = database();
    let initialized = append(&mut fixture, 1, ApplicationEvent::InstallationInitialized { installation_id: installation_id() });
    let started = append(&mut fixture, 2, ApplicationEvent::ProcessSessionStarted { session_id: session_id() });
    let mut state = reduce_all(&[initialized, started]);
    let before = state.clone();
    let wrong = append(&mut fixture, 4, ApplicationEvent::PreviousSessionInterrupted { session_id: second_session_id() });
    assert_eq!(reduce(&mut state, &wrong), Err(RecoveryError::InvalidEventRecord));
    assert_eq!(state, before);

    let (_other_temporary_directory, mut other_database) = database();
    let initialized = append(&mut other_database, 1, ApplicationEvent::InstallationInitialized { installation_id: installation_id() });
    let started_a = append(&mut other_database, 2, ApplicationEvent::ProcessSessionStarted { session_id: session_id() });
    let ended_a = append(&mut other_database, 3, ApplicationEvent::ProcessSessionEnded { session_id: session_id(), reason: ShutdownReason::UserQuit });
    let started_b = append(&mut other_database, 4, ApplicationEvent::ProcessSessionStarted { session_id: second_session_id() });
    let mut ended_state = reduce_all(&[initialized, started_a, ended_a, started_b]);
    let ended_before = ended_state.clone();
    let ended = append(&mut other_database, 5, ApplicationEvent::PreviousSessionInterrupted { session_id: session_id() });
    assert_eq!(reduce(&mut ended_state, &ended), Err(RecoveryError::InvalidEventRecord));
    assert_eq!(ended_state, ended_before);
}

#[test]
fn projection_lower_bound_rejects_each_underrepresented_installation_session_and_tombstone_shape() {
    let event_id = EventId::from_uuid(Uuid::from_u128(50));
    let installation = InstallationProjection { installation_id: installation_id(), created_event_id: event_id, created_at_ms: 1 };
    let started = SessionProjection { session_id: session_id(), started_event_id: event_id, started_at_ms: 1, ended: None };
    let ended = SessionProjection { ended: Some(SessionEndProjection { ended_event_id: event_id, ended_at_ms: 2, reason: ShutdownReason::UserQuit }), ..started.clone() };
    for (sessions, sequence) in [
        (BTreeMap::from([(session_id(), started)]), 1),
        (BTreeMap::from([(session_id(), ended)]), 2),
    ] {
        let state = ProjectionState { installation: Some(installation.clone()), sessions, setup_status: SetupStatus::NotStarted, last_sequence: sequence, last_event_digest: if sequence == 0 { None } else { Some(sha256(b"marker")) } };
        assert_eq!(serde_json::from_value::<ProjectionState>(serde_json::to_value(state).unwrap()).unwrap_err().to_string(), "event record is invalid");
    }
}

#[test]
fn newer_store_rejects_a_digest_consistent_fabricated_persisted_prefix() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let fabricated = ProjectionState { installation: reduce_all(&events[..1]).installation, sessions: BTreeMap::new(), setup_status: SetupStatus::NotStarted, last_sequence: 2, last_event_digest: Some(events[1].event_digest.clone()) };
    let digest = fabricated.digest().unwrap();
    database.connection().execute("INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (1, ?1, ?2, ?3)", (installation_id().to_string(), events[0].event_id.to_string(), events[0].occurred_at_ms)).unwrap();
    database.connection().execute("INSERT INTO projection_metadata (singleton, last_event_sequence, last_event_digest, projection_digest) VALUES (1, 2, ?1, ?2)", (events[1].event_digest.as_str(), digest.as_str())).unwrap();
    let newer_event = append(&mut database, 4, ApplicationEvent::HelpViewed);
    let mut newer = reduce_all(&events);
    reduce(&mut newer, &newer_event).unwrap();
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(ProjectionRepository::store(&transaction, &newer), Err(PersistenceError::ProjectionStateConflict));
    transaction.rollback().unwrap();
}

#[test]
fn load_uses_one_snapshot_across_stream_and_projection_reads() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut first = Database::open(&paths).unwrap();
    let mut second = Database::open(&paths).unwrap();
    let events = installation_session_events(&mut first);
    let initial = reduce_all(&events);
    let transaction = first.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &initial).unwrap();
    transaction.commit().unwrap();

    let loaded = ProjectionRepository::load_with_before_projection_rows(first.connection(), || {
        let next = append(&mut second, 4, ApplicationEvent::HelpViewed);
        let mut newer = initial.clone();
        reduce(&mut newer, &next).unwrap();
        let transaction = second.immediate_transaction().unwrap();
        ProjectionRepository::store(&transaction, &newer).unwrap();
        transaction.commit().unwrap();
    })
    .unwrap();
    assert_eq!(loaded, initial);
}
