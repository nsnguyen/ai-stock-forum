use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    app::ApplicationCommand,
    skills::{SkillProvenance, SkillVersionRef},
    ui::{
        skill_editor::{SkillEditor, SkillEditorField, SkillEditorMode, SkillEditorStep},
        tui::{
            layout::{skill_layout_mode, skill_workspace},
            model::{AssignmentKind, Severity, SkillDetailAction, SkillsPane, TuiModel},
            theme::Theme,
        },
    },
};

use super::{label_value, panel, safe_text, workspace_focused};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mode = skill_layout_mode(frame.area());
    let layout = skill_workspace(area, mode);
    if let Some(list) = layout.list {
        render_library(frame, list, model, theme);
    }
    render_active(frame, layout.active, model, theme, layout.list.is_none());
}

pub(super) fn content_height(_model: &TuiModel, _width: u16) -> u16 {
    0
}

fn render_active(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    theme: &Theme,
    one_pane: bool,
) {
    match model.skills.pane {
        SkillsPane::List if one_pane => render_library(frame, area, model, theme),
        SkillsPane::List | SkillsPane::Detail => render_detail(frame, area, model, theme),
        SkillsPane::CreateSource => render_create_source(frame, area, model, theme),
        SkillsPane::History => render_history(frame, area, model, theme),
        SkillsPane::Editor => render_editor(frame, area, model, theme),
        SkillsPane::AgentPicker => render_agent_picker(frame, area, model, theme),
        SkillsPane::AssignmentReview => render_assignment_review(frame, area, model, theme),
        SkillsPane::Confirmation => render_confirmation(frame, area, model, theme),
        SkillsPane::Result => render_result(frame, area, model, theme),
    }
}

fn render_lines(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    focused: bool,
    theme: &Theme,
) {
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(title, focused, theme))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_fixed_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    header: Vec<Line<'static>>,
    body: Vec<Line<'static>>,
    footer: Vec<Line<'static>>,
    focused: bool,
    theme: &Theme,
) {
    let block = panel(title, focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let header_height = u16::try_from(header.len()).unwrap_or(u16::MAX).min(inner.height);
    let footer = Paragraph::new(footer).wrap(Wrap { trim: false });
    let footer_height = u16::try_from(footer.line_count(inner.width.max(1)))
        .unwrap_or(u16::MAX)
        .min(inner.height.saturating_sub(header_height));
    let regions = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(0),
        Constraint::Length(footer_height),
    ])
    .split(inner);
    frame.render_widget(Paragraph::new(header), regions[0]);
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), regions[1]);
    frame.render_widget(footer, regions[2]);
}

