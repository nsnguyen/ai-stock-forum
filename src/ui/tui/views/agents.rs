use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentReadiness, ProfileDiffField, ProfileFieldDiff,
        ProfileFieldValue,
    },
    app::ApplicationCommand,
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorMode, ProfileEditorStep},
        tui::{
            layout::{agent_workspace, layout_mode},
            model::{AgentsPane, LayoutMode, TuiModel},
            theme::Theme,
        },
    },
};

use super::{label_value, panel, safe_text, workspace_focused};

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let mode = layout_mode(frame.area());
    let layout = agent_workspace(area, mode);
    if let Some(list) = layout.list {
        render_list(frame, list, model, theme);
    }
    render_active(frame, layout.active, model, theme, layout.list.is_none(), mode);
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
    mode: LayoutMode,
) {
    match model.agents.pane {
        AgentsPane::List if one_pane => render_list(frame, area, model, theme),
        AgentsPane::Editor => render_editor(frame, area, model, theme),
        AgentsPane::Confirmation => render_confirmation(frame, area, model, theme),
        AgentsPane::History if mode != LayoutMode::Wide => {
            render_history(frame, area, model, theme)
        }
        AgentsPane::List | AgentsPane::Detail | AgentsPane::History => {
            render_detail(frame, area, model, theme)
        }
    }
}

fn render_list(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let focused = workspace_focused(model) && model.agents.pane == AgentsPane::List;
    let lines = if model.agents.profiles.profiles.is_empty() {
        vec![
            Line::styled("No agent profiles yet", theme.accent),
            Line::default(),
            Line::raw("Press c to create your first profile."),
            Line::styled("Templates provide a safe starting point.", theme.muted),
        ]
    } else {
        model
            .agents
            .profiles
            .profiles
            .iter()
            .enumerate()
            .flat_map(|(index, profile)| {
                let selected = index == model.agents.selected_profile;
                let marker = if selected { ">" } else { " " };
                vec![
                    Line::styled(
                        format!("{marker} {}", safe_text(&profile.display_name)),
                        if selected { theme.focus } else { theme.accent },
                    ),
                    Line::from(vec![
                        Span::styled(format!("  {} | ", profile.role.as_str()), theme.muted),
                        Span::styled(
                            readiness_name(profile.readiness),
                            readiness_style(profile.readiness, theme),
                        ),
                        Span::styled(format!(" | v{}", profile.version.get()), theme.muted),
                    ]),
                    Line::styled(
                        format!("  {}", safe_text(&profile.primary_specialty)),
                        theme.muted,
                    ),
                    Line::default(),
                ]
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Agent list", focused, theme))
            .wrap(Wrap { trim: false })
            .scroll((scroll(model.agents.list_scroll, area.height), 0)),
        area,
    );
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let focused = workspace_focused(model) && model.agents.pane == AgentsPane::Detail;
    let lines = model
        .agents
        .detail
        .as_ref()
        .map(|detail| detail_lines(detail, theme))
        .unwrap_or_else(|| {
            if model.agents.profiles.profiles.is_empty() {
                vec![
                    Line::styled("No agent profiles yet", theme.accent),
                    Line::default(),
                    Line::raw("Press c to create your first profile."),
                ]
            } else {
                vec![
                    Line::styled("No profile selected", theme.accent),
                    Line::default(),
                    Line::raw("Choose a profile and press Enter to load its detail."),
                ]
            }
        });
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Agent detail", focused, theme))
            .wrap(Wrap { trim: false })
            .scroll((scroll(model.agents.detail_scroll, area.height), 0)),
        area,
    );
}

fn detail_lines(detail: &crate::app::AgentProfileView, theme: &Theme) -> Vec<Line<'static>> {
    let profile = &detail.profile;
    let mut lines = vec![
        Line::styled(safe_text(profile.display_name()), theme.accent),
        label_value("Profile ID", profile.profile_id().to_string(), theme),
        label_value(
            "Active version",
            format!("v{} / {}", profile.version().get(), profile.profile_version_id()),
            theme,
        ),
        label_value("Role", profile.role().as_str().to_owned(), theme),
        label_value("Specialty", safe_text(profile.primary_specialty()), theme),
        label_value("Tags", safe_text(&profile.specialty_tags().join(", ")), theme),
        readiness_line("Readiness", detail.readiness, theme),
        Line::default(),
        Line::styled("ACCEPTED CONTENT", theme.accent),
        label_value("Description", safe_text(profile.description()), theme),
        label_value("Personality", safe_text(profile.personality()), theme),
        label_value("Instructions", safe_text(profile.instructions()), theme),
        Line::default(),
        Line::styled("IMMUTABLE METADATA", theme.accent),
        label_value("Created ms", profile.created_at_ms().to_string(), theme),
        label_value("Memory", profile.memory_namespace_id().to_string(), theme),
        label_value("Policy", safe_text(profile.default_policy_ref()), theme),
        label_value(
            "Supersedes",
            profile
                .supersedes()
                .map(|id| id.to_string())
                .unwrap_or_else(|| "None".to_owned()),
            theme,
        ),
        label_value("Digest", profile.content_digest().to_string(), theme),
        Line::default(),
        Line::styled("Bindings", theme.accent),
        label_value(
            "Provider",
            optional_text(profile.bindings().model_provider.as_deref()),
            theme,
        ),
        label_value(
            "Model",
            optional_text(profile.bindings().model_name.as_deref()),
            theme,
        ),
    ];
    append_provenance(&mut lines, profile.template_provenance(), theme);
    lines
}

