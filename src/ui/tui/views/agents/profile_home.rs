use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::Line,
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::{
    agents::{AgentRole, ProfileTemplate, builtin_profile_templates},
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorMode, ProfileTuiField},
        tui::{
            model::{Focus, InputMode, ProfileSection, TuiModel},
            theme::Theme,
        },
    },
};

use super::{
    append_create_diffs, append_diffs, bindings_value, editor_message, field_guidance, field_label,
    panel, safe_text, workspace_focused,
};

const HOME_ITEMS: [HomeItem; 6] = [
    HomeItem {
        title: "Identity",
        caption: "Name · role · description",
        icon: &[" (o) ", " /|\\ ", " / \\ "],
    },
    HomeItem {
        title: "Focus",
        caption: "Specialty · tags",
        icon: &["  +  ", "-(o)-", "     "],
    },
    HomeItem {
        title: "Personality",
        caption: "How it communicates",
        icon: &[".---.", "| :) ", "'---'"],
    },
    HomeItem {
        title: "Instructions",
        caption: "What it should do",
        icon: &["[===]", "[---]", "     "],
    },
    HomeItem {
        title: "Review changes",
        caption: "Inspect before confirming",
        icon: &[" .-. ", "[OK] ", " '-' "],
    },
    HomeItem {
        title: "Discard draft",
        caption: "Leave without applying",
        icon: &["  x  ", " / \\ ", "     "],
    },
];

struct HomeItem {
    title: &'static str,
    caption: &'static str,
    icon: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct HomeCardState {
    compact_action: bool,
    selected: bool,
    focused: bool,
}

pub(super) fn columns_for(area: Rect) -> usize {
    usize::from(area.width >= 54 && area.height >= 20) + 1
}

pub(super) fn render_template_list(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    theme: &Theme,
) {
    let focused = model.focus == Focus::List;
    let block = panel("NEW AGENT / TEMPLATES", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let templates = builtin_profile_templates();
    let selected = model
        .agents
        .selected_template
        .min(templates.len().saturating_sub(1));
    let card_height = if inner.height >= 13 { 4 } else { 3 };
    let visible = usize::from(inner.height / card_height.max(1)).max(1);
    let first = selected
        .saturating_add(1)
        .saturating_sub(visible)
        .min(templates.len().saturating_sub(visible));

    for (slot, (index, template)) in templates
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
        .enumerate()
    {
        let y = inner.y + u16::try_from(slot).unwrap_or(u16::MAX) * card_height;
        let height = card_height.min(inner.bottom().saturating_sub(y));
        render_template_card(
            frame,
            Rect::new(inner.x, y, inner.width, height),
            template,
            index == selected,
            focused,
            theme,
        );
    }
}

fn render_template_card(
    frame: &mut Frame<'_>,
    area: Rect,
    template: &ProfileTemplate,
    selected: bool,
    focused: bool,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let block = Block::default()
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
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let badge_width = 4.min(inner.width);
    let badge = Rect::new(inner.x, inner.y, badge_width, inner.height.min(2));
    frame.render_widget(
        Block::default().style(if selected {
            theme.focus
        } else {
            role_style(template.role, theme)
        }),
        badge,
    );
    frame.render_widget(
        Paragraph::new(role_monogram(template.role))
            .alignment(Alignment::Center)
            .style(if selected {
                theme.focus
            } else {
                role_style(template.role, theme)
            }),
        Rect::new(badge.x, badge.y, badge.width, 1),
    );
    let text = Rect::new(
        inner.x.saturating_add(badge_width + 1),
        inner.y,
        inner.width.saturating_sub(badge_width + 1),
        inner.height,
    );
    let mut lines = vec![Line::styled(
        safe_text(template.suggested_name),
        if selected { theme.accent } else { theme.base },
    )];
    if text.height > 1 {
        lines.push(Line::styled(safe_text(template.description), theme.muted));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text);
}

pub(super) fn render_template_preview(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    theme: &Theme,
) {
    let focused = workspace_focused(model);
    let block = panel("TEMPLATE PREVIEW", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let templates = builtin_profile_templates();
    let Some(template) = templates.get(
        model
            .agents
            .selected_template
            .min(templates.len().saturating_sub(1)),
    ) else {
        return;
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let compact = inner.height < 15;
    let mut lines = vec![
        Line::styled(safe_text(template.suggested_name), theme.accent),
        Line::styled(safe_text(template.instructions), theme.muted),
    ];
    if !compact {
        lines.extend([
            Line::default(),
            Line::styled(
                "New agents only · does not change existing agents.",
                theme.muted,
            ),
            Line::default(),
            super::label_value("Role", template.role.as_str().to_owned(), theme),
            super::label_value("Focus", safe_text(template.primary_specialty), theme),
            super::label_value(
                "Tags",
                safe_text(&template.specialty_tags.join(", ")),
                theme,
            ),
            super::label_value("Personality", safe_text(template.personality), theme),
        ]);
    }
    if !compact && inner.width >= 54 {
        let artwork = match template.role {
            AgentRole::Bear => super::Artwork::Bear,
            AgentRole::Bull => super::Artwork::Bull,
            _ => super::Artwork::Bot,
        };
        super::render_artwork(
            frame,
            Rect::new(inner.right().saturating_sub(15), inner.y, 14, 5),
            artwork,
            theme.accent,
        );
    }
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect::new(
            inner.x + 1,
            inner.y,
            inner.width.saturating_sub(2),
            inner.height.saturating_sub(if compact { 3 } else { 5 }),
        ),
    );

    let action_y = inner.bottom().saturating_sub(if compact { 2 } else { 4 });
    frame.render_widget(
        Paragraph::new(Line::styled(" Use this template ", theme.focus))
            .alignment(Alignment::Right),
        Rect::new(inner.x, action_y, inner.width, 1),
    );
    if !compact {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("All profile fields can be customized next.", theme.muted),
                Line::styled(
                    "No agent is created until you review and confirm.",
                    theme.muted,
                ),
            ]),
            Rect::new(inner.x + 1, action_y + 2, inner.width.saturating_sub(2), 2),
        );
    }
}

