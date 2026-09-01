mod support;

use ai_stock_forum::{
    app::{ApplicationEvent, PendingEvent},
    audit::AuditEntry,
    persistence::{EventRepository, PersistenceError},
};

#[test]
fn append_allocates_a_contiguous_digest_chain() {
    let mut fixture = support::database();
    let first = fixture.append(ApplicationEvent::HelpViewed);
    let second = fixture.append(ApplicationEvent::StatusViewed);

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(second.previous_event_digest.as_ref(), Some(&first.event_digest));
    EventRepository::verify(fixture.database.connection()).unwrap();
}

#[test]
fn append_rolls_back_without_allocating_a_sequence() {
    let mut fixture = support::database();
    let pending = fixture.pending(ApplicationEvent::HelpViewed);
    let transaction = fixture.database.immediate_transaction().unwrap();
    EventRepository::append(&transaction, pending).unwrap();
    transaction.rollback().unwrap();

    assert!(EventRepository::load_all(fixture.database.connection())
        .unwrap()
        .is_empty());
    assert_eq!(fixture.append(ApplicationEvent::StatusViewed).sequence, 1);
}

#[test]
fn duplicate_event_id_returns_the_original_envelope_and_conflicts_on_different_content() {
    let mut fixture = support::database();
    let pending = fixture.pending(ApplicationEvent::HelpViewed);
    let transaction = fixture.database.immediate_transaction().unwrap();
    let first = EventRepository::append(&transaction, pending.clone()).unwrap();
    transaction.commit().unwrap();

    let transaction = fixture.database.immediate_transaction().unwrap();
    let repeated = EventRepository::append(&transaction, pending.clone()).unwrap();
    transaction.commit().unwrap();
    assert_eq!(repeated, first);

    let conflicting = PendingEvent {
        event: ApplicationEvent::StatusViewed,
        ..pending
    };
    let transaction = fixture.database.immediate_transaction().unwrap();
    assert_eq!(
        EventRepository::append(&transaction, conflicting).unwrap_err(),
        PersistenceError::IdempotencyConflict
    );
}

#[test]
fn tail_is_bounded_and_returns_latest_events_in_sequence_order() {
    let mut fixture = support::database();
    fixture.append(ApplicationEvent::HelpViewed);
    fixture.append(ApplicationEvent::StatusViewed);
    fixture.append(ApplicationEvent::ShutdownRequested);

    let entries = EventRepository::tail(
        fixture.database.connection(),
        ai_stock_forum::app::AuditLimit::new(2).unwrap(),
    )
    .unwrap();
    assert_eq!(entries.iter().map(|entry| entry.sequence).collect::<Vec<_>>(), [2, 3]);
}

#[test]
fn update_and_delete_are_rejected() {
    let mut fixture = support::database();
    fixture.append(ApplicationEvent::HelpViewed);
    assert!(fixture
        .database
        .connection()
        .execute("DELETE FROM event_stream", [])
        .is_err());
    assert!(fixture
        .database
        .connection()
        .execute(
            "UPDATE event_stream SET event_type = 'forged' WHERE sequence = 1",
            []
        )
        .is_err());
}

#[test]
fn forged_row_is_reported_without_modifying_the_stream() {
    let mut fixture = support::database();
    fixture.append(ApplicationEvent::HelpViewed);
    let first_digest = fixture
        .database
        .connection()
        .query_row("SELECT event_digest FROM event_stream WHERE sequence = 1", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap();
    fixture
        .database
        .connection()
        .execute(
            "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, previous_event_digest, payload_json, event_digest) VALUES (2, '00000000-0000-0000-0000-000000000099', 1, 'status_viewed', 'human', 1700000000000, '00000000-0000-0000-0000-000000000100', ?1, '{}', '0000000000000000000000000000000000000000000000000000000000000000')",
            [first_digest],
        )
        .unwrap();

    let error = EventRepository::verify(fixture.database.connection()).unwrap_err();
    assert_eq!(error.code(), "event_digest_mismatch");
    assert_eq!(
        fixture
            .database
            .connection()
            .query_row("SELECT COUNT(*) FROM event_stream", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn audit_entries_are_typed_and_redacted() {
    let event = support::rejected_event(b"/secret hunter2");
    let audit = AuditEntry::from_event(&event);
    assert!(!audit.summary.contains("hunter2"));
    assert_eq!(audit.kind, "command_rejected");
    assert!(serde_json::to_string(&audit).unwrap().contains("/secret"));
}