fn render_history(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    frame.render_widget(
        Paragraph::new(history_lines(model, theme))
            .block(panel(
                "Profile history",
                workspace_focused(model) && model.agents.pane == AgentsPane::History,
                theme,
            ))
            .wrap(Wrap { trim: false })
            .scroll((scroll(model.agents.history_scroll, area.height), 0)),
        area,
    );
}

fn history_lines(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    let Some(history) = &model.agents.history else {
        return vec![
            Line::styled("No history loaded", theme.accent),
            Line::raw("Press h from profile detail to load version history."),
        ];
    };
    let mut lines = vec![
        label_value("Profile ID", history.profile_id.to_string(), theme),
        label_value("Active version", history.active_version_id.to_string(), theme),
        label_value("Versions", history.versions.len().to_string(), theme),
        Line::default(),
    ];
    for entry in &history.versions {
        let active = entry.profile_version_id == history.active_version_id;
        lines.push(Line::styled(
            format!(
                "{} v{}  {}",
                if active { ">" } else { " " },
                entry.version.get(),
                readiness_name(entry.readiness)
            ),
            if active {
                theme.focus
            } else {
                readiness_style(entry.readiness, theme)
            },
        ));
        lines.push(label_value("Version ID", entry.profile_version_id.to_string(), theme));
        lines.push(label_value("Created ms", entry.created_at_ms.to_string(), theme));
        lines.push(label_value(
            "Supersedes",
            entry
                .supersedes
                .map(|id| id.to_string())
                .unwrap_or_else(|| "None".to_owned()),
            theme,
        ));
        lines.push(label_value("Digest", entry.content_digest.to_string(), theme));
        lines.push(Line::default());
    }
    lines
}

fn render_editor(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let lines = model
        .agents
        .editor
        .as_ref()
        .map(|editor| editor_lines(editor, theme))
        .unwrap_or_else(|| vec![Line::raw("Editor is unavailable. Press Esc to return.")]);
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel("Profile editor", workspace_focused(model), theme))
            .wrap(Wrap { trim: false })
            .scroll((scroll(model.agents.detail_scroll, area.height), 0)),
        area,
    );
}

fn editor_lines(editor: &ProfileEditor, theme: &Theme) -> Vec<Line<'static>> {
    let step = editor.step();
    let mode = match editor.mode() {
        ProfileEditorMode::Create { .. } => "Create",
        ProfileEditorMode::Edit { .. } => "Edit",
    };
    let mut lines = vec![
        Line::styled(format!("{mode} agent profile"), theme.accent),
        label_value("Progress", format!("Step {} of 7", step_number(step)), theme),
        label_value("Current step", step.as_str().replace('_', " "), theme),
        Line::styled(step_guidance(step), theme.muted),
        Line::styled("Enter text or a :control in the command bar.", theme.muted),
    ];
    if let Some(message) = editor.local_message() {
        lines.push(Line::default());
        lines.push(Line::styled(editor_message(message.code()), theme.warning));
    }
    if step == ProfileEditorStep::Review {
        lines.push(Line::default());
        lines.push(Line::styled("REVIEW CHANGES", theme.accent));
        if let Some(review) = editor.review() {
            append_diffs(&mut lines, &review.preview().diffs, theme);
            lines.push(Line::styled(
                "Use :activate to continue to explicit confirmation.",
                theme.muted,
            ));
        } else if let Some(baseline) = editor.create_baseline() {
            append_create_diffs(&mut lines, baseline, editor.draft(), theme);
            lines.push(Line::styled(
                "Use :activate to continue to explicit confirmation.",
                theme.muted,
            ));
        } else {
            lines.push(Line::styled(
                "Use :review to request an authoritative diff.",
                theme.warning,
            ));
        }
    }
    lines.push(Line::default());
    lines.push(Line::styled("CURRENT CANDIDATE", theme.accent));
    append_draft(&mut lines, editor.draft(), theme);
    lines
}

