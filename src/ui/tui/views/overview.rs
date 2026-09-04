use ratatui::{layout::Rect, text::Line, widgets::{Paragraph, Wrap}, Frame};

use crate::setup::SetupStatus;

use super::{label_value, panel, safe_text, workspace_focused};
use crate::ui::tui::{model::TuiModel, theme::Theme};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let runtime = if model.command_in_flight {
        "Busy - command in flight"
    } else {
        "Ready"
    };
    let runtime_style = if model.command_in_flight {
        theme.warning
    } else {
        theme.success
    };
    let setup = setup_state(&model.setup_status);
    let recent = model
        .audit_entries
        .last()
        .map(|entry| format!("#{} {}", entry.sequence, safe_text(&entry.summary)))
        .unwrap_or_else(|| "No recent activity".to_owned());

    let lines = vec![
        Line::styled("SYSTEM IDENTITY", theme.accent),
        label_value("Installation", model.installation_id.to_string(), theme),
        label_value("Session", model.session_id.to_string(), theme),
        Line::default(),
        Line::styled("HEALTH", theme.accent),
        Line::from(vec![
            ratatui::text::Span::styled(format!("{:<14}", "Runtime"), theme.muted),
            ratatui::text::Span::styled(runtime, runtime_style),
        ]),
        label_value("Setup", setup, theme),
        label_value("Recent", recent, theme),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Overview", workspace_focused(model), theme))
            .wrap(Wrap { trim: false })
            .scroll((model.workspace_scroll, 0)),
        area,
    );
}

fn setup_state(status: &SetupStatus) -> String {
    match status {
        SetupStatus::NotStarted => "Not started".to_owned(),
        SetupStatus::DraftSaved { draft_id } => format!("Draft saved: {draft_id}"),
        SetupStatus::Applied { configuration_id } => format!("Applied: {configuration_id}"),
    }
}
