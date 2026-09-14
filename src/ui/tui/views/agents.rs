use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentReadiness, DESCRIPTION_MAX_BYTES,
        DISPLAY_NAME_MAX_BYTES, INSTRUCTIONS_MAX_BYTES, MAX_SPECIALTY_TAGS, PERSONALITY_MAX_BYTES,
        PRIMARY_SPECIALTY_MAX_BYTES, ProfileDiffField, ProfileFieldDiff, ProfileFieldValue,
        SPECIALTY_TAG_MAX_BYTES,
    },
    app::ApplicationCommand,
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorMode},
        tui::{
            layout::{agent_layout_mode, agent_workspace, view_geometry},
            model::{
                AgentSkillAction, AgentSkillUpgradeAvailability, AgentsPane, Focus, TuiModel, View,
            },
            theme::Theme,
        },
    },
};

use super::visuals::{ActionCard, Artwork, Icon, render_action_card, render_artwork};
use super::{label_value, panel, safe_text, workspace_focused};

mod profile_home;

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    if model.agents.pane == AgentsPane::Memory {
        super::memory::render(frame, area, model, theme);
        return;
    }
    let mode = agent_layout_mode(frame.area());
    let layout = agent_workspace(area, mode);
    let templates = model.agents.pane == AgentsPane::Editor
        && model.agents.editor_page == crate::ui::tui::model::ProfileEditorPage::Templates;
    if layout.list.is_none() && model.focus == Focus::List {
        if templates {
            profile_home::render_template_list(frame, area, model, theme);
        } else {
            render_list(frame, area, model, theme);
        }
        return;
    }
    if let Some(list) = layout.list {
        if templates {
            profile_home::render_template_list(frame, list, model, theme);
        } else {
            render_list(frame, list, model, theme);
        }
    }
    render_active(frame, layout.active, model, theme, layout.list.is_none());
}

pub(super) fn content_height(model: &TuiModel, width: u16) -> u16 {
    if model.agents.pane == AgentsPane::Memory {
        super::memory::content_height(model, width)
    } else {
        0
    }
}

pub(super) fn profile_home_columns(model: &TuiModel) -> usize {
    profile_home::columns_for(active_area(model))
}

fn render_active(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    theme: &Theme,
    one_pane: bool,
) {
    match model.agents.pane {
        AgentsPane::List if one_pane => render_list(frame, area, model, theme),
        AgentsPane::Editor => render_editor(frame, area, model, theme),
        AgentsPane::Confirmation => render_confirmation(frame, area, model, theme),
        AgentsPane::History => render_history(frame, area, model, theme),
        AgentsPane::Memory => super::memory::render(frame, area, model, theme),
        AgentsPane::List | AgentsPane::Detail => render_detail(frame, area, model, theme),
    }
}

fn render_list(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let focused = model.focus == Focus::List
        || (workspace_focused(model) && model.agents.pane == AgentsPane::List);
    let block = panel("Agent list", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if model.agents.profiles.profiles.is_empty() {
        if area.width < frame.area().width {
            // The workspace owns the full welcome when both panes are visible.
            frame.render_widget(
                Paragraph::new(vec![
                    Line::styled("Your crew is empty.", theme.muted),
                    Line::default(),
                    Line::styled("N  New agent", theme.accent),
                ])
                .wrap(Wrap { trim: false }),
                Rect::new(
                    inner.x + 1,
                    inner.y + 1,
                    inner.width.saturating_sub(2),
                    inner.height.saturating_sub(1),
                ),
            );
        } else {
            render_empty_agents(frame, inner, theme);
        }
        return;
    }
    let mut y = inner.y;
    for (index, profile) in model
        .agents
        .profiles
        .profiles
        .iter()
        .enumerate()
        .skip(list_scroll_offset_for_area(model, area))
    {
        if inner.bottom().saturating_sub(y) < 3 {
            break;
        }
        let height = profile_card_height(profile, inner.width).min(inner.bottom() - y);
        let card = Rect::new(inner.x, y, inner.width, height);
        let selected = index == model.agents.selected_profile;
        let block = Block::default()
            .title(if selected && focused { ">" } else { "" })
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .style(if selected {
                theme.selected_surface()
            } else {
                theme.surface()
            })
            .border_style(if selected && focused {
                theme.accent
            } else {
                theme.border()
            });
        let content = block.inner(card);
        frame.render_widget(block, card);
        let badge = Rect::new(
            content.x,
            content.y,
            4.min(content.width),
            3.min(content.height),
        );
        render_badge(
            frame,
            badge,
            &profile.display_name,
            theme.agent_badge(profile.profile_id),
        );
        let text = Rect::new(
            content.x.saturating_add(5),
            content.y,
            content.width.saturating_sub(5),
            content.height,
        );
        frame.render_widget(
            Paragraph::new(profile_card_lines(profile, selected, theme)).wrap(Wrap { trim: false }),
            text,
        );
        y = y.saturating_add(height).saturating_add(1);
    }
}

fn render_badge(frame: &mut Frame<'_>, area: Rect, name: &str, style: ratatui::style::Style) {
    frame.render_widget(Block::default().style(style), area);
    if area.height > 0 {
        frame.render_widget(
            Paragraph::new(monogram(name))
                .style(style)
                .alignment(Alignment::Center),
            Rect::new(area.x, area.y + area.height / 2, area.width, 1),
        );
    }
}

fn render_empty_agents(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let art_height = if area.height >= 13 {
        6
    } else if area.height >= 10 {
        5
    } else {
        0
    };
    if art_height > 0 {
        render_artwork(
            frame,
            Rect::new(area.x, area.y, area.width, art_height),
            Artwork::Bot,
            theme.accent,
        );
    }
    let lines = vec![
        Line::styled("No agent profiles yet", theme.accent),
        Line::default(),
        Line::raw("Press N to create your first agent."),
        Line::default(),
        Line::styled("Start with a template, then make it your own.", theme.muted),
    ];
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect::new(
            area.x + 1,
            area.y + art_height,
            area.width.saturating_sub(2),
            area.height.saturating_sub(art_height),
        ),
    );
}

