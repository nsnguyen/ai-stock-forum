use ai_stock_forum::memory::MemoryProjection;
use serde_json::json;

#[test]
fn empty_projection_has_only_reference_maps_and_round_trips() {
    let projection = MemoryProjection::default();
    assert!(projection.is_empty());
    assert_eq!(
        serde_json::to_value(&projection).unwrap(),
        json!({ "current_entries": {}, "proposals": {} })
    );
    assert_eq!(
        serde_json::from_value::<MemoryProjection>(json!({
            "current_entries": {},
            "proposals": {}
        }))
        .unwrap(),
        projection
    );
}

#[test]
fn projection_rejects_noncanonical_map_shapes() {
    assert!(
        serde_json::from_value::<MemoryProjection>(json!({
            "current_entries": [],
            "proposals": {}
        }))
        .is_err()
    );
}
