use ai_stock_forum::{
    agents::{AgentRole, ProfileEditPreview, builtin_profile_templates},
    domain::{AgentProfileId, AgentProfileVersionId, ProfileReviewToken, sha256},
    ui::profile_editor::{ProfileEditor, ProfileTuiField},
};
use uuid::Uuid;

fn create_editor() -> ProfileEditor {
    ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap()
}

fn edit_editor() -> ProfileEditor {
    ProfileEditor::for_edit(
        AgentProfileId::from_uuid(Uuid::from_u128(501)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(502)),
        builtin_profile_templates()[0].copy_to_draft().unwrap(),
    )
}

fn move_to(editor: &mut ProfileEditor, field: ProfileTuiField) {
    while editor.tui_field() != field {
        editor.move_tui_field(true);
    }
}

#[test]
fn tui_fields_follow_the_complete_deliberate_order() {
    let mut editor = create_editor();
    let expected = [
        ProfileTuiField::Template,
        ProfileTuiField::DisplayName,
        ProfileTuiField::Role,
        ProfileTuiField::Description,
        ProfileTuiField::PrimarySpecialty,
        ProfileTuiField::Tags,
        ProfileTuiField::Personality,
        ProfileTuiField::Instructions,
        ProfileTuiField::Bindings,
        ProfileTuiField::Review,
        ProfileTuiField::Discard,
    ];

    assert_eq!(editor.tui_field(), expected[0]);
    for field in expected.into_iter().skip(1) {
        assert_eq!(editor.move_tui_field(true), field);
        assert_eq!(editor.tui_field(), field);
    }
    for field in expected.into_iter().rev().skip(1) {
        assert_eq!(editor.move_tui_field(false), field);
    }
}

#[test]
fn literal_tui_text_is_never_interpreted_as_navigation_or_controls() {
    let mut editor = create_editor();
    let literal = "wasd123456789/n :back";

    assert!(editor.set_tui_field(ProfileTuiField::DisplayName, literal));

    assert_eq!(editor.tui_field_text(ProfileTuiField::DisplayName), literal);
    assert_eq!(editor.draft().display_name, literal);
    assert_eq!(editor.tui_field(), ProfileTuiField::Template);
    assert_eq!(editor.tui_field_error(ProfileTuiField::DisplayName), None);
}

#[test]
fn role_is_an_explicit_selection_and_not_an_arbitrary_text_control() {
    let mut editor = create_editor();
    let original_role = editor.draft().role;

    assert!(!editor.set_tui_field(ProfileTuiField::Role, "bear"));
    assert_eq!(editor.draft().role, original_role);
    assert!(editor.select_tui_role(AgentRole::Bear));
    assert_eq!(editor.draft().role, AgentRole::Bear);
    assert_eq!(editor.tui_field_text(ProfileTuiField::Role), "bear");
}

#[test]
fn invalid_tui_fields_keep_exact_raw_text_and_independent_field_errors() {
    let mut editor = create_editor();
    let original_name = editor.draft().display_name.clone();
    let original_personality = editor.draft().personality.clone();

    assert!(!editor.set_tui_field(ProfileTuiField::DisplayName, "   "));
    assert!(!editor.set_tui_field(ProfileTuiField::Personality, "unsafe\ntext"));

    assert_eq!(editor.tui_field_text(ProfileTuiField::DisplayName), "   ");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Personality),
        "unsafe\ntext"
    );
    assert_eq!(editor.draft().display_name, original_name);
    assert_eq!(editor.draft().personality, original_personality);
    assert_eq!(
        editor.tui_field_error(ProfileTuiField::DisplayName),
        Some("invalid_profile_field")
    );
    assert_eq!(
        editor.tui_field_error(ProfileTuiField::Personality),
        Some("invalid_profile_field")
    );

    assert!(editor.set_tui_field(ProfileTuiField::DisplayName, "  Valid   Name  "));
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::DisplayName),
        "  Valid   Name  "
    );
    assert_eq!(editor.draft().display_name, "Valid Name");
    assert_eq!(editor.tui_field_error(ProfileTuiField::DisplayName), None);
    assert_eq!(
        editor.tui_field_error(ProfileTuiField::Personality),
        Some("invalid_profile_field")
    );
}

