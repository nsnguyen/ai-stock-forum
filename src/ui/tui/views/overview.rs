use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Wrap},
};

use crate::setup::SetupStatus;

use super::{label_value, panel, safe_text, workspace_focused, wrapped_height};
use crate::ui::tui::{
    model::{RuntimeStatus, TuiModel},
    theme::Theme,
};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(content(model, theme))
            .block(panel("Overview", workspace_focused(model), theme))
            .wrap(Wrap { trim: false })
            .scroll((model.workspace_scroll, 0)),
        area,
    );
}

pub(super) fn content_height(model: &TuiModel, width: u16) -> u16 {
    wrapped_height(content(model, &Theme::from_no_color(true)), width)
}

fn content(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    let (runtime, runtime_style) = match model.runtime_status {
        RuntimeStatus::Ready => ("Ready", theme.success),
        RuntimeStatus::Stopping => ("Stopping", theme.warning),
    };
    let (command, command_style) = if model.command_in_flight {
        ("Running", theme.warning)
    } else {
        ("Idle", theme.success)
    };
    let setup = setup_state(&model.setup_status);
    let recent = model
        .audit_entries
        .last()
        .map(|entry| format!("#{} {}", entry.sequence, safe_text(&entry.summary)))
        .unwrap_or_else(|| "No recent activity".to_owned());

    vec![
        Line::styled("SYSTEM IDENTITY", theme.accent),
        label_value("Installation", model.installation_id.to_string(), theme),
        label_value("Session", model.session_id.to_string(), theme),
        Line::default(),
        Line::styled("HEALTH", theme.accent),
        Line::from(vec![
            ratatui::text::Span::styled(format!("{:<14}", "Runtime"), theme.muted),
            ratatui::text::Span::styled(runtime, runtime_style),
        ]),
        Line::from(vec![
            ratatui::text::Span::styled(format!("{:<14}", "Command"), theme.muted),
            ratatui::text::Span::styled(command, command_style),
        ]),
        label_value("Database", "Ready".to_owned(), theme),
        label_value("Process guard", "Held".to_owned(), theme),
        label_value("Setup", setup, theme),
        label_value("Recent", recent, theme),
    ]
}

fn setup_state(status: &SetupStatus) -> String {
    match status {
        SetupStatus::NotStarted => "Not started".to_owned(),
        SetupStatus::DraftSaved { draft_id } => format!("Draft saved: {draft_id}"),
        SetupStatus::Applied { configuration_id } => format!("Applied: {configuration_id}"),
    }
}
