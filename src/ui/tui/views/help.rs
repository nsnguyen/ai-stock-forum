use ratatui::{
    Frame,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Wrap},
};

use super::{panel, workspace_focused, wrapped_height};
use crate::ui::tui::{model::TuiModel, theme::Theme};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(content(theme))
            .block(panel("Help", workspace_focused(model), theme))
            .wrap(Wrap { trim: false })
            .scroll((model.workspace_scroll, 0)),
        area,
    );
}

pub(super) fn content_height(width: u16) -> u16 {
    wrapped_height(content(&Theme::from_no_color(true)), width)
}

fn content(theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::styled("KEYS", theme.accent),
        Line::raw("1-4 / a / s         Open a view outside active text entry"),
        Line::raw("                    1 Overview / 2 Setup / 3 Audit / 4 Help"),
        Line::raw("                    a Agents / s Skills"),
        Line::raw("q                   Inert; does not quit"),
        Line::raw("Tab / Shift+Tab     Move focus forward / backward"),
        Line::raw("Arrow keys          Move within the focused list or field"),
        Line::raw("PageUp / PageDown   Move by one visible page"),
        Line::raw("Home/End            Move to the bounded start / end"),
        Line::raw("/                   Focus the command bar"),
        Line::raw("Enter               Submit or activate"),
        Line::raw("Agents detail       Left/Right selects Assigned Skills or Memory"),
        Line::raw("                    Enter opens the selected nested workspace"),
        Line::raw("Up/Down             Browse command history in the command bar"),
        Line::raw("Esc                 Close, cancel, or clear"),
        Line::raw("i                   Toggle or focus the inspector"),
        Line::raw("?                   Open Help"),
        Line::raw("Ctrl+C              Emergency interrupt from any focus"),
        Line::default(),
        Line::styled("SLASH COMMANDS", theme.accent),
        Line::raw("/help"),
        Line::raw("/status"),
        Line::raw("/setup status"),
        Line::raw("/audit tail [limit: 1-100]"),
        Line::raw("/skills              Open the skill library"),
        Line::raw("/skill list          List saved skills"),
        Line::raw("/skill add           Open the guided skill creator"),
        Line::raw("/quit               Request normal shutdown"),
    ]
}
