use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Wrap},
};

use crate::setup::SetupStatus;

use super::{label_value, panel, workspace_focused};
use crate::ui::tui::{model::TuiModel, theme::Theme};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let (state, reference) = match &model.setup_status {
        SetupStatus::NotStarted => ("Not started", None),
        SetupStatus::DraftSaved { draft_id } => ("Draft saved", Some(draft_id.to_string())),
        SetupStatus::Applied { configuration_id } => {
            ("Applied", Some(configuration_id.to_string()))
        }
    };
    let mut lines = vec![
        Line::styled("READ-ONLY SETUP STATUS", theme.accent),
        Line::default(),
        label_value("State", state.to_owned(), theme),
    ];
    if let Some(reference) = reference {
        lines.push(label_value("Reference", reference, theme));
    }
    lines.extend([
        Line::default(),
        Line::styled("Read-only in Phase 0B.", theme.warning),
        Line::raw("Setup editing is deferred to a later phase."),
        Line::default(),
        Line::styled(
            "Use /setup status to refresh this native view.",
            theme.muted,
        ),
    ]);

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Setup", workspace_focused(model), theme))
            .wrap(Wrap { trim: false })
            .scroll((model.workspace_scroll, 0)),
        area,
    );
}
