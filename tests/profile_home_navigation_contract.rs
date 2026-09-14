use ai_stock_forum::{
    agents::builtin_profile_templates,
    app::{AgentProfilesView, DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership},
    domain::{InstallationId, SessionId},
    setup::SetupStatus,
    ui::{
        profile_editor::{ProfileEditor, ProfileTuiField},
        tui::{
            ControllerEffect, TuiEvent, handle_event,
            model::{
                AgentsPane, Focus, InputMode, ProfileEditorPage, ProfileSection, TuiModel, View,
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
    handle_event(&mut model, TuiEvent::Resize(160, 40));
    model.select_view(View::Agents);
    model.agents.pane = AgentsPane::Editor;
    model.set_focus(Focus::Workspace);
    model
}

fn press(model: &mut TuiModel, code: KeyCode) {
    handle_event(
        model,
        TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
    );
}

#[test]
fn home_opens_identity_before_enter_starts_literal_field_editing() {
    let mut model = model();
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    press(&mut model, KeyCode::Enter);
    assert_eq!(model.input_mode, InputMode::Nav);
    assert_eq!(
        model.agents.editor.as_ref().unwrap().tui_field(),
        ProfileTuiField::DisplayName
    );
    press(&mut model, KeyCode::Enter);
    assert_eq!(model.input_mode, InputMode::Type);
    model.agents.field_input.clear();
    for c in "wasd123456789/n".chars() {
        press(&mut model, KeyCode::Char(c));
    }
    press(&mut model, KeyCode::Esc);
    let editor = model.agents.editor.as_ref().unwrap();
    assert_eq!(
        editor.tui_field_text(ProfileTuiField::DisplayName),
        "wasd123456789/n"
    );
    assert_eq!(model.active_view, View::Agents);
    press(&mut model, KeyCode::Tab);
    assert_eq!(
        model.focus,
        Focus::Navigation,
        "Tab leaves the pane, not the field"
    );
    assert_eq!(
        model.agents.editor.as_ref().unwrap().tui_field(),
        ProfileTuiField::DisplayName
    );
}

#[test]
fn template_browsing_does_not_replace_the_draft_until_enter() {
    let mut model = model();
    assert!(
        model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );
    model.set_focus(Focus::List);
    let before = model.agents.editor.as_ref().unwrap().draft().clone();
    press(&mut model, KeyCode::Char('s'));
    assert_eq!(model.agents.selected_template, 1);
    assert_eq!(model.agents.editor.as_ref().unwrap().draft(), &before);
    press(&mut model, KeyCode::Enter);
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft().display_name,
        builtin_profile_templates()[1].suggested_name
    );
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.agents.pane, AgentsPane::Editor);
}

#[test]
fn moving_between_identity_fields_does_not_change_role() {
    let mut model = model();
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Char('s'));
    let role = model.agents.editor.as_ref().unwrap().draft().role;
    assert_eq!(
        model.agents.editor.as_ref().unwrap().tui_field(),
        ProfileTuiField::Role
    );
    press(&mut model, KeyCode::Char('s'));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().tui_field(),
        ProfileTuiField::Description
    );
    assert_eq!(model.agents.editor.as_ref().unwrap().draft().role, role);
}

#[test]
fn home_grid_and_escape_preserve_the_draft_across_sections_and_destinations() {
    let mut model = model();
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    press(&mut model, KeyCode::Char('d'));
    assert_eq!(model.agents.profile_home_selection, 1);
    press(&mut model, KeyCode::Enter);
    assert_eq!(
        model.agents.editor_page,
        ProfileEditorPage::Section(ProfileSection::Focus)
    );
    press(&mut model, KeyCode::Char('s'));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().tui_field(),
        ProfileTuiField::Tags
    );
    let before = model.agents.editor.clone();
    press(&mut model, KeyCode::Char('1'));
    press(&mut model, KeyCode::Char('3'));
    assert_eq!(
        model.agents.editor_page,
        ProfileEditorPage::Section(ProfileSection::Focus)
    );
    assert_eq!(model.agents.editor, before);
    press(&mut model, KeyCode::Esc);
    assert_eq!(model.agents.editor_page, ProfileEditorPage::Home);
    assert_eq!(model.agents.profile_home_selection, 1);
    press(&mut model, KeyCode::Char('s'));
    assert_eq!(model.agents.profile_home_selection, 3);
    press(&mut model, KeyCode::Char('s'));
    assert_eq!(model.agents.profile_home_selection, 5);
    press(&mut model, KeyCode::Enter);
    assert_eq!(model.agents.editor_page, ProfileEditorPage::Discard);
    assert_eq!(model.agents.editor, before);
    press(&mut model, KeyCode::Esc);
    assert_eq!(model.agents.editor_page, ProfileEditorPage::Home);
    press(&mut model, KeyCode::Esc);
    assert_eq!(model.agents.pane, AgentsPane::Detail);
    assert_eq!(model.agents.editor, before);
}