#[test]
fn comma_separated_tags_keep_raw_input_but_update_only_a_valid_canonical_draft() {
    let mut editor = create_editor();

    assert!(editor.set_tui_field(ProfileTuiField::Tags, " Quality, catalysts "));
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Tags),
        " Quality, catalysts "
    );
    assert_eq!(
        editor.draft().specialty_tags,
        vec!["catalysts".to_owned(), "Quality".to_owned()]
    );

    assert!(!editor.set_tui_field(ProfileTuiField::Tags, "quality, QUALITY"));
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Tags),
        "quality, QUALITY"
    );
    assert_eq!(
        editor.draft().specialty_tags,
        vec!["catalysts".to_owned(), "Quality".to_owned()]
    );
    assert_eq!(
        editor.tui_field_error(ProfileTuiField::Tags),
        Some("invalid_profile_field")
    );
}

#[test]
fn an_empty_tags_field_is_valid_even_while_another_custom_field_needs_input() {
    let custom = builtin_profile_templates()
        .iter()
        .find(|template| template.id.as_str() == "builtin.custom")
        .unwrap();
    let mut editor = ProfileEditor::for_create(custom).unwrap();

    assert!(editor.set_tui_field(ProfileTuiField::Tags, ""));
    assert_eq!(editor.draft().specialty_tags, Vec::<String>::new());
    assert_eq!(editor.tui_field_error(ProfileTuiField::Tags), None);
}

#[test]
fn every_literal_field_keeps_raw_text_while_the_draft_stays_single_line() {
    let mut editor = create_editor();

    for (field, raw) in [
        (ProfileTuiField::DisplayName, "  Literal   Name  "),
        (
            ProfileTuiField::Description,
            "  A   literal :back description  ",
        ),
        (ProfileTuiField::PrimarySpecialty, "  event   research  "),
        (ProfileTuiField::Tags, " catalysts, Quality "),
        (ProfileTuiField::Personality, "  wasd123456789/n   calm  "),
        (ProfileTuiField::Instructions, "  Treat :back as   prose  "),
    ] {
        assert!(editor.set_tui_field(field, raw), "rejected {field:?}");
        assert_eq!(editor.tui_field_text(field), raw);
    }

    assert_eq!(editor.draft().display_name, "Literal Name");
    assert_eq!(editor.draft().description, "A literal :back description");
    assert_eq!(editor.draft().primary_specialty, "event research");
    assert_eq!(
        editor.draft().specialty_tags,
        vec!["catalysts".to_owned(), "Quality".to_owned()]
    );
    assert_eq!(editor.draft().personality, "wasd123456789/n calm");
    assert_eq!(editor.draft().instructions, "Treat :back as prose");
}

#[test]
fn primary_specialty_cannot_make_the_validated_draft_collide_with_an_existing_tag() {
    let mut editor = create_editor();
    let before = editor.draft().primary_specialty.clone();

    assert!(!editor.set_tui_field(ProfileTuiField::PrimarySpecialty, "growth"));

    assert_eq!(
        editor.tui_field_text(ProfileTuiField::PrimarySpecialty),
        "growth"
    );
    assert_eq!(editor.draft().primary_specialty, before);
    assert_eq!(
        editor.tui_field_error(ProfileTuiField::PrimarySpecialty),
        Some("invalid_profile_field")
    );
}

#[test]
fn selecting_another_template_replaces_all_raw_field_caches_and_errors() {
    let mut editor = create_editor();
    assert!(!editor.set_tui_field(ProfileTuiField::Instructions, "\0invalid"));

    let bear = &builtin_profile_templates()[1];
    assert!(editor.select_template(bear));

    assert_eq!(editor.tui_field(), ProfileTuiField::Template);
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Template),
        "builtin.bear"
    );
    assert_eq!(editor.tui_field_text(ProfileTuiField::Role), "bear");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::DisplayName),
        bear.suggested_name
    );
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Tags),
        bear.specialty_tags.join(", ")
    );
    for field in [
        ProfileTuiField::Template,
        ProfileTuiField::DisplayName,
        ProfileTuiField::Role,
        ProfileTuiField::Description,
        ProfileTuiField::PrimarySpecialty,
        ProfileTuiField::Tags,
        ProfileTuiField::Personality,
        ProfileTuiField::Instructions,
        ProfileTuiField::Bindings,
        ProfileTuiField::Review,
        ProfileTuiField::Discard,
    ] {
        assert_eq!(
            editor.tui_field_error(field),
            None,
            "stale error for {field:?}"
        );
    }
}