fn render_library(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let focused = workspace_focused(model) && model.skills.pane == SkillsPane::List;
    let mut lines = vec![Line::styled(
        "Up/Down: select | Enter: open | c Create | Esc: back",
        theme.focus,
    )];
    if model.skills.library.skills.is_empty() {
        lines.push(Line::default());
        if model.command_in_flight {
            lines.extend([
                Line::styled("Loading skill library", theme.accent),
                Line::styled("The local catalog is being read.", theme.muted),
            ]);
        } else {
            lines.extend([
                Line::styled("No skills saved", theme.accent),
                Line::raw("c Create first skill"),
                Line::default(),
                Line::raw("saved guidance only"),
                Line::raw("not executable"),
            ]);
        }
    } else {
        lines.push(Line::from(vec![
            Span::styled("LIBRARY  ", theme.accent),
            Span::raw(format!(
                "{} shown / {} total",
                model.skills.library.returned_count, model.skills.library.total_count
            )),
        ]));
        if model.skills.library.truncated {
            lines.push(Line::styled("Results are truncated.", theme.warning));
        }
        for (index, skill) in model.skills.library.skills.iter().enumerate() {
            let selected = index == model.skills.selected_skill;
            lines.push(Line::styled(
                format!(
                    "{} {}",
                    if selected { ">" } else { " " },
                    safe_text(&skill.display_name)
                ),
                if selected { theme.focus } else { theme.accent },
            ));
            lines.push(Line::from(vec![
                Span::styled("  Active version ", theme.muted),
                Span::raw(format!("v{}", skill.skill_ref.version().get())),
                Span::styled(format!(" | {}", provenance_short(&skill.provenance)), theme.muted),
            ]));
            if model
                .skills
                .detail
                .as_ref()
                .is_some_and(|detail| detail.skill_ref.skill_id() == skill.skill_ref.skill_id())
            {
                let purpose = model
                    .skills
                    .detail
                    .as_ref()
                    .map(|detail| safe_text(&detail.content.description))
                    .unwrap_or_default();
                lines.push(Line::styled(format!("  Purpose: {purpose}"), theme.muted));
            }
        }
    }
    render_lines(frame, area, "Skill library", lines, focused, theme);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mut lines = vec![action_selector(model, theme), Line::styled(
        "Left/Right: choose action | Enter: open | Esc: library",
        theme.focus,
    )];
    if model.skills.version_detail.is_some() {
        lines.push(Line::styled(
            "Historical detail is read-only; assignment and history remain available.",
            theme.warning,
        ));
    }
    let detail = model
        .skills
        .version_detail
        .as_ref()
        .or(model.skills.detail.as_ref());
    let Some(detail) = detail else {
        lines.extend([
            Line::default(),
            Line::styled("No skill detail loaded", theme.accent),
            Line::raw("Select a library item and press Enter to load its exact active version."),
        ]);
        append_error_guidance(&mut lines, model, theme);
        render_lines(
            frame,
            area,
            "Skill detail",
            lines,
            workspace_focused(model) && model.skills.pane == SkillsPane::Detail,
            theme,
        );
        return;
    };

    let active_ref = model
        .skills
        .library
        .skills
        .iter()
        .find(|summary| summary.skill_ref.skill_id() == detail.skill_ref.skill_id())
        .map(|summary| &summary.skill_ref);
    let (status, status_style) = match active_ref {
        Some(active) if active == &detail.skill_ref => ("ACTIVE", theme.success),
        Some(active) if active.version() > detail.skill_ref.version() => {
            ("HISTORICAL", theme.warning)
        }
        Some(_) => ("UNKNOWN - loaded active identity is inconsistent", theme.warning),
        None => ("UNKNOWN - active version not loaded", theme.warning),
    };
    lines.extend([
        Line::default(),
        Line::styled(safe_text(&detail.content.display_name), theme.accent),
        Line::from(vec![
            Span::styled(format!("{:<14}", "Status"), theme.muted),
            Span::styled(status, status_style),
        ]),
        label_value(
            "Exact version",
            format!("v{}", detail.skill_ref.version().get()),
            theme,
        ),
        label_value(
            "Active version",
            active_ref
                .map(|active| format!("v{} / {}", active.version().get(), active.skill_version_id()))
                .unwrap_or_else(|| "Not loaded".to_owned()),
            theme,
        ),
        label_value("Provenance", provenance_long(&detail.provenance), theme),
        label_value("Skill ID", detail.skill_ref.skill_id().to_string(), theme),
        label_value(
            "Version ID",
            detail.skill_ref.skill_version_id().to_string(),
            theme,
        ),
        Line::styled("Content digest", theme.muted),
        label_value(
            "SHA-256",
            detail.skill_ref.content_digest().to_string(),
            theme,
        ),
        label_value("Created ms", detail.created_at_ms.to_string(), theme),
        label_value(
            "Predecessor",
            detail
                .predecessor_version_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "None".to_owned()),
            theme,
        ),
        Line::default(),
        Line::styled("ACCEPTED GUIDANCE", theme.accent),
        label_value("Purpose", safe_text(&detail.content.description), theme),
        label_value("Use when", safe_text(&detail.content.use_when), theme),
        label_value("Tags", safe_text(&detail.content.tags.join(", ")), theme),
        Line::default(),
        Line::styled("INERT INSTRUCTIONS", theme.accent),
        Line::styled("Text guidance only; it grants no capability.", theme.muted),
        Line::styled("Long content may be truncated by the visible pane.", theme.muted),
        Line::raw(safe_text(&detail.content.instructions)),
        Line::default(),
        Line::styled("INERT REFERENCE NOTES", theme.accent),
        Line::styled("Reference text only; names and bodies are not executable.", theme.muted),
    ]);
    if detail.content.resources.is_empty() {
        lines.push(Line::raw("None"));
    } else {
        for resource in &detail.content.resources {
            lines.push(Line::styled(safe_text(&resource.name), theme.accent));
            lines.push(Line::raw(safe_text(&resource.body)));
        }
    }
    append_assignment_context(&mut lines, model, &detail.skill_ref, theme);
    append_error_guidance(&mut lines, model, theme);
    render_lines(
        frame,
        area,
        "Skill detail",
        lines,
        workspace_focused(model) && model.skills.pane == SkillsPane::Detail,
        theme,
    );
}

fn action_selector(model: &TuiModel, theme: &Theme) -> Line<'static> {
    let selected = model.skills.selected_action();
    let mut spans = vec![Span::styled("Actions  ", theme.accent)];
    for (index, action) in model.skills.available_detail_actions().iter().copied().enumerate() {
        let label = match action {
            SkillDetailAction::Assign => "Assign",
            SkillDetailAction::CreateVersion => "Create Version",
            SkillDetailAction::History => "History",
        };
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!("[{label}]"),
            if action == selected { theme.focus } else { theme.muted },
        ));
    }
    Line::from(spans)
}