#[test]
fn create_review_and_confirmation_require_separate_fresh_enter_presses() {
    let mut model = model();
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    press(&mut model, KeyCode::Char('s'));
    press(&mut model, KeyCode::Char('s'));
    assert_eq!(model.agents.profile_home_selection, 4);
    press(&mut model, KeyCode::Enter);
    assert_eq!(model.agents.editor_page, ProfileEditorPage::Review);
    assert_eq!(model.agents.pane, AgentsPane::Editor);
    assert!(model.agents.pending_confirmation.is_none());
    press(&mut model, KeyCode::Enter);
    assert_eq!(model.agents.pane, AgentsPane::Confirmation);
    let effect = handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
    );
    assert_eq!(effect, ControllerEffect::None);
    let effect = handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );
    assert!(matches!(effect, ControllerEffect::ExecuteProfile(_)));
}

#[test]
fn only_explicit_role_selection_changes_the_role_and_tab_retains_it() {
    let mut model = model();
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Char('s'));
    press(&mut model, KeyCode::Enter);
    assert!(model.agents.profile_role_selecting);
    press(&mut model, KeyCode::Char('s'));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft().role,
        ai_stock_forum::agents::AgentRole::Bear
    );
    press(&mut model, KeyCode::Tab);
    assert_eq!(model.focus, Focus::Navigation);
    assert!(model.agents.profile_role_selecting);
    press(&mut model, KeyCode::Tab);
    press(&mut model, KeyCode::Tab);
    press(&mut model, KeyCode::Esc);
    assert!(!model.agents.profile_role_selecting);
    assert_eq!(
        model.agents.editor_page,
        ProfileEditorPage::Section(ProfileSection::Identity)
    );
}

#[test]
fn template_footer_describes_template_keys_instead_of_agent_actions() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut model = model();
    model
        .agents
        .start_profile_create(0, builtin_profile_templates());
    for focus in [Focus::List, Focus::Workspace] {
        model.set_focus(focus);
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|frame| {
                ai_stock_forum::ui::tui::render::render(
                    frame,
                    &model,
                    &ai_stock_forum::ui::tui::theme::Theme::from_no_color(false),
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let footer: String = (38..40)
            .flat_map(|y| (0..160).map(move |x| (x, y)))
            .map(|position| buffer[position].symbol())
            .collect();
        assert!(footer.contains("template"), "{footer}");
        assert!(!footer.contains("W/S agent"));
        assert!(!footer.contains("H History"));
    }
}

#[test]
fn shifted_wasd_keeps_navigation_aliases_without_enabling_modified_actions() {
    let mut model = model();
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT)),
    );
    assert_eq!(model.agents.profile_home_selection, 1);
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)),
    );
    assert_eq!(model.agents.profile_home_selection, 3);
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
    );
    assert_eq!(model.agents.profile_home_selection, 3);
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
    );
    assert_eq!(model.agents.editor_page, ProfileEditorPage::Home);
    model
        .agents
        .start_profile_create(0, builtin_profile_templates());
    model.set_focus(Focus::List);
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT)),
    );
    assert_eq!(model.agents.selected_template, 1);
}

#[test]
fn home_grid_edges_never_jump_diagonally_or_wrap_to_another_row() {
    let mut model = model();
    model.agents.editor = Some(ProfileEditor::for_create(&builtin_profile_templates()[0]).unwrap());
    for (start, key, expected) in [
        (0, 'w', 0),
        (1, 'w', 1),
        (4, 's', 4),
        (5, 's', 5),
        (0, 'a', 0),
        (2, 'a', 2),
        (4, 'a', 4),
        (1, 'd', 1),
        (3, 'd', 3),
        (5, 'd', 5),
        (0, 'd', 1),
        (1, 'a', 0),
        (0, 's', 2),
        (2, 's', 4),
        (3, 'w', 1),
    ] {
        model.agents.profile_home_selection = start;
        press(&mut model, KeyCode::Char(key));
        assert_eq!(
            model.agents.profile_home_selection, expected,
            "start={start}, key={key}"
        );
    }
    handle_event(&mut model, TuiEvent::Resize(60, 18));
    model.agents.profile_home_selection = 0;
    press(&mut model, KeyCode::Char('s'));
    assert_eq!(model.agents.profile_home_selection, 1);
}
