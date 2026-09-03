use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier, mpsc},
    time::Duration,
};

use ai_stock_forum::{
    app::{
        ApplicationEvent, AuditLimit, EVENT_SCHEMA_VERSION, EventEnvelope, EventEnvelopeWire,
        InputRejection, PendingEvent, SafeToken,
    },
    audit::AuditEntry,
    config::AppPaths,
    domain::{Actor, CorrelationId, EventId, sha256},
    persistence::{Database, EventRepository, PersistenceError, RecoveryError},
};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn pending(event_id: u128, correlation_id: u128, event: ApplicationEvent) -> PendingEvent {
    PendingEvent {
        event_id: EventId::from_uuid(Uuid::from_u128(event_id)),
        event_schema_version: EVENT_SCHEMA_VERSION,
        actor: Actor::Human,
        occurred_at_ms: 1_700_000_000_000,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(correlation_id)),
        causation_id: None,
        object: None,
        event,
    }
}

fn append(database: &mut Database, pending: PendingEvent) -> EventEnvelope {
    let transaction = database.immediate_transaction().unwrap();
    let envelope = EventRepository::append(&transaction, pending).unwrap();
    transaction.commit().unwrap();
    envelope
}

fn independently_canonical(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(independently_canonical).collect())
        }
        serde_json::Value::Object(values) => {
            let sorted: BTreeMap<_, _> = values
                .into_iter()
                .map(|(key, value)| (key, independently_canonical(value)))
                .collect();
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        scalar => scalar,
    }
}

fn independently_sha256(value: serde_json::Value) -> String {
    let bytes = serde_json::to_vec(&independently_canonical(value)).unwrap();
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn audit_limit_wire_boundary_rejects_out_of_range_values() {
    assert_eq!(AuditLimit::try_from(1_u16).unwrap().get(), 1);
    assert!(AuditLimit::try_from(0_u16).is_err());
    assert!(AuditLimit::try_from(101_u16).is_err());
    assert!(serde_json::from_str::<AuditLimit>("0").is_err());
    assert!(serde_json::from_str::<AuditLimit>("101").is_err());
}

#[test]
fn safe_token_boundary_rejects_full_lines_controls_and_oversize_values() {
    assert!(SafeToken::new("/secret hunter2").is_err());
    assert!(SafeToken::new("/secret\u{1b}").is_err());
    assert!(SafeToken::new("x".repeat(65)).is_err());
    assert!(serde_json::from_value::<SafeToken>(json!("/secret hunter2")).is_err());

    let rejection: InputRejection = serde_json::from_value(json!({
        "category": "unknown",
        "safe_token": "/unknown",
        "byte_length": 15,
        "input_digest": sha256(b"/unknown hunter2"),
    }))
    .unwrap();
    let audit = AuditEntry::from_event(&EventEnvelope {
        sequence: 1,
        event_id: EventId::from_uuid(Uuid::from_u128(1)),
        event_schema_version: EVENT_SCHEMA_VERSION,
        actor: Actor::Human,
        occurred_at_ms: 1,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(2)),
        causation_id: None,
        object: None,
        event: ApplicationEvent::CommandRejected { rejection },
        previous_event_digest: None,
        event_digest: sha256(b"event"),
    });
    assert!(audit.summary.contains("unknown"));
    assert!(!audit.summary.contains("hunter2"));
}

#[test]
fn envelope_wire_rejects_invalid_shape_noncanonical_data_and_digest_mutation() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    let envelope = append(
        &mut database,
        pending(
            1,
            2,
            ApplicationEvent::AuditTailViewed {
                limit: AuditLimit::try_from(2_u16).unwrap(),
            },
        ),
    );
    let wire = EventEnvelopeWire::from(&envelope);

    assert_eq!(EventEnvelope::try_from(wire.clone()).unwrap(), envelope);
    assert!(matches!(
        EventEnvelope::try_from(EventEnvelopeWire {
            sequence: 0,
            ..wire.clone()
        }),
        Err(RecoveryError::InvalidEventRecord)
    ));
    assert!(matches!(
        EventEnvelope::try_from(EventEnvelopeWire {
            event_schema_version: 2,
            ..wire.clone()
        }),
        Err(RecoveryError::UnsupportedEventSchema)
    ));
    assert!(matches!(
        EventEnvelope::try_from(EventEnvelopeWire {
            previous_event_digest: Some(sha256(b"unexpected")),
            ..wire.clone()
        }),
        Err(RecoveryError::InvalidPredecessorShape)
    ));
    assert!(matches!(
        EventEnvelope::try_from(EventEnvelopeWire {
            event_type: "help_viewed".to_owned(),
            ..wire.clone()
        }),
        Err(RecoveryError::InvalidEventRecord)
    ));
    assert!(matches!(
        EventEnvelope::try_from(EventEnvelopeWire {
            payload_json: "{\"limit\":2 }".to_owned(),
            ..wire.clone()
        }),
        Err(RecoveryError::InvalidEventRecord)
    ));
    assert!(matches!(
        EventEnvelope::try_from(EventEnvelopeWire {
            occurred_at_ms: 2,
            ..wire
        }),
        Err(RecoveryError::EventDigestMismatch)
    ));
}

