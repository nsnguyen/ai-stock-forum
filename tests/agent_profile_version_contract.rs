use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole, BindingReferenceId,
        InferenceBindingRef, ProfileDiffField, ProfileFieldValue, diff_profile,
    },
    domain::{AgentProfileId, AgentProfileVersionId, MemoryNamespaceId, canonical_json_bytes},
};
use proptest::prelude::*;
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

fn valid_draft() -> AgentProfileDraft {
    AgentProfileDraft::new(
        "Research Analyst".to_owned(),
        "Evidence-led equity research.".to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical.".to_owned(),
        "Cite evidence before making a claim.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn bound_inference() -> AgentBindings {
    AgentBindings::new(
        Some(InferenceBindingRef::new(
            BindingReferenceId::new("connection.openai").unwrap(),
            BindingReferenceId::new("model.gpt-5").unwrap(),
        )),
        None,
    )
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
fn canonical_profile_digest_is_independent_of_json_map_order() {
    // Profile inputs contain no maps: bindings and provenance are structs, while tags and
    // references are ordered vectors. Equivalent bindings therefore cover the relevant
    // canonical profile payload ordering without inventing a map-only test fixture.
    let mut first_draft = valid_draft();
    first_draft.bindings = bound_inference();
    let mut second_draft = valid_draft();
    second_draft.bindings = bound_inference();

    let first = create(first_draft);
    let second = create(second_draft);

    assert_eq!(
        canonical_json_bytes(first.bindings()).unwrap(),
        canonical_json_bytes(second.bindings()).unwrap(),
    );
    assert_eq!(first.content_digest(), second.content_digest());
}

#[test]
fn changing_semantic_profile_content_changes_content_digest() {
    let original = create(valid_draft());
    let mut changed_draft = valid_draft();
    changed_draft.instructions = "State assumptions before making a claim.".to_owned();
    let changed = create(changed_draft);

    assert_ne!(original.content_digest(), changed.content_digest());
}

#[test]
fn version_one_has_no_predecessor_and_edit_increments_once() {
    let current = create(valid_draft());
    assert_eq!(current.version().get(), 1);
    assert_eq!(current.supersedes(), None);
    assert_eq!(current.default_policy_ref(), "profile-default/v1");

    let candidate = AgentProfileDraft::new(
        "Research Analyst".to_owned(),
        "Evidence-led global equity research.".to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical.".to_owned(),
        "Cite evidence before making a claim.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();
    let next = AgentProfileVersion::next_version(
        &current,
        profile_version_id(4),
        1_726_000_000_001,
        candidate,
    )
    .unwrap();

    assert_eq!(next.version().get(), 2);
    assert_eq!(next.supersedes(), Some(current.profile_version_id()));
    assert_eq!(next.profile_id(), current.profile_id());
    assert_eq!(next.memory_namespace_id(), current.memory_namespace_id());
    assert_eq!(next.default_policy_ref(), current.default_policy_ref());
    assert_eq!(next.template_provenance(), current.template_provenance());
}

#[test]
fn semantic_diff_lists_only_changed_fields_in_fixed_order() {
    let current = create(valid_draft());
    let candidate = AgentProfileDraft::new(
        "Senior Research Analyst".to_owned(),
        "Independent fundamental research.".to_owned(),
        AgentRole::Bull,
        "growth equities".to_owned(),
        vec!["growth".to_owned(), "quality".to_owned()],
        "Constructive and exacting.".to_owned(),
        "State assumptions and cite primary sources.".to_owned(),
        bound_inference(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap();

    let diff = diff_profile(&current, &candidate).unwrap();

    assert_eq!(
        diff.iter().map(|change| change.field).collect::<Vec<_>>(),
        vec![
            ProfileDiffField::DisplayName,
            ProfileDiffField::Description,
            ProfileDiffField::Role,
            ProfileDiffField::PrimarySpecialty,
            ProfileDiffField::SpecialtyTags,
            ProfileDiffField::Personality,
            ProfileDiffField::Instructions,
            ProfileDiffField::Bindings,
        ],
    );
    assert_eq!(diff[2].before, ProfileFieldValue::Role(AgentRole::Custom),);
    assert_eq!(diff[2].after, ProfileFieldValue::Role(AgentRole::Bull));
    assert_eq!(
        diff[4].after,
        ProfileFieldValue::SpecialtyTags(vec!["growth".to_owned(), "quality".to_owned()]),
    );
    assert_eq!(
        diff[7].after,
        ProfileFieldValue::Bindings(bound_inference()),
    );
}

#[test]
fn unchanged_candidate_is_rejected() {
    let current = create(valid_draft());

    assert_eq!(
        diff_profile(&current, &valid_draft()).unwrap_err().code(),
        "agent_profile_unchanged",
    );
}

fn valid_profile_drafts() -> impl Strategy<Value = AgentProfileDraft> {
    (
        "[A-Za-z]{1,20}",
        "[A-Za-z]{1,20}",
        "[A-Za-z]{1,20}",
        "[A-Za-z]{1,20}",
        "[A-Za-z]{1,20}",
        "[A-Za-z]{1,20}",
    )
        .prop_map(
            |(
                display_name,
                description,
                primary_specialty,
                specialty_tag,
                personality,
                instructions,
            )| {
                AgentProfileDraft::new(
                    display_name,
                    description,
                    AgentRole::Custom,
                    primary_specialty,
                    vec![format!("tag-{specialty_tag}")],
                    personality,
                    instructions,
                    AgentBindings::default(),
                    Vec::new(),
                    Vec::new(),
                )
                .unwrap()
            },
        )
}

proptest! {
    #[test]
    fn accepted_profile_round_trips_without_digest_drift(draft in valid_profile_drafts()) {
        let first = create(draft.clone());
        let second = create(draft);

        prop_assert_eq!(&first, &second);
        prop_assert_eq!(first.content_digest(), second.content_digest());
    }
}
