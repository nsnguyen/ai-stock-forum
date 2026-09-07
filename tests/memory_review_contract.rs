use ai_stock_forum::memory::{
    MEMORY_PLAINTEXT_WARNING, MemoryField, MemoryFieldDiff, MemoryFieldValue, MemoryMutationKind,
    MemoryNoChange, MemoryPlaintextAcknowledgement,
};

#[test]
fn review_diff_values_serialize_without_losing_state_or_tag_order() {
    let diff = MemoryFieldDiff {
        field: MemoryField::PurposeTags,
        before: MemoryFieldValue::Tags(vec!["investing".to_owned()]),
        after: MemoryFieldValue::Tags(vec!["research".to_owned(), "watchlist".to_owned()]),
    };
    let encoded = serde_json::to_value(&diff).unwrap();
    assert_eq!(encoded["field"], "PurposeTags");
    assert_eq!(
        serde_json::from_value::<MemoryFieldDiff>(encoded).unwrap(),
        diff
    );
}

#[test]
fn plaintext_acknowledgement_has_the_exact_retention_warning() {
    let acknowledgement = MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1;
    assert_eq!(acknowledgement.warning(), MEMORY_PLAINTEXT_WARNING);
    assert_eq!(
        acknowledgement.warning(),
        "Plaintext local memory — do not store credentials; history is retained after overwrite or delete"
    );
}

#[test]
fn review_wire_types_keep_noop_and_operation_variants_distinct() {
    assert_ne!(
        serde_json::to_value(MemoryNoChange::IdenticalContent).unwrap(),
        serde_json::to_value(MemoryNoChange::AlreadyAbsent).unwrap()
    );
    assert_ne!(
        serde_json::to_value(MemoryMutationKind::Set).unwrap(),
        serde_json::to_value(MemoryMutationKind::Delete).unwrap()
    );
}