#[test]
fn any_invalid_tui_field_invalidates_a_preview_and_blocks_another_review() {
    let mut editor = edit_editor();
    move_to(&mut editor, ProfileTuiField::Review);
    let request = match editor.submit_line(":review") {
        ai_stock_forum::ui::profile_editor::ProfileEditorEffect::PreviewEdit(request) => request,
        effect => panic!("expected preview request, got {effect:?}"),
    };
    editor.apply_preview(
        request.generation,
        ProfileEditPreview {
            profile_id: request.profile_id,
            expected_active_version_id: request.expected_active_version_id,
            diffs: Vec::new(),
            review_token: ProfileReviewToken::from_uuid(Uuid::from_u128(503)),
            review_digest: sha256(b"profile-tui-review"),
        },
    );
    assert!(editor.review().is_some());

    assert!(!editor.set_tui_field(ProfileTuiField::DisplayName, "   "));
    assert!(editor.review().is_none());
    move_to(&mut editor, ProfileTuiField::Review);
    assert_eq!(
        editor.submit_line(":review"),
        ai_stock_forum::ui::profile_editor::ProfileEditorEffect::None
    );
    assert_eq!(
        editor.local_message().map(|message| message.code()),
        Some("invalid_profile_field")
    );
}

#[test]
fn fallback_text_mutations_refresh_only_their_tui_field_and_repair_same_values() {
    let mut editor = create_editor();
    let original_name = editor.draft().display_name.clone();
    move_to(&mut editor, ProfileTuiField::DisplayName);

    assert!(!editor.set_tui_field(ProfileTuiField::DisplayName, "   "));
    editor.submit_line(&original_name);
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::DisplayName),
        original_name
    );
    assert_eq!(editor.tui_field_error(ProfileTuiField::DisplayName), None);

    editor.submit_line("  Fallback   Name  ");
    assert_eq!(editor.draft().display_name, "Fallback Name");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::DisplayName),
        "Fallback Name"
    );

    move_to(&mut editor, ProfileTuiField::Description);
    editor.submit_line("  Fallback   description  ");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Description),
        "Fallback description"
    );

    move_to(&mut editor, ProfileTuiField::PrimarySpecialty);
    editor.submit_line("  fallback   research  ");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::PrimarySpecialty),
        "fallback research"
    );

    move_to(&mut editor, ProfileTuiField::Personality);
    editor.submit_line("  Fallback   personality  ");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Personality),
        "Fallback personality"
    );

    move_to(&mut editor, ProfileTuiField::Instructions);
    editor.submit_line("  Fallback   instructions  ");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Instructions),
        "Fallback instructions"
    );
}

#[test]
fn fallback_clear_repairs_raw_text_and_error_even_when_the_draft_is_already_empty() {
    let mut editor = create_editor();
    move_to(&mut editor, ProfileTuiField::Description);
    editor.submit_line(":clear");
    assert_eq!(editor.draft().description, "");
    assert_eq!(editor.tui_field_text(ProfileTuiField::Description), "");

    assert!(!editor.set_tui_field(ProfileTuiField::Description, "bad\nvalue"));
    editor.submit_line(":clear");

    assert_eq!(editor.draft().description, "");
    assert_eq!(editor.tui_field_text(ProfileTuiField::Description), "");
    assert_eq!(editor.tui_field_error(ProfileTuiField::Description), None);
}

#[test]
fn fallback_tag_controls_replace_the_tags_raw_cache_and_clear_its_error() {
    let mut editor = create_editor();
    move_to(&mut editor, ProfileTuiField::Tags);
    assert!(!editor.set_tui_field(ProfileTuiField::Tags, "growth, GROWTH"));

    editor.submit_line(":tag add moat");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Tags),
        "catalysts, growth, moat"
    );
    assert_eq!(editor.tui_field_error(ProfileTuiField::Tags), None);

    editor.submit_line(":tag remove catalysts");
    assert_eq!(editor.tui_field_text(ProfileTuiField::Tags), "growth, moat");
    assert_eq!(
        editor.draft().specialty_tags,
        vec!["growth".to_owned(), "moat".to_owned()]
    );
}

#[test]
fn fallback_role_and_navigation_preserve_unrelated_invalid_raw_fields() {
    let mut editor = create_editor();
    assert!(!editor.set_tui_field(ProfileTuiField::Instructions, "unsafe\ntext"));

    editor.submit_line(":role bear");
    assert_eq!(editor.draft().role, AgentRole::Bear);
    assert_eq!(editor.tui_field_text(ProfileTuiField::Role), "bear");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Instructions),
        "unsafe\ntext"
    );
    assert_eq!(
        editor.tui_field_error(ProfileTuiField::Instructions),
        Some("invalid_profile_field")
    );

    editor.submit_line(":next");
    editor.submit_line(":back");
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::Instructions),
        "unsafe\ntext"
    );
    assert_eq!(
        editor.tui_field_error(ProfileTuiField::Instructions),
        Some("invalid_profile_field")
    );
}
