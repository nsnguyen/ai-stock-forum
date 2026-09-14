use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use super::{
    append_error_guidance, editor_field_name, panel, provenance_short, safe_text,
    validation_message, workspace_focused,
};
use crate::{
    app::SkillView,
    ui::{
        skill_editor::{SkillEditor, SkillEditorField, SkillEditorMode},
        tui::{
            model::{
                Focus, InputMode, SkillDetailAction, SkillEditorPage, SkillSection, SkillsPane,
                TuiModel,
            },
            theme::Theme,
        },
    },
};

pub(super) fn columns_for(area: Rect) -> usize {
    usize::from(area.width >= 54 && area.height >= 20) + 1
}

fn surface(selected: bool, focused: bool, theme: &Theme) -> Block<'static> {
    Block::default()
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
        })
}

fn color(index: usize, theme: &Theme) -> Style {
    if theme.base.fg.is_none() {
        return theme.accent;
    }
    Style::default()
        .fg([
            Color::Cyan,
            Color::Rgb(173, 155, 227),
            Color::Rgb(234, 176, 112),
            Color::Rgb(125, 198, 164),
        ][index % 4])
        .add_modifier(Modifier::BOLD)
}

fn monogram(name: &str) -> String {
    safe_text(name)
        .split_whitespace()
        .take(2)
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn card(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    summary: &str,
    icon: &str,
    index: usize,
    selected: bool,
    focused: bool,
    theme: &Theme,
) {
    let block = surface(selected, focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let icon_width = if inner.width >= 24 && inner.height >= 2 && !icon.is_empty() {
        6
    } else {
        0
    };
    if icon_width > 0 {
        frame.render_widget(
            Paragraph::new(icon).style(color(index, theme)),
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
        format!("{}{}", if selected { "> " } else { "  " }, safe_text(title)),
        if selected && focused {
            theme.focus
        } else if selected {
            theme.accent
        } else {
            theme.base
        },
    )];
    if text.height > 1 {
        lines.push(Line::styled(safe_text(summary), theme.muted));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text);
}

fn window(selected: usize, count: usize, visible: usize) -> usize {
    selected
        .saturating_add(1)
        .saturating_sub(visible)
        .min(count.saturating_sub(visible))
}

pub(super) fn library(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    theme: &Theme,
    source: bool,
) {
    let focused = if source {
        model.focus == Focus::List
    } else {
        model.focus == Focus::List
            || (workspace_focused(model) && model.skills.pane == SkillsPane::List)
    };
    let block = panel(
        if source {
            "STARTING POINT"
        } else {
            "Skill library"
        },
        focused,
        theme,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let footer_height = if !source && inner.height >= 12 { 3 } else { 2 }.min(inner.height);
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(footer_height),
    );
    let skills = &model.skills.library.skills;
    if skills.is_empty() && !source {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    if model.command_in_flight {
                        "Loading skill library"
                    } else {
                        "No skills saved"
                    },
                    theme.accent,
                ),
                Line::raw("N New skill"),
                Line::raw("Saved guidance only · not executable"),
            ])
            .wrap(Wrap { trim: false }),
            body,
        );
    } else {
        let selected = if source {
            model.skills.selected_create_source
        } else {
            model.skills.selected_skill
        };
        let count = skills.len() + usize::from(source);
        let card_height = if source && body.height >= 12 {
            4
        } else if body.height >= 12 {
            5
        } else {
            3
        }
        .min(body.height.max(1));
        let visible = usize::from(body.height / card_height).max(1);
        let first = window(selected, count, visible);
        for (slot, index) in (first..count.min(first + visible)).enumerate() {
            let rect = Rect::new(
                body.x,
                body.y + slot as u16 * card_height,
                body.width,
                card_height.min(body.height),
            );
            if source && index == 0 {
                library_card(
                    frame,
                    rect,
                    "Blank skill",
                    "Start from scratch",
                    "+",
                    0,
                    selected == index,
                    focused,
                    theme,
                );
            } else {
                let skill = &skills[index - usize::from(source)];
                let title = format!(
                    "{}{}",
                    if source { "Copy " } else { "" },
                    safe_text(&skill.display_name)
                );
                let summary = format!(
                    "{} · v{}",
                    provenance_short(&skill.provenance),
                    skill.skill_ref.version().get()
                );
                library_card(
                    frame,
                    rect,
                    &title,
                    &summary,
                    &monogram(&skill.display_name),
                    badge_index(skill.skill_ref.skill_id()),
                    selected == index,
                    focused,
                    theme,
                );
            }
        }
    }
    let footer = if source {
        vec![
            Line::styled("W/S browse · Enter: continue", theme.muted),
            Line::styled("Tab pane · Esc: library", theme.muted),
        ]
    } else {
        vec![
            Line::styled("N New skill", theme.accent),
            Line::styled(
                format!(
                    "{} skills · W/S · Enter open · Esc back",
                    model.skills.library.total_count
                ),
                theme.muted,
            ),
            Line::styled(
                if model.skills.library.truncated {
                    "More results available · Esc back"
                } else {
                    "Tab pane · Esc back"
                },
                theme.muted,
            ),
        ]
    };
    frame.render_widget(
        Paragraph::new(footer),
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(footer_height),
            inner.width,
            footer_height,
        ),
    );
}

