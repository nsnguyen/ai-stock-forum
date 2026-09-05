use ai_stock_forum::{
    app::{DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership, ShutdownReason},
    domain::{InstallationId, SessionId},
    setup::SetupStatus,
    ui::tui::{
        ControllerEffect, TuiEvent, handle_event,
        model::{AgentsPane, TuiModel, View},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

fn model() -> TuiModel {
    TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: Vec::new(),
        },
        false,
    )
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[test]
fn a_opens_agents_but_remains_text_when_command_entry_owns_input() {
    let mut model = model();

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('a'))), ControllerEffect::LoadAgentProfiles);
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(model.agents.pane, AgentsPane::List);

    handle_event(&mut model, key(KeyCode::Char('/')));
    assert_eq!(handle_event(&mut model, key(KeyCode::Char('a'))), ControllerEffect::Redraw);
    assert_eq!(model.command.text(), "/a");
}

#[test]
fn existing_numeric_navigation_remains_stable() {
    let mut model = model();
    for (key_code, expected) in [
        (KeyCode::Char('1'), View::Overview),
        (KeyCode::Char('2'), View::Setup),
        (KeyCode::Char('3'), View::Audit),
        (KeyCode::Char('4'), View::Help),
    ] {
        assert_eq!(handle_event(&mut model, key(key_code)), ControllerEffect::Redraw);
        assert_eq!(model.active_view, expected);
    }
}

#[test]
fn agents_local_navigation_tracks_panes_selection_and_effects() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('a')));

    assert_eq!(handle_event(&mut model, key(KeyCode::Down)), ControllerEffect::Redraw);
    assert_eq!(model.agents.selected_profile, 1);
    assert_eq!(model.agents.list_scroll, 1);

    assert_eq!(handle_event(&mut model, key(KeyCode::Enter)), ControllerEffect::LoadAgentProfile { selected_profile: 1 });
    assert_eq!(model.agents.pane, AgentsPane::Detail);

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('h'))), ControllerEffect::LoadAgentProfileHistory { selected_profile: 1 });
    assert_eq!(model.agents.pane, AgentsPane::History);
    assert_eq!(handle_event(&mut model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(model.agents.pane, AgentsPane::Detail);

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('c'))), ControllerEffect::StartProfileCreate { template_index: 0 });
    assert_eq!(model.agents.pane, AgentsPane::Editor);
}

#[test]
fn escape_and_quit_respect_active_agents_layers() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('a')));
    handle_event(&mut model, key(KeyCode::Char('c')));

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('q'))), ControllerEffect::Redraw);
    assert_ne!(model.runtime_status, ai_stock_forum::ui::tui::model::RuntimeStatus::Stopping);
    assert_eq!(handle_event(&mut model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(model.agents.pane, AgentsPane::List);
    assert_eq!(handle_event(&mut model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(model.active_view, View::Overview);
    assert_eq!(handle_event(&mut model, key(KeyCode::Char('q'))), ControllerEffect::RequestShutdown(ShutdownReason::UserQuit));
}

#[test]
fn resize_preserves_agents_selection_scroll_and_editor_draft() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('a')));
    handle_event(&mut model, key(KeyCode::Char('c')));
    for character in "Draft Analyst".chars() {
        handle_event(&mut model, key(KeyCode::Char(character)));
    }
    let selected_profile = model.agents.selected_profile;
    let selected_template = model.agents.selected_template;
    let list_scroll = model.agents.list_scroll;
    let draft = model.command.text().to_owned();

    for (width, height) in [(70, 24), (110, 32), (160, 44)] {
        assert_eq!(handle_event(&mut model, TuiEvent::Resize(width, height)), ControllerEffect::Redraw);
        assert_eq!(model.active_view, View::Agents);
        assert_eq!(model.agents.pane, AgentsPane::Editor);
        assert_eq!(model.agents.selected_profile, selected_profile);
        assert_eq!(model.agents.selected_template, selected_template);
        assert_eq!(model.agents.list_scroll, list_scroll);
        assert_eq!(model.command.text(), draft);
    }
}
