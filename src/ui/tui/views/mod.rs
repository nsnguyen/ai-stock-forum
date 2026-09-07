mod agents;
mod audit;
mod help;
mod overview;
mod setup;
mod skills;

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::{
    model::{Focus, TuiModel, View},
    theme::Theme,
};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    if model.skills.active {
        skills::render(frame, area, model, theme);
        return;
    }
    match model.active_view {
        View::Overview => overview::render(frame, area, model, theme),
        View::Setup => setup::render(frame, area, model, theme),
        View::Audit => audit::render(frame, area, model, theme),
        View::Help => help::render(frame, area, model, theme),
        View::Agents => agents::render(frame, area, model, theme),
    }
}

pub(super) fn workspace_content_height(model: &TuiModel, width: u16) -> u16 {
    if model.skills.active {
        return skills::content_height(model, width);
    }
    match model.active_view {
        View::Overview => overview::content_height(model, width),
        View::Setup => setup::content_height(model, width),
        View::Audit => 0,
        View::Help => help::content_height(width),
        View::Agents => agents::content_height(model, width),
    }
}

pub(super) fn agent_list_scroll_offset(model: &TuiModel) -> usize {
    agents::list_scroll_offset(model)
}

pub(super) fn wrapped_height(lines: Vec<Line<'static>>, width: u16) -> u16 {
    let count = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .line_count(width);
    u16::try_from(count).unwrap_or(u16::MAX)
}

pub(super) fn render_inspector(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let border_style = if model.focus == Focus::Inspector {
        theme.focus
    } else {
        theme.muted
    };
    let block = Block::default()
        .title(if model.skills.active {
            " Skill context "
        } else {
            " Inspector "
        })
        .borders(Borders::ALL)
        .border_style(border_style);
    let lines = if model.skills.active {
        skills::inspector_lines(model, theme)
    } else {
        match model.active_view {
            View::Audit => audit::inspector_lines(model, theme),
            View::Overview => {
                contextual_lines("Overview", "Runtime and installation health", theme)
            }
            View::Setup => contextual_lines("Setup", "State is read-only in Phase 0B", theme),
            View::Help => contextual_lines("Help", "Approved keyboard and slash grammar", theme),
            View::Agents => agents::inspector_lines(model, theme),
        }
    };
    let scroll = if !model.skills.active && model.active_view == View::Agents {
        u16::try_from(model.agents.history_scroll).unwrap_or(u16::MAX)
    } else {
        0
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        area,
    );
}

pub(super) fn panel<'a>(title: &'a str, focused: bool, theme: &Theme) -> Block<'a> {
    Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(if focused { theme.focus } else { theme.muted })
}

pub(super) fn label_value<'a>(label: &'a str, value: String, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:<14}"), theme.muted),
        Span::raw(value),
    ])
}

pub(super) fn safe_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

pub(super) fn actor_name(actor: &crate::domain::Actor) -> &'static str {
    match actor {
        crate::domain::Actor::Human => "Human",
        crate::domain::Actor::System => "System",
        crate::domain::Actor::Agent(_) => "Agent",
    }
}

fn contextual_lines<'a>(heading: &'a str, detail: &'a str, theme: &Theme) -> Vec<Line<'a>> {
    vec![
        Line::styled(heading, theme.accent),
        Line::default(),
        Line::raw(detail),
        Line::default(),
        Line::styled("i toggles this panel", theme.muted),
    ]
}

pub(super) fn workspace_focused(model: &TuiModel) -> bool {
    model.focus == Focus::Workspace
}