#[allow(clippy::too_many_arguments)]
fn library_card(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    summary: &str,
    badge: &str,
    index: usize,
    selected: bool,
    focused: bool,
    theme: &Theme,
) {
    let block = surface(selected, focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let style = match color(index, theme).fg {
        Some(value) => Style::default()
            .fg(Color::Black)
            .bg(value)
            .add_modifier(Modifier::BOLD),
        None => theme.accent,
    };
    let badge_width = 4.min(inner.width);
    frame.render_widget(
        Paragraph::new(badge)
            .alignment(Alignment::Center)
            .style(style),
        Rect::new(inner.x, inner.y, badge_width, inner.height.min(2)),
    );
    let text = Rect::new(
        inner.x + badge_width + 1,
        inner.y,
        inner.width.saturating_sub(badge_width + 1),
        inner.height,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!("{}{}", if selected { "> " } else { "" }, title),
                if selected { theme.accent } else { theme.base },
            ),
            Line::styled(summary.to_owned(), theme.muted),
        ])
        .wrap(Wrap { trim: false }),
        text,
    );
}

pub(super) fn loaded_detail(model: &TuiModel) -> Option<&SkillView> {
    let detail = model
        .skills
        .version_detail
        .as_ref()
        .or(model.skills.detail.as_ref())?;
    if model.skills.pane == SkillsPane::List
        && model
            .skills
            .selected_summary()
            .is_some_and(|summary| summary.skill_ref != detail.skill_ref)
    {
        return None;
    }
    Some(detail)
}

