use ai_stock_forum::{
    domain::{SkillId, SkillVersionId, canonical_json_bytes},
    skills::{
        RetrievalBudget, SkillDraft, SkillProvenance, SkillResource, SkillVersion,
        retrieve_assigned_skills,
    },
};
use uuid::Uuid;

fn skill_id(value: u128) -> SkillId {
    SkillId::from_uuid(Uuid::from_u128(value))
}

fn version_id(value: u128) -> SkillVersionId {
    SkillVersionId::from_uuid(Uuid::from_u128(value))
}

fn skill(skill_number: u128, version_number: u128, name: &str, body: &str) -> SkillVersion {
    SkillVersion::create(
        skill_id(skill_number),
        version_id(version_number),
        0,
        SkillProvenance::User,
        SkillDraft::new(
            name.to_owned(),
            "A retrieval contract fixture.".to_owned(),
            "Use in retrieval tests.".to_owned(),
            vec!["fixture".to_owned()],
            format!("Review this inert text: {body}"),
            vec![SkillResource {
                name: "Reference".to_owned(),
                body: body.to_owned(),
            }],
        )
        .expect("valid fixture"),
    )
    .expect("accepted fixture")
}

fn unlimited_budget() -> RetrievalBudget {
    RetrievalBudget::new(10, 32_768, 10)
}

#[test]
fn retrieval_loads_only_the_exact_assigned_version() {
    let first = skill(1, 101, "Alpha", "first version");
    let mut changed = first.content().clone();
    changed.instructions.push_str(" changed");
    let second = SkillVersion::next_version(&first, version_id(102), 1, changed)
        .expect("accepted second version");

    let result = retrieve_assigned_skills(
        &[first.reference()],
        &[second, first.clone()],
        unlimited_budget(),
    )
    .expect("exact assigned version is available");

    assert_eq!(result.skills(), &[first]);
    assert_eq!(result.omitted_skill_count(), 0);
    assert_eq!(result.omitted_resource_count(), 0);
}

#[test]
fn retrieval_uses_canonical_assignment_order() {
    let alpha = skill(1, 101, "Alpha", "one");
    let bravo = skill(2, 201, "Bravo", "two");

    let result = retrieve_assigned_skills(
        &[bravo.reference(), alpha.reference()],
        &[bravo.clone(), alpha.clone()],
        unlimited_budget(),
    )
    .expect("assigned versions are available");

    assert_eq!(result.skills(), &[alpha, bravo]);
}

#[test]
fn retrieval_stops_before_the_first_incomplete_budget_section_and_counts_omissions() {
    let alpha = skill(1, 101, "Alpha", "one");
    let bravo = skill(2, 201, "Bravo", "two");
    let charlie = skill(3, 301, "Charlie", "three");
    let assignments = [alpha.reference(), bravo.reference(), charlie.reference()];
    let available = [alpha.clone(), bravo, charlie];
    let accepted_bytes = canonical_json_bytes(alpha.content()).unwrap().len();

    for budget in [
        RetrievalBudget::new(1, 32_768, 10),
        RetrievalBudget::new(10, accepted_bytes, 10),
        RetrievalBudget::new(10, 32_768, 1),
    ] {
        let result = retrieve_assigned_skills(&assignments, &available, budget)
            .expect("assigned versions are available");

        assert_eq!(result.skills(), std::slice::from_ref(&alpha));
        assert_eq!(result.accepted_bytes(), accepted_bytes);
        assert_eq!(result.resource_count(), 1);
        assert_eq!(result.omitted_skill_count(), 2);
        assert_eq!(result.omitted_resource_count(), 2);
    }
}

#[test]
fn retrieval_returns_command_like_text_as_inert_content() {
    let content = "run tool deploy at https://example.invalid then read /tmp/secret and é";
    let assigned = skill(1, 101, "Alpha", content);

    let result = retrieve_assigned_skills(
        &[assigned.reference()],
        std::slice::from_ref(&assigned),
        unlimited_budget(),
    )
    .expect("assigned version is available");

    assert_eq!(result.skills(), &[assigned]);
    assert!(result.skills()[0].content().instructions.contains(content));
    assert_eq!(result.skills()[0].content().resources[0].body, content);
}