fn render_create_source(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mut lines = vec![
        Line::styled("Choose a starting point", theme.accent),
        Line::styled("Up/Down: select | Enter: continue", theme.focus),
        Line::styled("Esc: library", theme.focus),
        Line::default(),
        Line::styled(
            format!(
                "{} Blank skill",
                if model.skills.selected_create_source == 0 { ">" } else { " " }
            ),
            if model.skills.selected_create_source == 0 { theme.focus } else { theme.accent },
        ),
        Line::styled("  Start with empty saved guidance.", theme.muted),
    ];
    for (index, skill) in model.skills.library.skills.iter().enumerate() {
        let selected = model.skills.selected_create_source == index + 1;
        lines.push(Line::styled(
            format!(
                "{} Copy {}",
                if selected { ">" } else { " " },
                safe_text(&skill.display_name)
            ),
            if selected { theme.focus } else { theme.accent },
        ));
        lines.push(Line::styled(
            format!(
                "  v{} | {} | copied as a new custom skill",
                skill.skill_ref.version().get(),
                provenance_short(&skill.provenance)
            ),
            theme.muted,
        ));
    }
    render_lines(frame, area, "Create skill", lines, workspace_focused(model), theme);
}

fn render_history(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mut lines = vec![Line::styled(
        "Up/Down: select | Enter: open exact version | Esc: detail",
        theme.focus,
    )];
    let Some(history) = model.skills.history.as_ref() else {
        lines.extend([
            Line::default(),
            Line::styled("Loading skill history...", theme.accent),
            Line::raw("Esc returns to the current detail."),
        ]);
        render_lines(frame, area, "Skill history", lines, workspace_focused(model), theme);
        return;
    };
    lines.extend([
        label_value("Skill ID", history.skill_id.to_string(), theme),
        label_value("Versions", history.total_count.to_string(), theme),
        label_value("Returned", history.returned_count.to_string(), theme),
    ]);
    if history.truncated {
        lines.push(Line::styled("History results are truncated.", theme.warning));
    }
    for (index, entry) in history.versions.iter().enumerate() {
        let selected = index == model.skills.selected_history_version;
        let active = entry.skill_ref.skill_version_id() == history.active_version_id;
        lines.push(Line::styled(
            format!(
                "{} v{}  {}",
                if selected { ">" } else { " " },
                entry.skill_ref.version().get(),
                if active { "ACTIVE" } else { "HISTORICAL" }
            ),
            if selected {
                theme.focus
            } else if active {
                theme.success
            } else {
                theme.muted
            },
        ));
        lines.push(label_value(
            "Version ID",
            entry.skill_ref.skill_version_id().to_string(),
            theme,
        ));
        lines.push(label_value(
            "Digest",
            entry.skill_ref.content_digest().to_string(),
            theme,
        ));
    }
    render_lines(frame, area, "Skill history", lines, workspace_focused(model), theme);
}