pub(super) fn detail(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let focused = workspace_focused(model) && model.skills.pane == SkillsPane::Detail;
    let block = panel("SKILL HOME", focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let Some(detail) = loaded_detail(model) else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled("Choose a skill to open its Home", theme.accent),
                Line::raw("Select a library item and press Enter."),
                Line::raw("N New skill · Tab pane · Esc back"),
            ])
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };
    let active = model
        .skills
        .library
        .skills
        .iter()
        .find(|item| item.skill_ref.skill_id() == detail.skill_ref.skill_id());
    let status = match active {
        Some(item) if item.skill_ref == detail.skill_ref => "ACTIVE",
        Some(item) if item.skill_ref.version() > detail.skill_ref.version() => "HISTORICAL",
        _ => "UNKNOWN · active version not loaded",
    };
    let header_height = if inner.height >= 19 { 5 } else { 3 }.min(inner.height);
    let header = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", monogram(&detail.content.display_name)),
                badge_style(detail.skill_ref.skill_id(), theme),
            ),
            Span::styled(
                format!("  {}", safe_text(&detail.content.display_name)),
                theme.accent,
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!(
                    "{} · Version {} · ",
                    provenance_short(&detail.provenance),
                    detail.skill_ref.version().get()
                ),
                theme.muted,
            ),
            Span::styled(
                status,
                if status == "ACTIVE" {
                    theme.success
                } else {
                    theme.warning
                },
            ),
        ]),
        Line::raw(safe_text(&detail.content.description)),
    ];
    frame.render_widget(
        Paragraph::new(header).wrap(Wrap { trim: false }),
        Rect::new(inner.x, inner.y, inner.width, header_height),
    );
    let actions = model.skills.available_detail_actions();
    let horizontal = inner.width >= 68 && inner.height >= 16;
    let action_height =
        if horizontal { 7 } else { 3 }.min(inner.height.saturating_sub(header_height));
    let action_area = Rect::new(inner.x, inner.y + header_height, inner.width, action_height);
    if horizontal {
        let regions = Layout::horizontal(
            actions
                .iter()
                .map(|_| Constraint::Ratio(1, actions.len() as u32)),
        )
        .split(action_area);
        for (index, action) in actions.iter().enumerate() {
            let (title, caption, icon) = action_info(*action);
            card(
                frame,
                regions[index],
                title,
                caption,
                icon,
                index,
                *action == model.skills.selected_action(),
                focused,
                theme,
            );
        }
    } else {
        let (title, caption, icon) = action_info(model.skills.selected_action());
        card(
            frame,
            action_area,
            title,
            caption,
            icon,
            model.skills.selected_action_index,
            true,
            focused,
            theme,
        );
    }
    let footer_height = if inner.height >= 18 { 2 } else { 1 };
    let body = Rect::new(
        inner.x,
        action_area.bottom(),
        inner.width,
        inner
            .bottom()
            .saturating_sub(action_area.bottom() + footer_height),
    );
    let mut lines = detail_lines(model, detail, theme);
    append_error_guidance(&mut lines, model, theme);
    scroll_text(frame, body, lines, model.skills.content_scroll);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("A/D action · Enter: open · Esc: library", theme.muted),
            Line::styled(
                "W/S scroll · I technical details · Saved guidance only",
                theme.muted,
            ),
        ]),
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(footer_height),
            inner.width,
            footer_height,
        ),
    );
}

fn action_info(action: SkillDetailAction) -> (&'static str, &'static str, &'static str) {
    match action {
        SkillDetailAction::Assign => (
            "Assign to agent",
            "Choose who uses this skill.",
            "(o) +\n /|\\",
        ),
        SkillDetailAction::CreateVersion => (
            "Edit skill",
            "Make changes. Save a new version.",
            "[ /]\n[__]",
        ),
        SkillDetailAction::History => (
            "Version history",
            "See earlier versions and their guidance.",
            ".--.\n|< |",
        ),
    }
}

fn detail_lines(model: &TuiModel, detail: &SkillView, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::default(),
        Line::styled("WHEN TO USE", theme.accent),
        Line::raw(safe_text(&detail.content.use_when)),
        Line::default(),
        Line::styled("GUIDANCE AT A GLANCE", theme.accent),
        Line::raw(safe_text(&detail.content.instructions)),
        Line::default(),
        Line::styled(safe_text(&detail.content.tags.join(" · ")), color(2, theme)),
        Line::default(),
        Line::styled("REFERENCE NOTES", theme.accent),
    ];
    if detail.content.resources.is_empty() {
        lines.push(Line::styled("No reference notes", theme.muted));
    }
    for note in &detail.content.resources {
        lines.push(Line::styled(safe_text(&note.name), theme.accent));
        lines.push(Line::raw(safe_text(&note.body)));
    }
    lines.extend([
        Line::default(),
        Line::styled(
            "Saved guidance for agents · not an executable tool.",
            theme.muted,
        ),
    ]);
    if model.skills.technical_details {
        lines.extend([
            Line::default(),
            Line::styled("TECHNICAL DETAILS", theme.warning),
        ]);
        super::append_exact_ref(&mut lines, "Exact version", &detail.skill_ref, theme);
        lines.push(super::label_value(
            "Provenance",
            super::provenance_long(&detail.provenance),
            theme,
        ));
        lines.push(super::label_value(
            "Created ms",
            detail.created_at_ms.to_string(),
            theme,
        ));
    }
    lines
}

