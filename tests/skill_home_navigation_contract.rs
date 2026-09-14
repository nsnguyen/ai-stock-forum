use ai_stock_forum::{
    app::{AgentProfilesView, DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership},
    domain::{InstallationId, SessionId},
    setup::SetupStatus,
    skills::SkillDraft,
    ui::{
        skill_editor::SkillEditorField,
        tui::{
            ControllerEffect, TuiEvent, handle_event,
            model::{
                Focus, InputMode, SkillConfirmation, SkillEditorPage, SkillOperationOrigin,
                SkillsPane, TuiModel, View,
            },
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

fn model() -> TuiModel {
    let mut model = TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: vec![],
            agent_profiles: AgentProfilesView {
                profiles: vec![],
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            selected_agent_profile: None,
            selected_agent_profile_history: None,
        },
        false,
    );
    model.set_terminal_size(160, 40);
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE)),
    );
    model.skills.library_loaded = true;
    model.set_focus(Focus::Workspace);
    model
}

fn draft() -> SkillDraft {
    SkillDraft::new(
        "Catalyst Mapping".to_owned(),
        "Find upcoming catalysts".to_owned(),
        "Before company events".to_owned(),
        vec!["research".to_owned()],
        "List the timing and evidence".to_owned(),
        vec![],
    )
    .unwrap()
}

fn editor_model() -> TuiModel {
    let mut model = model();
    model.skills.start_create(Some(draft()));
    model
}

fn press(model: &mut TuiModel, code: KeyCode) -> ControllerEffect {
    handle_event(
        model,
        TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
    )
}

#[test]
fn home_navigation_never_enters_text_and_down_opens_instructions() {
    let mut model = editor_model();
    press(&mut model, KeyCode::Char('s'));
    press(&mut model, KeyCode::Enter);
    assert_eq!(
        model.skills.editor.as_ref().unwrap().field(),
        SkillEditorField::Instructions
    );
    assert_eq!(
        model.skills.editor.as_ref().unwrap().raw_display_name(),
        "Catalyst Mapping"
    );
    assert_eq!(model.input_mode, InputMode::Nav);
    assert!(model.command.text().is_empty());
}

#[test]
fn home_requires_opening_a_section_then_entering_a_field_to_type() {
    let mut model = editor_model();
    press(&mut model, KeyCode::Enter);
    assert_eq!(model.input_mode, InputMode::Nav);
    assert_eq!(
        model.skills.editor.as_ref().unwrap().field(),
        SkillEditorField::DisplayName
    );
    press(&mut model, KeyCode::Enter);
    assert_eq!(model.input_mode, InputMode::Type);
}

#[test]
fn tab_cycles_panes_without_consuming_or_cancelling_a_skill_draft() {
    let mut model = editor_model();
    press(&mut model, KeyCode::Tab);
    assert_eq!(model.focus, Focus::Navigation);
    press(&mut model, KeyCode::Tab);
    assert_eq!(model.focus, Focus::List);
    press(&mut model, KeyCode::Tab);
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(
        model.skills.editor.as_ref().unwrap().raw_display_name(),
        "Catalyst Mapping"
    );
}

#[test]
fn escape_from_home_keeps_the_draft_and_existing_editor_can_resume() {
    let mut model = editor_model();
    press(&mut model, KeyCode::Esc);
    assert!(model.skills.editor.is_some(), "Esc is back, not discard");
    assert_eq!(model.skills.pane, SkillsPane::Detail);
    press(&mut model, KeyCode::Char('e'));
    assert_eq!(model.skills.pane, SkillsPane::Editor);
    assert_eq!(
        model.skills.editor.as_ref().unwrap().raw_display_name(),
        "Catalyst Mapping"
    );
}

#[test]
fn number_navigation_preserves_a_skill_draft_outside_type_mode() {
    let mut model = editor_model();
    press(&mut model, KeyCode::Char('1'));
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Overview);
    press(&mut model, KeyCode::Char('4'));
    assert!(model.skills.active);
    assert_eq!(
        model.skills.editor.as_ref().unwrap().raw_display_name(),
        "Catalyst Mapping"
    );
}