fn render_editor(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let Some(editor) = model.skills.editor.as_ref() else {
        render_lines(
            frame,
            area,
            "Skill editor",
            vec![Line::styled(
                "Editor unavailable. Press Esc to return to detail.",
                theme.warning,
            )],
            workspace_focused(model),
            theme,
        );
        return;
    };
    let mut lines = vec![
        Line::styled(editor_title(editor), theme.accent),
        label_value(
            "Progress",
            format!("Step {} of 5", editor_step_number(editor.step())),
            theme,
        ),
        label_value("Focused pane", "Editor".to_owned(), theme),
        label_value("Current stage", editor_step_name(editor.step()).to_owned(), theme),
        label_value("Current field", editor_field_name(editor.field()).to_owned(), theme),
        Line::styled(editor_guidance(editor), theme.focus),
    ];
    if editor.step() == SkillEditorStep::References {
        lines.extend([Line::default(), Line::styled("ACCEPTED INERT NOTES", theme.accent)]);
        if editor.references().is_empty() {
            lines.push(Line::styled("None yet", theme.muted));
        } else {
            for (index, reference) in editor.references().iter().enumerate() {
                let selected = editor.selected_reference() == Some(index);
                lines.push(Line::styled(
                    format!(
                        "{} {}",
                        if selected { ">" } else { " " },
                        safe_text(&reference.name)
                    ),
                    if selected { theme.focus } else { theme.accent },
                ));
                lines.push(Line::styled(
                    format!("  {}", safe_text(&reference.body)),
                    theme.muted,
                ));
            }
        }
    }
    if let Some(error) = editor.local_error() {
        lines.extend([
            Line::default(),
            Line::styled("Validation", theme.error),
            Line::raw(validation_message(error.code(), error.field())),
        ]);
    }
    if editor.step() == SkillEditorStep::Review {
        lines.extend([
            Line::default(),
            Line::styled("REVIEW CANDIDATE", theme.warning),
            label_value("Operation", editor_operation(editor).to_owned(), theme),
            label_value("Exact version", editor_target_version(model, editor), theme),
            label_value("Provenance", editor_provenance(model, editor), theme),
            label_value(
                "Review risk",
                "Creates one immutable version; assigned agents do not auto-upgrade.".to_owned(),
                theme,
            ),
        ]);
        if let Some(review) = editor.review() {
            lines.push(label_value(
                "Candidate digest",
                review.preview().candidate_digest.to_string(),
                theme,
            ));
            lines.push(label_value(
                "Review digest",
                review.preview().review_digest.to_string(),
                theme,
            ));
        } else {
            lines.push(Line::styled(
                "Authoritative validation review is required before confirmation.",
                theme.warning,
            ));
        }
    }
    if let Ok(draft) = editor.try_draft() {
        lines.extend([
            Line::default(),
            Line::styled("CURRENT SAVED-GUIDANCE CANDIDATE", theme.accent),
            label_value("Display name", safe_text(&draft.display_name), theme),
            label_value("Purpose", safe_text(&draft.description), theme),
            label_value("Use when", safe_text(&draft.use_when), theme),
            label_value("Tags", safe_text(&draft.tags.join(", ")), theme),
            label_value("Instructions", safe_text(&draft.instructions), theme),
            label_value("Reference notes", draft.resources.len().to_string(), theme),
        ]);
    } else {
        lines.push(label_value(
            "Current value",
            safe_text(editor.current_value()),
            theme,
        ));
    }
    append_error_guidance(&mut lines, model, theme);
    render_lines(frame, area, "Skill editor", lines, workspace_focused(model), theme);
}

fn render_agent_picker(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mut lines = vec![
        Line::styled("Up/Down: select agent", theme.focus),
        Line::styled("Enter: review assignment", theme.focus),
        Line::styled("Esc: detail", theme.focus),
    ];
    if model.agents.profiles.profiles.is_empty() {
        lines.extend([
            Line::default(),
            Line::styled("No agents available", theme.warning),
            Line::raw("Create an agent profile, then return to Assign."),
        ]);
    } else {
        for (index, profile) in model.agents.profiles.profiles.iter().enumerate() {
            let selected = index == model.skills.selected_agent;
            lines.push(Line::styled(
                format!(
                    "{} {}",
                    if selected { ">" } else { " " },
                    safe_text(&profile.display_name)
                ),
                if selected { theme.focus } else { theme.accent },
            ));
            lines.push(Line::styled(
                format!("  Profile v{} | {}", profile.version.get(), profile.profile_id),
                theme.muted,
            ));
        }
    }
    render_lines(frame, area, "Agent picker", lines, workspace_focused(model), theme);
}

fn render_assignment_review(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let assignment = model.skills.assignment.as_ref();
    let mut header = vec![Line::from(vec![
        Span::styled("Operation  ", theme.muted),
        Span::styled(assignment_name(assignment), theme.warning),
    ])];
    let mut body = Vec::new();
    append_error_guidance(&mut body, model, theme);
    if let Some(agent) = model.skills.selected_agent_detail.as_ref() {
        header.push(Line::styled(
            format!("Agent  {}", safe_text(agent.profile.display_name())),
            theme.accent,
        ));
        body.push(label_value(
            "Agent",
            safe_text(agent.profile.display_name()),
            theme,
        ));
        body.push(label_value(
            "Agent version",
            format!("v{} / {}", agent.profile.version().get(), agent.profile.profile_version_id()),
            theme,
        ));
    } else {
        header.push(Line::styled("Agent  Loading exact profile", theme.warning));
    }
    if let Some(target) = model.skills.selected_skill_ref() {
        header.push(Line::raw(format!(
            "Target  v{}  {}  {}",
            target.version().get(),
            compact_identifier(&target.skill_version_id().to_string()),
            compact_identifier(target.content_digest().as_str())
        )));
        append_exact_ref(&mut body, "Target", target, theme);
    }
    match assignment {
        Some(AssignmentKind::Upgrade { expected }) => {
            append_exact_ref(&mut body, "Current pin", expected, theme)
        }
        Some(AssignmentKind::Unassign { expected }) => {
            append_exact_ref(&mut body, "Remove pin", expected, theme)
        }
        _ => {}
    }
    body.extend([
        Line::styled("No automatic upgrades.", theme.muted),
        label_value(
            "Review risk",
            "Only the displayed exact pin changes after confirmation.".to_owned(),
            theme,
        ),
    ]);
    if assignment == Some(&AssignmentKind::AlreadyAssigned) {
        body.insert(
            0,
            Line::styled("Choose another version or Esc to return.", theme.warning),
        );
    }
    let footer = if assignment == Some(&AssignmentKind::AlreadyAssigned) {
        vec![
            Line::styled("No operation: exact version already pinned", theme.warning),
            Line::styled("Esc: choose another version", theme.focus),
        ]
    } else {
        vec![
            Line::styled(
                format!("Enter: validate {}", assignment_verb(assignment)),
                theme.focus,
            ),
            Line::styled("Esc: agent picker", theme.focus),
        ]
    };
    render_fixed_panel(
        frame,
        area,
        "Assignment review",
        header,
        body,
        footer,
        workspace_focused(model),
        theme,
    );
}

