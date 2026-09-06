use ai_stock_forum::{
    app::ApplicationCommand,
    domain::{SkillId, SkillReviewToken, SkillVersionId, sha256},
    skills::{SkillDraft, SkillEditPreview},
    ui::skill_editor::{
        SkillEditor, SkillEditorEffect, SkillEditorField, SkillEditorStep, SkillPreviewRequest,
    },
};
use uuid::Uuid;

fn draft(name: &str) -> SkillDraft {
    SkillDraft::new(
        name.to_owned(),
        "Explain the evidence clearly.".to_owned(),
        "Use when a claim needs evidence review.".to_owned(),
        vec!["evidence".to_owned(), "review".to_owned()],
        "Separate observations from conclusions.".to_owned(),
        Vec::new(),
    )
    .expect("valid skill draft")
}

fn advance_create_to_review(editor: &mut SkillEditor) {
    for input in [
        "Evidence Notes",
        "Explain the evidence clearly.",
        "Use when a claim needs evidence review.",
        "review, evidence",
        "Separate observations from conclusions.",
        "",
    ] {
        assert_eq!(editor.submit_keyboard_line(input), SkillEditorEffect::None);
    }
    assert_eq!(editor.step(), SkillEditorStep::Review);
}

#[test]
fn keyboard_lines_advance_all_five_stages_and_escape_preserves_the_draft() {
    let mut editor = SkillEditor::for_create(None);
    assert_eq!(editor.step(), SkillEditorStep::Identity);
    assert_eq!(editor.field(), SkillEditorField::DisplayName);

    advance_create_to_review(&mut editor);
    assert_eq!(editor.draft().display_name, "Evidence Notes");
    assert_eq!(editor.draft().description, "Explain the evidence clearly.");
    assert_eq!(editor.draft().tags, ["evidence", "review"]);

    assert_eq!(editor.back(), SkillEditorEffect::None);
    assert_eq!(editor.step(), SkillEditorStep::References);
    assert_eq!(editor.draft().instructions, "Separate observations from conclusions.");
    assert_eq!(editor.submit_keyboard_line(""), SkillEditorEffect::None);
    assert_eq!(editor.step(), SkillEditorStep::Review);
}

#[test]
fn field_specific_validation_retains_recoverable_user_input() {
    let mut editor = SkillEditor::for_create(None);
    let invalid_name = "x".repeat(65);

    assert_eq!(
        editor.submit_keyboard_line(&invalid_name),
        SkillEditorEffect::None
    );

    assert_eq!(editor.step(), SkillEditorStep::Identity);
    assert_eq!(editor.field(), SkillEditorField::DisplayName);
    assert_eq!(editor.raw_display_name(), invalid_name);
    let error = editor.local_error().expect("field error");
    assert_eq!(error.field(), SkillEditorField::DisplayName);
    assert_eq!(error.code(), "skill_display_name_invalid");
}

#[test]
fn create_review_uses_the_application_preview_token_and_never_requires_colon_commands() {
    let mut editor = SkillEditor::for_create(None);
    advance_create_to_review(&mut editor);

    let SkillEditorEffect::Preview(SkillPreviewRequest::Create {
        generation,
        candidate,
    }) = editor.submit_keyboard_line("")
    else {
        panic!("create preview request")
    };
    let preview = SkillEditPreview {
        skill_id: SkillId::from_uuid(Uuid::from_u128(10)),
        expected_active_version_id: None,
        candidate_digest: sha256(b"candidate"),
        review_token: SkillReviewToken::from_uuid(Uuid::from_u128(11)),
        review_digest: sha256(b"review"),
    };
    assert!(editor.apply_preview(generation, preview.clone()));

    let SkillEditorEffect::Execute(ApplicationCommand::CreateSkill {
        skill_id,
        candidate: command_candidate,
        review_token,
        review_digest,
    }) = editor.submit_keyboard_line("")
    else {
        panic!("typed create command")
    };
    assert_eq!(skill_id, preview.skill_id);
    assert_eq!(command_candidate, candidate);
    assert_eq!(review_token, preview.review_token);
    assert_eq!(review_digest, preview.review_digest);
}

#[test]
fn version_review_binds_the_exact_active_version_and_returned_token() {
    let skill_id = SkillId::from_uuid(Uuid::from_u128(20));
    let active_version_id = SkillVersionId::from_uuid(Uuid::from_u128(21));
    let mut editor = SkillEditor::for_version(skill_id, active_version_id, draft("Versioned"));
    editor.go_to_review().expect("valid seeded draft");

    let SkillEditorEffect::Preview(SkillPreviewRequest::Version {
        generation,
        skill_id: requested_skill_id,
        expected_active_version_id,
        candidate,
    }) = editor.submit_keyboard_line("")
    else {
        panic!("version preview request")
    };
    assert_eq!(requested_skill_id, skill_id);
    assert_eq!(expected_active_version_id, active_version_id);

    let preview = SkillEditPreview {
        skill_id,
        expected_active_version_id: Some(active_version_id),
        candidate_digest: sha256(b"version-candidate"),
        review_token: SkillReviewToken::from_uuid(Uuid::from_u128(22)),
        review_digest: sha256(b"version-review"),
    };
    assert!(editor.apply_preview(generation, preview.clone()));

    let SkillEditorEffect::Execute(ApplicationCommand::ActivateSkillVersion {
        skill_id: command_skill_id,
        expected_active_version_id: command_version_id,
        candidate: command_candidate,
        review_token,
        review_digest,
    }) = editor.submit_keyboard_line("")
    else {
        panic!("typed version command")
    };
    assert_eq!(command_skill_id, skill_id);
    assert_eq!(command_version_id, active_version_id);
    assert_eq!(command_candidate, candidate);
    assert_eq!(review_token, preview.review_token);
    assert_eq!(review_digest, preview.review_digest);
}

#[test]
fn invalid_reference_body_keeps_its_name_and_accepts_a_corrected_body() {
    let mut editor = SkillEditor::for_create(None);
    for input in [
        "Reference Repair",
        "Purpose",
        "Use when reference notes help.",
        "",
        "Follow the note.",
        "Source note",
    ] {
        assert_eq!(editor.submit_keyboard_line(input), SkillEditorEffect::None);
    }
    assert_eq!(editor.field(), SkillEditorField::ReferenceBody);

    editor.submit_keyboard_line(&"x".repeat(4_097));
    assert_eq!(editor.field(), SkillEditorField::ReferenceBody);
    assert_eq!(editor.pending_reference_name(), Some("Source note"));

    editor.submit_keyboard_line("Inert text only.");
    editor.submit_keyboard_line("");
    assert_eq!(editor.step(), SkillEditorStep::Review);
    assert_eq!(editor.draft().resources[0].name, "Source note");
    assert_eq!(editor.draft().resources[0].body, "Inert text only.");
}
