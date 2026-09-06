use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentRole, ProfileEditPreview, builtin_profile_templates,
    },
    domain::{AgentProfileId, AgentProfileVersionId, ProfileReviewToken, sha256},
    ui::profile_editor::{
        PreviewEditRequest, ProfileEditor, ProfileEditorEffect, ProfileEditorMode,
        ProfileEditorStep,
    },
};
use uuid::Uuid;

fn template_draft() -> AgentProfileDraft {
    builtin_profile_templates()[0].copy_to_draft().unwrap()
}

fn create_editor() -> ProfileEditor {
    ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap()
}

fn empty_draft() -> AgentProfileDraft {
    AgentProfileDraft::new(
        "Name".to_owned(),
        "Description".to_owned(),
        AgentRole::Custom,
        "Research".to_owned(),
        Vec::new(),
        "Personality".to_owned(),
        "Instructions".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn preview_request(editor: &mut ProfileEditor) -> PreviewEditRequest {
    match editor.submit_line(":review") {
        ProfileEditorEffect::PreviewEdit(request) => request,
        effect => panic!("expected preview request, received {effect:?}"),
    }
}

fn preview(
    profile_id: AgentProfileId,
    expected_active_version_id: AgentProfileVersionId,
    token: u128,
) -> ProfileEditPreview {
    ProfileEditPreview {
        profile_id,
        expected_active_version_id,
        diffs: Vec::new(),
        review_token: ProfileReviewToken::from_uuid(Uuid::from_u128(token)),
        review_digest: sha256(&token.to_be_bytes()),
    }
}

fn advance_to_review(editor: &mut ProfileEditor) {
    assert_eq!(editor.step(), ProfileEditorStep::Template);
    assert_eq!(editor.submit_line(":next"), ProfileEditorEffect::None);
    assert_eq!(editor.step(), ProfileEditorStep::Identity);
    editor.submit_line("Focused Analyst");
    editor.submit_line(":next");
    editor.submit_line("Covers fundamentals.");
    editor.submit_line(":next");
    assert_eq!(editor.step(), ProfileEditorStep::Specialty);
    editor.submit_line("Long-term investing");
    editor.submit_line(":tag add valuation");
    editor.submit_line(":next");
    assert_eq!(editor.step(), ProfileEditorStep::Personality);
    editor.submit_line("Calm and evidence-led.");
    editor.submit_line("States uncertainty plainly.");
    editor.submit_line(":next");
    assert_eq!(editor.step(), ProfileEditorStep::Instructions);
    editor.submit_line("Use primary filings.");
    editor.submit_line("Separate facts from estimates.");
    editor.submit_line(":next");
    assert_eq!(editor.step(), ProfileEditorStep::OptionalBindings);
    editor.submit_line(":next");
    assert_eq!(editor.step(), ProfileEditorStep::Review);
}

#[test]
fn create_editor_copies_template_and_walks_the_exact_ordered_steps() {
    let mut editor = create_editor();
    assert!(matches!(editor.mode(), ProfileEditorMode::Create { .. }));
    assert_eq!(editor.step(), ProfileEditorStep::Template);
    assert_eq!(editor.draft(), &template_draft());

    advance_to_review(&mut editor);

    assert_eq!(editor.draft().display_name, "Focused Analyst");
    assert_eq!(editor.draft().description, "Covers fundamentals.");
    assert_eq!(editor.draft().primary_specialty, "Long-term investing");
    assert!(
        editor
            .draft()
            .specialty_tags
            .iter()
            .any(|tag| tag == "valuation")
    );
    assert_eq!(
        editor.draft().personality,
        "Calm and evidence-led. States uncertainty plainly."
    );
    assert_eq!(
        editor.draft().instructions,
        "Use primary filings. Separate facts from estimates."
    );
    assert_eq!(editor.draft().bindings, AgentBindings::default());

    assert_eq!(editor.submit_line(":review"), ProfileEditorEffect::None);
    assert_eq!(editor.submit_line(":activate"), ProfileEditorEffect::None);
    assert!(matches!(
        editor.submit_line(":create"),
        ProfileEditorEffect::Execute(_)
    ));
}

#[test]
fn editor_keeps_invalid_input_and_supports_back_navigation_without_terminal_state() {
    let mut editor = create_editor();
    editor.submit_line(":next");
    editor.submit_line("   ");
    assert_eq!(editor.submit_line(":next"), ProfileEditorEffect::None);
    assert_eq!(editor.step(), ProfileEditorStep::Identity);
    assert_eq!(editor.draft().display_name, "   ");
    assert!(editor.local_message().is_some());

    editor.submit_line("Valid Name");
    editor.submit_line(":next");
    editor.submit_line("A description");
    editor.submit_line(":next");
    assert_eq!(editor.step(), ProfileEditorStep::Specialty);
    assert_eq!(editor.submit_line(":back"), ProfileEditorEffect::None);
    assert_eq!(editor.step(), ProfileEditorStep::Identity);
    assert_eq!(editor.draft().description, "A description");
    assert_eq!(editor.submit_line(":bogus"), ProfileEditorEffect::None);
    assert!(editor.local_message().is_some());
}

#[test]
fn specialty_tag_cap_and_binding_clear_are_local_draft_operations() {
    let mut editor = ProfileEditor::for_edit(
        AgentProfileId::from_uuid(Uuid::from_u128(10)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(11)),
        empty_draft(),
    );
    editor.submit_line(":next");
    editor.submit_line("Name");
    editor.submit_line(":next");
    editor.submit_line("Description");
    editor.submit_line(":next");
    editor.submit_line("Research");
    for tag in ["one", "two", "three", "four", "five"] {
        assert_eq!(
            editor.submit_line(&format!(":tag add {tag}")),
            ProfileEditorEffect::None
        );
    }
    assert_eq!(
        editor.draft().specialty_tags,
        vec!["five", "four", "one", "three", "two"]
    );
    assert_eq!(
        editor.submit_line(":tag add six"),
        ProfileEditorEffect::None
    );
    assert_eq!(editor.draft().specialty_tags.len(), 5);
    assert!(editor.local_message().is_some());
    editor.submit_line(":tag remove three");
    assert_eq!(
        editor.draft().specialty_tags,
        vec!["five", "four", "one", "two"]
    );

    editor.submit_line(":next");
    editor.submit_line("Personality");
    editor.submit_line(":next");
    editor.submit_line("Instructions");
    editor.submit_line(":next");
    editor.submit_line(":clear");
    assert_eq!(editor.draft().bindings, AgentBindings::default());
}

#[test]
fn edit_review_requires_a_fresh_preview_after_any_field_change() {
    let profile_id = AgentProfileId::from_uuid(Uuid::from_u128(1));
    let active_version_id = AgentProfileVersionId::from_uuid(Uuid::from_u128(2));
    let mut editor = ProfileEditor::for_edit(profile_id, active_version_id, template_draft());
    assert!(matches!(editor.mode(), ProfileEditorMode::Edit { .. }));

    advance_to_review(&mut editor);
    let request = preview_request(&mut editor);
    editor.apply_preview(
        request.generation,
        preview(profile_id, active_version_id, 3),
    );
    assert!(editor.review().is_some());
    assert_eq!(editor.submit_line(":back"), ProfileEditorEffect::None);
    assert_eq!(editor.step(), ProfileEditorStep::OptionalBindings);
    editor.submit_line(":back");
    editor.submit_line(":back");
    assert_eq!(editor.step(), ProfileEditorStep::Personality);
    editor.submit_line("Updated personality.");
    assert!(editor.review().is_none());
    editor.submit_line(":next");
    editor.submit_line(":next");
    editor.submit_line(":next");
    assert_eq!(editor.step(), ProfileEditorStep::Review);
    assert!(matches!(
        editor.submit_line(":review"),
        ProfileEditorEffect::PreviewEdit(_)
    ));
}

#[test]
fn delayed_preview_after_candidate_mutation_is_rejected_until_the_current_preview_arrives() {
    let profile_id = AgentProfileId::from_uuid(Uuid::from_u128(20));
    let active_version_id = AgentProfileVersionId::from_uuid(Uuid::from_u128(21));
    let mut editor = ProfileEditor::for_edit(profile_id, active_version_id, template_draft());
    advance_to_review(&mut editor);
    let request_a = preview_request(&mut editor);

    editor.submit_line(":back");
    editor.submit_line(":back");
    editor.submit_line(":back");
    assert_eq!(editor.step(), ProfileEditorStep::Personality);
    editor.submit_line("Updated local personality.");
    editor.submit_line(":next");
    editor.submit_line(":next");
    editor.submit_line(":next");
    let request_b = preview_request(&mut editor);
    assert!(request_b.generation > request_a.generation);

    editor.apply_preview(
        request_a.generation,
        preview(profile_id, active_version_id, 22),
    );
    assert!(editor.review().is_none());
    assert_eq!(editor.submit_line(":activate"), ProfileEditorEffect::None);
    assert_eq!(editor.local_message().unwrap().code(), "stale_preview");

    editor.apply_preview(
        request_b.generation,
        preview(profile_id, active_version_id, 23),
    );
    assert!(editor.review().is_some());
    assert!(matches!(
        editor.submit_line(":activate"),
        ProfileEditorEffect::Execute(_)
    ));
}

#[test]
fn newer_preview_request_wins_when_responses_arrive_out_of_order() {
    let profile_id = AgentProfileId::from_uuid(Uuid::from_u128(30));
    let active_version_id = AgentProfileVersionId::from_uuid(Uuid::from_u128(31));
    let mut editor = ProfileEditor::for_edit(profile_id, active_version_id, template_draft());
    advance_to_review(&mut editor);
    let request_a = preview_request(&mut editor);
    let request_b = preview_request(&mut editor);
    assert!(request_b.generation > request_a.generation);

    editor.apply_preview(
        request_a.generation,
        preview(profile_id, active_version_id, 32),
    );
    assert!(editor.review().is_none());
    assert_eq!(editor.local_message().unwrap().code(), "stale_preview");

    editor.apply_preview(
        request_b.generation,
        preview(profile_id, active_version_id, 33),
    );
    assert!(editor.review().is_some());
    assert!(matches!(
        editor.submit_line(":activate"),
        ProfileEditorEffect::Execute(_)
    ));
}

#[test]
fn cancel_is_the_only_terminal_local_effect_and_never_exposes_prose_in_controls() {
    let mut editor = create_editor();
    editor.submit_line(":next");
    editor.submit_line("Private profile prose must remain local.");
    let controls = editor.control_summary();
    assert!(!controls.contains("Private profile prose"));
    assert_eq!(
        editor.submit_line(":cancel"),
        ProfileEditorEffect::Cancelled
    );
}

#[test]
fn custom_template_stays_editable_until_required_meaningful_fields_are_supplied() {
    let custom = builtin_profile_templates()
        .iter()
        .find(|template| template.id.as_str() == "builtin.custom")
        .unwrap();
    let mut editor = ProfileEditor::for_create(custom).unwrap();
    assert!(editor.draft().primary_specialty.is_empty());
    assert!(editor.draft().personality.is_empty());
    assert!(editor.draft().instructions.is_empty());

    editor.submit_line(":next");
    editor.submit_line("Special Situations Analyst");
    editor.submit_line(":next");
    editor.submit_line(":clear");
    editor.submit_line(":next");
    editor.submit_line("Event-driven research");
    editor.submit_line(":next");
    editor.submit_line("Skeptical and explicit");
    editor.submit_line(":next");
    editor.submit_line("List evidence and invalidators");
    editor.submit_line(":next");
    editor.submit_line(":next");

    assert_eq!(editor.step(), ProfileEditorStep::Review);
    assert!(matches!(
        editor.submit_line(":create"),
        ProfileEditorEffect::Execute(_)
    ));
}

#[test]
fn multiline_limits_are_cumulative_and_rejection_preserves_the_draft() {
    let mut editor = create_editor();
    editor.submit_line(":next");
    editor.submit_line("Name");
    editor.submit_line(":next");
    editor.submit_line("");
    editor.submit_line(":next");
    editor.submit_line("Research");
    editor.submit_line(":next");

    let maximum = "p".repeat(1_024);
    editor.submit_line(&maximum);
    let before = editor.draft().personality.clone();
    editor.submit_line("overflow");
    assert_eq!(editor.draft().personality, before);
    assert_eq!(
        editor.local_message().unwrap().code(),
        "profile_field_limit"
    );
}

#[test]
fn recoverable_application_errors_leave_editor_state_and_draft_intact() {
    let mut editor = create_editor();
    editor.submit_line(":next");
    editor.submit_line("Private retained draft");
    let before = editor.clone();

    editor.report_error("capability_denied");

    assert_eq!(editor.draft(), before.draft());
    assert_eq!(editor.step(), before.step());
    assert_eq!(editor.local_message().unwrap().code(), "capability_denied");
}
