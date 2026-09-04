use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Wrap},
};

use super::{panel, workspace_focused};
use crate::ui::tui::{model::TuiModel, theme::Theme};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let lines = vec![
        Line::styled("KEYS", theme.accent),
        Line::raw("1-4                 Open Overview / Setup / Audit / Help"),
        Line::raw("Tab / Shift+Tab     Move focus forward / backward"),
        Line::raw("Arrow keys          Move within the focused list or field"),
        Line::raw("PageUp / PageDown   Move by one visible page"),
        Line::raw("Home/End            Move to the bounded start / end"),
        Line::raw("/                   Focus the command bar"),
        Line::raw("Enter               Submit or activate"),
        Line::raw("Up/Down             Browse command history in the command bar"),
        Line::raw("Esc                 Close, cancel, or clear"),
        Line::raw("i                   Toggle or focus the inspector"),
        Line::raw("?                   Open Help"),
        Line::raw("q                   Request shutdown outside command entry"),
        Line::raw("Ctrl+C              Request shutdown from any focus"),
        Line::default(),
        Line::styled("SLASH COMMANDS", theme.accent),
        Line::raw("/help"),
        Line::raw("/status"),
        Line::raw("/setup status"),
        Line::raw("/audit tail [limit: 1-100]"),
        Line::raw("/quit"),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Help", workspace_focused(model), theme))
            .wrap(Wrap { trim: false })
            .scroll((model.workspace_scroll, 0)),
        area,
    );
}