#[test]
fn new_shortcut_opens_starter_picker_not_automatic_text_entry() {
    let mut model = model();
    model.set_focus(Focus::List);
    press(&mut model, KeyCode::Char('n'));
    assert_eq!(model.skills.pane, SkillsPane::CreateSource);
    assert!(model.skills.editor.is_none());
    assert_eq!(model.input_mode, InputMode::Nav);
}

#[test]
fn confirmation_tab_moves_focus_and_enter_in_another_pane_cannot_save() {
    let mut model = editor_model();
    model.skills.pane = SkillsPane::Confirmation;
    model.skills.pending_confirmation = Some(SkillConfirmation {
        command: ai_stock_forum::app::ApplicationCommand::RequestShutdown,
        origin: SkillOperationOrigin::Skills(SkillsPane::Editor),
    });
    press(&mut model, KeyCode::Tab);
    assert_eq!(model.focus, Focus::Navigation);
    press(&mut model, KeyCode::Tab);
    assert_eq!(model.focus, Focus::List);
    assert_eq!(press(&mut model, KeyCode::Enter), ControllerEffect::Redraw);
    assert_eq!(model.focus, Focus::Workspace);
    assert!(model.skills.pending_confirmation.is_some());
    press(&mut model, KeyCode::Esc);
    assert!(model.skills.pending_confirmation.is_none());
    assert_eq!(model.skills.editor_page, SkillEditorPage::Review);
}

#[test]
fn literal_text_and_invalid_raw_survive_tab_and_escape_without_running_commands() {
    let mut model = editor_model();
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Enter);
    model.skills.field_input.clear();
    for c in "wasd123456789/:".chars() {
        press(&mut model, KeyCode::Char(c));
    }
    press(&mut model, KeyCode::Tab);
    assert_eq!(
        model.skills.editor.as_ref().unwrap().raw_display_name(),
        "wasd123456789/:"
    );
    assert_eq!(model.focus, Focus::Navigation);
    assert!(model.skills.active);
    press(&mut model, KeyCode::Tab);
    press(&mut model, KeyCode::Tab);
    press(&mut model, KeyCode::Enter);
    model.skills.field_input.clear();
    press(&mut model, KeyCode::Esc);
    assert_eq!(model.skills.editor.as_ref().unwrap().raw_display_name(), "");
    assert!(model.skills.editor.as_ref().unwrap().try_draft().is_err());
    assert!(model.skills.pending_confirmation.is_none());
}

#[test]
fn edit_never_uses_a_different_library_items_cached_detail() {
    use ai_stock_forum::{
        app::{SkillSummary, SkillView, SkillsView},
        domain::{SkillId, SkillVersionId},
        skills::{SkillProvenance, SkillVersion},
    };
    let first = SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(10)),
        SkillVersionId::from_uuid(Uuid::from_u128(11)),
        1,
        SkillProvenance::User,
        draft(),
    )
    .unwrap();
    let second = SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(20)),
        SkillVersionId::from_uuid(Uuid::from_u128(21)),
        1,
        SkillProvenance::User,
        draft(),
    )
    .unwrap();
    let mut model = model();
    model.skills.library = SkillsView {
        skills: vec![SkillSummary {
            skill_ref: second.reference(),
            display_name: "Second".to_owned(),
            provenance: SkillProvenance::User,
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };
    model.skills.detail = Some(SkillView {
        skill_ref: first.reference(),
        content: first.content().clone(),
        created_at_ms: 1,
        provenance: SkillProvenance::User,
        predecessor_version_id: None,
    });
    model.skills.pane = SkillsPane::Detail;
    press(&mut model, KeyCode::Char('e'));
    assert!(
        model.skills.editor.is_none(),
        "must not edit cached first skill while second is selected"
    );
    assert_eq!(press(&mut model, KeyCode::Enter), ControllerEffect::Redraw);
    assert!(model.skills.selected_agent_detail.is_none());
}

#[test]
fn spatial_card_edges_do_not_wrap_and_shifted_wasd_is_navigation() {
    for (start, code, want) in [
        (0, 'a', 0),
        (0, 'w', 0),
        (1, 'd', 1),
        (2, 'a', 2),
        (3, 'd', 3),
        (4, 's', 4),
        (5, 's', 5),
        (4, 'd', 5),
        (5, 'a', 4),
    ] {
        let mut model = editor_model();
        model.skills.editor_home_selection = start;
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(
                KeyCode::Char(code.to_ascii_uppercase()),
                KeyModifiers::SHIFT,
            )),
        );
        assert_eq!(model.skills.editor_home_selection, want, "{start} {code}");
    }
    let mut compact = editor_model();
    compact.set_terminal_size(60, 18);
    press(&mut compact, KeyCode::Char('s'));
    assert_eq!(compact.skills.editor_home_selection, 1);
}

