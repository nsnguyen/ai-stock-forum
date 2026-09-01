use ai_stock_forum::domain::{
    canonical_json_bytes, sha256, CorrelationId, EventId, ObjectVersion, Sha256Digest,
};
use serde_json::json;
use uuid::Uuid;

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
fn digest_requires_lowercase_sha256_hex() {
    let digest = sha256(b"phase-zero");
    assert_eq!(digest.as_str().len(), 64);
    assert!(Sha256Digest::parse(&digest.to_string().to_uppercase()).is_err());
}