pub(super) fn list_scroll_offset(model: &TuiModel) -> usize {
    let terminal = Rect::new(0, 0, model.terminal_width, model.terminal_height);
    let geometry = view_geometry(terminal, View::Agents, model.inspector_open);
    let workspace = agent_workspace(geometry.cockpit.workspace, geometry.cockpit.mode);
    let area = workspace.list.unwrap_or(workspace.active);
    list_scroll_offset_for_area(model, area)
}

fn list_scroll_offset_for_area(model: &TuiModel, area: Rect) -> usize {
    let profiles = &model.agents.profiles.profiles;
    let Some(last) = profiles.len().checked_sub(1) else {
        return 0;
    };
    let selected = model.agents.selected_profile.min(last);
    let mut first = model.agents.list_scroll.min(last);
    if selected < first {
        return selected;
    }

    let viewport_height = usize::from(area.height.saturating_sub(2));
    if viewport_height == 0 {
        return selected;
    }
    let inner_width = area.width.saturating_sub(2);
    let heights = profiles[first..=selected]
        .iter()
        .map(|profile| usize::from(profile_card_height(profile, inner_width)) + 1)
        .collect::<Vec<_>>();
    let mut visible_height = heights
        .iter()
        .copied()
        .fold(0usize, usize::saturating_add)
        .saturating_sub(1);
    for height in heights {
        if visible_height <= viewport_height || first == selected {
            break;
        }
        visible_height = visible_height.saturating_sub(height);
        first = first.saturating_add(1);
    }
    first
}

fn profile_card_height(profile: &crate::app::AgentProfileSummary, width: u16) -> u16 {
    let lines = profile_card_lines(profile, false, &Theme::from_no_color(true));
    let height = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .line_count(width.saturating_sub(7).max(1));
    u16::try_from(height.max(3))
        .unwrap_or(u16::MAX)
        .saturating_add(2)
}

fn profile_card_lines(
    profile: &crate::app::AgentProfileSummary,
    selected: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    vec![
        Line::styled(
            safe_text(&profile.display_name),
            if selected {
                theme.accent
            } else {
                theme.agent_monogram(profile.profile_id)
            },
        ),
        Line::styled(
            readiness_name(profile.readiness),
            readiness_style(profile.readiness, theme),
        ),
        Line::styled(safe_text(&profile.primary_specialty), theme.muted),
    ]
}

fn detail_header(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    let Some(row) = model.agents.selected_summary() else {
        return Vec::new();
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", monogram(&row.display_name)),
                theme.agent_badge(row.profile_id),
            ),
            Span::styled(format!("  {}", safe_text(&row.display_name)), theme.accent),
        ]),
        readiness_line("Connection", row.readiness, theme),
    ];
    if model.agents.skill_panel_open {
        lines.push(Line::styled(
            "Assigned skills · Esc returns to Profile",
            theme.muted,
        ));
    } else {
        lines.extend(agent_detail_action_lines(model, theme).into_iter().take(2));
    }
    lines
}

fn detail_body(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    let Some(row) = model.agents.selected_summary() else {
        return vec![Line::raw(
            "No agent profiles yet. Press N to create your first agent.",
        )];
    };
    if let Some(detail) = model.agents.matching_detail() {
        if model.agents.skill_panel_open {
            assigned_skill_lines(model, theme)
        } else {
            detail_lines(detail, theme)
        }
    } else {
        vec![
            Line::styled(
                format!("Loading {}", safe_text(&row.display_name)),
                theme.muted,
            ),
            Line::raw("Actions become available when this profile is loaded."),
        ]
    }
}

fn active_area(model: &TuiModel) -> Rect {
    let terminal = Rect::new(0, 0, model.terminal_width, model.terminal_height);
    let cockpit = crate::ui::tui::layout::calculate_with_input(
        terminal,
        model.inspector_is_visible(),
        model.input_is_visible(),
    );
    agent_workspace(cockpit.workspace, agent_layout_mode(terminal)).active
}

fn document_geometry(area: Rect, header_height: usize) -> (Rect, Rect) {
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let height = u16::try_from(header_height)
        .unwrap_or(u16::MAX)
        .min(inner.height.saturating_sub(1));
    (
        Rect { height, ..inner },
        Rect {
            y: inner.y + height,
            height: inner.height - height,
            ..inner
        },
    )
}

fn body_limit(lines: Vec<Line<'static>>, body: Rect) -> usize {
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .line_count(body.width.max(1))
        .saturating_sub(usize::from(body.height))
}

pub(super) fn detail_scroll_geometry(model: &TuiModel) -> (usize, usize) {
    let theme = Theme::from_no_color(true);
    let (_, body) = detail_geometry(active_area(model), model);
    (
        body_limit(detail_body(model, &theme), body),
        usize::from(body.height.max(1)),
    )
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let (heading, body) = detail_geometry(area, model);
    frame.render_widget(
        panel("Agent workspace", agent_workspace_focused(model), theme),
        area,
    );
    // Keep selected identity and actions visible; only the document body scrolls.
    if model.agents.selected_summary().is_none() {
        let inner = panel("", false, theme).inner(area);
        render_empty_agents(frame, inner, theme);
        return;
    }
    if visual_header_height(area, model).is_some() {
        render_detail_hero(frame, heading, model, theme);
    } else {
        frame.render_widget(Paragraph::new(detail_header(model, theme)), heading);
    }
    let lines = detail_body(model, theme);
    let offset = model
        .agents
        .detail_scroll
        .min(body_limit(lines.clone(), body));
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
        body,
    );
}

fn visual_header_height(area: Rect, model: &TuiModel) -> Option<usize> {
    if model.agents.skill_panel_open || area.height < 17 || area.width < 62 {
        None
    } else if area.height >= 22 {
        Some(14)
    } else {
        Some(10)
    }
}

