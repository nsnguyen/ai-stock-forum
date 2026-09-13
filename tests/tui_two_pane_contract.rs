use ai_stock_forum::{
    app::{AgentProfilesView, DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership},
    audit::AuditEntry,
    domain::{Actor, CorrelationId, InstallationId, SessionId},
    setup::SetupStatus,
    ui::tui::{
        ControllerEffect, TuiEvent, handle_event,
        layout::{agent_workspace, calculate, layout_mode},
        model::{Focus, InputMode, LayoutMode, TuiModel, View},
        render,
        theme::Theme,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use uuid::Uuid;

fn model() -> TuiModel {
    let mut model = TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: Vec::new(),
            agent_profiles: AgentProfilesView {
                profiles: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            selected_agent_profile: None,
            selected_agent_profile_history: None,
        },
        false,
    );
    model.skills.library_loaded = true;
    model
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, modifiers))
}

fn render_text(model: &TuiModel, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .expect("render shell");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn numbered_agent_and_skills_routes_replace_letter_shortcuts() {
    let mut model = model();

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('3'))),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Agents);
    assert!(!model.skills.active);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('4'))),
        ControllerEffect::Redraw
    );
    assert!(model.skills.active);

    let before = model.clone();
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('a'))),
        ControllerEffect::None
    );
    assert_eq!(model, before);
}

#[test]
fn shared_shell_has_no_vertical_navigation_or_permanent_third_pane() {
    for area in [
        Rect::new(0, 0, 100, 24),
        Rect::new(0, 0, 120, 30),
        Rect::new(0, 0, 160, 40),
    ] {
        let shell = calculate(area, false);
        assert_eq!(shell.navigation, None, "area={area:?}");
        assert_eq!(shell.inspector, None, "area={area:?}");
        assert_eq!(shell.workspace.x, area.x, "area={area:?}");
        assert_eq!(shell.workspace.width, area.width, "area={area:?}");
    }
}

#[test]
fn two_pane_split_starts_at_100_columns_and_clamps_the_list_width() {
    let cases = [
        (99, LayoutMode::Narrow, None),
        (100, LayoutMode::Medium, Some(28)),
        (120, LayoutMode::Wide, Some(33)),
        (160, LayoutMode::Wide, Some(36)),
    ];
    for (width, expected_mode, expected_list_width) in cases {
        let area = Rect::new(0, 0, width, 24);
        let mode = layout_mode(area);
        assert_eq!(mode, expected_mode, "width={width}");
        let panes = agent_workspace(area, mode);
        assert_eq!(
            panes.list.map(|list| list.width),
            expected_list_width,
            "width={width}"
        );
        assert_eq!(
            panes.active.width + panes.list.map_or(0, |list| list.width),
            width
        );
    }
}

#[test]
fn every_supported_size_renders_all_numbered_destination_labels() {
    let expected = [
        "1 Home",
        "2 Chat",
        "3 Agents",
        "4 Skills",
        "5 Connections",
        "6 Activity",
        "7 Setup",
        "8 Audit",
        "9 Help",
    ];
    for (width, height) in [(60, 18), (80, 24), (100, 24), (120, 30), (160, 40)] {
        let text = render_text(&model(), width, height);
        for label in expected {
            assert!(
                text.contains(label),
                "missing {label:?} at {width}x{height}"
            );
        }
    }
}

#[test]
fn all_nine_numbered_routes_are_literal_and_command_text_stays_literal() {
    let mut model = model();
    let routes = [
        ('1', View::Overview, false),
        ('2', View::Chat, false),
        ('3', View::Agents, false),
        ('4', View::Agents, true),
        ('5', View::Connections, false),
        ('6', View::Activity, false),
        ('7', View::Setup, false),
        ('8', View::Audit, false),
        ('9', View::Help, false),
    ];
    for (number, view, skills_active) in routes {
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Char(number))),
            ControllerEffect::Redraw,
            "number={number}"
        );
        assert_eq!(model.active_view, view, "number={number}");
        assert_eq!(model.skills.active, skills_active, "number={number}");
    }

    handle_event(&mut model, key(KeyCode::Char('2')));
    handle_event(&mut model, key(KeyCode::Char('/')));
    handle_event(&mut model, TuiEvent::Paste("wasd123456789".into()));
    assert_eq!(model.command.text(), "/wasd123456789");
    assert_eq!(model.active_view, View::Chat);
    assert_eq!(model.input_mode, InputMode::Type);
}

