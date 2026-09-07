use ai_stock_forum::{
    domain::{
        Actor, EventId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId, ObjectVersion,
        canonical_json_bytes,
    },
    memory::{MemoryEntryDraft, MemoryEntryState, MemoryEntryVersion},
};
use serde_json::Value;
use uuid::Uuid;

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn present_entry_fixture() -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(uuid(1)),
        MemoryEntryId::from_uuid(uuid(2)),
        MemoryEntryVersionId::from_uuid(uuid(3)),
        draft(
            "Portfolio Thesis",
            "Own durable companies",
            vec!["investing"],
        ),
        Actor::Human,
        10,
        None,
        EventId::from_uuid(uuid(4)),
    )
    .unwrap()
}

fn draft(display_key: &str, value: &str, purpose_tags: Vec<&str>) -> MemoryEntryDraft {
    MemoryEntryDraft::new(
        display_key.to_owned(),
        value.to_owned(),
        purpose_tags.into_iter().map(str::to_owned).collect(),
    )
    .unwrap()
}

fn next_version_id() -> MemoryEntryVersionId {
    MemoryEntryVersionId::from_uuid(uuid(5))
}

fn replacement_draft() -> MemoryEntryDraft {
    draft(
        "Portfolio thesis",
        "Prefer free cash flow",
        vec!["research"],
    )
}

#[test]
fn creates_a_present_first_version_with_a_stable_reference() {
    let entry = present_entry_fixture();
    let reference = entry.reference();

    assert_eq!(reference.version(), ObjectVersion::new(1).unwrap());
    assert_eq!(reference.state(), MemoryEntryState::Present);
    assert_eq!(reference.normalized_key().as_str(), "portfolio thesis");
    assert_eq!(entry.predecessor_version_id(), None);
    assert_eq!(entry.value(), Some("Own durable companies"));
    assert_eq!(entry.purpose_tags(), ["investing"]);
    assert_eq!(entry.created_by(), &Actor::Human);
}

#[test]
fn delete_then_recreate_preserves_logical_id_and_advances_exactly_one_version() {
    let present = present_entry_fixture();
    let deleted = present
        .next_deleted(
            next_version_id(),
            Actor::Human,
            20,
            None,
            EventId::from_uuid(uuid(6)),
        )
        .unwrap();
    let recreated = deleted
        .next_present(
            MemoryEntryVersionId::from_uuid(uuid(7)),
            replacement_draft(),
            Actor::Human,
            30,
            None,
            EventId::from_uuid(uuid(8)),
        )
        .unwrap();

    assert_eq!(deleted.reference().state(), MemoryEntryState::Deleted);
    assert_eq!(deleted.value(), None);
    assert_eq!(
        recreated.reference().entry_id(),
        present.reference().entry_id()
    );
    assert_eq!(
        recreated.reference().version().get(),
        present.reference().version().get() + 2
    );
    assert_eq!(
        recreated.predecessor_version_id(),
        Some(deleted.reference().entry_version_id())
    );
}

#[test]
fn overwrite_keeps_the_logical_entry_id_and_changes_display_key_without_changing_normalized_key() {
    let present = present_entry_fixture();
    let overwritten = present
        .next_present(
            next_version_id(),
            replacement_draft(),
            Actor::Human,
            20,
            None,
            EventId::from_uuid(uuid(6)),
        )
        .unwrap();

    assert_eq!(
        overwritten.reference().entry_id(),
        present.reference().entry_id()
    );
    assert_eq!(overwritten.display_key(), "Portfolio thesis");
    assert_eq!(
        overwritten.reference().normalized_key(),
        present.reference().normalized_key()
    );
    assert_ne!(
        overwritten.reference().content_digest(),
        present.reference().content_digest()
    );
}

#[test]
fn serialized_versions_reject_tampering_and_noncanonical_shapes() {
    let entry = present_entry_fixture();
    let mut value: Value = serde_json::from_slice(&canonical_json_bytes(&entry).unwrap()).unwrap();
    value["value"] = Value::Null;
    assert!(serde_json::from_value::<MemoryEntryVersion>(value).is_err());

    let mut reference: Value =
        serde_json::from_slice(&canonical_json_bytes(&entry.reference()).unwrap()).unwrap();
    reference["normalized_key"] = serde_json::json!("Portfolio Thesis");
    assert!(serde_json::from_value::<ai_stock_forum::memory::MemoryEntryRef>(reference).is_err());
}

#[test]
fn content_and_record_digests_are_deterministic_and_include_versioned_metadata() {
    let entry = present_entry_fixture();
    let same = present_entry_fixture();
    let next = entry
        .next_present(
            next_version_id(),
            draft(
                "Portfolio Thesis",
                "Own durable companies",
                vec!["investing"],
            ),
            Actor::Human,
            20,
            None,
            EventId::from_uuid(uuid(6)),
        )
        .unwrap();

    assert_eq!(entry.content_digest(), same.content_digest());
    assert_eq!(entry.record_digest(), same.record_digest());
    assert_eq!(entry.content_digest(), next.content_digest());
    assert_ne!(entry.record_digest(), next.record_digest());
}