fn render_confirmation(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let Some(confirmation) = model.skills.pending_confirmation.as_ref() else {
        render_lines(
            frame,
            area,
            "Skill confirmation",
            vec![Line::styled(
                "Confirmation unavailable. Press Esc to return and review again.",
                theme.warning,
            )],
            workspace_focused(model),
            theme,
        );
        return;
    };
    let (title, header, mut body) = confirmation_lines(model, &confirmation.command, theme);
    body.extend([
        label_value(
            "Review risk",
            "Commits the displayed immutable version or exact agent pin only.".to_owned(),
            theme,
        ),
        Line::styled("No automatic upgrades or executable capabilities.", theme.muted),
    ]);
    append_error_guidance(&mut body, model, theme);
    render_fixed_panel(
        frame,
        area,
        &format!("Confirm {title}"),
        header,
        body,
        vec![
            Line::styled("Enter: confirm", theme.focus),
            Line::styled("Esc: return", theme.focus),
        ],
        workspace_focused(model),
        theme,
    );
}

fn render_result(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mut lines = vec![
        Line::styled("Skill action completed", theme.success),
        Line::raw("The immutable result is saved. Agent pins change only when explicitly chosen."),
        Line::default(),
        Line::styled("Enter or Esc: return to skill detail", theme.focus),
    ];
    append_error_guidance(&mut lines, model, theme);
    render_lines(frame, area, "Skill result", lines, workspace_focused(model), theme);
}

pub(super) fn inspector_lines(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled("FOCUSED PANE", theme.accent),
        label_value("Pane", pane_name(model.skills.pane).to_owned(), theme),
        label_value("Next action", next_action(model).to_owned(), theme),
        Line::default(),
    ];
    if let Some(skill_ref) = model.skills.selected_skill_ref() {
        lines.push(Line::styled("EXACT SELECTION", theme.accent));
        append_exact_ref(&mut lines, "Selected", skill_ref, theme);
    } else {
        lines.push(Line::styled("No exact skill version selected.", theme.muted));
    }
    if let Some(detail) = model
        .skills
        .version_detail
        .as_ref()
        .or(model.skills.detail.as_ref())
    {
        lines.push(label_value(
            "Provenance",
            provenance_long(&detail.provenance),
            theme,
        ));
    }
    lines.extend([
        Line::default(),
        Line::styled("SAFETY", theme.accent),
        Line::raw("Skill content and reference notes are inert text."),
        Line::raw("Assignments pin exact versions. No automatic upgrades."),
    ]);
    append_error_guidance(&mut lines, model, theme);
    lines
}

fn append_assignment_context(
    lines: &mut Vec<Line<'static>>,
    model: &TuiModel,
    target: &SkillVersionRef,
    theme: &Theme,
) {
    lines.extend([Line::default(), Line::styled("AGENT ASSIGNMENT CONTEXT", theme.accent)]);
    let agent = model
        .skills
        .selected_agent_detail
        .as_ref()
        .or(model.agents.detail.as_ref());
    let Some(agent) = agent else {
        lines.push(Line::styled(
            "Open Assign to choose an agent and review its exact pinned version.",
            theme.muted,
        ));
        return;
    };
    let assigned = agent
        .profile
        .skill_refs()
        .iter()
        .find(|assigned| assigned.skill_id() == target.skill_id());
    lines.push(label_value("Agent", safe_text(agent.profile.display_name()), theme));
    match assigned {
        Some(assigned) if assigned == target => {
            lines.push(label_value("Assigned", "This exact version".to_owned(), theme))
        }
        Some(assigned) => {
            lines.push(label_value("Assigned", "Another exact version".to_owned(), theme));
            append_exact_ref(lines, "Pinned", assigned, theme);
        }
        None => lines.push(label_value("Assigned", "Not assigned".to_owned(), theme)),
    }
}

