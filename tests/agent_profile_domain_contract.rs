use ai_stock_forum::agents::{
    builtin_profile_templates, normalize_profile_name_key, AgentBindings, AgentProfileDraft,
    AgentReadiness, AgentRole,
};

fn draft(
    display_name: String,
    description: String,
    primary_specialty: String,
    specialty_tags: Vec<String>,
    personality: String,
    instructions: String,
    bindings: AgentBindings,
) -> Result<AgentProfileDraft, ai_stock_forum::domain::DomainError> {
    AgentProfileDraft::new(
        display_name,
        description,
        AgentRole::Custom,
        primary_specialty,
        specialty_tags,
        personality,
        instructions,
        bindings,
        Vec::new(),
        Vec::new(),
    )
}

fn valid_draft() -> AgentProfileDraft {
    draft(
        "Research Analyst".to_owned(),
        "Evidence-led equity research.".to_owned(),
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical.".to_owned(),
        "Cite evidence before making a claim.".to_owned(),
        AgentBindings::default(),
    )
    .unwrap()
}

#[test]
fn active_name_key_uses_nfkc_casefold_and_whitespace_fold() {
    assert_eq!(
        normalize_profile_name_key("  Maße\tDesk  ").unwrap(),
        normalize_profile_name_key("MASSE desk").unwrap(),
    );
    assert_eq!(
        normalize_profile_name_key("Kelvin").unwrap(),
        normalize_profile_name_key("kelvin").unwrap(),
    );
    assert_eq!(
        normalize_profile_name_key("Ａｌｐｈａ").unwrap(),
        normalize_profile_name_key("alpha").unwrap(),
    );
    assert_eq!(
        normalize_profile_name_key("Cafe\u{301}").unwrap(),
        normalize_profile_name_key("Café").unwrap(),
    );
}

#[test]
fn builtin_templates_are_pinned_complete_and_copied_into_editable_drafts() {
    let templates = builtin_profile_templates();
    assert_eq!(templates.len(), 5);
    assert_eq!(
        templates.iter().map(|item| item.role).collect::<Vec<_>>(),
        vec![
            AgentRole::Bull,
            AgentRole::Bear,
            AgentRole::Chief,
            AgentRole::Engineering,
            AgentRole::Custom,
        ],
    );
    assert!(templates.iter().all(|item| item.version.get() == 1));
    assert!(templates.iter().all(|item| !item.digest.as_str().is_empty()));

    let template = &templates[0];
    let mut copied = template.copy_to_draft().unwrap();
    copied.display_name.push_str(" II");
    assert_eq!(template.suggested_name, "Bull Researcher");
    assert_eq!(copied.display_name, "Bull Researcher II");
    assert_eq!(
        &copied.template_provenance().unwrap().template_digest,
        &template.digest
    );
}

#[test]
fn profile_draft_accepts_exact_utf8_byte_boundaries_and_absent_bindings() {
    let profile = draft(
        "n".repeat(64),
        "d".repeat(256),
        "s".repeat(64),
        (0..5)
            .map(|index| format!("{index}{}", "a".repeat(47)))
            .collect(),
        "p".repeat(1_024),
        "i".repeat(4_096),
        AgentBindings::default(),
    )
    .unwrap();

    assert_eq!(profile.readiness(), AgentReadiness::NotReady);
}

#[test]
fn profile_draft_rejects_over_limit_and_duplicate_folded_values() {
    assert_eq!(
        draft(
            "n".repeat(65),
            "description".to_owned(),
            "specialty".to_owned(),
            vec![],
            "personality".to_owned(),
            "instructions".to_owned(),
            AgentBindings::default(),
        )
        .unwrap_err()
        .code(),
        "invalid_profile_field",
    );
    assert!(draft(
        "name".to_owned(),
        "d".repeat(257),
        "specialty".to_owned(),
        vec![],
        "personality".to_owned(),
        "instructions".to_owned(),
        AgentBindings::default(),
    )
    .is_err());
    assert!(draft(
        "name".to_owned(),
        "description".to_owned(),
        "s".repeat(65),
        vec![],
        "personality".to_owned(),
        "instructions".to_owned(),
        AgentBindings::default(),
    )
    .is_err());
    assert!(draft(
        "name".to_owned(),
        "description".to_owned(),
        "specialty".to_owned(),
        vec!["a".repeat(49)],
        "personality".to_owned(),
        "instructions".to_owned(),
        AgentBindings::default(),
    )
    .is_err());
    assert!(draft(
        "name".to_owned(),
        "description".to_owned(),
        "specialty".to_owned(),
        vec!["one".to_owned(); 6],
        "personality".to_owned(),
        "instructions".to_owned(),
        AgentBindings::default(),
    )
    .is_err());
    assert_eq!(
        draft(
            "name".to_owned(),
            "description".to_owned(),
            "specialty".to_owned(),
            vec!["Maße".to_owned(), "MASSE".to_owned()],
            "personality".to_owned(),
            "instructions".to_owned(),
            AgentBindings::default(),
        )
        .unwrap_err()
        .code(),
        "duplicate_profile_tag",
    );
    assert!(draft(
        "name".to_owned(),
        "description".to_owned(),
        "specialty".to_owned(),
        vec![],
        "p".repeat(1_025),
        "instructions".to_owned(),
        AgentBindings::default(),
    )
    .is_err());
    assert!(draft(
        "name".to_owned(),
        "description".to_owned(),
        "specialty".to_owned(),
        vec![],
        "personality".to_owned(),
        "i".repeat(4_097),
        AgentBindings::default(),
    )
    .is_err());
}

#[test]
fn profile_draft_rejects_whitespace_only_and_terminal_unsafe_text() {
    for unsafe_value in [
        " \t\n ",
        "bad\u{001b}[31m",
        "bad\0text",
        "bad\u{0001}text",
        "bad\ntext",
        "bad\u{0081}text",
        "bad\u{202e}text",
        "bad\u{2028}text",
        "bad\u{2029}text",
    ] {
        assert_eq!(
            draft(
                unsafe_value.to_owned(),
                "description".to_owned(),
                "specialty".to_owned(),
                vec![],
                "personality".to_owned(),
                "instructions".to_owned(),
                AgentBindings::default(),
            )
            .unwrap_err()
            .code(),
            if unsafe_value.trim().is_empty() {
                "invalid_profile_field"
            } else {
                "unsafe_profile_text"
            },
        );
    }
}

#[test]
fn profile_readiness_requires_both_model_binding_values_without_rejecting_partial_binding() {
    let mut profile = valid_draft();
    profile.bindings.model_provider = Some("openai".to_owned());
    assert_eq!(profile.readiness(), AgentReadiness::NotReady);

    profile.bindings.model_name = Some("gpt-5".to_owned());
    assert_eq!(profile.readiness(), AgentReadiness::Ready);
}
