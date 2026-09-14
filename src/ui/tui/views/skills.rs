mod home;

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
        skill_editor::{SkillEditor, SkillEditorField, SkillEditorMode},
        tui::{
            layout::{skill_layout_mode, skill_workspace},
            model::{AssignmentKind, Focus, Severity, SkillEditorPage, SkillsPane, TuiModel},
            theme::Theme,
        },
    },
};

use super::{label_value, panel, safe_text, workspace_focused};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mode = skill_layout_mode(frame.area());
    let layout = skill_workspace(area, mode);
    if model.skills.pane == SkillsPane::CreateSource {
        if let Some(list) = layout.list {
            home::library(frame, list, model, theme, true);
            home::source_preview(frame, layout.active, model, theme);
        } else if model.focus == Focus::List {
            home::library(frame, area, model, theme, true);
        } else {
            home::source_preview(frame, area, model, theme);
        }
        return;
    }
    if layout.list.is_none() && model.focus == Focus::List {
        render_library(frame, area, model, theme);
        return;
    }
    if let Some(list) = layout.list {
        render_library(frame, list, model, theme);
    }
    render_active(frame, layout.active, model, theme, layout.list.is_none());
}

pub(super) fn content_height(_model: &TuiModel, _width: u16) -> u16 {
    home::content_height(_model, _width)
}

pub(super) fn home_columns(model: &TuiModel) -> usize {
    home::columns_for(active_area(model))
}

fn active_area(model: &TuiModel) -> Rect {
    let terminal = Rect::new(0, 0, model.terminal_width, model.terminal_height);
    let cockpit = crate::ui::tui::layout::calculate_with_input(
        terminal,
        model.inspector_is_visible(),
        model.input_is_visible(),
    );
    skill_workspace(cockpit.workspace, skill_layout_mode(terminal)).active
}

pub(super) fn scroll_limit(model: &TuiModel) -> u16 {
    let area = active_area(model);
    let theme = Theme::from_no_color(true);
    if model.skills.pane == SkillsPane::AssignmentReview {
        let (header, body, footer) = assignment_lines(model, &theme);
        return panel_scroll_limit(area, &header, body, footer);
    }
    if model.skills.pane == SkillsPane::Confirmation
        && let Some(confirmation) = model.skills.pending_confirmation.as_ref()
    {
        let (_, header, mut body) = confirmation_lines(model, &confirmation.command, &theme);
        append_confirmation_risk(&mut body, model, &theme);
        return panel_scroll_limit(
            area,
            &header,
            body,
            vec![Line::raw("Enter: confirm"), Line::raw("Esc: return")],
        );
    }
    if model.skills.pane == SkillsPane::Editor
        && model.skills.editor_page == SkillEditorPage::Review
    {
        let Some(editor) = model.skills.editor.as_ref() else {
            return 0;
        };
        let width = area.width.saturating_sub(2).max(1);
        let footer = Paragraph::new(vec![
            Line::raw("Enter: request validation review"),
            Line::raw("Esc: keep editing · W/S scroll · I technical details"),
        ])
        .wrap(Wrap { trim: false })
        .line_count(width);
        let visible = usize::from(area.height.saturating_sub(4)).saturating_sub(footer);
        return Paragraph::new(review_body(model, editor, &Theme::from_no_color(true)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .saturating_sub(visible)
            .min(usize::from(u16::MAX)) as u16;
    }
    if model.skills.pane == SkillsPane::Editor
        && model.skills.editor_page != SkillEditorPage::Review
    {
        return home::section_scroll_limit(model, area);
    }
    home::detail_scroll_limit(model, area)
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

#[expect(
    clippy::too_many_arguments,
    reason = "the panel renderer accepts its explicit layout and content components"
)]
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
    let header_height = u16::try_from(header.len())
        .unwrap_or(u16::MAX)
        .min(inner.height);
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
    home::library(frame, area, model, theme, false);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    home::detail(frame, area, model, theme);
}

fn render_create_source(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    home::source_preview(frame, area, model, theme);
}

