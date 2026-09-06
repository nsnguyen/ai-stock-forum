use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentReadiness, AgentRole,
        ProfileDiffField, diff_profile,
    },
    app::{AgentProfileView, CommandView},
    domain::{AgentProfileId, AgentProfileVersionId, MemoryNamespaceId, SkillId, SkillVersionId},
    skills::{SkillDraft, SkillProvenance, SkillVersion, SkillVersionRef},
    ui::command::TextRenderer,
};
use uuid::Uuid;

fn profile_id(value: u128) -> AgentProfileId {
    AgentProfileId::from_uuid(Uuid::from_u128(value))
}

fn profile_version_id(value: u128) -> AgentProfileVersionId {
    AgentProfileVersionId::from_uuid(Uuid::from_u128(value))
}

fn memory_namespace_id(value: u128) -> MemoryNamespaceId {
    MemoryNamespaceId::from_uuid(Uuid::from_u128(value))
}

fn skill_id(value: u128) -> SkillId {
    SkillId::from_uuid(Uuid::from_u128(value))
}

fn skill_version_id(value: u128) -> SkillVersionId {
    SkillVersionId::from_uuid(Uuid::from_u128(value))
}

fn skill_draft(name: &str) -> SkillDraft {
    SkillDraft::new(
        name.to_owned(),
        "Evidence-led research guidance.".to_owned(),
        "Use when reviewing an investment thesis.".to_owned(),
        vec!["research".to_owned()],
        "State the evidence and the disconfirming case.".to_owned(),
        Vec::new(),
    )
    .unwrap()
}

fn first_skill_version(skill: u128, version: u128, name: &str) -> SkillVersion {
    SkillVersion::create(
        skill_id(skill),
        skill_version_id(version),
        1_726_000_000_000,
        SkillProvenance::User,
        skill_draft(name),
    )
    .unwrap()
}

fn draft_with_skill_refs(
    skill_refs: Vec<SkillVersionRef>,
) -> Result<AgentProfileDraft, ai_stock_forum::domain::DomainError> {
    AgentProfileDraft::new(
        "Research Analyst".to_owned(),
        "Evidence-led equity research.".to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical.".to_owned(),
        "Cite evidence before making a claim.".to_owned(),
        AgentBindings::default(),
        skill_refs,
        Vec::new(),
    )
}

fn valid_draft(skill_refs: Vec<SkillVersionRef>) -> AgentProfileDraft {
    draft_with_skill_refs(skill_refs).unwrap()
}

fn create(draft: AgentProfileDraft) -> AgentProfileVersion {
    AgentProfileVersion::create(
        profile_id(1),
        profile_version_id(2),
        memory_namespace_id(3),
        1_726_000_000_000,
        draft,
        None,
    )
    .unwrap()
}

#[test]
fn empty_skill_refs_preserve_the_existing_legacy_profile_digest() {
    let profile = create(valid_draft(Vec::new()));

    assert_eq!(
        profile.content_digest().as_str(),
        "df5df861405d3d6cf0e48da6e78a98e40e07aee473ee4b1cd55e3c7d8305d7cb",
    );
}

#[test]
fn skill_refs_are_typed_canonical_and_limited_per_skill_id() {
    let first = first_skill_version(10, 11, "Evidence Review").reference();
    let second = first_skill_version(20, 21, "Filing Analysis").reference();
    let profile = valid_draft(vec![second.clone(), first.clone()]);

    assert_eq!(profile.skill_refs(), &[first.clone(), second]);

    let replacement = SkillVersion::next_version(
        &first_skill_version(10, 11, "Evidence Review"),
        skill_version_id(12),
        1_726_000_000_001,
        skill_draft("Evidence Review Revised"),
    )
    .unwrap()
    .reference();
    assert_eq!(
        AgentProfileDraft::new(
            "Research Analyst".to_owned(),
            "Evidence-led equity research.".to_owned(),
            AgentRole::Custom,
            "equity research".to_owned(),
            vec!["research".to_owned()],
            "Calm and skeptical.".to_owned(),
            "Cite evidence before making a claim.".to_owned(),
            AgentBindings::default(),
            vec![first, replacement],
            Vec::new(),
        )
        .unwrap_err()
        .code(),
        "invalid_profile_field",
    );

    let too_many = (1..=17)
        .map(|index| first_skill_version(index, index + 100, &format!("Skill {index}")).reference())
        .collect();
    assert_eq!(
        draft_with_skill_refs(too_many).unwrap_err().code(),
        "invalid_profile_field"
    );
}

