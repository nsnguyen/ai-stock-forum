use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::{
    layout::{MIN_HEIGHT, MIN_WIDTH, calculate},
    model::{Focus, LayoutMode, Severity, TuiModel, View},
    theme::Theme,
    views,
};

pub fn render(frame: &mut Frame<'_>, model: &TuiModel, theme: &Theme) {
    let cockpit = calculate(frame.area(), model.inspector_open);
    frame.render_widget(Clear, cockpit.viewport);
    if cockpit.mode == LayoutMode::TooSmall {
        render_too_small(frame, cockpit.viewport);
        return;
    }

    render_header(frame, cockpit.header, model, cockpit.mode, theme);
    if let Some(navigation) = cockpit.navigation {
        render_navigation(frame, navigation, model, theme);
    }
    views::render(frame, cockpit.workspace, model, theme);
    render_message(frame, cockpit.message, model, theme);
    render_command(frame, cockpit.command, model, theme);
    if let Some(inspector) = cockpit.inspector {
        frame.render_widget(Clear, inspector);
        views::render_inspector(frame, inspector, model, theme);
    }
}

fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    mode: LayoutMode,
    theme: &Theme,
) {
    let identity = Line::from(vec![
        Span::styled("AI STOCK FORUM", theme.accent),
        Span::raw("  /  "),
        Span::styled(view_name(model.active_view), theme.focus),
        Span::raw(format!("  /  {}", mode_name(mode))),
    ]);
    let second = if mode == LayoutMode::Narrow {
        numbered_tabs(model, theme)
    } else if model.previous_session_interrupted {
        Line::styled("WARNING  Previous session interrupted", theme.warning)
    } else {
        Line::styled(
            "Local cockpit  |  native views  |  typed audit",
            theme.muted,
        )
    };
    let third = if mode == LayoutMode::Narrow && model.previous_session_interrupted {
        Line::styled("WARNING  Previous session interrupted", theme.warning)
    } else {
        Line::styled("-".repeat(usize::from(area.width)), theme.muted)
    };
    frame.render_widget(Paragraph::new(vec![identity, second, third]), area);
}

fn numbered_tabs(model: &TuiModel, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, view) in [View::Overview, View::Setup, View::Audit, View::Help]
        .into_iter()
        .enumerate()
    {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let style = if view == model.active_view {
            theme.focus
        } else {
            theme.muted
        };
        spans.push(Span::styled(
            format!("{} {}", index + 1, view_name(view)),
            style,
        ));
    }
    Line::from(spans)
}

fn render_navigation(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let border_style = if model.focus == Focus::Navigation {
        theme.focus
    } else {
        theme.muted
    };
    let mut lines = vec![Line::styled("VIEWS", theme.accent), Line::default()];
    for (index, view) in [View::Overview, View::Setup, View::Audit, View::Help]
        .into_iter()
        .enumerate()
    {
        let selected = view == model.active_view;
        lines.push(Line::styled(
            format!(
                "{} {} {}",
                if selected { ">" } else { " " },
                index + 1,
                view_name(view)
            ),
            if selected { theme.focus } else { theme.muted },
        ));
    }
    lines.extend([
        Line::default(),
        Line::styled("/ command", theme.muted),
        Line::styled("? help", theme.muted),
        Line::styled("q quit", theme.muted),
    ]);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" Navigation ")
                .borders(Borders::ALL)
                .border_style(border_style),
        ),
        area,
    );
}

