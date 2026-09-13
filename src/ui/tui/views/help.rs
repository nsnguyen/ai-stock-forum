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
        Line::raw("1-9                 Open a destination while in NAV"),
        Line::raw("                    1 Home / 2 Chat / 3 Agents / 4 Skills"),
        Line::raw("                    5 Connections / 6 Activity / 7 Setup"),
        Line::raw("                    8 Audit / 9 Help"),
        Line::raw("q                   Inert; does not quit"),
        Line::raw("Tab / Shift+Tab     Move focus forward / backward"),
        Line::raw("W/S                 Move up / down in NAV; literal text in TYPE"),
        Line::raw("A/D                 Move left / right in NAV; literal text in TYPE"),
        Line::raw("Arrow keys          Quiet equivalents for WASD in NAV"),
        Line::raw("PageUp / PageDown   Move by one visible page"),
        Line::raw("Home/End            Move to the bounded start / end"),
        Line::raw("/                   Focus the command bar"),
        Line::raw("Enter               Submit or activate"),
        Line::raw("Agents detail       Left/Right selects Assigned Skills or Memory"),
        Line::raw("                    Enter opens the selected nested workspace"),
        Line::raw("Up/Down             Browse command history in the command bar"),
        Line::raw("Esc                 Close, cancel, or clear"),
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
        Line::default(),
        Line::styled("TRANSITIONAL EDITORS", theme.accent),
        Line::raw("Profile, Memory, and Skills keep their current guided controls."),
        Line::raw("Their validation, review, confirmation, and Esc behavior are unchanged."),
    ]
}