fn append_draft(lines: &mut Vec<Line<'static>>, draft: &AgentProfileDraft, theme: &Theme) {
    lines.extend([
        label_value("Display name", safe_text(&draft.display_name), theme),
        label_value("Description", safe_text(&draft.description), theme),
        label_value("Role", draft.role.as_str().to_owned(), theme),
        label_value("Specialty", safe_text(&draft.primary_specialty), theme),
        label_value("Tags", safe_text(&draft.specialty_tags.join(", ")), theme),
        label_value("Personality", safe_text(&draft.personality), theme),
        label_value("Instructions", safe_text(&draft.instructions), theme),
        label_value("Bindings", bindings_value(&draft.bindings), theme),
    ]);
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
        ("Display name", safe_text(&baseline.display_name), safe_text(&draft.display_name)),
        ("Description", safe_text(&baseline.description), safe_text(&draft.description)),
        ("Role", baseline.role.as_str().to_owned(), draft.role.as_str().to_owned()),
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
        ("Personality", safe_text(&baseline.personality), safe_text(&draft.personality)),
        ("Instructions", safe_text(&baseline.instructions), safe_text(&draft.instructions)),
        ("Bindings", bindings_value(&baseline.bindings), bindings_value(&draft.bindings)),
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
    let action = model
        .agents
        .pending_confirmation
        .as_ref()
        .map(|confirmation| match confirmation.command {
            ApplicationCommand::CreateAgentProfile { .. } => "Create",
            ApplicationCommand::ActivateAgentProfileVersion { .. } => "Activate",
            _ => "Apply",
        })
        .unwrap_or("Apply");
    let lines = vec![
        Line::styled(format!("Confirm {action}"), theme.warning),
        Line::default(),
        Line::raw("This action writes a new immutable profile version."),
        Line::default(),
        Line::styled(format!("Enter  Confirm {action}"), theme.focus),
        Line::styled("Esc    Cancel and return to review", theme.muted),
    ];
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

pub(super) fn inspector_lines(model: &TuiModel, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled("Readiness & history", theme.accent),
        Line::default(),
    ];
    let Some(detail) = &model.agents.detail else {
        lines.push(Line::raw("Select a profile to inspect readiness and history."));
        return lines;
    };
    let profile = &detail.profile;
    lines.push(Line::styled(safe_text(profile.display_name()), theme.accent));
    lines.push(readiness_line("Readiness", detail.readiness, theme));
    lines.push(label_value(
        "Provider",
        optional_text(profile.bindings().model_provider.as_deref()),
        theme,
    ));
    lines.push(label_value(
        "Model",
        optional_text(profile.bindings().model_name.as_deref()),
        theme,
    ));
    lines.push(Line::default());
    append_provenance(&mut lines, profile.template_provenance(), theme);
    lines.push(Line::default());
    if let Some(history) = &model.agents.history {
        lines.push(Line::styled("HISTORY", theme.accent));
        lines.push(label_value("Versions", history.versions.len().to_string(), theme));
        lines.push(label_value("Active version", history.active_version_id.to_string(), theme));
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
        lines.push(Line::styled("Press h to load version history.", theme.muted));
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
        lines.push(label_value("Template", provenance.template_id.to_string(), theme));
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
        Span::styled(
            readiness_name(readiness),
            readiness_style(readiness, theme),
        ),
    ])
}

fn readiness_name(readiness: AgentReadiness) -> &'static str {
    match readiness {
        AgentReadiness::Ready => "Ready",
        AgentReadiness::NotReady => "Not Ready",
    }
}

fn readiness_style(readiness: AgentReadiness, theme: &Theme) -> ratatui::style::Style {
    match readiness {
        AgentReadiness::Ready => theme.success,
        AgentReadiness::NotReady => theme.warning,
    }
}

fn optional_text(value: Option<&str>) -> String {
    value.map(safe_text).unwrap_or_else(|| "Not configured".to_owned())
}

fn bindings_value(bindings: &AgentBindings) -> String {
    format!(
        "{} / {}",
        optional_text(bindings.model_provider.as_deref()),
        optional_text(bindings.model_name.as_deref())
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
    }
}

fn step_number(step: ProfileEditorStep) -> u8 {
    match step {
        ProfileEditorStep::Template => 1,
        ProfileEditorStep::Identity => 2,
        ProfileEditorStep::Specialty => 3,
        ProfileEditorStep::Personality => 4,
        ProfileEditorStep::Instructions => 5,
        ProfileEditorStep::OptionalBindings => 6,
        ProfileEditorStep::Review => 7,
    }
}

fn step_guidance(step: ProfileEditorStep) -> &'static str {
    match step {
        ProfileEditorStep::Template => "Choose a template role with :role <role>, then :next.",
        ProfileEditorStep::Identity => "Set display name and description; use :next between fields.",
        ProfileEditorStep::Specialty => "Set specialty; use :tag add <tag> or :tag remove <tag>.",
        ProfileEditorStep::Personality => "Describe the agent's working style, then use :next.",
        ProfileEditorStep::Instructions => "Enter operating instructions, then use :next.",
        ProfileEditorStep::OptionalBindings => {
            "Use :provider and :model, or leave bindings unconfigured (Not Ready)."
        }
        ProfileEditorStep::Review => "Review ordered accepted-field changes before confirmation.",
    }
}

fn editor_message(code: &str) -> &'static str {
    match code {
        "invalid_profile_field" => "Validation: this field is invalid; revise it before continuing.",
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