#[test]
fn tab_from_the_library_activates_the_right_hand_home_controls() {
    let mut model = model();
    model.skills.pane = SkillsPane::List;
    model.set_focus(Focus::List);
    press(&mut model, KeyCode::Tab);
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.skills.pane, SkillsPane::Detail);
    press(&mut model, KeyCode::Char('d'));
    assert_eq!(model.skills.selected_action_index, 1);
}

#[test]
fn opening_multiline_instructions_and_leaving_keeps_the_original_text() {
    let mut model = editor_model();
    model.skills.editor.as_mut().unwrap().set_tui_field(
        SkillEditorField::Instructions,
        "First line\nSecond line\tvalue",
    );
    press(&mut model, KeyCode::Char('s'));
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Esc);
    assert_eq!(
        model
            .skills
            .editor
            .as_ref()
            .unwrap()
            .tui_field_text(SkillEditorField::Instructions),
        "First line\nSecond line\tvalue"
    );
}

#[test]
fn pasted_multiline_instructions_keep_line_breaks_and_tabs() {
    let mut model = editor_model();
    model
        .skills
        .editor
        .as_mut()
        .unwrap()
        .set_tui_field(SkillEditorField::Instructions, "");
    press(&mut model, KeyCode::Char('s'));
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Enter);
    handle_event(
        &mut model,
        TuiEvent::Paste("First line\nSecond line\tvalue".into()),
    );
    press(&mut model, KeyCode::Enter);
    assert_eq!(
        model
            .skills
            .editor
            .as_ref()
            .unwrap()
            .tui_field_text(SkillEditorField::Instructions),
        "First line\nSecond line\tvalue"
    );
}

#[test]
fn library_selection_keeps_moving_while_a_preview_is_pending() {
    use ai_stock_forum::{
        app::{SkillSummary, SkillsView},
        domain::{SkillId, SkillVersionId},
        skills::{SkillProvenance, SkillVersion},
    };
    let skills = [10, 20, 30].map(|id| {
        let skill = SkillVersion::create(
            SkillId::from_uuid(Uuid::from_u128(id)),
            SkillVersionId::from_uuid(Uuid::from_u128(id + 1)),
            1,
            SkillProvenance::User,
            draft(),
        )
        .unwrap();
        SkillSummary {
            skill_ref: skill.reference(),
            display_name: skill.content().display_name.clone(),
            provenance: skill.provenance().clone(),
        }
    });
    let mut model = model();
    model.skills.replace_skills(SkillsView {
        skills: skills.into(),
        total_count: 3,
        returned_count: 3,
        truncated: false,
    });
    model.skills.pane = SkillsPane::List;
    model.set_focus(Focus::List);
    model.set_command_in_flight(true);
    assert_eq!(
        press(&mut model, KeyCode::Char('s')),
        ControllerEffect::LoadSkillPreview {
            selected_skill: 1,
            starter: false
        }
    );
    assert_eq!(model.skills.selected_skill, 1);
    assert_eq!(
        press(&mut model, KeyCode::Char('s')),
        ControllerEffect::LoadSkillPreview {
            selected_skill: 2,
            starter: false
        }
    );
    assert_eq!(model.skills.selected_skill, 2);
    assert!(model.command_in_flight);
}