fn render_message(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let line = match &model.message {
        Some(message) => {
            let (label, style) = match message.severity {
                Severity::Info => ("INFO", theme.accent),
                Severity::Warning => ("WARNING", theme.warning),
                Severity::Error => ("ERROR", theme.error),
            };
            Line::from(vec![
                Span::styled(format!(" {label:<7}"), style),
                Span::raw(views::safe_text(&message.text)),
            ])
        }
        None => Line::from(vec![
            Span::styled(" INFO   ", theme.accent),
            Span::styled("Ready", theme.muted),
        ]),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn render_command(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let focused = model.focus == Focus::Command;
    let block = Block::default()
        .title(if model.command_in_flight {
            " Command - working "
        } else {
            " Command "
        })
        .borders(Borders::ALL)
        .border_style(if focused { theme.focus } else { theme.muted });
    let line = if focused {
        let cursor = model.command.cursor_byte().min(model.command.text().len());
        let prefix = model
            .command
            .text()
            .get(..cursor)
            .unwrap_or(model.command.text());
        let suffix = model.command.text().get(cursor..).unwrap_or_default();
        Line::from(vec![
            Span::styled("> ", theme.accent),
            Span::raw(prefix.to_owned()),
            Span::styled("|", theme.focus),
            Span::raw(suffix.to_owned()),
        ])
    } else if model.command.text().is_empty() {
        Line::styled("Type /help for commands", theme.muted)
    } else {
        Line::from(model.command.text().to_owned())
    };
    frame.render_widget(Paragraph::new(line).block(block), area);

    if focused && area.width > 2 && area.height > 2 {
        let prefix_width = Line::from(model.command.prefix()).width();
        let desired = area
            .x
            .saturating_add(1)
            .saturating_add(2)
            .saturating_add(u16::try_from(prefix_width).unwrap_or(u16::MAX));
        let cursor_x = desired.min(area.right().saturating_sub(2));
        frame.set_cursor_position((cursor_x, area.y.saturating_add(1)));
    }
}

fn render_too_small(frame: &mut Frame<'_>, area: Rect) {
    let content = vec![
        Line::raw("Terminal too small"),
        Line::raw(format!("Minimum: {MIN_WIDTH} x {MIN_HEIGHT}")),
        Line::raw(format!("Current: {} x {}", area.width, area.height)),
        Line::raw("q / Ctrl+C to exit"),
    ];
    let centered = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(4.min(area.height)),
        Constraint::Fill(1),
    ])
    .split(area)[1];
    frame.render_widget(
        Paragraph::new(content).alignment(Alignment::Center),
        centered,
    );
}

fn view_name(view: View) -> &'static str {
    match view {
        View::Overview => "Overview",
        View::Setup => "Setup",
        View::Audit => "Audit",
        View::Help => "Help",
    }
}