#[test]
fn shifted_wasd_moves_in_nav_while_modified_shortcuts_are_ignored() {
    let mut model = model();
    model.set_focus(Focus::Navigation);

    assert_eq!(
        handle_event(
            &mut model,
            modified_key(KeyCode::Char('D'), KeyModifiers::SHIFT)
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Chat);
    assert_eq!(model.focus, Focus::Navigation);

    assert_eq!(
        handle_event(
            &mut model,
            modified_key(KeyCode::Char('A'), KeyModifiers::SHIFT)
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Overview);

    for modifiers in [
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
    ] {
        for character in ['3', 'w', 'a', 's', 'd'] {
            let before = model.clone();
            assert_eq!(
                handle_event(
                    &mut model,
                    modified_key(KeyCode::Char(character), modifiers)
                ),
                ControllerEffect::None,
                "character={character} modifiers={modifiers:?}"
            );
            assert_eq!(model, before);
        }
    }
}

#[test]
fn paste_outside_type_mode_is_inert_and_tab_never_focuses_an_inspector() {
    let mut model = model();
    let before = model.clone();
    assert_eq!(
        handle_event(&mut model, TuiEvent::Paste("3wasd".into())),
        ControllerEffect::None
    );
    assert_eq!(model, before);

    for _ in 0..6 {
        handle_event(&mut model, key(KeyCode::Tab));
        assert_ne!(model.focus, Focus::Inspector);
        assert_ne!(model.focus, Focus::Command);
    }

    handle_event(&mut model, key(KeyCode::Char('3')));
    let mut seen = Vec::new();
    for _ in 0..3 {
        handle_event(&mut model, key(KeyCode::Tab));
        seen.push(model.focus);
    }
    assert_eq!(seen, vec![Focus::Navigation, Focus::List, Focus::Workspace]);
}

#[test]
fn destination_state_and_draft_are_retained_across_numbered_switches() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('9')));
    model.workspace_scroll = 5;
    model.command.insert('/');
    model.command.insert('h');

    handle_event(&mut model, key(KeyCode::Char('1')));
    assert_eq!(model.workspace_scroll, 0);
    assert_eq!(model.command.text(), "");
    handle_event(&mut model, key(KeyCode::Char('9')));

    assert_eq!(model.workspace_scroll, 5);
    assert_eq!(model.command.text(), "/h");
}

#[test]
fn future_destinations_are_honest_and_idle_shell_has_no_command_field() {
    let mut future_model = model();
    for (number, heading) in [('2', "Chat"), ('5', "Connections")] {
        handle_event(&mut future_model, key(KeyCode::Char(number)));
        let text = render_text(&future_model, 100, 24);
        assert!(text.contains(&format!("{heading} is coming in Phase 3")));
        assert!(text.contains("not interactive yet"));
        assert!(!text.contains("sample reply"));
        assert!(!text.contains("API key"));
    }

    let idle = render_text(&model(), 100, 24);
    assert!(idle.contains("NAV"));
    assert!(!idle.contains("Type /help for commands"));
    let mut typing = model();
    handle_event(&mut typing, key(KeyCode::Char('/')));
    let typing = render_text(&typing, 100, 24);
    assert!(typing.contains("TYPE"));
    assert!(typing.contains("Command"));
}

#[test]
fn minimum_size_footer_keeps_every_required_control_visible() {
    let text = render_text(&model(), 60, 18);
    for control in ["Tab", "WASD", "Enter", "Esc"] {
        assert!(
            text.contains(control),
            "minimum-size footer clipped {control:?}"
        );
    }
}

#[test]
fn model_geometry_reserves_space_only_while_input_is_visible() {
    let mut model = model();
    model.set_terminal_size(100, 24);
    let idle_height = model.workspace_body_height;

    handle_event(&mut model, key(KeyCode::Char('/')));
    assert_eq!(model.input_mode, InputMode::Type);
    assert_eq!(model.workspace_body_height, idle_height.saturating_sub(3));

    handle_event(&mut model, key(KeyCode::Esc));
    assert_eq!(model.input_mode, InputMode::Nav);
    assert_eq!(model.workspace_body_height, idle_height);
}

#[test]
fn command_footer_matches_tab_enter_and_escape_controller_behavior() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('/')));
    model.command.insert('h');
    let footer = render_text(&model, 100, 24);
    for hint in ["Tab leave input", "WASD text", "Enter run", "Esc clear"] {
        assert!(footer.contains(hint), "missing command hint {hint:?}");
    }
    assert!(!footer.contains("Tab next field"));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.command.text(), "/h");

    model.set_focus(Focus::Command);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.command.text(), "");
}

#[test]
fn activity_end_reaches_the_final_wrapped_summary_at_minimum_size() {
    let mut model = model();
    model.replace_audit(
        (1..=6)
            .map(|sequence| AuditEntry {
                sequence,
                occurred_at_ms: 1_800_000_000_000 + i64::try_from(sequence).unwrap(),
                actor: Actor::Human,
                kind: "activity_test".to_owned(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(sequence.into())),
                summary: if sequence == 1 {
                    format!(
                        "{} FINAL-WRAPPED-ACTIVITY",
                        "oldest long summary ".repeat(8)
                    )
                } else {
                    format!("{} {sequence}", "newer long summary ".repeat(8))
                },
            })
            .collect(),
    );
    model.set_terminal_size(60, 18);
    handle_event(&mut model, key(KeyCode::Char('6')));

    assert_eq!(
        handle_event(&mut model, key(KeyCode::End)),
        ControllerEffect::Redraw
    );
    assert!(model.workspace_scroll > 0);
    assert!(render_text(&model, 60, 18).contains("FINAL-WRAPPED-ACTIVITY"));
}