fn detail_geometry(area: Rect, model: &TuiModel) -> (Rect, Rect) {
    let height = visual_header_height(area, model)
        .unwrap_or_else(|| detail_header(model, &Theme::from_no_color(true)).len());
    document_geometry(area, height)
}

fn render_detail_hero(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    use crate::ui::tui::model::AgentDetailAction;
    let Some(row) = model.agents.selected_summary() else {
        return;
    };
    let spacious = area.height >= 14;
    let hero_height = if spacious { 6 } else { 4 };
    let card_height = if spacious { 7 } else { 5 };
    let art_width = if spacious && area.width >= 104 { 24 } else { 0 };
    let accent = theme.agent_monogram(row.profile_id);
    render_badge(
        frame,
        Rect::new(area.x + 1, area.y, 5, 3),
        &row.display_name,
        theme.agent_badge(row.profile_id),
    );
    let identity = Rect::new(
        area.x + 8,
        area.y,
        area.width.saturating_sub(9 + art_width),
        3,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(safe_text(&row.display_name), accent),
            Line::styled(
                readiness_name(row.readiness),
                readiness_style(row.readiness, theme),
            ),
            Line::styled(
                format!(
                    "{} · {}",
                    row.role.as_str(),
                    safe_text(&row.primary_specialty)
                ),
                theme.muted,
            ),
        ]),
        identity,
    );
    if spacious {
        let purpose = model
            .agents
            .matching_detail()
            .map(|detail| safe_text(detail.profile.description()))
            .unwrap_or_else(|| format!("Loading {}", safe_text(&row.display_name)));
        frame.render_widget(
            Paragraph::new(purpose)
                .style(theme.muted)
                .wrap(Wrap { trim: false }),
            Rect::new(
                area.x + 1,
                area.y + 4,
                area.width.saturating_sub(2 + art_width),
                2,
            ),
        );
        if art_width > 0 {
            let artwork = match row.role {
                crate::agents::AgentRole::Bear => Artwork::Bear,
                crate::agents::AgentRole::Bull => Artwork::Bull,
                _ => Artwork::Bot,
            };
            render_artwork(
                frame,
                Rect::new(area.right() - art_width, area.y, art_width, 6),
                artwork,
                accent,
            );
        }
    }
    let actions = [
        (
            AgentDetailAction::Profile,
            Icon::Profile,
            "Profile",
            "Role & focus",
        ),
        (
            AgentDetailAction::Memory,
            Icon::Memory,
            "Memory",
            "Saved notes",
        ),
        (
            AgentDetailAction::AssignedSkills,
            Icon::Skills,
            "Skills",
            "Agent skills",
        ),
        (
            AgentDetailAction::History,
            Icon::History,
            "History",
            "Past versions",
        ),
    ];
    for (index, (action, icon, title, description)) in actions.into_iter().enumerate() {
        let start = area.width * index as u16 / 4;
        let end = area.width * (index as u16 + 1) / 4;
        render_action_card(
            frame,
            Rect::new(
                area.x + start,
                area.y + hero_height,
                end - start - 1,
                card_height,
            ),
            ActionCard {
                icon,
                title,
                description,
                selected: model.agents.selected_detail_action == action,
                focused: agent_workspace_focused(model),
                accent,
            },
            theme,
        );
    }
    frame.render_widget(
        Paragraph::new("A/D choose   Enter open   W/S scroll").style(theme.muted),
        Rect::new(
            area.x + 1,
            area.y + hero_height + card_height,
            area.width.saturating_sub(2),
            1,
        ),
    );
}
fn agent_workspace_focused(model: &TuiModel) -> bool {
    workspace_focused(model) && model.agents.pane != AgentsPane::List
}

fn agent_detail_action_lines(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    use crate::ui::tui::model::AgentDetailAction;
    let mut spans = Vec::new();
    for (action, label) in [
        (AgentDetailAction::Profile, "Profile"),
        (AgentDetailAction::Memory, "Memory"),
        (AgentDetailAction::AssignedSkills, "Skills"),
        (AgentDetailAction::History, "History"),
    ] {
        spans.push(Span::styled(
            format!(" {label} "),
            if model.agents.selected_detail_action == action && agent_workspace_focused(model) {
                theme.focus
            } else if model.agents.selected_detail_action == action {
                theme.accent
            } else {
                theme.muted
            },
        ));
        spans.push(Span::raw(" "));
    }
    vec![
        Line::from(spans),
        Line::styled("A/D choose   Enter open   W/S scroll", theme.muted),
        Line::default(),
    ]
}

