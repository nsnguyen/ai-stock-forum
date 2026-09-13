use ai_stock_forum::{
    app::{AgentProfilesView, DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership},
    audit::AuditEntry,
    domain::{Actor, CorrelationId, InstallationId, SessionId},
    setup::SetupStatus,
    ui::tui::{
        ControllerEffect, TuiEvent, handle_event,
        model::{Focus, TuiModel, View},
        render,
        theme::Theme,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};
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
    )
}

fn key(model: &mut TuiModel, code: KeyCode) -> ControllerEffect {
    handle_event(
        model,
        TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
    )
}

fn draw(model: &mut TuiModel, width: u16, height: u16, no_color: bool) -> Buffer {
    handle_event(model, TuiEvent::Resize(width, height));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(no_color)))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn rows(buffer: &Buffer) -> Vec<String> {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

#[test]
fn active_navigation_is_cyan_filled_without_inverting_its_colors() {
    let mut model = model();
    for width in [60, 100, 120, 160] {
        let buffer = draw(&mut model, width, 30, false);
        let rendered = rows(&buffer);
        let (y, x) = rendered
            .iter()
            .enumerate()
            .find_map(|(y, row)| row.find("1 Home").map(|x| (y, x)))
            .unwrap();
        let cell = &buffer[(x as u16, y as u16)];
        assert_eq!(cell.bg, Color::Cyan);
        assert!(
            !cell.modifier.contains(Modifier::REVERSED),
            "reverse video undoes the cyan navigation fill"
        );
    }
    let buffer = draw(&mut model, 60, 18, true);
    assert!(
        buffer
            .content
            .iter()
            .any(|cell| cell.modifier.contains(Modifier::REVERSED)),
        "NO_COLOR still needs a visible selection"
    );
}

#[test]
fn focused_panel_outline_keeps_the_dark_canvas() {
    let mut model = model();
    key(&mut model, KeyCode::Char('7'));
    let buffer = draw(&mut model, 120, 30, false);
    let border = &buffer[(0, 3)];
    assert_eq!(border.fg, Color::Cyan);
    assert_eq!(border.bg, Theme::from_no_color(false).base.bg.unwrap());
}

#[test]
fn home_has_real_workspace_actions_instead_of_a_raw_audit_dump() {
    let mut model = model();
    model.audit_entries.push(AuditEntry {
        sequence: 1,
        occurred_at_ms: 1,
        actor: Actor::Human,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(3)),
        kind: "agent.profiles.listed".into(),
        summary: "agent profiles listed: total_count=0, returned_count=0, truncated=false".into(),
    });
    for (width, height) in [(60, 18), (80, 24), (100, 24), (120, 30), (160, 40)] {
        for no_color in [false, true] {
            let buffer = draw(&mut model, width, height, no_color);
            let text = rows(&buffer).join("\n");
            for action in ["3 Agents", "4 Skills", "7 Setup", "Enter", "Esc"] {
                assert!(text.contains(action), "{action} missing at{width}x{height}");
            }
            assert!(
                text.contains("Build your research crew"),
                "missing first-run guidance at{width}x{height}"
            );
            for internal in [
                "total_count",
                "returned_count",
                "truncated=",
                "agent profiles listed:",
                "00000000-",
            ] {
                assert!(!text.contains(internal), "Home leaked {internal}");
            }
        }
    }
}

#[test]
fn minimum_home_uses_complete_copy_that_fits_its_cards_and_hero() {
    let mut model = model();
    let buffer = draw(&mut model, 60, 18, false);
    let text = rows(&buffer).join("\n");
    for complete in [
        "Make it yours.",
        "Your crew",
        "Guidance",
        "Connections come later.",
    ] {
        assert!(text.contains(complete), "compact copy clipped: {complete}");
    }
}

#[test]
fn home_cards_open_real_destinations_and_retain_selection_on_return() {
    let mut model = model();
    key(&mut model, KeyCode::Char('d'));
    assert_eq!(
        key(&mut model, KeyCode::Enter),
        ControllerEffect::LoadSkills
    );
    assert!(model.skills.active);
    key(&mut model, KeyCode::Char('1'));
    assert_eq!(model.active_view, View::Overview);
    key(&mut model, KeyCode::Char('d'));
    key(&mut model, KeyCode::Enter);
    assert_eq!(model.active_view, View::Setup);
    key(&mut model, KeyCode::Char('1'));
    key(&mut model, KeyCode::Char('w'));
    key(&mut model, KeyCode::Char('a'));
    assert_eq!(
        key(&mut model, KeyCode::Enter),
        ControllerEffect::LoadAgentProfiles
    );
    assert_eq!(model.active_view, View::Agents);
}

#[test]
fn home_shortcuts_do_not_steal_command_text_or_main_navigation_focus() {
    let mut model = model();
    key(&mut model, KeyCode::Char('/'));
    handle_event(&mut model, TuiEvent::Paste("wasd123456789".into()));
    assert_eq!(model.command.text(), "/wasd123456789");
    assert_eq!(model.active_view, View::Overview);
    key(&mut model, KeyCode::Esc);
    model.set_focus(Focus::Navigation);
    key(&mut model, KeyCode::Char('d'));
    assert_eq!(model.active_view, View::Chat);
}