pub(super) fn content_height(model: &TuiModel, width: u16) -> u16 {
    loaded_detail(model)
        .map(|detail| {
            Paragraph::new(detail_lines(model, detail, &Theme::from_no_color(true)))
                .wrap(Wrap { trim: false })
                .line_count(width.max(1))
                .min(usize::from(u16::MAX)) as u16
        })
        .unwrap_or(0)
}

pub(super) fn scroll_text(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: Vec<Line<'static>>,
    scroll: u16,
) {
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let max_scroll = paragraph
        .line_count(area.width.max(1))
        .saturating_sub(usize::from(area.height));
    frame.render_widget(
        paragraph.scroll((scroll.min(max_scroll.min(usize::from(u16::MAX)) as u16), 0)),
        area,
    );
}

pub(super) fn source_preview(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let block = panel("STARTER PREVIEW", workspace_focused(model), theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let source = model
        .skills
        .selected_create_source
        .checked_sub(1)
        .and_then(|index| model.skills.library.skills.get(index));
    let mut lines = if let Some(source) = source {
        vec![
            Line::from(vec![
                Span::styled(
                    format!(" {} ", monogram(&source.display_name)),
                    badge_style(source.skill_ref.skill_id(), theme),
                ),
                Span::styled(
                    format!("  Start with {}", safe_text(&source.display_name)),
                    theme.accent,
                ),
            ]),
            Line::styled("Copy its guidance into your own custom skill.", theme.muted),
            Line::default(),
        ]
    } else {
        vec![
            Line::styled("Blank skill", theme.accent),
            Line::raw("Start from scratch."),
            Line::raw("Give it a name, purpose, usage, and instructions."),
            Line::default(),
        ]
    };
    if let Some(source) = source {
        if let Some(detail) = model
            .skills
            .create_source_detail
            .as_ref()
            .filter(|detail| detail.skill_ref == source.skill_ref)
        {
            lines.extend([
                Line::styled("PURPOSE", theme.accent),
                Line::raw(safe_text(&detail.content.description)),
                Line::default(),
                Line::styled("WHEN TO USE", theme.accent),
                Line::raw(safe_text(&detail.content.use_when)),
                Line::default(),
                Line::styled("INSTRUCTIONS PREVIEW", theme.accent),
                Line::raw(safe_text(&detail.content.instructions)),
            ]);
        } else {
            lines.push(Line::styled(
                "Loading selected starting point...",
                theme.muted,
            ));
        }
    }
    let footer_height = if inner.height >= 12 { 4 } else { 2 }.min(inner.height);
    scroll_text(
        frame,
        Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(footer_height),
        ),
        lines,
        model.skills.content_scroll,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("Enter: continue · Use this starting point", theme.muted),
            Line::styled("W/S browse · Esc: library · Tab pane", theme.muted),
            Line::styled(
                "The original stays unchanged. Rename and edit your copy next.",
                theme.muted,
            ),
            Line::styled(
                "Choosing a starting point does not save a skill.",
                theme.muted,
            ),
        ])
        .wrap(Wrap { trim: false }),
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(footer_height),
            inner.width,
            footer_height,
        ),
    );
}

pub(super) fn editor(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let Some(editor) = model.skills.editor.as_ref() else {
        return;
    };
    if model.skills.editor_page == SkillEditorPage::Home {
        editor_home(frame, area, model, editor, theme);
        return;
    }
    editor_section(frame, area, model, editor, theme);
}