#[test]
fn event_payload_is_variant_data_without_a_duplicate_discriminator() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    append(
        &mut database,
        pending(
            1,
            2,
            ApplicationEvent::AuditTailViewed {
                limit: AuditLimit::try_from(2_u16).unwrap(),
            },
        ),
    );

    let (event_type, payload): (String, String) = database
        .connection()
        .query_row(
            "SELECT event_type, payload_json FROM event_stream WHERE sequence = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(event_type, "audit_tail_viewed");
    assert_eq!(payload, "{\"limit\":2}");
    assert!(!payload.contains("type"));
}

#[test]
fn digest_matches_an_independent_canonical_material_fixture() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    let envelope = append(&mut database, pending(1, 2, ApplicationEvent::HelpViewed));
    let expected = independently_sha256(json!({
        "actor_id": null,
        "actor_kind": "human",
        "causation_id": null,
        "correlation_id": "00000000-0000-0000-0000-000000000002",
        "digest_format_version": 1,
        "event_id": "00000000-0000-0000-0000-000000000001",
        "event_schema_version": 1,
        "event_type": "help_viewed",
        "object": null,
        "occurred_at_ms": 1_700_000_000_000_i64,
        "payload_json": "{}",
        "previous_event_digest": null,
        "sequence": 1,
    }));

    assert_eq!(envelope.event_digest.as_str(), expected);
}

#[test]
fn unsupported_schema_is_classified_before_malformed_payload_decoding() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    database
        .connection()
        .execute(
            "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (1, '00000000-0000-0000-0000-000000000001', 2, 'help_viewed', 'human', 1, '00000000-0000-0000-0000-000000000002', '{\"unexpected\":true}', '0000000000000000000000000000000000000000000000000000000000000000')",
            [],
        )
        .unwrap();

    assert_eq!(
        EventRepository::verify(database.connection()).unwrap_err(),
        RecoveryError::UnsupportedEventSchema
    );
}

#[test]
fn repository_scope_is_one_event_and_event_id_replay_is_not_command_idempotency() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    let first = append(&mut database, pending(1, 2, ApplicationEvent::HelpViewed));
    let transaction = database.immediate_transaction().unwrap();
    let replay =
        EventRepository::append(&transaction, pending(1, 2, ApplicationEvent::HelpViewed)).unwrap();
    transaction.commit().unwrap();

    assert_eq!(replay, first);
    assert_eq!(
        EventRepository::load_all(database.connection())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn immediate_transactions_report_contention_then_replay_or_conflict_deterministically() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut first_database = Database::open(&paths).unwrap();
    let mut second_database = Database::open(&paths).unwrap();
    second_database
        .connection()
        .busy_timeout(Duration::ZERO)
        .unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let (first_result_sender, first_result_receiver) = mpsc::channel();
    let (retry_sender, retry_receiver) = mpsc::channel();
    let barrier_for_thread = Arc::clone(&barrier);
    let thread = std::thread::spawn(move || {
        barrier_for_thread.wait();
        let contention = match second_database.immediate_transaction() {
            Ok(_) => panic!("expected immediate write contention"),
            Err(error) => error,
        };
        first_result_sender.send(contention).unwrap();
        retry_receiver.recv().unwrap();
        let transaction = second_database.immediate_transaction().unwrap();
        let replay =
            EventRepository::append(&transaction, pending(1, 2, ApplicationEvent::HelpViewed));
        transaction.commit().unwrap();
        let transaction = second_database.immediate_transaction().unwrap();
        let conflict =
            EventRepository::append(&transaction, pending(1, 2, ApplicationEvent::StatusViewed))
                .unwrap_err();
        transaction.rollback().unwrap();
        (replay, conflict)
    });

    let transaction = first_database.immediate_transaction().unwrap();
    barrier.wait();
    assert_eq!(
        first_result_receiver.recv().unwrap(),
        PersistenceError::Contention
    );
    let first =
        EventRepository::append(&transaction, pending(1, 2, ApplicationEvent::HelpViewed)).unwrap();
    transaction.commit().unwrap();
    retry_sender.send(()).unwrap();
    let (replay, conflict) = thread.join().unwrap();

    assert_eq!(replay.unwrap(), first);
    assert_eq!(conflict, PersistenceError::IdempotencyConflict);
}