pub(super) fn render_home(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let focused = workspace_focused(model);
    let block = panel("PROFILE HOME", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(editor) = model.agents.editor.as_ref() else {
        return;
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let selected = model
        .agents
        .profile_home_selection
        .min(HOME_ITEMS.len() - 1);
    let header_height = if inner.height >= 12 { 4 } else { 2 };
    let draft_heading = format!(
        "{} / {} · DRAFT NOT APPLIED",
        editor_mode(editor),
        safe_text(&editor.draft().display_name)
    );
    frame.render_widget(
        Paragraph::new(if header_height > 1 {
            vec![
                Line::styled(draft_heading, theme.accent),
                Line::styled("Make this agent yours.", theme.base),
                Line::styled("Pick a section. Change only what you need.", theme.muted),
            ]
        } else {
            vec![Line::styled("Make this agent yours.", theme.base)]
        }),
        Rect::new(inner.x, inner.y, inner.width, header_height),
    );

    let footer_height = if inner.height >= 13 { 2 } else { 1 };
    let cards = Rect::new(
        inner.x,
        inner.y + header_height,
        inner.width,
        inner.height.saturating_sub(header_height + footer_height),
    );
    render_home_cards(
        frame,
        cards,
        editor,
        selected,
        focused,
        columns_for(area),
        theme,
    );

    let source = template_source(editor);
    let footer_y = inner.bottom().saturating_sub(footer_height);
    let footer = if footer_height > 1 {
        vec![
            Line::styled("Changes stay in this draft until confirmed.", theme.muted),
            Line::styled(
                format!("Template used: {source} · reference only"),
                theme.muted,
            ),
        ]
    } else {
        vec![Line::styled(
            format!("Template used: {source} · reference only"),
            theme.muted,
        )]
    };
    frame.render_widget(
        Paragraph::new(footer).wrap(Wrap { trim: false }),
        Rect::new(inner.x, footer_y, inner.width, footer_height),
    );
}

fn render_home_cards(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &ProfileEditor,
    selected: usize,
    focused: bool,
    columns: usize,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if columns == 2 {
        render_wide_home_cards(frame, area, editor, selected, focused, theme);
        return;
    }
    let rows = HOME_ITEMS.len();
    let row_height = 4.min(area.height.max(1));
    let visible_rows = usize::from(area.height / row_height.max(1)).max(1);
    let selected_row = selected / columns;
    let first_row = selected_row
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(rows.saturating_sub(visible_rows));
    for row in first_row..rows.min(first_row + visible_rows) {
        for column in 0..columns {
            let index = row * columns + column;
            let Some(item) = HOME_ITEMS.get(index) else {
                continue;
            };
            let start = area.width * u16::try_from(column).unwrap_or(0)
                / u16::try_from(columns).unwrap_or(1);
            let end = area.width * u16::try_from(column + 1).unwrap_or(1)
                / u16::try_from(columns).unwrap_or(1);
            let y = area.y + u16::try_from(row - first_row).unwrap_or(u16::MAX) * row_height;
            render_home_card(
                frame,
                Rect::new(
                    area.x + start,
                    y,
                    end.saturating_sub(start)
                        .saturating_sub(u16::from(column + 1 < columns)),
                    row_height.min(area.bottom().saturating_sub(y)),
                ),
                item,
                home_summary(editor, index),
                HomeCardState {
                    compact_action: false,
                    selected: index == selected,
                    focused,
                },
                theme,
            );
        }
    }
}

fn render_wide_home_cards(
    frame: &mut Frame<'_>,
    area: Rect,
    editor: &ProfileEditor,
    selected: usize,
    focused: bool,
    theme: &Theme,
) {
    let action_height = 3.min(area.height);
    let main_height = area.height.saturating_sub(action_height);
    for (index, item) in HOME_ITEMS.iter().enumerate().take(4) {
        let row = index / 2;
        let column = index % 2;
        let start_x = area.width * u16::try_from(column).unwrap_or(0) / 2;
        let end_x = area.width * u16::try_from(column + 1).unwrap_or(1) / 2;
        let start_y = main_height * u16::try_from(row).unwrap_or(0) / 2;
        let end_y = main_height * u16::try_from(row + 1).unwrap_or(1) / 2;
        render_home_card(
            frame,
            Rect::new(
                area.x + start_x,
                area.y + start_y,
                end_x
                    .saturating_sub(start_x)
                    .saturating_sub(u16::from(column == 0)),
                end_y.saturating_sub(start_y),
            ),
            item,
            home_summary(editor, index),
            HomeCardState {
                compact_action: false,
                selected: index == selected,
                focused,
            },
            theme,
        );
    }
    for (index, item) in HOME_ITEMS.iter().enumerate().skip(4) {
        let column = index - 4;
        let start_x = area.width * u16::try_from(column).unwrap_or(0) / 2;
        let end_x = area.width * u16::try_from(column + 1).unwrap_or(1) / 2;
        render_home_card(
            frame,
            Rect::new(
                area.x + start_x,
                area.y + main_height,
                end_x
                    .saturating_sub(start_x)
                    .saturating_sub(u16::from(column == 0)),
                action_height,
            ),
            item,
            home_summary(editor, index),
            HomeCardState {
                compact_action: true,
                selected: index == selected,
                focused,
            },
            theme,
        );
    }
}

fn render_home_card(
    frame: &mut Frame<'_>,
    area: Rect,
    item: &HomeItem,
    summary: String,
    state: HomeCardState,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(if state.selected {
            theme.selected_surface()
        } else {
            theme.surface()
        })
        .border_style(if state.selected && state.focused {
            theme.accent
        } else {
            theme.border()
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let icon_width = if !state.compact_action && inner.width >= 18 {
        6
    } else {
        0
    };
    if icon_width > 0 {
        frame.render_widget(
            Paragraph::new(
                item.icon
                    .iter()
                    .take(usize::from(inner.height))
                    .map(|line| Line::styled(*line, theme.accent))
                    .collect::<Vec<_>>(),
            ),
            Rect::new(inner.x, inner.y, icon_width, inner.height),
        );
    }
    let text = Rect::new(
        inner.x + icon_width,
        inner.y,
        inner.width.saturating_sub(icon_width),
        inner.height,
    );
    let mut lines = vec![Line::styled(
        format!("{}{}", if state.selected { "> " } else { "  " }, item.title),
        if state.selected && state.focused {
            theme.focus
        } else if state.selected {
            theme.accent
        } else {
            theme.base
        },
    )];
    if text.height > 1 {
        lines.push(Line::styled(summary, theme.muted));
    }
    if text.height > 2 {
        lines.push(Line::styled(item.caption, theme.muted));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text);
}

pub(super) fn render_section(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    section: ProfileSection,
    theme: &Theme,
) {
    let focused = workspace_focused(model);
    let title = section.label().to_uppercase();
    let block = panel(&title, focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(editor) = model.agents.editor.as_ref() else {
        return;
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let header_height = u16::from(inner.height >= 8);
    if header_height > 0 {
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!(
                    "{} / {} · DRAFT NOT APPLIED",
                    section.label(),
                    safe_text(&editor.draft().display_name)
                ),
                theme.accent,
            )),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
    }
    let footer_height = if inner.height >= 11 { 2 } else { 1 };
    let body = Rect::new(
        inner.x,
        inner.y + header_height,
        inner.width,
        inner.height.saturating_sub(header_height + footer_height),
    );
    render_section_fields(frame, body, model, editor, section, theme);

    let source = template_source(editor);
    let footer_y = inner.bottom().saturating_sub(footer_height);
    let lines = if footer_height > 1 {
        vec![
            Line::styled(
                format!("Template used: {source} · reference only"),
                theme.muted,
            ),
            Line::styled(
                format!(
                    "Bindings: {} · read-only",
                    bindings_value(&editor.draft().bindings)
                ),
                theme.muted,
            ),
        ]
    } else {
        vec![Line::styled(
            format!("Template used: {source} · reference only"),
            theme.muted,
        )]
    };
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        Rect::new(inner.x, footer_y, inner.width, footer_height),
    );
}

fn render_section_fields(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    editor: &ProfileEditor,
    section: ProfileSection,
    theme: &Theme,
) {
    let fields = section.fields();
    let selected = fields
        .iter()
        .position(|field| *field == editor.tui_field())
        .unwrap_or(0);
    let selected_invalid = editor
        .tui_field_error(fields.get(selected).copied().unwrap_or(editor.tui_field()))
        .is_some();
    let card_height = if selected_invalid { 5 } else { 4 }.min(area.height.max(1));
    let visible = usize::from(area.height / card_height.max(1)).max(1);
    let first = selected
        .saturating_add(1)
        .saturating_sub(visible)
        .min(fields.len().saturating_sub(visible));
    for (slot, field) in fields.iter().copied().skip(first).take(visible).enumerate() {
        let y = area.y + u16::try_from(slot).unwrap_or(u16::MAX) * card_height;
        render_field_card(
            frame,
            Rect::new(
                area.x,
                y,
                area.width,
                card_height.min(area.bottom().saturating_sub(y)),
            ),
            model,
            editor,
            field,
            theme,
        );
    }
}

fn render_field_card(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    editor: &ProfileEditor,
    field: ProfileTuiField,
    theme: &Theme,
) {
    let selected = field == editor.tui_field();
    let focused = workspace_focused(model);
    let invalid = editor.tui_field_error(field).is_some();
    let block = Block::default()
        .title(format!(
            " {}{} ",
            if selected { "> " } else { "" },
            field_label(field)
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(if selected {
            theme.selected_surface()
        } else {
            theme.surface()
        })
        .border_style(if selected && invalid {
            theme.warning
        } else if selected && focused {
            theme.accent
        } else {
            theme.border()
        });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let value = if selected && focused && model.input_mode == InputMode::Type {
        input_with_cursor(model, inner.width)
    } else {
        safe_text(editor.tui_field_text(field))
    };
    let display_value = if value.trim().is_empty() {
        "Not set".to_owned()
    } else {
        value
    };
    let mut lines = if invalid {
        vec![Line::styled(
            format!("{}: invalid; revise before review", field_label(field)),
            theme.warning,
        )]
    } else {
        vec![Line::styled(
            display_value.clone(),
            if selected { theme.base } else { theme.muted },
        )]
    };
    if inner.height > 1 {
        let hint = if invalid {
            format!("Input: {display_value}")
        } else if field == ProfileTuiField::Role && model.agents.profile_role_selecting {
            "W/S choose role · Enter accept".to_owned()
        } else if field == ProfileTuiField::Role {
            "Role · Enter choose role".to_owned()
        } else {
            field_guidance(field)
        };
        lines.push(Line::styled(hint, theme.muted));
    }
    if invalid && inner.height > 2 {
        lines.push(Line::styled(
            "Please revise this field before reviewing.",
            theme.warning,
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn input_with_cursor(model: &TuiModel, width: u16) -> String {
    let input = &model.agents.field_input;
    let cursor = input.cursor_byte().min(input.text().len());
    let prefix = safe_text(&input.text()[..cursor]);
    let tail = safe_text(&input.text()[cursor..]);
    let capacity = usize::from(width.saturating_sub(1));
    let visible_prefix = prefix
        .chars()
        .rev()
        .take(capacity)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{visible_prefix}│{tail}")
}

pub(super) fn render_discard(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let name = model
        .agents
        .editor
        .as_ref()
        .map(|editor| safe_text(&editor.draft().display_name))
        .unwrap_or_else(|| "this profile".to_owned());
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("Discard this draft?", theme.warning),
            Line::default(),
            Line::raw(format!("No changes to {name} have been applied.")),
            Line::default(),
            Line::styled("Enter discard", theme.focus),
            Line::styled("Esc Home", theme.muted),
        ])
        .block(panel("DISCARD DRAFT", workspace_focused(model), theme))
        .wrap(Wrap { trim: false }),
        area,
    );
}

pub(super) fn review_lines(
    editor: &ProfileEditor,
    theme: &Theme,
    focused: bool,
) -> Vec<Line<'static>> {
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
        Line::styled("Review changes", theme.accent),
        Line::default(),
    ];
    if let Some(review) = editor.review() {
        append_diffs(&mut lines, &review.preview().diffs, theme);
    } else if let Some(baseline) = editor.create_baseline() {
        append_create_diffs(&mut lines, baseline, editor.draft(), theme);
    } else {
        lines.push(Line::raw("Enter requests an authoritative review."));
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        if focused {
            "Enter continues to a separate confirmation · Esc Home"
        } else {
            "Draft retained · Return focus to review or confirm"
        },
        if focused { theme.focus } else { theme.muted },
    ));
    if let Some(message) = editor.local_message() {
        lines.push(Line::styled(editor_message(message.code()), theme.warning));
    }
    lines
}

fn template_source(editor: &ProfileEditor) -> String {
    builtin_profile_templates()
        .iter()
        .find(|template| template.id.as_str() == editor.tui_field_text(ProfileTuiField::Template))
        .map(|template| safe_text(template.suggested_name))
        .unwrap_or_else(|| "Current profile".to_owned())
}

fn editor_mode(editor: &ProfileEditor) -> &'static str {
    if matches!(editor.mode(), ProfileEditorMode::Create { .. }) {
        "New agent"
    } else {
        "Editing"
    }
}

fn home_summary(editor: &ProfileEditor, index: usize) -> String {
    let draft = editor.draft();
    match index {
        0 => format!(
            "{} · {}",
            draft.role.as_str(),
            safe_text(&draft.description)
        ),
        1 => {
            let tags = safe_text(&draft.specialty_tags.join(", "));
            if tags.is_empty() {
                safe_text(&draft.primary_specialty)
            } else {
                format!("{} · {tags}", safe_text(&draft.primary_specialty))
            }
        }
        2 => safe_text(&draft.personality),
        3 => safe_text(&draft.instructions),
        4 => "Draft not applied".to_owned(),
        _ => "No changes applied".to_owned(),
    }
}

fn role_monogram(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Bull => "BU",
        AgentRole::Bear => "BE",
        AgentRole::Chief => "CH",
        AgentRole::Engineering => "EN",
        AgentRole::Custom => "CU",
    }
}

fn role_style(role: AgentRole, theme: &Theme) -> Style {
    let _ = role;
    theme.accent
}