fn editor_home(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    editor: &SkillEditor,
    theme: &Theme,
) {
    let block = panel(
        if matches!(editor.mode(), SkillEditorMode::Create) {
            "NEW SKILL"
        } else {
            "EDIT SKILL"
        },
        workspace_focused(model),
        theme,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let header_height = if inner.height >= 16 { 4 } else { 2 }.min(inner.height);
    let name = if editor.raw_display_name().is_empty() {
        "Untitled skill".to_owned()
    } else {
        safe_text(editor.raw_display_name())
    };
    let mut header = vec![
        Line::styled(format!("Editing / {name}"), theme.accent),
        Line::styled("DRAFT · NOT SAVED", theme.warning),
        Line::styled("Pick a section. Change only what you need.", theme.muted),
    ];
    if let Some(error) = editor.local_error() {
        header[1] = Line::styled(
            format!(
                "Validation: {}",
                validation_message(error.code(), error.field())
            ),
            theme.error,
        );
    }
    if model
        .message
        .as_ref()
        .is_some_and(|message| message.severity == crate::ui::tui::model::Severity::Error)
    {
        header[2] = Line::styled(
            "Review current state, then retry the visible action.",
            theme.error,
        );
    }
    frame.render_widget(
        Paragraph::new(header).wrap(Wrap { trim: false }),
        Rect::new(inner.x, inner.y, inner.width, header_height),
    );
    let footer_height = if inner.height >= 16 { 2 } else { 1 };
    let cards = Rect::new(
        inner.x,
        inner.y + header_height,
        inner.width,
        inner.height.saturating_sub(header_height + footer_height),
    );
    let titles = [
        "Basics",
        "When to use",
        "Instructions",
        "Reference notes",
        "Review changes",
        "Discard draft",
    ];
    let summaries = [
        format!(
            "{}\n{}",
            safe_text(editor.tui_field_text(SkillEditorField::DisplayName)),
            safe_text(editor.tui_field_text(SkillEditorField::Purpose))
        ),
        format!(
            "{}\n{}",
            safe_text(editor.tui_field_text(SkillEditorField::UseWhen)),
            safe_text(editor.tui_field_text(SkillEditorField::Tags))
        ),
        safe_text(editor.tui_field_text(SkillEditorField::Instructions)),
        if editor.references().is_empty() {
            "No notes yet · optional\nAdd supporting text.".to_owned()
        } else {
            format!("{} notes · supporting text", editor.references().len())
        },
        "Inspect before confirming".to_owned(),
        "Leave without saving".to_owned(),
    ];
    let icons = [
        "[abc]\n[---]",
        "  +\n-(o)-",
        "[===]\n[---]",
        "[+ ]\n[__]",
        "",
        "",
    ];
    let selected = model.skills.editor_home_selection.min(5);
    if columns_for(area) == 2 {
        let main_height = cards.height.saturating_sub(3);
        for index in 0..6 {
            let column = index % 2;
            let start = cards.width * column as u16 / 2;
            let end = cards.width * (column + 1) as u16 / 2;
            let (y, height) = if index < 4 {
                (
                    main_height * (index / 2) as u16 / 2,
                    main_height * ((index / 2) + 1) as u16 / 2
                        - main_height * (index / 2) as u16 / 2,
                )
            } else {
                (main_height, 3)
            };
            card(
                frame,
                Rect::new(
                    cards.x + start,
                    cards.y + y,
                    end - start - u16::from(column == 0),
                    height,
                ),
                titles[index],
                &summaries[index],
                icons[index],
                index,
                selected == index,
                workspace_focused(model),
                theme,
            );
        }
    } else {
        let height = 3.min(cards.height.max(1));
        let visible = usize::from(cards.height / height).max(1);
        let first = window(selected, 6, visible);
        for (slot, index) in (first..6.min(first + visible)).enumerate() {
            card(
                frame,
                Rect::new(cards.x, cards.y + slot as u16 * height, cards.width, height),
                titles[index],
                &summaries[index],
                icons[index],
                index,
                selected == index,
                workspace_focused(model),
                theme,
            );
        }
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                if model.message.as_ref().is_some_and(|message| {
                    message.severity == crate::ui::tui::model::Severity::Error
                }) {
                    "Review current state, then retry the visible action."
                } else {
                    "Review, then confirm. Nothing is saved while you browse."
                },
                theme.muted,
            ),
            Line::styled(
                "Saving creates a new version. Existing assignments stay unchanged.",
                theme.muted,
            ),
        ])
        .wrap(Wrap { trim: false }),
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(footer_height),
            inner.width,
            footer_height,
        ),
    );
}

fn section_title(page: SkillEditorPage) -> &'static str {
    match page {
        SkillEditorPage::Section(SkillSection::Basics) => "BASICS",
        SkillEditorPage::Section(SkillSection::Usage) => "WHEN TO USE",
        SkillEditorPage::Section(SkillSection::Instructions) => "INSTRUCTIONS",
        SkillEditorPage::Section(SkillSection::References) => "REFERENCE NOTES",
        SkillEditorPage::ReferenceEdit => "EDIT REFERENCE NOTE",
        SkillEditorPage::ReferenceRemove => "REMOVE REFERENCE NOTE",
        SkillEditorPage::Discard => "DISCARD DRAFT",
        SkillEditorPage::Review => "REVIEW CHANGES",
        SkillEditorPage::Home => "EDIT SKILL",
    }
}