fn render_history(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let Some(history) = model.skills.history.as_ref() else {
        render_lines(
            frame,
            area,
            "Skill history",
            vec![
                Line::styled("Loading skill history...", theme.accent),
                Line::raw("Esc: detail"),
            ],
            workspace_focused(model),
            theme,
        );
        return;
    };
    let name = model
        .skills
        .library
        .skills
        .iter()
        .find(|skill| skill.skill_ref.skill_id() == history.skill_id)
        .map(|skill| safe_text(&skill.display_name))
        .or_else(|| {
            model
                .skills
                .detail
                .as_ref()
                .filter(|detail| detail.skill_ref.skill_id() == history.skill_id)
                .map(|detail| safe_text(&detail.content.display_name))
        })
        .unwrap_or_else(|| "Skill history".to_owned());
    let mut header = vec![
        Line::styled(name, theme.accent),
        Line::styled(format!("{} versions", history.total_count), theme.muted),
    ];
    if history.truncated {
        header.push(Line::styled(
            "History results are truncated.",
            theme.warning,
        ));
    }
    let mut body = Vec::new();
    let row_height = if model.skills.technical_details { 5 } else { 2 };
    let visible = usize::from(area.height.saturating_sub(7)) / row_height;
    let visible = visible.max(1);
    let first = model
        .skills
        .selected_history_version
        .saturating_add(1)
        .saturating_sub(visible)
        .min(history.versions.len().saturating_sub(visible));
    for (index, entry) in history
        .versions
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
    {
        let active = entry.skill_ref.skill_version_id() == history.active_version_id;
        body.push(Line::styled(
            format!(
                "{} v{}  {}",
                if index == model.skills.selected_history_version {
                    ">"
                } else {
                    " "
                },
                entry.skill_ref.version().get(),
                if active { "ACTIVE" } else { "HISTORICAL" }
            ),
            if index == model.skills.selected_history_version {
                theme.focus
            } else if active {
                theme.success
            } else {
                theme.muted
            },
        ));
        if model.skills.technical_details {
            append_exact_ref(&mut body, "Exact version", &entry.skill_ref, theme);
        } else {
            body.push(Line::styled(
                "Saved guidance · Enter to inspect",
                theme.muted,
            ));
        }
    }
    render_fixed_panel(
        frame,
        area,
        "Skill history",
        header,
        body,
        vec![
            Line::styled("W/S: select · Enter: open exact version", theme.muted),
            Line::styled("Esc: detail · I technical details", theme.muted),
        ],
        workspace_focused(model),
        theme,
    );
}

fn render_editor(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    if model.skills.editor_page != SkillEditorPage::Review {
        home::editor(frame, area, model, theme);
        return;
    }
    let Some(editor) = model.skills.editor.as_ref() else {
        return;
    };
    let header = vec![
        Line::styled(
            format!("Review / {}", safe_text(editor.raw_display_name())),
            theme.accent,
        ),
        Line::styled("DRAFT · NOT SAVED", theme.warning),
    ];
    let body = review_body(model, editor, theme);
    render_scrolled_panel(
        frame,
        area,
        "REVIEW CHANGES",
        header,
        body,
        vec![
            Line::styled(
                if editor.review().is_some() {
                    "Enter: continue to confirmation"
                } else {
                    "Enter: request validation review"
                },
                theme.focus,
            ),
            Line::styled(
                "Esc: keep editing · W/S scroll · I technical details",
                theme.muted,
            ),
        ],
        model,
        theme,
    );
}

fn render_agent_picker(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mut body = Vec::new();
    let profiles = &model.agents.profiles.profiles;
    if profiles.is_empty() {
        body.extend([
            Line::styled("No agents available", theme.warning),
            Line::raw("Create an agent profile, then return to Assign."),
        ]);
    } else {
        let visible = usize::from(area.height.saturating_sub(7) / 2).max(1);
        let first = model
            .skills
            .selected_agent
            .saturating_add(1)
            .saturating_sub(visible)
            .min(profiles.len().saturating_sub(visible));
        for (index, profile) in profiles.iter().enumerate().skip(first).take(visible) {
            body.push(Line::styled(
                format!(
                    "{} {}",
                    if index == model.skills.selected_agent {
                        ">"
                    } else {
                        " "
                    },
                    safe_text(&profile.display_name)
                ),
                if index == model.skills.selected_agent {
                    theme.focus
                } else {
                    theme.accent
                },
            ));
            body.push(Line::styled(
                format!("  Profile v{}", profile.version.get()),
                theme.muted,
            ));
        }
    }
    render_fixed_panel(
        frame,
        area,
        "Agent picker",
        vec![Line::styled("Choose who uses this skill", theme.accent)],
        body,
        vec![
            Line::styled("W/S: select agent · Enter: review", theme.muted),
            Line::styled("Esc: detail", theme.muted),
        ],
        workspace_focused(model),
        theme,
    );
}