fn append_exact_ref(
    lines: &mut Vec<Line<'static>>,
    label: &'static str,
    reference: &SkillVersionRef,
    theme: &Theme,
) {
    lines.extend([
        label_value(label, format!("v{}", reference.version().get()), theme),
        label_value("Skill ID", reference.skill_id().to_string(), theme),
        label_value("Version ID", reference.skill_version_id().to_string(), theme),
        label_value("Digest", reference.content_digest().to_string(), theme),
    ]);
}

fn append_error_guidance(lines: &mut Vec<Line<'static>>, model: &TuiModel, theme: &Theme) {
    if model.message.as_ref().is_some_and(|message| message.severity == Severity::Error) {
        lines.extend([
            Line::default(),
            Line::styled("RECOVERY", theme.error),
            Line::raw("Review current state, then retry the visible action."),
        ]);
    }
}

fn confirmation_lines(
    model: &TuiModel,
    command: &ApplicationCommand,
    theme: &Theme,
) -> (&'static str, Vec<Line<'static>>, Vec<Line<'static>>) {
    match command {
        ApplicationCommand::CreateSkill {
            skill_id,
            candidate,
            review_digest,
            ..
        } => candidate_confirmation_lines(
            model,
            "creation",
            "Create",
            *skill_id,
            candidate,
            None,
            review_digest.to_string(),
            theme,
        ),
        ApplicationCommand::ActivateSkillVersion {
            skill_id,
            expected_active_version_id,
            candidate,
            review_digest,
            ..
        } => candidate_confirmation_lines(
            model,
            "new version",
            "Create Version",
            *skill_id,
            candidate,
            Some(*expected_active_version_id),
            review_digest.to_string(),
            theme,
        ),
        ApplicationCommand::AssignAgentSkill {
            profile_id,
            skill,
            review_digest,
            ..
        } => ref_confirmation_lines(
            model,
            "assignment",
            "Assign",
            *profile_id,
            skill,
            None,
            review_digest.to_string(),
            theme,
        ),
        ApplicationCommand::UpgradeAgentSkill {
            profile_id,
            expected,
            replacement,
            review_digest,
            ..
        } => ref_confirmation_lines(
            model,
            "upgrade",
            "Upgrade",
            *profile_id,
            replacement,
            Some(expected),
            review_digest.to_string(),
            theme,
        ),
        ApplicationCommand::UnassignAgentSkill {
            profile_id,
            expected,
            review_digest,
            ..
        } => ref_confirmation_lines(
            model,
            "unassignment",
            "Unassign",
            *profile_id,
            expected,
            Some(expected),
            review_digest.to_string(),
            theme,
        ),
        _ => (
            "skill action",
            vec![Line::styled("Operation  Unavailable", theme.warning)],
            vec![Line::raw("Return and rebuild the review.")],
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn candidate_confirmation_lines(
    model: &TuiModel,
    title: &'static str,
    operation: &'static str,
    skill_id: crate::domain::SkillId,
    candidate: &crate::skills::SkillDraft,
    expected_active_version_id: Option<crate::domain::SkillVersionId>,
    review_digest: String,
    theme: &Theme,
) -> (&'static str, Vec<Line<'static>>, Vec<Line<'static>>) {
    let candidate_digest = model
        .skills
        .editor
        .as_ref()
        .and_then(|editor| editor.review())
        .map(|review| review.preview())
        .filter(|preview| preview.skill_id == skill_id)
        .map(|preview| preview.candidate_digest.to_string());
    let digest_label = candidate_digest
        .as_deref()
        .map(compact_identifier)
        .unwrap_or_else(|| "Pending".to_owned());
    let header = vec![
        Line::styled(format!("Operation  {operation}"), theme.warning),
        Line::styled(
            format!("Candidate  {}", safe_text(&candidate.display_name)),
            theme.accent,
        ),
        Line::raw(format!(
            "Skill  {}  Digest  {}",
            compact_identifier(&skill_id.to_string()),
            digest_label
        )),
        Line::raw("Version Pending | Provenance Pending"),
    ];
    let mut body = vec![
        label_value("Candidate", safe_text(&candidate.display_name), theme),
        label_value("Skill ID", skill_id.to_string(), theme),
        label_value(
            "Candidate digest",
            candidate_digest.unwrap_or_else(|| "Pending validation".to_owned()),
            theme,
        ),
        label_value("Version", "Pending authoritative commit".to_owned(), theme),
        label_value("Provenance", "Pending authoritative commit".to_owned(), theme),
    ];
    if let Some(expected) = expected_active_version_id {
        body.push(label_value("Reviewed base", expected.to_string(), theme));
    }
    body.push(label_value("Review digest", review_digest, theme));
    (title, header, body)
}

#[allow(clippy::too_many_arguments)]
fn ref_confirmation_lines(
    model: &TuiModel,
    title: &'static str,
    operation: &'static str,
    profile_id: crate::domain::AgentProfileId,
    target: &SkillVersionRef,
    prior: Option<&SkillVersionRef>,
    review_digest: String,
    theme: &Theme,
) -> (&'static str, Vec<Line<'static>>, Vec<Line<'static>>) {
    let header = vec![
        Line::styled(format!("Operation  {operation}"), theme.warning),
        Line::raw(format!(
            "Exact  v{}  {}",
            target.version().get(),
            compact_identifier(&target.skill_version_id().to_string())
        )),
        Line::raw(format!(
            "Digest  {}",
            compact_identifier(target.content_digest().as_str())
        )),
    ];
    let mut body = vec![label_value("Agent ID", profile_id.to_string(), theme)];
    append_exact_ref(&mut body, "Exact version", target, theme);
    if let Some(prior) = prior {
        append_exact_ref(&mut body, "Current pin", prior, theme);
    }
    let provenance = model
        .skills
        .version_detail
        .as_ref()
        .or(model.skills.detail.as_ref())
        .filter(|detail| detail.skill_ref == *target)
        .map(|detail| provenance_long(&detail.provenance))
        .unwrap_or_else(|| "Not loaded for this exact version".to_owned());
    body.push(label_value("Provenance", provenance, theme));
    body.push(label_value("Review digest", review_digest, theme));
    (title, header, body)
}

fn provenance_short(provenance: &SkillProvenance) -> &'static str {
    match provenance {
        SkillProvenance::BuiltIn { .. } => "Built-in",
        SkillProvenance::User => "Custom",
    }
}

fn provenance_long(provenance: &SkillProvenance) -> String {
    match provenance {
        SkillProvenance::BuiltIn {
            manifest_id,
            manifest_version,
            manifest_digest,
        } => format!(
            "Built-in {}@{} / {}",
            safe_text(manifest_id),
            manifest_version,
            manifest_digest
        ),
        SkillProvenance::User => "Custom / user-created".to_owned(),
    }
}

fn editor_title(editor: &SkillEditor) -> &'static str {
    match editor.mode() {
        SkillEditorMode::Create => "Create skill",
        SkillEditorMode::Version { .. } => "Create immutable skill version",
    }
}

fn editor_operation(editor: &SkillEditor) -> &'static str {
    match editor.mode() {
        SkillEditorMode::Create => "Create skill",
        SkillEditorMode::Version { .. } => "Create version",
    }
}

