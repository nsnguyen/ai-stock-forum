use ai_stock_forum::{
    app::ApplicationCommand,
    domain::{SkillId, SkillReviewToken, sha256},
    skills::{SkillDraft, SkillEditPreview, SkillResource},
    ui::skill_editor::{SkillEditor, SkillEditorEffect, SkillEditorField, SkillEditorStep},
};
use uuid::Uuid;

fn seeded_editor() -> SkillEditor {
    SkillEditor::for_create(Some(
        SkillDraft::new(
            "Evidence".to_owned(),
            "Review evidence".to_owned(),
            "Use for research".to_owned(),
            vec!["research".to_owned()],
            "Cite sources".to_owned(),
            vec![],
        )
        .unwrap(),
    ))
}

fn preview() -> SkillEditPreview {
    SkillEditPreview {
        skill_id: SkillId::from_uuid(Uuid::from_u128(410)),
        expected_active_version_id: None,
        candidate_digest: sha256(b"candidate"),
        review_token: SkillReviewToken::from_uuid(Uuid::from_u128(411)),
        review_digest: sha256(b"review"),
    }
}

fn request_review(editor: &mut SkillEditor) -> u64 {
    editor.go_to_review().unwrap();
    let SkillEditorEffect::Preview(request) = editor.submit_keyboard_line("") else {
        panic!("preview required before confirmation");
    };
    request.generation()
}

#[test]
fn arbitrary_field_navigation_preserves_invalid_raw_without_advancing() {
    let mut editor = SkillEditor::for_create(None);
    editor.select_tui_field(SkillEditorField::Instructions);
    assert!(editor.set_tui_field(SkillEditorField::Instructions, "Read\nthen explain"));
    assert_eq!(editor.field(), SkillEditorField::Instructions);
    assert_eq!(editor.step(), SkillEditorStep::Instructions);

    editor.select_tui_field(SkillEditorField::DisplayName);
    let invalid = "x".repeat(65);
    assert!(!editor.set_tui_field(SkillEditorField::DisplayName, &invalid));
    editor.select_tui_field(SkillEditorField::Tags);
    assert!(editor.set_tui_field(SkillEditorField::Tags, "  research, evidence  "));
    assert_eq!(editor.field(), SkillEditorField::Tags);
    assert_eq!(
        editor.tui_field_text(SkillEditorField::DisplayName),
        invalid
    );
    assert_eq!(
        editor.tui_field_text(SkillEditorField::Tags),
        "  research, evidence  "
    );
    assert_eq!(
        editor.tui_field_text(SkillEditorField::Instructions),
        "Read\nthen explain"
    );
    assert!(editor.go_to_review().is_err());

    assert!(editor.set_tui_field(SkillEditorField::DisplayName, "Evidence"));
    assert!(editor.set_tui_field(SkillEditorField::UseWhen, "Use for research"));
    editor.go_to_review().unwrap();
    assert_eq!(editor.draft().tags, ["evidence", "research"]);
}

#[test]
fn navigation_preserves_review_but_invalid_raw_changes_reject_late_previews() {
    let mut editor = seeded_editor();
    let generation = request_review(&mut editor);
    editor.select_tui_field(SkillEditorField::Purpose);
    assert!(editor.apply_preview(generation, preview()));
    editor.select_tui_field(SkillEditorField::Review);
    assert!(matches!(
        editor.submit_keyboard_line(""),
        SkillEditorEffect::Execute(ApplicationCommand::CreateSkill { .. })
    ));

    editor.select_tui_field(SkillEditorField::Purpose);
    assert!(!editor.set_tui_field(SkillEditorField::Purpose, &"x".repeat(257)));
    assert!(editor.review().is_none());
    assert!(!editor.apply_preview(generation, preview()));
    assert!(editor.set_tui_field(SkillEditorField::Purpose, "Repair"));
    let next_generation = request_review(&mut editor);
    assert!(next_generation > generation);
    assert!(editor.set_tui_field(SkillEditorField::Instructions, "New instructions"));
    assert!(!editor.apply_preview(next_generation, preview()));
    assert!(matches!(
        editor.submit_keyboard_line(""),
        SkillEditorEffect::Preview(_)
    ));
}

