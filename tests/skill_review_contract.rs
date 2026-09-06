use ai_stock_forum::{
    domain::{SkillId, SkillReviewToken, SkillVersionId},
    skills::{SkillDraft, SkillResource, SkillReviewRegistry},
};
use uuid::Uuid;

fn id(value: u128) -> SkillId {
    SkillId::from_uuid(Uuid::from_u128(value))
}

fn version_id(value: u128) -> SkillVersionId {
    SkillVersionId::from_uuid(Uuid::from_u128(value))
}

fn token(value: u128) -> SkillReviewToken {
    SkillReviewToken::from_uuid(Uuid::from_u128(value))
}

fn candidate(instructions: &str) -> SkillDraft {
    SkillDraft::new(
        "Evidence Review".to_owned(),
        "Check sources.".to_owned(),
        "Use when evidence quality matters.".to_owned(),
        vec!["research".to_owned()],
        instructions.to_owned(),
        vec![SkillResource { name: "Checklist".to_owned(), body: "Verify sources.".to_owned() }],
    )
    .unwrap()
}

#[test]
fn review_token_rejects_changed_candidate_and_stale_expected_version() {
    let registry = SkillReviewRegistry::default();
    let original = candidate("Compare claims to evidence.");
    registry.issue(token(1), id(2), Some(version_id(3)), &original).unwrap();

    assert!(registry.consume(token(1), id(2), Some(version_id(3)), &candidate("Changed candidate.")).is_err());
    registry.issue(token(4), id(2), Some(version_id(3)), &original).unwrap();
    assert!(registry.consume(token(4), id(2), Some(version_id(5)), &original).is_err());
}