#[test]
fn skill_assignment_upgrade_and_unassignment_build_exact_candidates() {
    let initial = first_skill_version(10, 11, "Evidence Review");
    let initial_ref = initial.reference();
    let upgraded_ref = SkillVersion::next_version(
        &initial,
        skill_version_id(12),
        1_726_000_000_001,
        skill_draft("Evidence Review Revised"),
    )
    .unwrap()
    .reference();
    let current = create(valid_draft(Vec::new()));

    let assigned = current.assign_skill(initial_ref.clone()).unwrap();
    assert_eq!(assigned.skill_refs(), &[initial_ref.clone()]);
    assert_eq!(
        assigned.assign_skill(initial_ref.clone()).unwrap_err().code(),
        "agent_profile_unchanged",
    );

    let upgraded = assigned
        .upgrade_skill(initial_ref.clone(), upgraded_ref.clone())
        .unwrap();
    assert_eq!(upgraded.skill_refs(), &[upgraded_ref.clone()]);
    assert_eq!(
        assigned
            .upgrade_skill(upgraded_ref.clone(), initial_ref.clone())
            .unwrap_err()
            .code(),
        "agent_profile_unchanged",
    );

    let unassigned = upgraded.unassign_skill(upgraded_ref.clone()).unwrap();
    assert!(unassigned.skill_refs().is_empty());
    assert_eq!(
        unassigned.unassign_skill(upgraded_ref).unwrap_err().code(),
        "agent_profile_unchanged",
    );
}

#[test]
fn skill_diffs_classify_additions_upgrades_and_removals_without_affecting_readiness() {
    let initial = first_skill_version(10, 11, "Evidence Review");
    let initial_ref = initial.reference();
    let upgraded_ref = SkillVersion::next_version(
        &initial,
        skill_version_id(12),
        1_726_000_000_001,
        skill_draft("Evidence Review Revised"),
    )
    .unwrap()
    .reference();
    let empty = create(valid_draft(Vec::new()));
    let assigned = empty.assign_skill(initial_ref.clone()).unwrap();
    let upgraded = assigned
        .upgrade_skill(initial_ref, upgraded_ref.clone())
        .unwrap();
    let unassigned = upgraded.unassign_skill(upgraded_ref).unwrap();

    assert_eq!(
        diff_profile(&empty, &assigned)
            .unwrap()
            .iter()
            .map(|diff| diff.field)
            .collect::<Vec<_>>(),
        vec![ProfileDiffField::SkillRefsAdded],
    );
    assert_eq!(
        diff_profile(&create(assigned.clone()), &upgraded)
            .unwrap()
            .iter()
            .map(|diff| diff.field)
            .collect::<Vec<_>>(),
        vec![ProfileDiffField::SkillRefsUpgraded],
    );
    assert_eq!(
        diff_profile(&create(upgraded), &unassigned)
            .unwrap()
            .iter()
            .map(|diff| diff.field)
            .collect::<Vec<_>>(),
        vec![ProfileDiffField::SkillRefsRemoved],
    );
    assert_eq!(assigned.readiness(), AgentReadiness::Unbound);
}

#[test]
fn profile_renderer_shows_exact_skill_metadata_without_skill_prose() {
    let skill_ref = first_skill_version(10, 11, "Evidence Review").reference();
    let profile = create(valid_draft(vec![skill_ref]));
    let mut rendered = Vec::new();

    TextRenderer::render_view(
        &CommandView::AgentProfile(AgentProfileView {
            profile,
            readiness: AgentReadiness::Unbound,
        }),
        &mut rendered,
    )
    .unwrap();
    let rendered = String::from_utf8(rendered).unwrap();

    assert!(rendered.contains(
        "Skill refs: 00000000-0000-0000-0000-00000000000a@00000000-0000-0000-0000-00000000000b#1"
    ));
    assert!(!rendered.contains("Evidence-led research guidance."));
    assert!(!rendered.contains("State the evidence and the disconfirming case."));
}
