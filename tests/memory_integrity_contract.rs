use ai_stock_forum::{
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, PendingEvent},
    config::AppPaths,
    domain::{
        Actor, CorrelationId, EventId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId,
    },
    memory::{MemoryEntryDraft, MemoryEntryVersion},
    persistence::{Database, EventRepository, MemoryRepository, PersistenceError},
};
use uuid::Uuid;

fn database() -> Database {
    Database::open(&AppPaths::for_test(&tempfile::tempdir().unwrap().keep())).unwrap()
}

#[test]
fn every_authenticated_entry_column_tamper_fails_closed_without_exposing_content() {
    for mutation in [
        "UPDATE memory_entry_versions SET content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE memory_entry_versions SET display_key='Changed'",
        "UPDATE memory_entry_versions SET record_json=CAST('{}' AS BLOB)",
        "UPDATE memory_entry_versions SET created_at_ms=created_at_ms+1",
    ] {
        let mut database = database();
        let event_id = EventId::from_uuid(Uuid::from_u128(100));
        let tx = database.immediate_transaction().unwrap();
        EventRepository::append(
            &tx,
            PendingEvent {
                event_id,
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 1,
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(101)),
                causation_id: None,
                object: None,
                event: ApplicationEvent::HelpViewed,
            },
        )
        .unwrap();
        let entry = MemoryEntryVersion::create_present(
            MemoryNamespaceId::from_uuid(Uuid::from_u128(1)),
            MemoryEntryId::from_uuid(Uuid::from_u128(2)),
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(3)),
            MemoryEntryDraft::new("Thesis".into(), "Private value".into(), vec![]).unwrap(),
            Actor::Human,
            2,
            None,
            event_id,
        )
        .unwrap();
        MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
        MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
        tx.commit().unwrap();
        database
            .connection()
            .execute_batch("PRAGMA foreign_keys=OFF; DROP TRIGGER memory_entry_versions_no_update;")
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        let error = MemoryRepository::load_current_entry(
            &tx,
            entry.reference().namespace_id(),
            entry.reference().normalized_key(),
        )
        .unwrap_err();
        tx.rollback().unwrap();
        assert_eq!(error, PersistenceError::MemoryRowMismatch, "{mutation}");
        assert_eq!(error.code(), "memory_row_mismatch");
        assert!(!error.to_string().contains("Private value"));
    }
}
