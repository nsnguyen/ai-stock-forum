mod agents;
mod audit;
mod help;
mod memory;
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
        View::Chat => render_future(frame, area, model, "Chat", theme),
        View::Connections => render_future(frame, area, model, "Connections", theme),
        View::Activity => render_activity(frame, area, model, theme),
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
        View::Chat | View::Connections => 5,
        View::Activity => {
            u16::try_from(model.audit_entries.len().saturating_add(3)).unwrap_or(u16::MAX)
        }
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
            View::Chat => contextual_lines("Chat", "Coming in Phase 3", theme),
            View::Connections => contextual_lines("Connections", "Coming in Phase 3", theme),
            View::Activity => contextual_lines("Activity", "Readable recent events", theme),
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

pub(super) fn memory_escape_bounded(value: &str, max_bytes: usize) -> String {
    let escaped_bytes = value
        .chars()
        .map(|character| character.escape_default().len())
        .fold(0_usize, usize::saturating_add);
    let truncated = escaped_bytes > max_bytes;
    let content_limit = if truncated && max_bytes >= 3 {
        max_bytes - 3
    } else {
        max_bytes
    };
    let mut escaped = String::with_capacity(max_bytes.min(escaped_bytes));
    for character in value.chars() {
        let fragment = character.escape_default().to_string();
        if escaped.len().saturating_add(fragment.len()) > content_limit {
            break;
        }
        escaped.push_str(&fragment);
    }
    if truncated && max_bytes >= 3 {
        escaped.push_str("...");
    }
    escaped
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

fn render_future(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, name: &str, theme: &Theme) {
    let lines = vec![
        Line::styled(format!("{name} is coming in Phase 3"), theme.accent),
        Line::default(),
        Line::raw("This destination is not interactive yet."),
        Line::styled(
            "No input, credentials, or sample data are shown.",
            theme.muted,
        ),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(name, model.focus == Focus::Workspace, theme))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_activity(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mut lines = vec![
        Line::styled("Recent local activity", theme.accent),
        Line::styled(
            "Readable summaries from the bounded audit history.",
            theme.muted,
        ),
        Line::default(),
    ];
    if model.audit_entries.is_empty() {
        lines.push(Line::raw("No recent activity."));
    } else {
        lines.extend(
            model
                .audit_entries
                .iter()
                .rev()
                .map(|entry| Line::raw(format!("• {}", safe_text(&entry.summary)))),
        );
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Activity", model.focus == Focus::Workspace, theme))
            .wrap(Wrap { trim: false })
            .scroll((model.workspace_scroll, 0)),
        area,
    );
}

#[cfg(test)]
mod memory_text_tests {
    use ratatui::text::Line;

    use super::memory_escape_bounded;

    #[test]
    fn memory_escape_is_visible_fragment_safe_and_bounded_after_escaping() {
        let escaped = memory_escape_bounded("a\\\n\u{1b}\u{202e}\u{e9}z", 25);
        assert_eq!(escaped, "a\\\\\\n\\u{1b}\\u{202e}...");
        assert!(escaped.len() <= 25);
        assert!(Line::from(escaped.clone()).width() <= 25);
        assert!(!escaped.contains('\n'));
        assert!(!escaped.contains('\u{1b}'));
        assert!(!escaped.contains('\u{202e}'));
        assert!(!escaped.contains('\u{e9}'));
    }

    #[test]
    fn memory_escape_does_not_split_an_escape_fragment_at_the_cap() {
        assert_eq!(memory_escape_bounded("ab\u{202e}c", 9), "ab...");
        assert_eq!(memory_escape_bounded("plain", 5), "plain");
    }
}