fn mode_name(mode: LayoutMode) -> &'static str {
    match mode {
        LayoutMode::Wide => "Wide",
        LayoutMode::Medium => "Medium",
        LayoutMode::Narrow => "Narrow",
        LayoutMode::TooSmall => "Too small",
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{
        Terminal,
        backend::{Backend, ClearType, TestBackend, WindowSize},
        buffer::Cell as BufferCell,
        layout::{Position, Size},
        style::Modifier,
    };
    use uuid::Uuid;

    use super::render;
    use crate::{
        app::PresentationSnapshot,
        audit::AuditEntry,
        domain::{Actor, CorrelationId, InstallationId, SessionId},
        setup::SetupStatus,
        ui::tui::{
            TuiEvent, handle_event,
            model::{Focus, RuntimeStatus, Severity, TuiModel, View},
            theme::Theme,
        },
    };

    #[derive(Debug)]
    struct CursorTrackingBackend {
        inner: TestBackend,
        cursor_visible: bool,
    }

    impl CursorTrackingBackend {
        fn new(width: u16, height: u16) -> Self {
            Self {
                inner: TestBackend::new(width, height),
                cursor_visible: true,
            }
        }
    }

    impl Backend for CursorTrackingBackend {
        fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
        where
            I: Iterator<Item = (u16, u16, &'a BufferCell)>,
        {
            self.inner.draw(content)
        }

        fn append_lines(&mut self, count: u16) -> io::Result<()> {
            self.inner.append_lines(count)
        }

        fn hide_cursor(&mut self) -> io::Result<()> {
            self.cursor_visible = false;
            self.inner.hide_cursor()
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.cursor_visible = true;
            self.inner.show_cursor()
        }

        fn get_cursor_position(&mut self) -> io::Result<Position> {
            self.inner.get_cursor_position()
        }

        fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
            self.inner.set_cursor_position(position)
        }

        fn clear(&mut self) -> io::Result<()> {
            self.inner.clear()
        }

        fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
            self.inner.clear_region(clear_type)
        }

        fn size(&self) -> io::Result<Size> {
            self.inner.size()
        }

        fn window_size(&mut self) -> io::Result<WindowSize> {
            self.inner.window_size()
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    fn model(view: View) -> TuiModel {
        let mut model = TuiModel::new(
            PresentationSnapshot {
                installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
                session_id: SessionId::from_uuid(Uuid::from_u128(2)),
                database_readiness: crate::app::DatabaseReadiness::Ready,
                process_guard_ownership: crate::app::ProcessGuardOwnership::Held,
                setup_status: SetupStatus::NotStarted,
                recent_audit: vec![AuditEntry {
                    sequence: 7,
                    occurred_at_ms: 1_725_000_000_000,
                    actor: Actor::Human,
                    kind: "status_viewed".to_owned(),
                    correlation_id: CorrelationId::from_uuid(Uuid::from_u128(3)),
                    summary: "status viewed".to_owned(),
                }],
            },
            false,
        );
        model.select_view(view);
        model
    }

    fn rendered(model: TuiModel, width: u16, height: u16, no_color: bool) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::from_no_color(no_color);
        terminal
            .draw(|frame| render(frame, &model, &theme))
            .expect("render model");
        terminal
    }

    fn rendered_with_cursor_tracking(
        model: TuiModel,
        width: u16,
        height: u16,
    ) -> Terminal<CursorTrackingBackend> {
        let backend = CursorTrackingBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let theme = Theme::from_no_color(false);
        terminal
            .draw(|frame| render(frame, &model, &theme))
            .expect("render model");
        terminal
    }

    fn render_text(model: TuiModel, width: u16, height: u16, no_color: bool) -> String {
        rendered(model, width, height, no_color)
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn wide_overview_renders_identity_health_navigation_and_command_bar() {
        let text = render_text(model(View::Overview), 140, 40, false);
        assert!(text.contains("AI STOCK FORUM"));
        assert!(text.contains("Overview"));
        assert!(text.contains("Installation"));
        assert!(text.contains("Session"));
        assert!(text.contains("Runtime"));
        assert!(text.contains("Database      Ready"));
        assert!(text.contains("Process guard Held"));
        assert!(text.contains("Type /help"));
    }

    #[test]
    fn end_reveals_final_help_content_and_further_scroll_is_stable_in_every_layout() {
        for (width, height) in [(60, 18), (80, 24), (120, 30)] {
            let mut model = model(View::Help);
            handle_event(&mut model, TuiEvent::Resize(width, height));
            handle_event(
                &mut model,
                TuiEvent::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            );
            let end = model.workspace_scroll;
            assert!(
                render_text(model.clone(), width, height, true).contains("/quit"),
                "size={width}x{height}, scroll={end}"
            );

            for code in [KeyCode::Down, KeyCode::PageDown, KeyCode::End] {
                handle_event(
                    &mut model,
                    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
                );
            }
            assert_eq!(model.workspace_scroll, end, "size={width}x{height}");
            assert!(render_text(model, width, height, true).contains("/quit"));
        }
    }

    #[test]
    fn overview_renders_every_runtime_and_command_state_independently() {
        let cases = [
            ("ready_idle", RuntimeStatus::Ready, false, "Ready", "Idle"),
            (
                "ready_running",
                RuntimeStatus::Ready,
                true,
                "Ready",
                "Running",
            ),
            (
                "stopping_idle",
                RuntimeStatus::Stopping,
                false,
                "Stopping",
                "Idle",
            ),
            (
                "stopping_running",
                RuntimeStatus::Stopping,
                true,
                "Stopping",
                "Running",
            ),
        ];

        for (case, runtime_status, command_in_flight, expected_runtime, expected_command) in cases {
            let mut model = model(View::Overview);
            model.set_runtime_status(runtime_status);
            model.set_command_in_flight(command_in_flight);

            let text = render_text(model, 140, 40, false);

            assert!(
                text.contains(&format!("Runtime       {expected_runtime}")),
                "case={case}"
            );
            assert!(
                text.contains(&format!("Command       {expected_command}")),
                "case={case}"
            );
        }
    }

    #[test]
    fn each_command_view_is_native_and_no_transcript_heading_exists() {
        for view in [View::Overview, View::Setup, View::Audit, View::Help] {
            let text = render_text(model(view), 100, 30, false);
            assert!(!text.contains("Transcript"));
            assert!(!text.contains("Command output"));
        }
    }

    #[test]
    fn too_small_screen_contains_only_size_guidance() {
        let text = render_text(model(View::Audit), 59, 17, false);
        assert!(text.contains("Terminal too small"));
        assert!(text.contains("60 x 18"));
        assert!(text.contains("q / Ctrl+C"));
        assert!(!text.contains("Installation"));
        assert!(!text.contains("AI STOCK FORUM"));
    }

    #[test]
    fn no_color_theme_uses_modifiers_without_terminal_colors() {
        let theme = Theme::from_no_color(true);
        for style in theme.styles() {
            assert_eq!(style.fg, None);
            assert_eq!(style.bg, None);
        }
        assert!(theme.focus.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn native_views_show_phase_safe_content_and_complete_help() {
        let setup = render_text(model(View::Setup), 100, 30, false);
        assert!(setup.contains("Read-only"));
        assert!(setup.contains("Phase 0B"));

        let audit = render_text(model(View::Audit), 140, 40, false);
        for heading in ["Sequence", "Kind", "Actor", "Summary", "Correlation"] {
            assert!(audit.contains(heading), "missing audit heading: {heading}");
        }

        let help = render_text(model(View::Help), 100, 30, false);
        for command in [
            "/help",
            "/status",
            "/setup status",
            "/audit tail [limit: 1-100]",
            "/quit",
        ] {
            assert!(help.contains(command), "missing command: {command}");
        }
        for key in ["1-4", "Tab", "Enter", "Esc", "Up/Down", "Home/End", "q"] {
            assert!(help.contains(key), "missing key: {key}");
        }
    }

    #[test]
    fn narrow_mode_uses_numbered_tabs_and_message_severity_is_typed() {
        let mut model = model(View::Audit);
        model.set_message(Severity::Warning, "Setup needs attention");
        let text = render_text(model, 70, 20, true);
        assert!(text.contains("1 Overview"));
        assert!(text.contains("2 Setup"));
        assert!(text.contains("3 Audit"));
        assert!(text.contains("4 Help"));
        assert!(text.contains("WARNING"));
        assert!(text.contains("Setup needs attention"));
    }

    #[test]
    fn command_focus_places_cursor_after_wide_unicode_prefix() {
        let mut model = model(View::Overview);
        model.set_focus(Focus::Command);
        model.command.insert('界');
        model.command.insert('x');
        model.command.move_left();
        let mut terminal = rendered(model, 100, 30, false);
        assert_eq!(
            terminal.get_cursor_position().expect("cursor position"),
            Position::new(5, 28)
        );
    }

    #[test]
    fn cursor_is_hidden_outside_command_focus() {
        let terminal = rendered_with_cursor_tracking(model(View::Overview), 100, 30);
        assert!(!terminal.backend().cursor_visible);
    }

    #[test]
    fn interrupted_session_warning_and_selected_audit_details_are_visible() {
        let mut model = model(View::Audit);
        model.previous_session_interrupted = true;
        model.inspector_open = true;
        let text = render_text(model, 100, 30, false);
        assert!(text.contains("Previous session interrupted"));
        assert!(text.contains("Audit detail"));
        assert!(text.contains("1725000000000"));
        assert!(text.contains("00000000-0000-0000-0000-000000000003"));
    }
}
