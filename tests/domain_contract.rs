use ai_stock_forum::domain::{
    CorrelationId, EventId, ObjectRef, ObjectVersion, Sha256Digest, canonical_json_bytes, sha256,
};
use serde_json::json;
use uuid::Uuid;

macro_rules! object_ref_json {
    ($kind:expr, $id:expr) => {
        json!({
            "kind": $kind,
            "id": $id,
            "version": 1,
            "digest": sha256(b"object-ref").to_string(),
        })
    };
}

#[test]
fn canonical_json_sorts_nested_object_keys() {
    let left = json!({"z": {"b": 2, "a": 1}, "a": true});
    let right = json!({"a": true, "z": {"a": 1, "b": 2}});
    assert_eq!(
        canonical_json_bytes(&left).unwrap(),
        canonical_json_bytes(&right).unwrap()
    );
}

#[test]
fn typed_ids_do_not_interchange() {
    let raw = Uuid::from_u128(7);
    let event = EventId::from_uuid(raw);
    let correlation = CorrelationId::from_uuid(raw);
    assert_eq!(event.to_string(), correlation.to_string());
}

#[test]
fn object_versions_reject_zero() {
    assert!(ObjectVersion::new(0).is_err());
    assert_eq!(ObjectVersion::new(1).unwrap().get(), 1);
}

#[test]
fn object_versions_reject_zero_when_deserialized() {
    assert!(serde_json::from_value::<ObjectVersion>(json!(0)).is_err());
}

#[test]
fn object_refs_reject_empty_kind_when_deserialized() {
    assert!(serde_json::from_value::<ObjectRef>(object_ref_json!("", "object")).is_err());
}

#[test]
fn object_refs_reject_whitespace_kind_when_deserialized() {
    assert!(serde_json::from_value::<ObjectRef>(object_ref_json!("   ", "object")).is_err());
}

#[test]
fn object_refs_reject_empty_id_when_deserialized() {
    assert!(serde_json::from_value::<ObjectRef>(object_ref_json!("event", "")).is_err());
}

#[test]
fn object_refs_reject_whitespace_id_when_deserialized() {
    assert!(serde_json::from_value::<ObjectRef>(object_ref_json!("event", "   ")).is_err());
}

#[test]
fn digest_requires_lowercase_sha256_hex() {
    let digest = sha256(b"phase-zero");
    assert_eq!(digest.as_str().len(), 64);
    assert!(Sha256Digest::parse(&digest.to_string().to_uppercase()).is_err());
}