fn assigned_skill_lines(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    let Some(detail) = model.agents.detail.as_ref() else {
        return vec![
            Line::styled("Assigned skills unavailable", theme.warning),
            Line::raw("Press Esc and reload the agent detail."),
        ];
    };
    let Some(reference) = detail
        .profile
        .skill_refs()
        .get(model.agents.selected_assigned_skill)
    else {
        return vec![
            Line::styled("No assigned skills", theme.accent),
            Line::raw("Assign a skill from the Skills workspace."),
            Line::styled("Esc: agent detail", theme.focus),
        ];
    };
    let selected_action = model.selected_available_agent_skill_action();
    let mut action_spans = vec![Span::styled("Actions  ", theme.accent)];
    for (index, action) in model
        .available_agent_skill_actions()
        .iter()
        .copied()
        .enumerate()
    {
        if index > 0 {
            action_spans.push(Span::raw("  "));
        }
        let label = match action {
            AgentSkillAction::View => "View",
            AgentSkillAction::Upgrade => "Upgrade",
            AgentSkillAction::Unassign => "Unassign",
        };
        action_spans.push(Span::styled(
            format!("[{label}]"),
            if action == selected_action && agent_workspace_focused(model) {
                theme.focus
            } else {
                theme.muted
            },
        ));
    }
    let availability = model.agent_skill_upgrade_availability();
    let availability_text = match &availability {
        AgentSkillUpgradeAvailability::Unknown => {
            "UNKNOWN - active skill version data could not be verified".to_owned()
        }
        AgentSkillUpgradeAvailability::Current => {
            "CURRENT - exact pin equals the loaded active reference".to_owned()
        }
        AgentSkillUpgradeAvailability::Available(active) => format!(
            "AVAILABLE - Upgrade available: active v{}; explicit review required",
            active.version().get()
        ),
        AgentSkillUpgradeAvailability::Inconsistent => {
            "INCONSISTENT - r: reload active skill data; Upgrade is hidden".to_owned()
        }
    };
    let enter_guidance = match selected_action {
        AgentSkillAction::View => "Enter: View",
        AgentSkillAction::Upgrade => "Enter: Upgrade",
        AgentSkillAction::Unassign => "Enter: Unassign",
    };
    let skill_refs = detail.profile.skill_refs();
    let mut lines = vec![
        Line::styled(
            format!(
                "Skill {} of {}",
                model.agents.selected_assigned_skill.saturating_add(1),
                skill_refs.len()
            ),
            theme.accent,
        ),
        Line::from(action_spans),
        Line::styled(enter_guidance, theme.focus),
        Line::styled("Esc: detail", theme.focus),
        Line::styled("Left/Right: action | Up/Down: assigned skill", theme.muted),
        Line::styled(
            availability_text,
            match availability {
                AgentSkillUpgradeAvailability::Available(_) => theme.success,
                AgentSkillUpgradeAvailability::Current => theme.accent,
                AgentSkillUpgradeAvailability::Unknown
                | AgentSkillUpgradeAvailability::Inconsistent => theme.warning,
            },
        ),
        Line::default(),
        Line::styled("ASSIGNED ROWS", theme.accent),
    ];
    for (index, assigned) in skill_refs.iter().enumerate() {
        lines.push(Line::styled(
            format!(
                "{} {}  v{}  {}",
                if index == model.agents.selected_assigned_skill {
                    ">"
                } else {
                    " "
                },
                index + 1,
                assigned.version().get(),
                model
                    .skills
                    .library
                    .skills
                    .iter()
                    .find(|skill| skill.skill_ref.skill_id() == assigned.skill_id())
                    .map(|skill| safe_text(&skill.display_name))
                    .unwrap_or_else(|| "Assigned skill".to_owned())
            ),
            if index == model.agents.selected_assigned_skill {
                theme.focus
            } else {
                theme.muted
            },
        ));
    }
    lines.extend([
        Line::default(),
        Line::styled("PINNED EXACT VERSION", theme.accent),
        label_value("Version", format!("v{}", reference.version().get()), theme),
        Line::styled(
            "This exact skill version is pinned to the agent.",
            theme.muted,
        ),
        Line::styled(
            "No automatic upgrades. Skill text grants no capability.",
            theme.muted,
        ),
    ]);
    lines
}

fn detail_lines(detail: &crate::app::AgentProfileView, theme: &Theme) -> Vec<Line<'static>> {
    // The hero can clip a long name in compact layouts. Keep the complete name
    // in the wrapping, scrollable profile as well as the pinned identity.
    profile_version_lines(&detail.profile, detail.readiness, "Active version", theme)
}

fn profile_version_lines(
    profile: &crate::agents::AgentProfileVersion,
    readiness: AgentReadiness,
    version_label: &'static str,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let historical = version_label.contains("Historical");
    let mut lines = vec![
        Line::styled(safe_text(profile.display_name()), theme.accent),
        Line::styled(
            format!(
                "Version {} · {} · {}",
                profile.version().get(),
                if historical {
                    "Historical · Read-only"
                } else {
                    "Current"
                },
                readable_date(profile.created_at_ms())
            ),
            theme.muted,
        ),
        Line::default(),
        label_value("Role", profile.role().as_str().to_owned(), theme),
        label_value("Specialty", safe_text(profile.primary_specialty()), theme),
        label_value(
            "Tags",
            safe_text(&profile.specialty_tags().join(", ")),
            theme,
        ),
        readiness_line("Connection", readiness, theme),
        Line::default(),
        label_value("Description", safe_text(profile.description()), theme),
        Line::default(),
        label_value("Personality", safe_text(profile.personality()), theme),
        Line::default(),
        label_value("Instructions", safe_text(profile.instructions()), theme),
        Line::default(),
        label_value(
            "Skills",
            format!(
                "{} assigned · open Skills to inspect",
                profile.skill_refs().len()
            ),
            theme,
        ),
        label_value("Bindings", bindings_value(profile.bindings()), theme),
    ];
    if profile.role() == crate::agents::AgentRole::Engineering {
        lines.push(Line::styled(
            "Engineering requires both inference and engineering bindings.",
            theme.muted,
        ));
    }
    lines.push(Line::styled(
        "Bindings are read-only here. Configure connections separately.",
        theme.muted,
    ));
    if historical {
        lines.push(Line::styled(
            "E edits the current active profile.",
            theme.focus,
        ));
    }
    lines
}

fn history_cards(model: &TuiModel, theme: &Theme) -> Vec<Vec<Line<'static>>> {
    let Some(row) = model.agents.selected_summary() else {
        return Vec::new();
    };
    let Some(history) = model.agents.history.as_ref().filter(|history| {
        history.profile_id == row.profile_id && history.active_version_id == row.profile_version_id
    }) else {
        return Vec::new();
    };
    history
        .versions
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            vec![
                Line::styled(
                    format!(
                        "{} Version {} · {}",
                        if index == model.agents.selected_history_version {
                            "›"
                        } else {
                            " "
                        },
                        entry.version.get(),
                        if entry.profile_version_id == history.active_version_id {
                            "Current"
                        } else {
                            "Historical · Read-only"
                        }
                    ),
                    if index == model.agents.selected_history_version
                        && agent_workspace_focused(model)
                    {
                        theme.focus
                    } else {
                        theme.muted
                    },
                ),
                Line::styled(
                    format!(
                        "  {} · {}",
                        readable_date(entry.created_at_ms),
                        readiness_name(entry.readiness)
                    ),
                    theme.muted,
                ),
                Line::default(),
            ]
        })
        .collect()
}