#[test]
fn unchanged_field_submission_keeps_the_existing_review() {
    let mut editor = seeded_editor();
    let generation = request_review(&mut editor);
    assert!(editor.apply_preview(generation, preview()));
    assert!(editor.set_tui_field(SkillEditorField::DisplayName, "Evidence"));
    assert!(editor.review().is_some());
}

#[test]
fn pending_reference_survives_navigation_and_commits_only_after_repair() {
    let mut editor = seeded_editor();
    assert!(editor.begin_add_reference());
    assert!(!editor.has_pending_reference());
    editor.go_to_review().unwrap();
    editor.select_tui_field(SkillEditorField::ReferenceName);
    assert!(editor.set_tui_field(SkillEditorField::ReferenceName, "Source note"));
    assert!(!editor.set_tui_field(SkillEditorField::ReferenceBody, &"x".repeat(4_097)));
    assert!(!editor.commit_tui_reference());
    editor.select_tui_field(SkillEditorField::UseWhen);
    assert!(editor.has_pending_reference());
    assert!(!editor.begin_add_reference());
    assert!(editor.go_to_review().is_err());
    editor.select_tui_field(SkillEditorField::Review);
    assert_eq!(editor.submit_keyboard_line(""), SkillEditorEffect::None);
    assert!(editor.references().is_empty());
    assert_eq!(
        editor.tui_field_text(SkillEditorField::ReferenceName),
        "Source note"
    );
    assert_eq!(
        editor.tui_field_text(SkillEditorField::ReferenceBody).len(),
        4_097
    );

    assert!(editor.set_tui_field(
        SkillEditorField::ReferenceBody,
        "Quoted evidence\nSecond line"
    ));
    assert!(editor.commit_tui_reference());
    assert!(!editor.has_pending_reference());
    assert_eq!(
        editor.references(),
        &[SkillResource {
            name: "Source note".to_owned(),
            body: "Quoted evidence\nSecond line".to_owned(),
        }]
    );
    editor.go_to_review().unwrap();
}

#[test]
fn reference_edits_are_atomic_and_removal_invalidates_review() {
    let mut editor = seeded_editor();
    assert!(editor.begin_add_reference());
    assert!(editor.set_tui_field(SkillEditorField::ReferenceName, "First"));
    assert!(editor.set_tui_field(SkillEditorField::ReferenceBody, "Original"));
    assert!(editor.commit_tui_reference());
    assert!(editor.begin_add_reference());
    assert!(editor.set_tui_field(SkillEditorField::ReferenceName, "Second"));
    assert!(editor.commit_tui_reference());

    editor.select_reference(true);
    assert!(editor.begin_edit_selected_reference());
    assert!(editor.set_tui_field(SkillEditorField::ReferenceName, "second"));
    assert!(!editor.commit_tui_reference());
    editor.select_reference(true);
    assert!(!editor.begin_edit_selected_reference());
    assert!(!editor.remove_selected_reference());
    assert_eq!(editor.references()[0].name, "First");
    assert!(editor.set_tui_field(SkillEditorField::ReferenceName, "Renamed"));
    assert!(editor.set_tui_field(SkillEditorField::ReferenceBody, "Updated"));
    assert!(editor.commit_tui_reference());
    assert_eq!(
        editor.references()[0],
        SkillResource {
            name: "Renamed".to_owned(),
            body: "Updated".to_owned(),
        }
    );
    assert_eq!(editor.references().len(), 2);

    let generation = request_review(&mut editor);
    assert!(editor.apply_preview(generation, preview()));
    editor.select_reference(true);
    assert!(editor.remove_selected_reference());
    assert!(editor.review().is_none());
    assert!(!editor.apply_preview(generation, preview()));
    assert_eq!(editor.references().len(), 1);
}