fn editor_target_version(_model: &TuiModel, editor: &SkillEditor) -> String {
    match editor.mode() {
        SkillEditorMode::Create | SkillEditorMode::Version { .. } => "Pending".to_owned(),
    }
}

fn editor_provenance(model: &TuiModel, editor: &SkillEditor) -> String {
    match editor.mode() {
        SkillEditorMode::Create => "Custom / user-created".to_owned(),
        SkillEditorMode::Version { .. } => model
            .skills
            .detail
            .as_ref()
            .map(|detail| provenance_long(&detail.provenance))
            .unwrap_or_else(|| "Inherited from active version".to_owned()),
    }
}

fn editor_step_number(step: SkillEditorStep) -> u8 {
    match step {
        SkillEditorStep::Identity => 1,
        SkillEditorStep::Usage => 2,
        SkillEditorStep::Instructions => 3,
        SkillEditorStep::References => 4,
        SkillEditorStep::Review => 5,
    }
}

fn editor_step_name(step: SkillEditorStep) -> &'static str {
    match step {
        SkillEditorStep::Identity => "Identity",
        SkillEditorStep::Usage => "Usage",
        SkillEditorStep::Instructions => "Instructions",
        SkillEditorStep::References => "References",
        SkillEditorStep::Review => "Review",
    }
}

fn editor_field_name(field: SkillEditorField) -> &'static str {
    match field {
        SkillEditorField::DisplayName => "Display name",
        SkillEditorField::Purpose => "Purpose",
        SkillEditorField::UseWhen => "Use when",
        SkillEditorField::Tags => "Tags",
        SkillEditorField::Instructions => "Instructions",
        SkillEditorField::ReferenceName => "Reference note name",
        SkillEditorField::ReferenceBody => "Reference note body",
        SkillEditorField::Review => "Review",
    }
}