fn history_document(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    let Some(row) = model.agents.selected_summary() else {
        return vec![Line::raw("Select an agent to view history.")];
    };
    if let Some(detail) = model
        .agents
        .version_detail
        .as_ref()
        .filter(|detail| detail.profile.profile_id() == row.profile_id)
    {
        let current = detail.profile.profile_version_id() == row.profile_version_id;
        let mut lines = profile_version_lines(
            &detail.profile,
            detail.readiness,
            if current { "Current" } else { "Historical" },
            theme,
        );
        lines.push(Line::default());
        lines.push(Line::styled("Changes from previous version", theme.accent));
        append_diffs(&mut lines, &detail.predecessor_diff, theme);
        lines
    } else {
        vec![Line::raw(format!(
            "Loading history for {}",
            safe_text(&row.display_name)
        ))]
    }
}

fn history_rows_for_area(model: &TuiModel, area: Rect) -> (usize, usize) {
    let (_, body) = document_geometry(area, 2);
    let cards = history_cards(model, &Theme::from_no_color(true));
    if cards.is_empty() {
        return (0, 1);
    }
    let heights: Vec<_> = cards
        .iter()
        .map(|card| {
            Paragraph::new(card.clone())
                .wrap(Wrap { trim: false })
                .line_count(body.width.max(1))
        })
        .collect();
    let selected = model.agents.selected_history_version.min(cards.len() - 1);
    let mut first = model.agents.history_scroll.min(selected);
    let mut height: usize = heights[first..=selected].iter().sum();
    while first < selected && height > usize::from(body.height) {
        height = height.saturating_sub(heights[first]);
        first += 1;
    }
    let mut visible = 0;
    let mut used = 0;
    for height in &heights[first..] {
        if used + height > usize::from(body.height) && visible > 0 {
            break;
        }
        used += height;
        visible += 1;
    }
    (first, visible.max(1))
}

pub(super) fn history_scroll_geometry(model: &TuiModel) -> (usize, usize) {
    history_rows_for_area(model, active_area(model))
}

pub(super) fn version_scroll_geometry(model: &TuiModel) -> (usize, usize) {
    let (_, body) = document_geometry(active_area(model), 2);
    (
        body_limit(history_document(model, &Theme::from_no_color(true)), body),
        usize::from(body.height.max(1)),
    )
}

