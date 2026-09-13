use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::setup::SetupStatus;
use crate::ui::tui::{
    model::{RuntimeStatus, TuiModel},
    theme::Theme,
};

use super::{
    visuals::{ActionCard, Artwork, Icon, render_action_card, render_artwork},
    workspace_focused,
};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let roomy = area.height >= 22;
    let inset_y = u16::from(roomy);
    let page = Rect {
        x: area.x + 2,
        y: area.y + inset_y,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(inset_y * 2),
    };
    let hero_height = if roomy {
        7
    } else if page.height >= 16 {
        4
    } else {
        3
    };
    let cards_height = if page.height >= 16 { 8 } else { 4 };
    let gap = u16::from(page.height >= 16);
    let bands = Layout::vertical([
        Constraint::Length(hero_height),
        Constraint::Length(gap),
        Constraint::Length(cards_height),
        Constraint::Length(gap),
        Constraint::Min(0),
    ])
    .split(page);
    render_hero(frame, bands[0], model, theme, roomy);

    let columns = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .split(bands[2]);
    for (index, (icon, title, description, compact_description)) in [
        (Icon::Agents, "3 Agents", "Your research crew", "Your crew"),
        (Icon::Skills, "4 Skills", "Reusable guidance", "Guidance"),
        (Icon::Setup, "7 Setup", "Local status", "Local status"),
    ]
    .into_iter()
    .enumerate()
    {
        render_action_card(
            frame,
            columns[index * 2],
            ActionCard {
                icon,
                title,
                description: if columns[index * 2].width < 24 {
                    compact_description
                } else {
                    description
                },
                selected: model.selected_home_action.min(2) == index,
                focused: workspace_focused(model),
                accent: theme.accent,
            },
            theme,
        );
    }
    render_health(frame, bands[4], model, theme);
}

pub(super) fn content_height(_model: &TuiModel, _width: u16) -> u16 {
    // Home is a responsive set of controls, not a scrollable audit document.
    0
}

fn render_hero(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme, roomy: bool) {
    let total = model.agents.profiles.total_count;
    let title = if total == 0 {
        "Build your research crew"
    } else {
        "Your research desk"
    };
    let summary = if total == 0 {
        "Start with an agent. Give it a focus. Make it yours.".to_owned()
    } else {
        format!(
            "{total} {} in your crew. Choose a workspace below.",
            if total == 1 { "agent" } else { "agents" }
        )
    };
    let art_width = if roomy && area.width >= 90 { 24 } else { 0 };
    let text_area = Rect {
        width: area.width.saturating_sub(art_width),
        ..area
    };
    let mut lines = vec![
        Line::styled("YOUR LOCAL WORKSPACE", theme.accent),
        Line::styled(title, theme.base.add_modifier(Modifier::BOLD)),
        Line::styled(summary, theme.muted),
    ];
    if roomy {
        lines.insert(1, Line::default());
        lines.push(Line::default());
        lines.push(Line::styled(
            "Profiles, skills, and memory — one place to think.",
            theme.muted,
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text_area);
    if art_width > 0 {
        render_artwork(
            frame,
            Rect {
                x: area.right() - art_width,
                width: art_width,
                height: area.height.min(5),
                ..area
            },
            Artwork::Bot,
            theme.accent,
        );
    }
}

fn render_health(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    if area.height == 0 {
        return;
    }
    let (runtime, runtime_style) = match model.runtime_status {
        RuntimeStatus::Ready => ("Ready", theme.success),
        RuntimeStatus::Stopping => ("Stopping", theme.warning),
    };
    let (command, command_style) = if model.command_in_flight {
        ("Running", theme.warning)
    } else {
        ("Idle", theme.muted)
    };
    let setup = match &model.setup_status {
        SetupStatus::NotStarted => "Not started",
        SetupStatus::DraftSaved { .. } => "Draft saved",
        SetupStatus::Applied { .. } => "Applied",
    };
    let field = |name: &str, value: &str, style: Style| {
        Line::from(vec![
            Span::styled(format!("{name:<14}"), theme.muted),
            Span::styled(value.to_owned(), style),
        ])
    };
    let lines = vec![
        field("Local app", runtime, runtime_style),
        field("Task", command, command_style),
        field("Local data", "Ready", theme.base),
        field("Setup", setup, theme.muted),
    ];
    if area.height >= 6 {
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .title(" On this device ")
                    .borders(Borders::TOP)
                    .border_type(BorderType::Rounded)
                    .border_style(theme.border()),
            ),
            area,
        );
    } else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Local app ", theme.muted),
                    Span::styled(runtime, runtime_style),
                    Span::styled("  ·  Task ", theme.muted),
                    Span::styled(command, command_style),
                ]),
                Line::styled(
                    "Stored locally in plaintext. Connections come later.",
                    theme.muted,
                ),
            ]),
            area,
        );
    }
}