fn editor_guidance(editor: &SkillEditor) -> &'static str {
    match editor.field() {
        SkillEditorField::Review if editor.review().is_some() => {
            "Enter: continue to confirmation | Esc: back to references"
        }
        SkillEditorField::Review => "Enter: request validation review | Esc: back to references",
        SkillEditorField::DisplayName => "Type display name | Enter: purpose | Esc: cancel",
        SkillEditorField::ReferenceName => {
            "Type note name, or leave blank to review | Up/Down: select note | Enter: edit | Delete: remove | Esc: back"
        }
        _ => "Type the focused value | Enter: continue | Esc: back",
    }
}

fn validation_message(code: &str, field: SkillEditorField) -> String {
    if code.contains("stale") {
        return "Stale review or active version. Esc back, reload current state, and review again."
            .to_owned();
    }
    if code.contains("already_assigned") {
        return "Already assigned at this exact version. Choose another version or Esc back."
            .to_owned();
    }
    if code.contains("not_assigned") {
        return "Skill is not assigned. Review current state, then retry with a loaded current pin."
            .to_owned();
    }
    format!(
        "{} is invalid. Revise the focused value, then press Enter again.",
        editor_field_name(field)
    )
}

fn assignment_name(assignment: Option<&AssignmentKind>) -> &'static str {
    match assignment {
        Some(AssignmentKind::Add) => "Assign exact version",
        Some(AssignmentKind::Upgrade { .. }) => "Upgrade explicit exact version",
        Some(AssignmentKind::AlreadyAssigned) => "Already assigned",
        Some(AssignmentKind::Unassign { .. }) => "Unassign exact version",
        None => "Loading assignment state",
    }
}

fn pane_name(pane: SkillsPane) -> &'static str {
    match pane {
        SkillsPane::List => "Library",
        SkillsPane::CreateSource => "Create source",
        SkillsPane::Detail => "Detail",
        SkillsPane::History => "History",
        SkillsPane::Editor => "Editor",
        SkillsPane::AgentPicker => "Agent picker",
        SkillsPane::AssignmentReview => "Assignment review",
        SkillsPane::Confirmation => "Confirmation",
        SkillsPane::Result => "Result",
    }
}

fn next_action(model: &TuiModel) -> &'static str {
    match model.skills.pane {
        SkillsPane::List => "Up/Down select; Enter open; c Create; Esc back",
        SkillsPane::CreateSource => "Up/Down select; Enter continue; Esc library",
        SkillsPane::Detail => "Left/Right action; Enter open; Esc library",
        SkillsPane::History => "Up/Down select; Enter exact version; Esc detail",
        SkillsPane::Editor => "Type focused value; Enter continue; Esc back",
        SkillsPane::AgentPicker => "Up/Down agent; Enter review; Esc detail",
        SkillsPane::AssignmentReview
            if model.skills.assignment == Some(AssignmentKind::AlreadyAssigned) => {
                "Esc choose another version"
            }
        SkillsPane::AssignmentReview => "Enter validate; Esc agent picker",
        SkillsPane::Confirmation => "Enter confirm; Esc return to review",
        SkillsPane::Result => "Enter or Esc return to detail",
    }
}

fn assignment_verb(assignment: Option<&AssignmentKind>) -> &'static str {
    match assignment {
        Some(AssignmentKind::Add) => "assignment",
        Some(AssignmentKind::Upgrade { .. }) => "upgrade",
        Some(AssignmentKind::Unassign { .. }) => "unassignment",
        Some(AssignmentKind::AlreadyAssigned) | None => "operation",
    }
}

fn compact_identifier(value: &str) -> String {
    if value.len() <= 19 {
        return value.to_owned();
    }
    format!("{}..{}", &value[..8], &value[value.len() - 8..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{SkillDraft, SkillResource};

    #[test]
    fn review_regression_reference_editor_guidance_exposes_keyboard_controls() {
        let draft = SkillDraft {
            display_name: "Research".to_owned(),
            description: "Purpose".to_owned(),
            use_when: "Use when reviewing evidence".to_owned(),
            tags: vec!["evidence".to_owned()],
            instructions: "Review carefully".to_owned(),
            resources: vec![SkillResource {
                name: "Source".to_owned(),
                body: "Accepted note".to_owned(),
            }],
        };
        let mut editor = SkillEditor::for_create(Some(draft.clone()));
        for value in [
            draft.display_name.as_str(),
            draft.description.as_str(),
            draft.use_when.as_str(),
            "evidence",
            draft.instructions.as_str(),
        ] {
            editor.submit_keyboard_line(value);
        }

        let guidance = editor_guidance(&editor);
        assert!(guidance.contains("Up/Down: select note"));
        assert!(guidance.contains("Enter: edit"));
        assert!(guidance.contains("Delete: remove"));
    }
}