fn render_history(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let (heading, body) = document_geometry(area, 2);
    let name = model
        .agents
        .selected_summary()
        .map(|row| safe_text(&row.display_name))
        .unwrap_or_else(|| "Agent".into());
    let inspecting = model.agents.version_detail.is_some();
    frame.render_widget(
        panel("Profile history", agent_workspace_focused(model), theme),
        area,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(format!("{name} · History"), theme.accent),
            Line::styled(
                if inspecting {
                    if model.agents.editor.is_some() {
                        "W/S scroll · Esc history · E Resume draft"
                    } else {
                        "W/S scroll · Esc history · E edit current"
                    }
                } else {
                    "W/S choose · Enter inspect · Esc profile"
                },
                theme.muted,
            ),
        ]),
        heading,
    );
    let cards = history_cards(model, theme);
    let lines = if inspecting || cards.is_empty() {
        history_document(model, theme)
    } else {
        cards
            .into_iter()
            .skip(history_rows_for_area(model, area).0)
            .flatten()
            .collect()
    };
    let offset = if inspecting {
        model
            .agents
            .version_scroll
            .min(body_limit(lines.clone(), body))
    } else {
        0
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0)),
        body,
    );
}
fn render_editor(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    use crate::ui::tui::model::ProfileEditorPage;
    match model.agents.editor_page {
        ProfileEditorPage::Templates => {
            profile_home::render_template_preview(frame, area, model, theme);
            return;
        }
        ProfileEditorPage::Home => {
            profile_home::render_home(frame, area, model, theme);
            return;
        }
        ProfileEditorPage::Section(section) => {
            profile_home::render_section(frame, area, model, section, theme);
            return;
        }
        ProfileEditorPage::Discard => {
            profile_home::render_discard(frame, area, model, theme);
            return;
        }
        ProfileEditorPage::Review => {}
    }
    use crate::ui::tui::model::InputMode;
    let active = model
        .agents
        .editor
        .as_ref()
        .map(|editor| editor.tui_field());
    let focused = agent_workspace_focused(model);
    let is_typing = focused && model.input_mode == InputMode::Type;
    let active_error = model.agents.editor.as_ref().and_then(|editor| {
        editor.tui_field_error(editor.tui_field()).map(|_| {
            format!(
                "{}: invalid; revise before review",
                field_label(editor.tui_field())
            )
        })
    });
    let active_height = if is_typing {
        5.min(area.height)
    } else if active_error.is_some() {
        4.min(area.height)
    } else {
        3.min(area.height)
    };
    if is_typing {
        let input = &model.agents.field_input;
        let cursor = input.cursor_byte().min(input.text().len());
        let prefix = &input.text()[..cursor];
        let tail = &input.text()[cursor..];
        let capacity = usize::from(area.width.saturating_sub(3));
        let safe_prefix = safe_text(prefix);
        let mut width = 0;
        let visible_prefix: String = safe_prefix
            .chars()
            .rev()
            .take_while(|character| {
                width += Span::raw(character.to_string()).width();
                width <= capacity
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let text = format!("{}│{}", visible_prefix, safe_text(tail));
        let label = active.map(field_label).unwrap_or_default();
        let mut input_lines = vec![
            Line::raw(text),
            Line::styled(
                "Enter accepts · Esc keeps draft · Tab next field",
                theme.muted,
            ),
        ];
        if let Some(error) = &active_error {
            input_lines.push(Line::styled(error.clone(), theme.warning));
        }
        frame.render_widget(
            Paragraph::new(input_lines).block(panel(&format!("TYPE · {label}"), focused, theme)),
            Rect {
                height: active_height,
                ..area
            },
        );
    }
    if !is_typing && let Some(field) = active {
        let mut guidance = vec![Line::raw(field_guidance(field))];
        if let Some(error) = active_error {
            guidance.push(Line::styled(error, theme.warning));
        }
        frame.render_widget(
            Paragraph::new(guidance).block(panel("Selected field", focused, theme)),
            Rect {
                height: active_height,
                ..area
            },
        );
    }
    let area = Rect {
        y: area.y + active_height,
        height: area.height.saturating_sub(active_height),
        ..area
    };
    let lines = model
        .agents
        .editor
        .as_ref()
        .map(|editor| editor_lines(editor, theme, focused))
        .unwrap_or_else(|| vec![Line::raw("Editor is unavailable. Press Esc to return.")]);
    let marker = lines
        .iter()
        .position(|line| line.to_string().starts_with('›'))
        .unwrap_or(0);
    let prefix_height = Paragraph::new(lines[..marker].to_vec())
        .wrap(Wrap { trim: false })
        .line_count(area.width.saturating_sub(2));
    let automatic_scroll = prefix_height.saturating_sub(usize::from(area.height.saturating_sub(8)));
    let is_review = active == Some(crate::ui::profile_editor::ProfileTuiField::Review);
    let max_scroll = Paragraph::new(lines.clone())
        .wrap(Wrap { trim: false })
        .line_count(area.width.saturating_sub(2))
        .saturating_sub(usize::from(area.height.saturating_sub(2)));
    let offset = if is_review {
        model.agents.detail_scroll.min(max_scroll)
    } else {
        automatic_scroll
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Profile editor", workspace_focused(model), theme))
            .wrap(Wrap { trim: false })
            .scroll((scroll(offset, area.height), 0)),
        area,
    );
}

fn editor_lines(editor: &ProfileEditor, theme: &Theme, focused: bool) -> Vec<Line<'static>> {
    use crate::ui::profile_editor::ProfileTuiField;
    let field = editor.tui_field();
    if field == ProfileTuiField::Review {
        return profile_home::review_lines(editor, theme, focused);
    }
    let mode = if matches!(editor.mode(), ProfileEditorMode::Create { .. }) {
        "New agent"
    } else {
        "Edit agent"
    };
    let mut lines = vec![
        Line::styled(
            format!("{mode} · {}", safe_text(&editor.draft().display_name)),
            theme.accent,
        ),
        Line::styled(
            if focused {
                "Tab next field   Shift+Tab previous   Enter edit/select   Esc keep draft"
            } else {
                "Draft retained · Tab to the workspace to resume"
            },
            theme.muted,
        ),
        Line::default(),
    ];
    for (item, label) in [
        (ProfileTuiField::Template, "Template"),
        (ProfileTuiField::DisplayName, "Display name"),
        (ProfileTuiField::Role, "Role"),
        (ProfileTuiField::Description, "Description"),
        (ProfileTuiField::PrimarySpecialty, "Specialty"),
        (ProfileTuiField::Tags, "Tags"),
        (ProfileTuiField::Personality, "Personality"),
        (ProfileTuiField::Instructions, "Instructions"),
        (ProfileTuiField::Bindings, "Bindings"),
        (ProfileTuiField::Review, "Review changes"),
        (ProfileTuiField::Discard, "Discard draft"),
    ] {
        let value = match item {
            ProfileTuiField::Template => crate::agents::builtin_profile_templates()
                .iter()
                .find(|template| template.id.as_str() == editor.tui_field_text(item))
                .map(|template| template.suggested_name.to_owned())
                .unwrap_or_else(|| "Current profile · unchanged".to_owned()),
            ProfileTuiField::Bindings => {
                format!("{} · read-only", bindings_value(&editor.draft().bindings))
            }
            ProfileTuiField::Review => "Enter to review".to_owned(),
            ProfileTuiField::Discard => "Enter to discard this draft".to_owned(),
            _ => safe_text(editor.tui_field_text(item)),
        };
        lines.push(Line::styled(
            format!("{} {label}", if field == item { "›" } else { " " }),
            if field == item && focused {
                theme.focus
            } else if field == item {
                theme.accent
            } else {
                theme.muted
            },
        ));
        lines.push(Line::raw(format!("  {value}")));
        if editor.tui_field_error(item).is_some() {
            lines.push(Line::styled(
                "  Please revise this field before reviewing.",
                theme.warning,
            ));
        }
    }
    if matches!(field, ProfileTuiField::Template | ProfileTuiField::Role) {
        lines.push(Line::styled(
            "WASD selects a choice; these controls do not accept text.",
            theme.muted,
        ));
    }
    if let Some(message) = editor.local_message() {
        lines.push(Line::styled(editor_message(message.code()), theme.warning));
    }
    lines
}

fn append_diffs(lines: &mut Vec<Line<'static>>, diffs: &[ProfileFieldDiff], theme: &Theme) {
    if diffs.is_empty() {
        lines.push(Line::raw("No accepted fields changed."));
        return;
    }
    for diff in diffs {
        lines.push(Line::styled(diff_name(diff.field), theme.accent));
        lines.push(label_value("Before", field_value(&diff.before), theme));
        lines.push(label_value("After", field_value(&diff.after), theme));
    }
}

fn append_create_diffs(
    lines: &mut Vec<Line<'static>>,
    baseline: &AgentProfileDraft,
    draft: &AgentProfileDraft,
    theme: &Theme,
) {
    let changes = [
        (
            "Display name",
            safe_text(&baseline.display_name),
            safe_text(&draft.display_name),
        ),
        (
            "Description",
            safe_text(&baseline.description),
            safe_text(&draft.description),
        ),
        (
            "Role",
            baseline.role.as_str().to_owned(),
            draft.role.as_str().to_owned(),
        ),
        (
            "Primary specialty",
            safe_text(&baseline.primary_specialty),
            safe_text(&draft.primary_specialty),
        ),
        (
            "Specialty tags",
            safe_text(&baseline.specialty_tags.join(", ")),
            safe_text(&draft.specialty_tags.join(", ")),
        ),
        (
            "Personality",
            safe_text(&baseline.personality),
            safe_text(&draft.personality),
        ),
        (
            "Instructions",
            safe_text(&baseline.instructions),
            safe_text(&draft.instructions),
        ),
        (
            "Bindings",
            bindings_value(&baseline.bindings),
            bindings_value(&draft.bindings),
        ),
    ];
    let mut count = 0;
    for (field, before, after) in changes {
        if before != after {
            count += 1;
            lines.push(Line::styled(field, theme.accent));
            lines.push(label_value("Before", before, theme));
            lines.push(label_value("After", after, theme));
        }
    }
    if count == 0 {
        lines.push(Line::raw("Template defaults are unchanged."));
    }
}

fn render_confirmation(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let (action, lines) = confirmation_lines(model, theme);
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(
                &format!("Confirm {action}"),
                workspace_focused(model),
                theme,
            ))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn confirmation_lines(model: &TuiModel, theme: &Theme) -> (&'static str, Vec<Line<'static>>) {
    let Some(confirmation) = &model.agents.pending_confirmation else {
        return (
            "Apply",
            vec![Line::raw(
                "Profile confirmation is unavailable. Press Esc to return.",
            )],
        );
    };
    let (action, name, explanation) = match &confirmation.command {
        ApplicationCommand::CreateAgentProfile { draft, .. } => (
            "Create",
            safe_text(&draft.display_name),
            "Create this agent and make Version 1 current.",
        ),
        ApplicationCommand::ActivateAgentProfileVersion { .. } => (
            "Activate",
            model
                .agents
                .editor
                .as_ref()
                .map(|editor| safe_text(&editor.draft().display_name))
                .unwrap_or_else(|| "selected agent".to_owned()),
            "Make the reviewed changes the current profile version.",
        ),
        _ => {
            return (
                "Apply",
                vec![Line::raw(
                    "Profile confirmation is unavailable. Press Esc to return.",
                )],
            );
        }
    };
    (
        action,
        vec![
            Line::styled(format!("Confirm {action} · {name}"), theme.warning),
            Line::default(),
            Line::raw(explanation),
            Line::default(),
            Line::styled(
                format!("Enter: {}   Esc: return to review", action.to_lowercase()),
                theme.focus,
            ),
        ],
    )
}

pub(super) fn inspector_lines(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled("Readiness & history", theme.accent),
        Line::default(),
    ];
    let Some(detail) = &model.agents.detail else {
        lines.push(Line::raw(
            "Select a profile to inspect readiness and history.",
        ));
        return lines;
    };
    let profile = &detail.profile;
    lines.push(Line::styled(
        safe_text(profile.display_name()),
        theme.accent,
    ));
    lines.push(readiness_line("Readiness", detail.readiness, theme));
    lines.push(label_value(
        "Inference",
        inference_text(profile.bindings()),
        theme,
    ));
    lines.push(label_value(
        "Engineering",
        engineering_text(profile.bindings()),
        theme,
    ));
    lines.push(Line::default());
    append_provenance(&mut lines, profile.template_provenance(), theme);
    lines.push(Line::default());
    if let Some(history) = &model.agents.history {
        lines.push(Line::styled("HISTORY", theme.accent));
        lines.push(label_value(
            "Versions",
            history.versions.len().to_string(),
            theme,
        ));
        lines.push(label_value(
            "Active version",
            history.active_version_id.to_string(),
            theme,
        ));
        for entry in &history.versions {
            lines.push(Line::from(vec![
                Span::styled(
                    if entry.profile_version_id == history.active_version_id {
                        "> "
                    } else {
                        "  "
                    },
                    theme.focus,
                ),
                Span::styled(
                    format!("v{} ", entry.version.get()),
                    readiness_style(entry.readiness, theme),
                ),
                Span::raw(entry.created_at_ms.to_string()),
            ]));
        }
    } else {
        lines.push(Line::styled(
            "Press h to load version history.",
            theme.muted,
        ));
    }
    lines
}

