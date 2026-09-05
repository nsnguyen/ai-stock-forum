use std::collections::BTreeMap;

use ai_stock_forum::{
    agents::{
        diff_profile, AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole,
        ProfileField, ProfileFieldValue,
    },
    domain::{
        canonical_json_bytes, sha256, AgentProfileId, AgentProfileVersionId, MemoryNamespaceId,
    },
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
    let mut first = BTreeMap::new();
    first.insert("display_name", "Research Analyst");
    first.insert("role", "custom");
    let mut second = BTreeMap::new();
    second.insert("role", "custom");
    second.insert("display_name", "Research Analyst");

    assert_eq!(
        sha256(&canonical_json_bytes(&first).unwrap()),
        sha256(&canonical_json_bytes(&second).unwrap()),
    );
    assert_eq!(create(valid_draft()).content_digest, create(valid_draft()).content_digest);
}

#[test]
fn version_one_has_no_predecessor_and_edit_increments_once() {
    let current = create(valid_draft());
    assert_eq!(current.version.get(), 1);
    assert_eq!(current.supersedes, None);
    assert_eq!(current.default_policy_ref, "profile-default/v1");

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

    assert_eq!(next.version.get(), 2);
    assert_eq!(next.supersedes, Some(current.profile_version_id));
    assert_eq!(next.profile_id, current.profile_id);
    assert_eq!(next.memory_namespace_id, current.memory_namespace_id);
    assert_eq!(next.default_policy_ref, current.default_policy_ref);
    assert_eq!(next.template_provenance, current.template_provenance);
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
        AgentBindings {
            model_provider: Some("openai".to_owned()),
            model_name: Some("gpt-5".to_owned()),
        },
        Vec::new(),
        Vec::new(),
    )
    .unwrap();

    let diff = diff_profile(&current, &candidate).unwrap();

    assert_eq!(
        diff.iter().map(|change| change.field).collect::<Vec<_>>(),
        vec![
            ProfileField::DisplayName,
            ProfileField::Description,
            ProfileField::Role,
            ProfileField::PrimarySpecialty,
            ProfileField::SpecialtyTags,
            ProfileField::Personality,
            ProfileField::Instructions,
            ProfileField::Bindings,
        ],
    );
    assert_eq!(
        diff[2].before,
        ProfileFieldValue::Role(AgentRole::Custom),
    );
    assert_eq!(diff[2].after, ProfileFieldValue::Role(AgentRole::Bull));
    assert_eq!(
        diff[4].after,
        ProfileFieldValue::SpecialtyTags(vec!["growth".to_owned(), "quality".to_owned()]),
    );
    assert_eq!(
        diff[7].after,
        ProfileFieldValue::Bindings(AgentBindings {
            model_provider: Some("openai".to_owned()),
            model_name: Some("gpt-5".to_owned()),
        }),
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
            |(display_name, description, primary_specialty, specialty_tag, personality, instructions)| {
                AgentProfileDraft::new(
                    display_name,
                    description,
                    AgentRole::Custom,
                    primary_specialty,
                    vec![specialty_tag],
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
        prop_assert_eq!(&first.content_digest, &second.content_digest);
    }
}