fn editor_section(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    editor: &SkillEditor,
    theme: &Theme,
) {
    let block = panel(
        section_title(model.skills.editor_page),
        workspace_focused(model),
        theme,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.is_empty() {
        return;
    }
    let (lines, footer) = section_lines(model, editor, theme, inner);
    let footer_height = if inner.width < 65 { 2 } else { 1 };
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(footer_height),
    );
    let selected_line = lines
        .iter()
        .position(|line| line.to_string().starts_with("> "));
    let selection_scroll = selected_line
        .map(|index| {
            let preceding = Paragraph::new(lines[..index].to_vec())
                .wrap(Wrap { trim: false })
                .line_count(body.width.max(1));
            preceding.saturating_sub(usize::from(body.height.saturating_sub(2))) as u16
        })
        .unwrap_or(0);
    scroll_text(
        frame,
        body,
        lines,
        model.skills.content_scroll.max(selection_scroll),
    );
    frame.render_widget(
        Paragraph::new(Line::styled(footer, theme.muted)).wrap(Wrap { trim: false }),
        Rect::new(
            inner.x,
            inner.bottom().saturating_sub(footer_height),
            inner.width,
            footer_height,
        ),
    );
}

fn field_lines(
    lines: &mut Vec<Line<'static>>,
    model: &TuiModel,
    editor: &SkillEditor,
    field: SkillEditorField,
    selected: bool,
    width: u16,
    theme: &Theme,
) {
    lines.push(Line::default());
    lines.push(Line::styled(
        format!(
            "{}{}",
            if selected { "> " } else { "  " },
            editor_field_name(field)
        ),
        if selected { theme.focus } else { theme.accent },
    ));
    let value = if selected && model.input_mode == InputMode::Type {
        let input = &model.skills.field_input;
        let cursor = input.cursor_byte().min(input.text().len());
        let prefix = visible_input(&input.text()[..cursor]);
        let suffix = visible_input(&input.text()[cursor..]);
        let available = usize::from(width.saturating_sub(2)).max(1);
        let mut visible = String::new();
        for character in prefix.chars().rev() {
            if Line::raw(format!("{character}{visible}")).width() > available {
                break;
            }
            visible.insert(0, character);
        }
        format!("{visible}|{suffix}")
    } else {
        safe_text(editor.tui_field_text(field))
    };
    lines.push(Line::styled(
        if value.is_empty() {
            "Not set · Enter edit".to_owned()
        } else {
            value
        },
        theme.muted,
    ));
}