fn append_provenance(
    lines: &mut Vec<Line<'static>>,
    provenance: Option<&crate::agents::ProfileTemplateProvenance>,
    theme: &Theme,
) {
    lines.push(Line::styled("Template provenance", theme.accent));
    if let Some(provenance) = provenance {
        lines.push(label_value(
            "Template",
            provenance.template_id.to_string(),
            theme,
        ));
        lines.push(label_value(
            "Template ver.",
            provenance.template_version.get().to_string(),
            theme,
        ));
        lines.push(label_value(
            "Template hash",
            provenance.template_digest.to_string(),
            theme,
        ));
    } else {
        lines.push(label_value("Template", "Custom / none".to_owned(), theme));
    }
}

fn readiness_line(label: &'static str, readiness: AgentReadiness, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<14}"), theme.muted),
        Span::styled(readiness_name(readiness), readiness_style(readiness, theme)),
    ])
}

fn readiness_name(readiness: AgentReadiness) -> &'static str {
    match readiness {
        AgentReadiness::Unbound => "Needs connection",
        AgentReadiness::BindingUnavailable => "Connection unavailable",
        AgentReadiness::Ready => "Bindings configured",
    }
}

fn readiness_style(readiness: AgentReadiness, theme: &Theme) -> ratatui::style::Style {
    match readiness {
        AgentReadiness::Ready => theme.success,
        AgentReadiness::Unbound | AgentReadiness::BindingUnavailable => theme.warning,
    }
}

fn bindings_value(bindings: &AgentBindings) -> String {
    format!(
        "{} / {}",
        inference_text(bindings),
        engineering_text(bindings)
    )
}

