use ai_stock_forum::agents::{
    AgentBindingCatalogSnapshot, AgentBindings, AgentProfileDraft, AgentReadiness, AgentRole,
    BindingReferenceId, EngineeringBindingRef, InferenceBindingRef, ProfileTemplateProvenance,
};
use serde_json::json;

fn raw_draft() -> AgentProfileDraft {
    serde_json::from_value(json!({
        "role": "bull",
        "display_name": "  Bull\u{2003}Researcher  ",
        "description": "  Finds\u{00a0}durable   growth  ",
        "primary_specialty": "  Growth\u{2003}Research  ",
        "specialty_tags": [" Valuation ", "catalysts", "  Moats  "],
        "personality": "  Evidence-led   and calm  ",
        "instructions": "  State\u{2003}the thesis, risks, and disconfirming evidence.  ",
        "bindings": {
            "inference": null,
            "engineering": null
        },
        "skill_refs": [],
        "mcp_refs": [],
        "template_provenance": null
    }))
    .expect("draft fixture must deserialize")
}

#[test]
fn canonicalizes_before_byte_limits_and_persistence() {
    let mut draft = raw_draft();
    draft.display_name = format!("{}{}{}", "A".repeat(32), " ".repeat(100), "B".repeat(31));

    let canonical = draft.canonicalized().expect("canonical name is 64 bytes");

    assert_eq!(
        canonical.display_name,
        format!("{} {}", "A".repeat(32), "B".repeat(31))
    );
    assert_eq!(canonical.description, "Finds durable growth");
    assert_eq!(canonical.primary_specialty, "Growth Research");
    assert_eq!(canonical.personality, "Evidence-led and calm");
    assert_eq!(
        canonical.instructions,
        "State the thesis, risks, and disconfirming evidence."
    );
    assert_eq!(
        canonical.specialty_tags,
        vec!["catalysts", "Moats", "Valuation"]
    );
}

#[test]
fn permits_an_empty_description_but_rejects_tabs_in_every_visible_field() {
    let mut empty_description = raw_draft();
    empty_description.description = " \u{2003} ".to_owned();
    assert_eq!(
        empty_description
            .canonicalized()
            .expect("description has a zero-byte minimum")
            .description,
        ""
    );

    for field in [
        "display_name",
        "description",
        "primary_specialty",
        "specialty_tag",
        "personality",
        "instructions",
    ] {
        let mut draft = raw_draft();
        match field {
            "display_name" => draft.display_name = "Bull\tResearcher".to_owned(),
            "description" => draft.description = "Bull\tResearcher".to_owned(),
            "primary_specialty" => draft.primary_specialty = "Bull\tResearcher".to_owned(),
            "specialty_tag" => draft.specialty_tags = vec!["Bull\tResearcher".to_owned()],
            "personality" => draft.personality = "Bull\tResearcher".to_owned(),
            "instructions" => draft.instructions = "Bull\tResearcher".to_owned(),
            _ => unreachable!(),
        }
        assert!(draft.canonicalized().is_err(), "{field} accepted a tab");
    }
}

#[test]
fn tags_are_unique_across_primary_and_secondary_after_normalization() {
    let mut draft = raw_draft();
    draft.primary_specialty = "Ma\u{00df}e".to_owned();
    draft.specialty_tags = vec!["MASSE".to_owned()];

    assert!(draft.canonicalized().is_err());
}

#[test]
fn custom_profiles_remain_incomplete_until_meaningful_fields_are_supplied() {
    let mut draft = raw_draft();
    draft.role = AgentRole::Custom;
    draft.primary_specialty.clear();
    draft.personality.clear();
    draft.instructions.clear();
    assert!(draft.canonicalized().is_err());

    draft.primary_specialty = "Event-driven special situations".to_owned();
    draft.personality = "Skeptical, explicit, and evidence-led".to_owned();
    draft.instructions = "Separate observations from hypotheses and list invalidators.".to_owned();
    assert!(draft.canonicalized().is_ok());
}

#[test]
fn nested_profile_payloads_reject_unknown_fields_recursively() {
    let bindings = json!({
        "inference": null,
        "engineering": null,
        "unexpected": true
    });
    assert!(serde_json::from_value::<AgentBindings>(bindings).is_err());

    let provenance = json!({
        "template_id": "builtin.bull",
        "template_version": 1,
        "template_digest": "0000000000000000000000000000000000000000000000000000000000000000",
        "unexpected": true
    });
    assert!(serde_json::from_value::<ProfileTemplateProvenance>(provenance).is_err());
}

#[test]
fn readiness_is_derived_only_from_typed_catalog_references() {
    let inference = InferenceBindingRef::new(
        BindingReferenceId::new("connection.openai").unwrap(),
        BindingReferenceId::new("model.gpt-5").unwrap(),
    );
    let engineering =
        EngineeringBindingRef::new(BindingReferenceId::new("runtime.local-codex").unwrap());
    let available =
        AgentBindingCatalogSnapshot::new(vec![inference.clone()], vec![engineering.clone()]);
    let empty = AgentBindingCatalogSnapshot::default();

    assert_eq!(
        available.readiness(AgentRole::Bull, &AgentBindings::default()),
        AgentReadiness::Unbound
    );
    assert_eq!(
        empty.readiness(
            AgentRole::Bull,
            &AgentBindings::new(Some(inference.clone()), None),
        ),
        AgentReadiness::BindingUnavailable
    );
    assert_eq!(
        available.readiness(
            AgentRole::Bull,
            &AgentBindings::new(Some(inference.clone()), None),
        ),
        AgentReadiness::Ready
    );
    assert_eq!(
        available.readiness(
            AgentRole::Engineering,
            &AgentBindings::new(Some(inference), None),
        ),
        AgentReadiness::BindingUnavailable
    );
    assert_eq!(
        available.readiness(
            AgentRole::Engineering,
            &AgentBindings::new(
                Some(InferenceBindingRef::new(
                    BindingReferenceId::new("connection.openai").unwrap(),
                    BindingReferenceId::new("model.gpt-5").unwrap(),
                )),
                Some(engineering),
            ),
        ),
        AgentReadiness::Ready
    );
}