fn render_assignment_review(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let (header, body, footer) = assignment_lines(model, theme);
    render_scrolled_panel(
        frame,
        area,
        "Assignment review",
        header,
        body,
        footer,
        model,
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
    append_confirmation_risk(&mut body, model, theme);
    render_scrolled_panel(
        frame,
        area,
        &format!("Confirm {title}"),
        header,
        body,
        vec![
            Line::styled("Enter: confirm", theme.muted),
            Line::styled("Esc: return", theme.muted),
        ],
        model,
        theme,
    );
}

fn render_result(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mut lines = vec![
        Line::styled("Skill action completed", theme.success),
        Line::raw("The immutable result is saved. Agent pins change only when explicitly chosen."),
        Line::default(),
        Line::styled("Enter or Esc: return to skill detail", theme.muted),
    ];
    append_error_guidance(&mut lines, model, theme);
    render_lines(
        frame,
        area,
        "Skill result",
        lines,
        workspace_focused(model),
        theme,
    );
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
        lines.push(Line::styled(
            "No exact skill version selected.",
            theme.muted,
        ));
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

fn append_exact_ref(
    lines: &mut Vec<Line<'static>>,
    label: &'static str,
    reference: &SkillVersionRef,
    theme: &Theme,
) {
    lines.extend([
        label_value(label, format!("v{}", reference.version().get()), theme),
        label_value("Skill ID", reference.skill_id().to_string(), theme),
        label_value(
            "Version ID",
            reference.skill_version_id().to_string(),
            theme,
        ),
        label_value("Digest", reference.content_digest().to_string(), theme),
    ]);
}

fn append_error_guidance(lines: &mut Vec<Line<'static>>, model: &TuiModel, theme: &Theme) {
    if model
        .message
        .as_ref()
        .is_some_and(|message| message.severity == Severity::Error)
    {
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
        } => {
            let (title, operation) = if replacement.version().get() > expected.version().get() {
                ("upgrade", "Upgrade")
            } else {
                ("historical reassignment", "Reassign Historical")
            };
            ref_confirmation_lines(
                model,
                title,
                operation,
                *profile_id,
                replacement,
                Some(expected),
                review_digest.to_string(),
                theme,
            )
        }
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
    let version = forthcoming_version(model, expected_active_version_id);
    let header = vec![
        Line::styled(format!("Operation  {operation}"), theme.warning),
        Line::styled(
            format!("Candidate  {}", safe_text(&candidate.display_name)),
            theme.accent,
        ),
        Line::raw(format!("Version {version}")),
    ];
    let mut body = vec![
        label_value("Purpose", safe_text(&candidate.description), theme),
        label_value(
            "Provenance",
            "Pending authoritative commit".to_owned(),
            theme,
        ),
        Line::raw("Saving creates a new version. Existing assignments stay unchanged."),
    ];
    if model.skills.technical_details {
        body.push(label_value("Skill ID", skill_id.to_string(), theme));
        if let Some(review) = model
            .skills
            .editor
            .as_ref()
            .and_then(|editor| editor.review())
            .filter(|review| review.preview().skill_id == skill_id)
        {
            body.push(label_value(
                "Candidate digest",
                review.preview().candidate_digest.to_string(),
                theme,
            ));
        }
        if let Some(expected) = expected_active_version_id {
            body.push(label_value("Reviewed base", expected.to_string(), theme));
        }
        body.push(label_value("Review digest", review_digest, theme));
    }
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
    let agent = model
        .skills
        .selected_agent_detail
        .as_ref()
        .filter(|agent| agent.profile.profile_id() == profile_id)
        .map(|agent| safe_text(agent.profile.display_name()))
        .or_else(|| {
            model
                .agents
                .profiles
                .profiles
                .iter()
                .find(|agent| agent.profile_id == profile_id)
                .map(|agent| safe_text(&agent.display_name))
        })
        .unwrap_or_else(|| "Agent name unavailable".to_owned());
    let header = vec![
        Line::styled(format!("Operation  {operation}"), theme.warning),
        Line::styled(format!("Agent  {agent}"), theme.accent),
        Line::raw(format!(
            "{} · v{}",
            skill_name(model, target),
            target.version().get()
        )),
    ];
    let mut body = Vec::new();
    append_readable_ref(&mut body, "Exact version", model, target, theme);
    if let Some(prior) = prior {
        append_readable_ref(&mut body, "Current pin", model, prior, theme);
    }
    let provenance = model
        .skills
        .version_detail
        .as_ref()
        .or(model.skills.detail.as_ref())
        .filter(|detail| detail.skill_ref == *target)
        .map(|detail| provenance_short(&detail.provenance))
        .unwrap_or("Not loaded for this exact version");
    body.push(label_value("Provenance", provenance.to_owned(), theme));
    if model.skills.technical_details {
        body.push(label_value("Agent ID", profile_id.to_string(), theme));
        body.push(label_value("Review digest", review_digest, theme));
    }
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

fn editor_operation(editor: &SkillEditor) -> &'static str {
    match editor.mode() {
        SkillEditorMode::Create => "Create skill",
        SkillEditorMode::Version { .. } => "Create version",
    }
}

fn editor_target_version(model: &TuiModel, editor: &SkillEditor) -> String {
    match editor.mode() {
        SkillEditorMode::Create => forthcoming_version(model, None),
        SkillEditorMode::Version {
            expected_active_version_id,
            ..
        } => forthcoming_version(model, Some(*expected_active_version_id)),
    }
}

fn forthcoming_version(
    model: &TuiModel,
    expected_active_version_id: Option<crate::domain::SkillVersionId>,
) -> String {
    let object_version = match expected_active_version_id {
        None => Some(1),
        Some(expected) => model
            .skills
            .detail
            .as_ref()
            .filter(|detail| detail.skill_ref.skill_version_id() == expected)
            .and_then(|detail| detail.skill_ref.version().get().checked_add(1)),
    };
    object_version
        .map(|version| format!("v{version}"))
        .unwrap_or_else(|| "Unavailable; reload the active version".to_owned())
}

fn editor_provenance(model: &TuiModel, editor: &SkillEditor) -> String {
    match editor.mode() {
        SkillEditorMode::Create => "Custom / user-created".to_owned(),
        SkillEditorMode::Version {
            skill_id,
            expected_active_version_id,
        } => model
            .skills
            .detail
            .as_ref()
            .filter(|detail| {
                detail.skill_ref.skill_id() == *skill_id
                    && detail.skill_ref.skill_version_id() == *expected_active_version_id
            })
            .map(|detail| provenance_short(&detail.provenance).to_owned())
            .unwrap_or_else(|| "Reviewed source not loaded".to_owned()),
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
        Some(AssignmentKind::Reassign { .. }) => "Reassign historical exact version",
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
        SkillsPane::List => "W/S select; Enter open; c Create; Esc back",
        SkillsPane::CreateSource => "W/S select; Enter continue; Esc library",
        SkillsPane::Detail => "A/D action; Enter open; Esc library",
        SkillsPane::History => "W/S select; Enter exact version; Esc detail",
        SkillsPane::Editor => "Type focused value; Enter continue; Esc back",
        SkillsPane::AgentPicker => "W/S agent; Enter review; Esc detail",
        SkillsPane::AssignmentReview
            if model.skills.assignment == Some(AssignmentKind::AlreadyAssigned) =>
        {
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
        Some(AssignmentKind::Reassign { .. }) => "historical reassignment",
        Some(AssignmentKind::Unassign { .. }) => "unassignment",
        Some(AssignmentKind::AlreadyAssigned) | None => "operation",
    }
}

fn skill_name(model: &TuiModel, target: &SkillVersionRef) -> String {
    model
        .skills
        .version_detail
        .as_ref()
        .or(model.skills.detail.as_ref())
        .filter(|detail| detail.skill_ref == *target)
        .map(|detail| safe_text(&detail.content.display_name))
        .or_else(|| {
            model
                .skills
                .library
                .skills
                .iter()
                .find(|summary| summary.skill_ref == *target)
                .map(|summary| safe_text(&summary.display_name))
        })
        .unwrap_or_else(|| "Skill name unavailable".to_owned())
}

fn append_readable_ref(
    lines: &mut Vec<Line<'static>>,
    label: &'static str,
    model: &TuiModel,
    target: &SkillVersionRef,
    theme: &Theme,
) {
    lines.push(label_value(
        label,
        format!(
            "{} · v{}",
            skill_name(model, target),
            target.version().get()
        ),
        theme,
    ));
    if model.skills.technical_details {
        append_exact_ref(lines, "Exact pin", target, theme);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_scrolled_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    header: Vec<Line<'static>>,
    body: Vec<Line<'static>>,
    footer: Vec<Line<'static>>,
    model: &TuiModel,
    theme: &Theme,
) {
    let block = panel(title, workspace_focused(model), theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let header_height = (header.len() as u16).min(inner.height);
    let footer_widget = Paragraph::new(footer).wrap(Wrap { trim: false });
    let footer_height = (footer_widget.line_count(inner.width.max(1)) as u16)
        .min(inner.height.saturating_sub(header_height));
    let regions = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(0),
        Constraint::Length(footer_height),
    ])
    .split(inner);
    frame.render_widget(Paragraph::new(header), regions[0]);
    home::scroll_text(frame, regions[1], body, model.skills.content_scroll);
    frame.render_widget(footer_widget, regions[2]);
}

fn review_body(model: &TuiModel, editor: &SkillEditor, theme: &Theme) -> Vec<Line<'static>> {
    let mut body = vec![
        label_value("Operation", editor_operation(editor).to_owned(), theme),
        label_value("Exact version", editor_target_version(model, editor), theme),
        label_value("Provenance", editor_provenance(model, editor), theme),
        Line::styled(
            "Saving creates a new version. Existing assignments stay unchanged.",
            theme.muted,
        ),
        Line::default(),
    ];
    for field in [
        SkillEditorField::DisplayName,
        SkillEditorField::Purpose,
        SkillEditorField::UseWhen,
        SkillEditorField::Tags,
        SkillEditorField::Instructions,
    ] {
        body.push(Line::styled(editor_field_name(field), theme.accent));
        body.push(Line::raw(safe_text(editor.tui_field_text(field))));
    }
    for note in editor.references() {
        body.push(Line::styled(safe_text(&note.name), theme.accent));
        body.push(Line::raw(safe_text(&note.body)));
    }
    if let Some(error) = editor.local_error() {
        body.push(Line::styled(
            validation_message(error.code(), error.field()),
            theme.error,
        ));
    }
    if model.skills.technical_details
        && let Some(review) = editor.review()
    {
        body.push(label_value(
            "Candidate digest",
            review.preview().candidate_digest.to_string(),
            theme,
        ));
        body.push(label_value(
            "Review digest",
            review.preview().review_digest.to_string(),
            theme,
        ));
    }
    append_error_guidance(&mut body, model, theme);
    body
}

fn assignment_lines(
    model: &TuiModel,
    theme: &Theme,
) -> (Vec<Line<'static>>, Vec<Line<'static>>, Vec<Line<'static>>) {
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
            format!("v{}", agent.profile.version().get()),
            theme,
        ));
    } else {
        header.push(Line::styled("Agent  Loading exact profile", theme.warning));
    }
    if let Some(target) = model.skills.selected_skill_ref() {
        header.push(Line::raw(format!(
            "Target  {} · v{}",
            skill_name(model, target),
            target.version().get()
        )));
        append_readable_ref(&mut body, "Target", model, target, theme);
    }
    match assignment {
        Some(AssignmentKind::Upgrade { expected })
        | Some(AssignmentKind::Reassign { expected }) => {
            append_readable_ref(&mut body, "Current pin", model, expected, theme)
        }
        Some(AssignmentKind::Unassign { expected }) => {
            append_readable_ref(&mut body, "Remove pin", model, expected, theme)
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
            Line::styled("Esc: choose another version", theme.muted),
        ]
    } else {
        vec![
            Line::styled(
                format!("Enter: validate {}", assignment_verb(assignment)),
                theme.focus,
            ),
            Line::styled("Esc: agent picker", theme.muted),
        ]
    };
    (header, body, footer)
}

fn append_confirmation_risk(body: &mut Vec<Line<'static>>, model: &TuiModel, theme: &Theme) {
    body.extend([
        label_value(
            "Review risk",
            "Commits the displayed immutable version or exact agent pin only.".to_owned(),
            theme,
        ),
        Line::styled(
            "No automatic upgrades or executable capabilities.",
            theme.muted,
        ),
    ]);
    append_error_guidance(body, model, theme);
}

fn panel_scroll_limit(
    area: Rect,
    header: &[Line<'static>],
    body: Vec<Line<'static>>,
    footer: Vec<Line<'static>>,
) -> u16 {
    let width = area.width.saturating_sub(2).max(1);
    let footer_height = Paragraph::new(footer)
        .wrap(Wrap { trim: false })
        .line_count(width);
    let visible =
        usize::from(area.height.saturating_sub(2)).saturating_sub(header.len() + footer_height);
    Paragraph::new(body)
        .wrap(Wrap { trim: false })
        .line_count(width)
        .saturating_sub(visible)
        .min(usize::from(u16::MAX)) as u16
}