fn inference_text(bindings: &AgentBindings) -> String {
    bindings.inference.as_ref().map_or_else(
        || "Not configured".to_owned(),
        |_| "Inference configured".to_owned(),
    )
}

fn engineering_text(bindings: &AgentBindings) -> String {
    bindings.engineering.as_ref().map_or_else(
        || "Not configured".to_owned(),
        |_| "Engineering configured".to_owned(),
    )
}

fn field_value(value: &ProfileFieldValue) -> String {
    match value {
        ProfileFieldValue::Text(value) => safe_text(value),
        ProfileFieldValue::Role(role) => role.as_str().to_owned(),
        ProfileFieldValue::SpecialtyTags(tags) => safe_text(&tags.join(", ")),
        ProfileFieldValue::Bindings(bindings) => bindings_value(bindings),
    }
}

fn diff_name(field: ProfileDiffField) -> &'static str {
    match field {
        ProfileDiffField::DisplayName => "Display name",
        ProfileDiffField::Description => "Description",
        ProfileDiffField::Role => "Role",
        ProfileDiffField::PrimarySpecialty => "Primary specialty",
        ProfileDiffField::SpecialtyTags => "Specialty tags",
        ProfileDiffField::Personality => "Personality",
        ProfileDiffField::Instructions => "Instructions",
        ProfileDiffField::Bindings => "Bindings",
        ProfileDiffField::SkillRefsAdded => "Skill references added",
        ProfileDiffField::SkillRefsUpgraded => "Skill references upgraded",
        ProfileDiffField::SkillRefsRemoved => "Skill references removed",
    }
}

fn editor_message(code: &str) -> &'static str {
    match code {
        "invalid_profile_field" => {
            "Validation: this field is invalid; revise it before continuing."
        }
        "preview_required" => "Validation: request an authoritative review before activation.",
        "stale_preview" => "Validation: that preview is stale; request another review.",
        "preview_mismatch" => "Validation: preview did not match this edit.",
        "review_unavailable" => "Validation: review is available only on the final step.",
        "activation_unavailable" => "Validation: activation is available only after review.",
        "specialty_tag_limit" => "Validation: no more specialty tags can be added.",
        "unknown_specialty_tag" => "Validation: that specialty tag is not present.",
        "editor_first_step" => "This is the first editor step.",
        "editor_last_step" => "This is the final editor step.",
        "preview_generation_exhausted" => "Validation: no further previews can be requested.",
        "preview_unavailable" => "Validation: preview is unavailable for this editor mode.",
        "template_selection_create_only" => {
            "Validation: complete templates can be selected only while creating a profile."
        }
        "editor_field_unavailable" => "Validation: text entry is unavailable on this step.",
        "unknown_editor_control" => "Validation: unknown editor control.",
        _ => "Validation: review the current field and try again.",
    }
}

fn scroll(value: usize, area_height: u16) -> u16 {
    let bounded = value.min(10_000);
    u16::try_from(bounded)
        .unwrap_or(10_000)
        .min(u16::MAX.saturating_sub(area_height))
}

fn monogram(name: &str) -> String {
    safe_text(name)
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

fn readable_date(milliseconds: i64) -> String {
    let days = milliseconds.div_euclid(86_400_000) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} UTC")
}

fn field_guidance(field: crate::ui::profile_editor::ProfileTuiField) -> String {
    use crate::ui::profile_editor::ProfileTuiField;
    match field {
        ProfileTuiField::Template => "Template · WASD choose a starting profile".to_owned(),
        ProfileTuiField::Role => "Role · WASD choose agent role".to_owned(),
        ProfileTuiField::DisplayName => {
            format!("Display name · Up to {DISPLAY_NAME_MAX_BYTES} UTF-8 bytes")
        }
        ProfileTuiField::Description => {
            format!("Description · Up to {DESCRIPTION_MAX_BYTES} UTF-8 bytes")
        }
        ProfileTuiField::PrimarySpecialty => {
            format!("Specialty · Up to {PRIMARY_SPECIALTY_MAX_BYTES} UTF-8 bytes")
        }
        ProfileTuiField::Tags => format!(
            "Tags · At most {MAX_SPECIALTY_TAGS}, comma-separated; {SPECIALTY_TAG_MAX_BYTES} UTF-8 bytes each"
        ),
        ProfileTuiField::Personality => {
            format!("Personality · Up to {PERSONALITY_MAX_BYTES} UTF-8 bytes")
        }
        ProfileTuiField::Instructions => {
            format!("Instructions · Up to {INSTRUCTIONS_MAX_BYTES} UTF-8 bytes")
        }
        ProfileTuiField::Bindings => {
            "Bindings · Read-only; configure connections separately".to_owned()
        }
        ProfileTuiField::Review => "Review · Inspect changes before confirming".to_owned(),
        ProfileTuiField::Discard => "Discard · Enter discards this draft".to_owned(),
    }
}

fn field_label(field: crate::ui::profile_editor::ProfileTuiField) -> &'static str {
    use crate::ui::profile_editor::ProfileTuiField;
    match field {
        ProfileTuiField::Template => "Template",
        ProfileTuiField::DisplayName => "Display name",
        ProfileTuiField::Role => "Role",
        ProfileTuiField::Description => "Description",
        ProfileTuiField::PrimarySpecialty => "Primary specialty",
        ProfileTuiField::Tags => "Tags",
        ProfileTuiField::Personality => "Personality",
        ProfileTuiField::Instructions => "Instructions",
        ProfileTuiField::Bindings => "Bindings",
        ProfileTuiField::Review => "Review changes",
        ProfileTuiField::Discard => "Discard draft",
    }
}

pub(super) fn editor_scroll_limit(model: &TuiModel) -> usize {
    let active = active_area(model);
    let lines = model
        .agents
        .editor
        .as_ref()
        .map(|editor| editor_lines(editor, &Theme::from_no_color(true), true))
        .unwrap_or_default();
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .line_count(active.width.saturating_sub(2))
        .saturating_sub(usize::from(active.height.saturating_sub(5)))
}