fn section_lines(
    model: &TuiModel,
    editor: &SkillEditor,
    theme: &Theme,
    inner: Rect,
) -> (Vec<Line<'static>>, &'static str) {
    let mut lines = vec![Line::styled(
        format!("{} · DRAFT NOT SAVED", safe_text(editor.raw_display_name())),
        theme.accent,
    )];
    if let Some(error) = editor.local_error() {
        lines.push(Line::styled(
            format!(
                "Validation: {}",
                validation_message(error.code(), error.field())
            ),
            theme.error,
        ));
    }
    let mut footer = "Tab pane · W/S select · Enter edit · Esc keep draft";
    match model.skills.editor_page {
        SkillEditorPage::Discard => {
            lines.extend([
                Line::styled("Discard all changes to this draft?", theme.warning),
                Line::raw("Saved skills and existing assignments remain unchanged."),
            ]);
            footer = "Enter discard · Esc keep editing";
        }
        SkillEditorPage::ReferenceRemove => {
            if let Some(note) = editor.references().get(model.skills.reference_selection) {
                lines.push(Line::raw(format!(
                    "Remove {} from this draft?",
                    safe_text(&note.name)
                )));
            }
            footer = "Enter remove · Esc keep note";
        }
        SkillEditorPage::Section(SkillSection::References) => {
            if editor.has_pending_reference() {
                lines.push(Line::styled(
                    "Resume unfinished note · Enter",
                    theme.warning,
                ));
            }
            let visible = usize::from(inner.height.saturating_sub(lines.len() as u16 + 3)).max(1);
            let count = editor.references().len() + 1;
            let first = window(model.skills.reference_selection, count, visible);
            for index in first..count.min(first + visible) {
                let title = editor
                    .references()
                    .get(index)
                    .map(|note| safe_text(&note.name))
                    .unwrap_or_else(|| "Add reference note".to_owned());
                lines.push(Line::styled(
                    format!(
                        "{}{}",
                        if index == model.skills.reference_selection {
                            "> "
                        } else {
                            "  "
                        },
                        title
                    ),
                    if index == model.skills.reference_selection {
                        theme.focus
                    } else {
                        theme.base
                    },
                ));
            }
            footer = "W/S note · Enter open · N add · X remove · Esc back";
        }
        SkillEditorPage::ReferenceEdit => {
            for (index, field) in [
                SkillEditorField::ReferenceName,
                SkillEditorField::ReferenceBody,
            ]
            .iter()
            .enumerate()
            {
                field_lines(
                    &mut lines,
                    model,
                    editor,
                    *field,
                    index == model.skills.reference_action,
                    inner.width,
                    theme,
                );
            }
            lines.push(Line::styled(
                format!(
                    "{}Save note",
                    if model.skills.reference_action == 2 {
                        "> "
                    } else {
                        "  "
                    }
                ),
                if model.skills.reference_action == 2 {
                    theme.focus
                } else {
                    theme.accent
                },
            ));
        }
        SkillEditorPage::Section(section) => {
            let fields: &[SkillEditorField] = match section {
                SkillSection::Basics => &[SkillEditorField::DisplayName, SkillEditorField::Purpose],
                SkillSection::Usage => &[SkillEditorField::UseWhen, SkillEditorField::Tags],
                SkillSection::Instructions => &[SkillEditorField::Instructions],
                SkillSection::References => &[],
            };
            for field in fields {
                field_lines(
                    &mut lines,
                    model,
                    editor,
                    *field,
                    editor.field() == *field,
                    inner.width,
                    theme,
                );
            }
        }
        _ => {}
    }
    if model.input_mode == InputMode::Type {
        footer = "Type value · Enter accept · Esc keep value";
    }

    (lines, footer)
}

pub(super) fn section_scroll_limit(model: &TuiModel, area: Rect) -> u16 {
    let Some(editor) = model.skills.editor.as_ref() else {
        return 0;
    };
    let inner = panel("", false, &Theme::from_no_color(true)).inner(area);
    let (lines, _) = section_lines(model, editor, &Theme::from_no_color(true), inner);
    let height = inner
        .height
        .saturating_sub(if inner.width < 65 { 2 } else { 1 });
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .line_count(inner.width.max(1))
        .saturating_sub(usize::from(height))
        .min(usize::from(u16::MAX)) as u16
}

fn visible_input(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\n' => '↵',
            '\t' => '⇥',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect()
}

fn badge_index(id: crate::domain::SkillId) -> usize {
    id.as_uuid().as_bytes().iter().fold(0usize, |value, byte| {
        value.wrapping_mul(31).wrapping_add(usize::from(*byte))
    }) % 4
}

fn badge_style(id: crate::domain::SkillId, theme: &Theme) -> Style {
    match color(badge_index(id), theme).fg {
        Some(value) => Style::default()
            .fg(Color::Black)
            .bg(value)
            .add_modifier(Modifier::BOLD),
        None => theme.accent,
    }
}

pub(super) fn detail_scroll_limit(model: &TuiModel, area: Rect) -> u16 {
    let inner = panel("", false, &Theme::from_no_color(true)).inner(area);
    let header = if inner.height >= 19 { 5 } else { 3 };
    let actions = if inner.width >= 68 && inner.height >= 16 {
        7
    } else {
        3
    };
    let footer = if inner.height >= 18 { 2 } else { 1 };
    content_height(model, inner.width)
        .saturating_sub(inner.height.saturating_sub(header + actions + footer))
}
