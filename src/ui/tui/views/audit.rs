use ratatui::{
    layout::{Constraint, Rect},
    text::Line,
    widgets::{Cell, Row, Table, TableState},
    Frame,
};

use super::{actor_name, label_value, panel, safe_text, workspace_focused};
use crate::ui::tui::{model::TuiModel, theme::Theme};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let header = Row::new(["Sequence", "Kind", "Actor", "Summary"])
        .style(theme.accent)
        .bottom_margin(1);
    let rows = model.audit_entries.iter().map(|entry| {
        Row::new([
            Cell::from(entry.sequence.to_string()),
            Cell::from(safe_text(&entry.kind)),
            Cell::from(actor_name(&entry.actor)),
            Cell::from(safe_text(&entry.summary)),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(20),
            Constraint::Length(9),
            Constraint::Min(8),
        ],
    )
    .header(header)
    .block(panel("Audit - recent activity", workspace_focused(model), theme))
    .row_highlight_style(theme.focus)
    .highlight_symbol("> ");
    let mut state = TableState::default().with_selected(model.audit_selection);
    frame.render_stateful_widget(table, area, &mut state);
}

pub(super) fn inspector_lines<'a>(model: &'a TuiModel, theme: &Theme) -> Vec<Line<'a>> {
    let Some(entry) = model
        .audit_selection
        .and_then(|selection| model.audit_entries.get(selection))
    else {
        return vec![
            Line::styled("Audit detail", theme.accent),
            Line::default(),
            Line::raw("No audit entry selected."),
        ];
    };

    vec![
        Line::styled("Audit detail", theme.accent),
        Line::default(),
        label_value("Sequence", entry.sequence.to_string(), theme),
        label_value("Occurred ms", entry.occurred_at_ms.to_string(), theme),
        label_value("Kind", safe_text(&entry.kind), theme),
        label_value("Actor", actor_name(&entry.actor).to_owned(), theme),
        Line::styled("Correlation", theme.muted),
        Line::raw(entry.correlation_id.to_string()),
        Line::styled("Summary", theme.muted),
        Line::raw(safe_text(&entry.summary)),
    ]
}
