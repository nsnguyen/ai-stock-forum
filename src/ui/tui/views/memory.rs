use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::{
    app::{ApplicationCommand, MemoryEditPreview},
    memory::{
        ExpectedMemoryEntryState, MemoryField, MemoryFieldValue, MemoryMutationKind,
        MemoryNoChange, MemoryProposalFilter, MemoryProposalOperation, MemoryProposalOperationKind,
        MemoryProposalStatus, MemoryResolutionAction,
    },
    policy::ApprovalStatus,
    ui::{
        memory_editor::{MEMORY_PLAINTEXT_WARNING, MemoryEditorStep},
        tui::{
            layout::{memory_layout_mode, memory_list_visible_items, memory_workspace},
            model::{Focus, MemoryPageCounts, MemoryPane, TuiModel},
            theme::Theme,
        },
    },
};

use super::{actor_name, memory_escape_bounded, panel, workspace_focused, wrapped_height};

const LIST_ITEM_CAP: usize = 100;
const KEY_MAX_ESCAPED_BYTES: usize = 384;
const TAG_MAX_ESCAPED_BYTES: usize = 128;
const VALUE_MAX_ESCAPED_BYTES: usize = 16_384;
const PROFILE_LABEL_MAX_ESCAPED_BYTES: usize = 512;
const EPISODIC_LABEL_MAX_ESCAPED_BYTES: usize = 512;
const EPISODIC_BODY_MAX_ESCAPED_BYTES: usize = 32_768;
const EPISODIC_EVENT_TYPE_MAX_ESCAPED_BYTES: usize = 512;
const MAX_PURPOSE_TAGS: usize = 8;
const MAX_EPISODIC_SOURCES: usize = 128;

struct PanelContent {
    title: &'static str,
    lines: Vec<Line<'static>>,
    scroll: usize,
}

pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let layout = memory_workspace(area, memory_layout_mode(area));
    if layout.detail.is_none() {
        if model.focus == Focus::List || narrow_uses_primary(model) {
            render_primary(frame, layout.primary, model, theme);
        } else {
            render_detail(frame, layout.primary, model, theme);
        }
        return;
    }

    render_primary(frame, layout.primary, model, theme);
    if let Some(detail) = layout.detail {
        render_detail(frame, detail, model, theme);
    }
    if let Some(context) = layout.context {
        render_context(frame, context, model, theme);
    }
}

pub(super) fn content_height(model: &TuiModel, width: u16) -> u16 {
    let content_width = width.saturating_sub(2).max(1);
    wrapped_height(memory_content_lines(model, width), content_width).max(1)
}

fn render_primary(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let content = primary_content(model, area, theme);
    render_panel(frame, area, model.focus == Focus::List, theme, content);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let content = active_content(model, panel_inner_width(area), theme);
    render_panel(frame, area, workspace_focused(model), theme, content);
}

fn render_context(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    let content = context_content(model, panel_inner_width(area), theme);
    render_panel(frame, area, false, theme, content);
}

fn render_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    focused: bool,
    theme: &Theme,
    mut content: PanelContent,
) {
    let block = panel(content.title, focused, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let fixed_count = fixed_prefix_lines(content.title).min(content.lines.len());
    let body = content.lines.split_off(fixed_count);
    let fixed_height = wrapped_height(content.lines.clone(), inner.width.max(1)).min(inner.height);
    if fixed_height > 0 {
        frame.render_widget(
            Paragraph::new(content.lines).wrap(Wrap { trim: false }),
            Rect::new(inner.x, inner.y, inner.width, fixed_height),
        );
    }

    let body_area = Rect::new(
        inner.x,
        inner.y.saturating_add(fixed_height),
        inner.width,
        inner.height.saturating_sub(fixed_height),
    );
    if body_area.height == 0 {
        return;
    }
    let height = wrapped_height(body.clone(), body_area.width.max(1));
    let max_scroll = usize::from(height.saturating_sub(body_area.height));
    let scroll = u16::try_from(content.scroll.min(max_scroll)).unwrap_or(u16::MAX);
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        body_area,
    );
}

fn fixed_prefix_lines(title: &str) -> usize {
    match title {
        "Entry detail" | "Proposal detail" => 8,
        "Historical entry version" => 7,
        "Memory editor"
        | "Mutation review"
        | "Proposal resolution review"
        | "Confirm memory change"
        | "Confirm proposal resolution" => 5,
        "Episodic detail" => 6,
        "Resolution target" | "Retained resolution review" => 7,
        "Mutation diff" | "Retained mutation review" | "Episodic sources" | "Memory result" => 3,
        _ => 0,
    }
}

fn primary_content(model: &TuiModel, area: Rect, theme: &Theme) -> PanelContent {
    match model.agents.memory.pane {
        MemoryPane::EntryHistory => history_list_content(model, area, theme),
        MemoryPane::EntryList
        | MemoryPane::EntryDetail
        | MemoryPane::Editor
        | MemoryPane::MutationReview => entry_list_content(model, area, theme),
        MemoryPane::Confirmation => match model.agents.memory.confirmed_command() {
            Some(
                ApplicationCommand::SetMemoryEntry { .. }
                | ApplicationCommand::DeleteMemoryEntry { .. },
            ) => entry_list_content(model, area, theme),
            Some(
                ApplicationCommand::ApproveMemoryProposal { .. }
                | ApplicationCommand::RejectMemoryProposal { .. },
            ) => proposal_list_content(model, area, theme),
            _ => confirmation_content(model, panel_inner_width(area), theme),
        },
        MemoryPane::Result => match model.agents.memory.result_origin {
            crate::ui::tui::model::MemoryResultOrigin::Mutation => {
                entry_list_content(model, area, theme)
            }
            crate::ui::tui::model::MemoryResultOrigin::Resolution => {
                proposal_list_content(model, area, theme)
            }
        },
        MemoryPane::Proposals
        | MemoryPane::ProposalDetail
        | MemoryPane::ProposalResolutionReview => proposal_list_content(model, area, theme),
        MemoryPane::EpisodicSummaries | MemoryPane::EpisodicDetail => {
            episodic_list_content(model, area, theme)
        }
    }
}

fn detail_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    match model.agents.memory.pane {
        MemoryPane::EntryList => entry_preview_content(model, theme),
        MemoryPane::EntryDetail => entry_detail_content(model, width, theme),
        MemoryPane::EntryHistory => history_detail_content(model, width, theme),
        MemoryPane::Editor => editor_content(model, width, theme),
        MemoryPane::MutationReview => mutation_review_content(model, width, theme),
        MemoryPane::Confirmation => confirmation_content(model, width, theme),
        MemoryPane::Result => result_content(model, theme),
        MemoryPane::Proposals => proposal_preview_content(model, theme),
        MemoryPane::ProposalDetail => proposal_detail_content(model, width, theme),
        MemoryPane::ProposalResolutionReview => {
            proposal_resolution_review_content(model, width, theme)
        }
        MemoryPane::EpisodicSummaries => episodic_preview_content(model, theme),
        MemoryPane::EpisodicDetail => episodic_detail_content(model, width, theme),
    }
}

fn context_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    match model.agents.memory.pane {
        MemoryPane::EntryList => entry_counts_content(model, theme),
        MemoryPane::EntryDetail => entry_context_content(model, theme),
        MemoryPane::EntryHistory => history_context_content(model, theme),
        MemoryPane::Editor => editor_context_content(model, width, theme),
        MemoryPane::MutationReview => mutation_diff_content(model, "Mutation diff", theme),
        MemoryPane::Confirmation => confirmation_context_content(model, theme),
        MemoryPane::Result => result_context_content(model, theme),
        MemoryPane::Proposals => proposal_counts_content(model, theme),
        MemoryPane::ProposalDetail => proposal_context_content(model, theme),
        MemoryPane::ProposalResolutionReview => {
            resolution_context_content(model, "Resolution target", theme)
        }
        MemoryPane::EpisodicSummaries => episodic_counts_content(model, theme),
        MemoryPane::EpisodicDetail => episodic_sources_content(model, theme),
    }
}

fn memory_content_lines(model: &TuiModel, width: u16) -> Vec<Line<'static>> {
    let theme = Theme::from_no_color(true);
    if narrow_uses_primary(model) {
        primary_content(model, Rect::new(0, 0, width, u16::MAX), &theme).lines
    } else {
        active_content(model, usize::from(width.saturating_sub(2).max(1)), &theme).lines
    }
}

fn active_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    let mut detail = detail_content(model, width, theme);
    let context = context_content(model, width, theme);
    detail.lines.push(Line::default());
    detail.lines.push(Line::styled(context.title, theme.accent));
    detail.lines.extend(context.lines);
    detail
}

fn narrow_uses_primary(model: &TuiModel) -> bool {
    matches!(
        model.agents.memory.pane,
        MemoryPane::EntryList | MemoryPane::Proposals | MemoryPane::EpisodicSummaries
    ) || (model.agents.memory.pane == MemoryPane::EntryHistory
        && model.agents.memory.entry_version.is_none())
}

fn entry_list_content(model: &TuiModel, area: Rect, theme: &Theme) -> PanelContent {
    let memory = &model.agents.memory;
    let Some((page, counts)) = authenticated_entries(model) else {
        return unavailable_panel(
            "Memory entries",
            "Memory entries unavailable.",
            "Reload the exact profile memory.",
            theme,
        );
    };
    if counts.displayed > 0 && model.agents.memory.authenticated_selected_entry().is_none() {
        return unavailable_panel(
            "Memory entries",
            "Memory entries unavailable.",
            "Reload the exact profile memory.",
            theme,
        );
    }
    let retained = &page.entries[..counts.displayed];
    let selected = memory.selected_entry;
    let visible_items = memory_list_visible_items(area);
    let first = first_visible_item(memory.entry_scroll, selected, retained.len(), visible_items);
    let width = panel_inner_width(area);

    let mut lines = vec![
        memory_heading(theme),
        profile_identity_line(model, width, theme),
    ];
    if retained.is_empty() {
        lines.extend([
            Line::styled(
                bounded_ascii_line("No memory entries saved", width),
                theme.accent,
            ),
            Line::raw(bounded_ascii_line(
                "c: create | p: proposals | e: episodic",
                width,
            )),
            Line::styled(bounded_ascii_line("Esc: agent detail", width), theme.focus),
        ]);
    } else {
        for (index, summary) in retained.iter().enumerate().skip(first).take(visible_items) {
            let prefix = if index == selected { "> " } else { "  " };
            lines.push(Line::styled(
                escaped_value_line(prefix, &summary.display_key, KEY_MAX_ESCAPED_BYTES, width),
                if index == selected {
                    theme.focus
                } else {
                    theme.accent
                },
            ));
            lines.push(Line::styled(
                bounded_ascii_line(
                    &format!(
                        "  v{} | {} bytes | {} tags",
                        summary.entry.version().get(),
                        summary.value_bytes,
                        summary.purpose_tags.len()
                    ),
                    width,
                ),
                theme.muted,
            ));
        }
        lines.push(Line::styled(
            bounded_ascii_line(
                "Up/Down | Enter: detail | c:new p:props e:episodes | Esc",
                width,
            ),
            theme.muted,
        ));
    }
    PanelContent {
        title: "Memory entries",
        lines,
        scroll: 0,
    }
}

fn entry_preview_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some((page, counts)) = authenticated_entries(model) else {
        return unavailable_panel(
            "Entry metadata",
            "Entry metadata unavailable.",
            "Reload the exact profile memory.",
            theme,
        );
    };
    let selected = model.agents.memory.selected_entry;
    if counts.displayed == 0 {
        return static_panel("Entry metadata", "No memory entry is selected.", theme);
    }
    let Some(summary) = page
        .entries
        .get(selected)
        .filter(|_| selected < counts.displayed)
    else {
        return unavailable_panel(
            "Entry metadata",
            "Entry metadata unavailable.",
            "Select an exact retained entry and reload its metadata.",
            theme,
        );
    };
    PanelContent {
        title: "Entry metadata",
        lines: vec![
            memory_heading(theme),
            Line::raw(format!("Entry ID      {}", summary.entry.entry_id())),
            Line::raw(format!("Version       {}", summary.entry.version().get())),
            Line::raw(format!("Value bytes   {}", summary.value_bytes)),
            Line::raw("Enter: load exact detail | Esc: agent detail"),
        ],
        scroll: 0,
    }
}

fn entry_detail_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    let Some(detail) = authenticated_entry_detail(model) else {
        return unavailable_panel(
            "Entry detail",
            "Entry detail unavailable.",
            "Select the entry and reload its exact detail.",
            theme,
        );
    };
    let entry = &detail.entry;
    let reference = entry.reference();
    let state = match reference.state() {
        crate::memory::MemoryEntryState::Present => "Present",
        crate::memory::MemoryEntryState::Deleted => "Deleted",
    };
    let mut action_spans = Vec::new();
    for (index, (action, label)) in [
        (crate::ui::tui::model::MemoryEntryDetailAction::Edit, "Edit"),
        (
            crate::ui::tui::model::MemoryEntryDetailAction::Delete,
            "Delete",
        ),
        (
            crate::ui::tui::model::MemoryEntryDetailAction::History,
            "History",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            action_spans.push(Span::raw(" "));
        }
        action_spans.push(Span::styled(
            format!("[{label}]"),
            if model.agents.memory.selected_entry_detail_action == action {
                theme.focus
            } else {
                theme.muted
            },
        ));
    }
    let tags = entry
        .purpose_tags()
        .iter()
        .take(MAX_PURPOSE_TAGS)
        .map(|tag| memory_escape_bounded(tag, TAG_MAX_ESCAPED_BYTES))
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = vec![
        memory_heading(theme),
        Line::raw(format!("Profile        {}", detail.profile.profile_id())),
        Line::raw(format!("Entry ID       {}", reference.entry_id())),
        Line::raw(escaped_value_line(
            "Key            ",
            entry.display_key(),
            KEY_MAX_ESCAPED_BYTES,
            width,
        )),
        Line::raw(format!("State          {state}")),
        Line::raw(format!("Version        {}", reference.version().get())),
        Line::from(action_spans),
        Line::styled(
            "Left/Right: action | Enter: open | Esc: entries",
            theme.focus,
        ),
        Line::default(),
        Line::styled("VALUE", theme.accent),
        Line::raw(memory_escape_bounded(
            entry.value().unwrap_or("Unavailable"),
            VALUE_MAX_ESCAPED_BYTES,
        )),
        Line::raw(format!(
            "Purpose tags   {}",
            if tags.is_empty() { "None" } else { &tags }
        )),
        Line::raw(format!("Created by     {}", actor_name(entry.created_by()))),
        Line::raw(format!("Created ms     {}", entry.created_at_ms())),
        Line::raw(format!("Event ID       {}", entry.creation_event_id())),
        Line::raw(format!("Content digest {}", entry.content_digest())),
        Line::raw(format!("Record digest  {}", entry.record_digest())),
    ];
    if let Some(predecessor) = entry.predecessor_version_id() {
        lines.push(Line::raw(format!("Predecessor    {predecessor}")));
    }
    PanelContent {
        title: "Entry detail",
        lines,
        scroll: model.agents.memory.detail_scroll,
    }
}

fn entry_context_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some(detail) = authenticated_entry_detail(model) else {
        return unavailable_panel(
            "Entry context",
            "Entry context unavailable.",
            "Reload the exact selected entry.",
            theme,
        );
    };
    let entry = &detail.entry;
    PanelContent {
        title: "Entry context",
        lines: vec![
            Line::styled("AVAILABLE ACTIONS", theme.accent),
            Line::raw("Edit: reviewed immutable successor"),
            Line::raw("Delete: reviewed tombstone"),
            Line::raw("History: immutable versions"),
            Line::default(),
            Line::raw(format!(
                "History {}",
                if entry.predecessor_version_id().is_some() {
                    "available"
                } else {
                    "starts at this version"
                }
            )),
        ],
        scroll: 0,
    }
}

fn authenticated_entry_detail(model: &TuiModel) -> Option<&crate::app::MemoryEntryView> {
    model.agents.memory.authenticated_entry_detail()
}

fn history_list_content(model: &TuiModel, area: Rect, theme: &Theme) -> PanelContent {
    let memory = &model.agents.memory;
    let Some((history, counts)) = authenticated_history(model) else {
        return unavailable_panel(
            "Entry history",
            "Entry history unavailable.",
            "Reload the exact selected entry history.",
            theme,
        );
    };
    if counts.displayed > 0
        && model
            .agents
            .memory
            .authenticated_selected_history()
            .is_none()
    {
        return unavailable_panel(
            "Entry history",
            "Entry history unavailable.",
            "Reload the exact selected entry history.",
            theme,
        );
    }
    let retained = &history.versions[..counts.displayed];
    let selected = memory.selected_history_version;
    let visible_items = memory_list_visible_items(area);
    let first = first_visible_item(
        memory.history_scroll,
        selected,
        retained.len(),
        visible_items,
    );
    let width = panel_inner_width(area);
    let mut lines = vec![
        memory_heading(theme),
        Line::raw(bounded_ascii_line(
            &format!("Entry ID       {}", history.current.entry_id()),
            width,
        )),
    ];
    if retained.is_empty() {
        lines.push(Line::styled(
            bounded_ascii_line("No entry versions returned", width),
            theme.warning,
        ));
    } else {
        for (index, version) in retained.iter().enumerate().skip(first).take(visible_items) {
            let prefix = format!(
                "{} v{} ",
                if index == selected { ">" } else { " " },
                version.entry.version().get()
            );
            lines.push(Line::styled(
                escaped_value_line(&prefix, &version.display_key, KEY_MAX_ESCAPED_BYTES, width),
                if index == selected {
                    theme.focus
                } else {
                    theme.accent
                },
            ));
        }
        lines.push(Line::styled(
            bounded_ascii_line(
                "Up/Down: select | Enter: load exact version | Esc: entry detail",
                width,
            ),
            theme.muted,
        ));
    }
    PanelContent {
        title: "Entry history",
        lines,
        scroll: 0,
    }
}

fn history_detail_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    match authenticated_history_version(model) {
        Ok(Some(detail)) => entry_version_content(model, detail, width, theme),
        Ok(None) => history_selection_content(model, theme),
        Err(()) => unavailable_panel(
            "Historical entry version",
            "Historical version unavailable.",
            "Reload the exact selected history version.",
            theme,
        ),
    }
}

fn history_selection_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some((history, counts)) = authenticated_history(model) else {
        return unavailable_panel(
            "History version",
            "History selection unavailable.",
            "Reload the exact selected entry history.",
            theme,
        );
    };
    let Some(selected) = history
        .versions
        .get(model.agents.memory.selected_history_version)
        .filter(|_| model.agents.memory.selected_history_version < counts.displayed)
    else {
        return static_panel("History version", "No history version is selected.", theme);
    };
    PanelContent {
        title: "History version",
        lines: vec![
            memory_heading(theme),
            Line::raw(format!("Entry ID       {}", selected.entry.entry_id())),
            Line::raw(format!("Version        {}", selected.entry.version().get())),
            Line::raw(format!(
                "Version ID     {}",
                selected.entry.entry_version_id()
            )),
            Line::raw(format!("Created ms     {}", selected.created_at_ms)),
            Line::styled("Enter: load exact version | Esc: entry detail", theme.focus),
        ],
        scroll: 0,
    }
}

fn entry_version_content(
    model: &TuiModel,
    detail: &crate::app::MemoryEntryVersionView,
    width: usize,
    theme: &Theme,
) -> PanelContent {
    let entry = &detail.entry;
    let reference = entry.reference();
    PanelContent {
        title: "Historical entry version",
        lines: vec![
            memory_heading(theme),
            Line::raw(format!("Profile        {}", detail.profile.profile_id())),
            Line::raw(format!("Entry ID       {}", reference.entry_id())),
            Line::raw(format!("Version        {}", reference.version().get())),
            Line::raw(format!("Version ID     {}", reference.entry_version_id())),
            Line::raw(escaped_value_line(
                "Key            ",
                entry.display_key(),
                KEY_MAX_ESCAPED_BYTES,
                width,
            )),
            Line::styled("Read-only historical version | Esc: history", theme.focus),
            Line::styled("VALUE", theme.accent),
            Line::raw(memory_escape_bounded(
                entry.value().unwrap_or("Deleted"),
                VALUE_MAX_ESCAPED_BYTES,
            )),
        ],
        scroll: model.agents.memory.detail_scroll,
    }
}

fn history_context_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some((history, counts)) = authenticated_history(model) else {
        return unavailable_panel(
            "Parent entry",
            "Parent entry unavailable.",
            "Reload the exact selected history.",
            theme,
        );
    };
    let mut lines = vec![
        Line::styled("PARENT ENTRY", theme.accent),
        Line::raw(format!("Entry ID {}", history.current.entry_id())),
        Line::raw(format!("Versions {}", counts.displayed)),
        Line::raw(format!("Omitted {}", counts.omitted)),
    ];
    if let Ok(Some(detail)) = authenticated_history_version(model) {
        lines.push(Line::raw(format!(
            "Created by {}",
            actor_name(detail.entry.created_by())
        )));
        lines.push(Line::raw(format!(
            "Event ID {}",
            detail.entry.creation_event_id()
        )));
    }
    PanelContent {
        title: "Parent entry",
        lines,
        scroll: 0,
    }
}

fn authenticated_history(
    model: &TuiModel,
) -> Option<(&crate::app::MemoryEntryHistoryView, MemoryPageCounts)> {
    model.agents.memory.authenticated_history()
}

fn authenticated_history_version(
    model: &TuiModel,
) -> Result<Option<&crate::app::MemoryEntryVersionView>, ()> {
    model.agents.memory.authenticated_history_version()
}

fn entry_counts_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some((_, counts)) = authenticated_entries(model) else {
        return unavailable_panel(
            "Memory context",
            "Memory counts unavailable.",
            "Reload the exact profile memory.",
            theme,
        );
    };
    PanelContent {
        title: "Memory context",
        lines: vec![
            Line::styled("ENTRY COUNTS", theme.accent),
            Line::raw(format!("Displayed {}", counts.displayed)),
            Line::raw(format!("Omitted {}", counts.omitted)),
            Line::raw(format!("Total {}", counts.total)),
            Line::default(),
            Line::styled("Enter: detail | c: create", theme.muted),
            Line::styled(
                "p: proposals | e: episodic | Esc: agent detail",
                theme.muted,
            ),
        ],
        scroll: 0,
    }
}

fn authenticated_entries(
    model: &TuiModel,
) -> Option<(&crate::app::MemoryEntriesView, MemoryPageCounts)> {
    model.agents.memory.authenticated_entries()
}

fn proposal_list_content(model: &TuiModel, area: Rect, theme: &Theme) -> PanelContent {
    let memory = &model.agents.memory;
    let Some((page, counts)) = authenticated_proposals(model) else {
        return unavailable_panel(
            "Memory proposals",
            "Memory proposals unavailable.",
            "Reload proposals for the exact profile.",
            theme,
        );
    };
    if counts.displayed > 0
        && model
            .agents
            .memory
            .authenticated_selected_proposal()
            .is_none()
    {
        return unavailable_panel(
            "Memory proposals",
            "Memory proposals unavailable.",
            "Reload proposals for the exact profile.",
            theme,
        );
    }
    let retained = &page.proposals[..counts.displayed];
    let selected = memory.selected_proposal;
    let visible_items = memory_list_visible_items(area);
    let first = first_visible_item(
        memory.proposal_scroll,
        selected,
        retained.len(),
        visible_items,
    );
    let width = panel_inner_width(area);
    let mut lines = vec![
        memory_heading(theme),
        profile_identity_line(model, width, theme),
    ];
    if retained.is_empty() {
        lines.extend([
            Line::styled(
                bounded_ascii_line("No proposals returned", width),
                theme.accent,
            ),
            Line::styled(
                bounded_ascii_line("Esc: memory entries", width),
                theme.focus,
            ),
        ]);
    } else {
        for (index, proposal) in retained.iter().enumerate().skip(first).take(visible_items) {
            let prefix = if index == selected { "> " } else { "  " };
            lines.push(Line::styled(
                escaped_value_line(prefix, &proposal.display_key, KEY_MAX_ESCAPED_BYTES, width),
                if index == selected {
                    theme.focus
                } else {
                    theme.accent
                },
            ));
            lines.push(Line::styled(
                bounded_ascii_line(
                    &format!(
                        "  {} | {} | v{}",
                        proposal_operation_kind_name(proposal.operation),
                        proposal_status_name(proposal.status),
                        proposal.proposal.version().get()
                    ),
                    width,
                ),
                theme.muted,
            ));
        }
        lines.push(Line::styled(
            bounded_ascii_line(
                "Up/Down: select | Enter: detail | Esc: memory entries",
                width,
            ),
            theme.muted,
        ));
    }
    PanelContent {
        title: "Memory proposals",
        lines,
        scroll: 0,
    }
}

fn proposal_preview_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some(summary) = authenticated_selected_proposal(model) else {
        return unavailable_panel(
            "Proposal metadata",
            "Proposal metadata unavailable.",
            "Select a proposal and reload its exact detail.",
            theme,
        );
    };
    PanelContent {
        title: "Proposal metadata",
        lines: vec![
            memory_heading(theme),
            Line::raw(format!("Proposal ID    {}", summary.proposal.proposal_id())),
            Line::raw(format!(
                "Operation      {}",
                proposal_operation_kind_name(summary.operation)
            )),
            Line::raw(format!(
                "Status         {}",
                proposal_status_name(summary.status)
            )),
            Line::raw(format!("Created ms     {}", summary.created_at_ms)),
            Line::styled(
                "Enter: load exact detail | Esc: memory entries",
                theme.focus,
            ),
        ],
        scroll: 0,
    }
}

fn proposal_counts_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some((page, counts)) = authenticated_proposals(model) else {
        return unavailable_panel(
            "Proposal context",
            "Proposal counts unavailable.",
            "Reload proposals for the exact profile.",
            theme,
        );
    };
    PanelContent {
        title: "Proposal context",
        lines: vec![
            Line::styled("PROPOSAL COUNTS", theme.accent),
            Line::raw(format!("Filter {}", proposal_filter_name(page.filter))),
            Line::raw(format!("Displayed {}", counts.displayed)),
            Line::raw(format!("Omitted {}", counts.omitted)),
            Line::raw(format!("Total {}", counts.total)),
            Line::styled("Enter: detail | Esc: memory entries", theme.muted),
        ],
        scroll: 0,
    }
}

fn proposal_detail_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    let Some(detail) = authenticated_proposal_detail(model) else {
        return unavailable_panel(
            "Proposal detail",
            "Proposal detail unavailable.",
            "Select the proposal and reload its exact detail.",
            theme,
        );
    };
    let proposal = &detail.proposal;
    let reference = proposal.reference();
    let mut action_spans = Vec::new();
    for (index, (action, label)) in [
        (
            crate::ui::tui::model::MemoryProposalDetailAction::Approve,
            "Approve",
        ),
        (
            crate::ui::tui::model::MemoryProposalDetailAction::Reject,
            "Reject",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            action_spans.push(Span::raw(" "));
        }
        action_spans.push(Span::styled(
            format!("[{label}]"),
            if model.agents.memory.selected_proposal_detail_action == action {
                theme.focus
            } else {
                theme.muted
            },
        ));
    }
    let mut lines = vec![
        memory_heading(theme),
        Line::raw(format!(
            "Profile        {}",
            detail.namespace_owner_identity.profile.profile_id()
        )),
        Line::raw(format!("Proposal ID    {}", reference.proposal_id())),
        Line::raw(escaped_value_line(
            "Key            ",
            proposal.display_key(),
            KEY_MAX_ESCAPED_BYTES,
            width,
        )),
        Line::raw(format!(
            "Operation      {}",
            proposal_operation_name(proposal.operation())
        )),
        Line::raw(format!(
            "Status         {}",
            proposal_status_name(detail.status)
        )),
        Line::from(action_spans),
        Line::styled(
            "Left/Right: action | Enter: review | Esc: proposals",
            theme.focus,
        ),
        Line::default(),
    ];
    match proposal.operation() {
        MemoryProposalOperation::Set { candidate } => {
            lines.push(Line::styled("CANDIDATE VALUE", theme.accent));
            lines.push(Line::raw(memory_escape_bounded(
                candidate.value(),
                VALUE_MAX_ESCAPED_BYTES,
            )));
            let tags = candidate
                .purpose_tags()
                .iter()
                .take(MAX_PURPOSE_TAGS)
                .map(|tag| memory_escape_bounded(tag, TAG_MAX_ESCAPED_BYTES))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(Line::raw(format!(
                "Purpose tags   {}",
                if tags.is_empty() { "None" } else { &tags }
            )));
        }
        MemoryProposalOperation::Delete => {
            lines.push(Line::styled("Candidate is an exact delete.", theme.warning));
        }
    }
    lines.extend([
        Line::styled("RATIONALE", theme.accent),
        Line::raw(memory_escape_bounded(proposal.rationale(), 2_048)),
        Line::raw(format!(
            "Proposer       {}",
            proposal.proposer().profile_id()
        )),
        Line::raw(format!("Created ms     {}", proposal.created_at_ms())),
        Line::raw(format!("Approval ID    {}", proposal.approval_id())),
    ]);
    PanelContent {
        title: "Proposal detail",
        lines,
        scroll: model.agents.memory.detail_scroll,
    }
}

fn proposal_context_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some(detail) = authenticated_proposal_detail(model) else {
        return unavailable_panel(
            "Proposal context",
            "Proposal context unavailable.",
            "Reload the exact selected proposal.",
            theme,
        );
    };
    PanelContent {
        title: "Proposal context",
        lines: vec![
            Line::styled("RESOLUTION", theme.accent),
            Line::raw("Approve applies the exact candidate."),
            Line::raw("Reject records a terminal decision."),
            Line::default(),
            Line::raw(format!(
                "Current entry {}",
                expected_state_name(&detail.current_entry)
            )),
            Line::raw(format!(
                "Historical proposer {}",
                detail.proposer_is_historical
            )),
        ],
        scroll: 0,
    }
}

fn proposal_resolution_review_content(
    model: &TuiModel,
    width: usize,
    theme: &Theme,
) -> PanelContent {
    let Some(review) = authenticated_resolution_review(model) else {
        return unavailable_panel(
            "Proposal resolution review",
            "Proposal resolution review unavailable.",
            "Reload and request a new exact proposal review.",
            theme,
        );
    };
    let action = resolution_action_name(review.action);
    let proposal = &review.proposal;
    let proposal_reference = proposal.reference();
    let mut lines = fixed_safety_lines(
        review.namespace_owner_identity.profile.profile_id(),
        action,
        &proposal_reference.proposal_id().to_string(),
        proposal.display_key(),
        "Enter: continue | Esc: cancel review",
        width,
        theme,
    );
    lines.extend([
        Line::styled("OWNER AND PROPOSER", theme.accent),
        Line::raw(format!("Action          {action}")),
        Line::raw(format!(
            "Proposal ID     {}",
            proposal_reference.proposal_id()
        )),
        Line::raw(format!(
            "Owner name      {}",
            memory_escape_bounded(
                &review.namespace_owner_identity.display_name,
                PROFILE_LABEL_MAX_ESCAPED_BYTES,
            )
        )),
        Line::raw(format!(
            "Proposer name   {}",
            memory_escape_bounded(
                &review.proposer_identity.display_name,
                PROFILE_LABEL_MAX_ESCAPED_BYTES,
            )
        )),
    ]);
    if review.proposer_is_historical {
        lines.push(Line::styled(
            "proposer version is historical",
            theme.warning,
        ));
    }
    lines.extend(profile_reference_lines(
        "Proposer",
        &review.proposer_identity.profile,
    ));
    lines.extend(profile_reference_lines(
        "Owner",
        &review.namespace_owner_identity.profile,
    ));
    lines.extend([
        Line::raw(format!("Namespace ID    {}", proposal.namespace_id())),
        Line::styled("PROPOSAL", theme.accent),
        Line::raw(format!(
            "Proposal version {}",
            proposal_reference.version().get()
        )),
        Line::raw(format!(
            "Proposal digest {}",
            proposal_reference.content_digest()
        )),
        Line::raw(format!(
            "Underlying operation {}",
            proposal_operation_name(proposal.operation())
        )),
        Line::raw(escaped_value_line(
            "Display key     ",
            proposal.display_key(),
            KEY_MAX_ESCAPED_BYTES,
            width,
        )),
        Line::raw(escaped_value_line(
            "Normalized key  ",
            proposal.normalized_key().as_str(),
            KEY_MAX_ESCAPED_BYTES,
            width,
        )),
        Line::styled("PROPOSAL EXPECTED CURRENT", theme.accent),
    ]);
    lines.extend(expected_entry_reference_lines(
        "Proposal expected",
        proposal.expected(),
    ));
    lines.push(Line::styled("RE-RESOLVED CURRENT", theme.accent));
    lines.extend(expected_entry_reference_lines(
        "Review current",
        &review.expected_entry,
    ));
    if let MemoryProposalOperation::Set { candidate } = proposal.operation() {
        lines.extend([
            Line::styled("PROPOSED CANDIDATE", theme.accent),
            Line::raw(format!(
                "Proposed value  {}",
                memory_escape_bounded(candidate.value(), VALUE_MAX_ESCAPED_BYTES)
            )),
        ]);
        for (index, tag) in candidate
            .purpose_tags()
            .iter()
            .take(MAX_PURPOSE_TAGS)
            .enumerate()
        {
            lines.push(Line::raw(format!(
                "Purpose tag {} {}",
                index + 1,
                memory_escape_bounded(tag, TAG_MAX_ESCAPED_BYTES)
            )));
        }
    }
    lines.extend([
        Line::styled("RATIONALE", theme.accent),
        Line::raw(memory_escape_bounded(proposal.rationale(), 2_048)),
        Line::styled("APPROVAL", theme.accent),
        Line::raw(format!("Approval ID     {}", review.approval_id)),
        Line::raw(format!(
            "Expected approval {}",
            approval_status_name(review.expected_approval_status)
        )),
        Line::raw(format!("Review digest   {}", review.review_digest)),
    ]);
    PanelContent {
        title: "Proposal resolution review",
        lines,
        scroll: model.agents.memory.detail_scroll,
    }
}

fn authenticated_proposals(
    model: &TuiModel,
) -> Option<(&crate::app::MemoryProposalsView, MemoryPageCounts)> {
    model.agents.memory.authenticated_proposals()
}

fn authenticated_selected_proposal(model: &TuiModel) -> Option<&crate::app::MemoryProposalSummary> {
    model.agents.memory.authenticated_selected_proposal()
}

fn authenticated_proposal_detail(model: &TuiModel) -> Option<&crate::app::MemoryProposalView> {
    model.agents.memory.authenticated_proposal_detail()
}

fn authenticated_resolution_review(
    model: &TuiModel,
) -> Option<&crate::app::MemoryProposalResolutionReview> {
    model.agents.memory.authenticated_resolution_review()
}

fn proposal_operation_name(operation: &MemoryProposalOperation) -> &'static str {
    match operation {
        MemoryProposalOperation::Set { .. } => "Set",
        MemoryProposalOperation::Delete => "Delete",
    }
}

fn proposal_operation_kind_name(operation: MemoryProposalOperationKind) -> &'static str {
    match operation {
        MemoryProposalOperationKind::Set => "Set",
        MemoryProposalOperationKind::Delete => "Delete",
    }
}

fn proposal_status_name(status: MemoryProposalStatus) -> &'static str {
    match status {
        MemoryProposalStatus::Pending => "Pending",
        MemoryProposalStatus::Accepted => "Accepted",
        MemoryProposalStatus::Rejected => "Rejected",
        MemoryProposalStatus::Expired => "Expired",
    }
}

fn proposal_filter_name(filter: MemoryProposalFilter) -> &'static str {
    match filter {
        MemoryProposalFilter::Pending => "Pending",
        MemoryProposalFilter::All => "All",
    }
}

fn resolution_action_name(action: MemoryResolutionAction) -> &'static str {
    match action {
        MemoryResolutionAction::Approve => "Approve",
        MemoryResolutionAction::Reject => "Reject",
    }
}

fn expected_state_name(expected: &ExpectedMemoryEntryState) -> &'static str {
    match expected {
        ExpectedMemoryEntryState::Absent => "Absent",
        ExpectedMemoryEntryState::Present(_) => "Present",
        ExpectedMemoryEntryState::Deleted(_) => "Deleted",
    }
}

fn profile_reference_lines(
    label: &str,
    profile: &crate::agents::AgentProfileVersionRef,
) -> Vec<Line<'static>> {
    vec![
        Line::raw(format!("{label} profile ID {}", profile.profile_id())),
        Line::raw(format!(
            "{label} version ID {}",
            profile.profile_version_id()
        )),
        Line::raw(format!("{label} version {}", profile.version().get())),
        Line::raw(format!("{label} digest {}", profile.content_digest())),
    ]
}

fn expected_entry_reference_lines(
    label: &str,
    expected: &ExpectedMemoryEntryState,
) -> Vec<Line<'static>> {
    let (state, reference) = match expected {
        ExpectedMemoryEntryState::Absent => {
            return vec![Line::raw(format!("{label} Absent"))];
        }
        ExpectedMemoryEntryState::Present(reference) => ("Present", reference),
        ExpectedMemoryEntryState::Deleted(reference) => ("Deleted", reference),
    };
    vec![
        Line::raw(format!("{label} {state}")),
        Line::raw(format!("{label} entry ID {}", reference.entry_id())),
        Line::raw(format!(
            "{label} version ID {}",
            reference.entry_version_id()
        )),
        Line::raw(format!("{label} version {}", reference.version().get())),
        Line::raw(format!("{label} digest {}", reference.content_digest())),
    ]
}

fn episodic_list_content(model: &TuiModel, area: Rect, theme: &Theme) -> PanelContent {
    let memory = &model.agents.memory;
    let Some((page, counts)) = authenticated_episodes(model) else {
        return unavailable_panel(
            "Episodic summaries",
            "Episodic summaries unavailable.",
            "Reload summaries for the exact profile.",
            theme,
        );
    };
    if counts.displayed > 0
        && model
            .agents
            .memory
            .authenticated_selected_episode()
            .is_none()
    {
        return unavailable_panel(
            "Episodic summaries",
            "Episodic summaries unavailable.",
            "Reload summaries for the exact profile.",
            theme,
        );
    }
    let retained = &page.summaries[..counts.displayed];
    let selected = memory.selected_episode;
    let visible_items = memory_list_visible_items(area);
    let first = first_visible_item(
        memory.episode_scroll,
        selected,
        retained.len(),
        visible_items,
    );
    let width = panel_inner_width(area);
    let mut lines = vec![
        memory_heading(theme),
        profile_identity_line(model, width, theme),
    ];
    if retained.is_empty() {
        lines.extend([
            Line::styled(
                bounded_ascii_line("No episodic summaries returned", width),
                theme.accent,
            ),
            Line::styled(
                bounded_ascii_line("Esc: memory entries", width),
                theme.focus,
            ),
        ]);
    } else {
        for (index, summary) in retained.iter().enumerate().skip(first).take(visible_items) {
            let prefix = if index == selected { "> " } else { "  " };
            lines.push(Line::styled(
                escaped_value_line(
                    prefix,
                    &summary.label,
                    EPISODIC_LABEL_MAX_ESCAPED_BYTES,
                    width,
                ),
                if index == selected {
                    theme.focus
                } else {
                    theme.accent
                },
            ));
            lines.push(Line::styled(
                bounded_ascii_line(
                    &format!(
                        "  Sources {} | {} tags",
                        summary.source_count,
                        summary.purpose_tags.len()
                    ),
                    width,
                ),
                theme.muted,
            ));
        }
        lines.push(Line::styled(
            bounded_ascii_line("Up/Down: select | Enter: read | Esc: memory entries", width),
            theme.muted,
        ));
    }
    PanelContent {
        title: "Episodic summaries",
        lines,
        scroll: 0,
    }
}

fn episodic_preview_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some(summary) = authenticated_selected_episode(model) else {
        return unavailable_panel(
            "Summary metadata",
            "Summary metadata unavailable.",
            "Select a summary and reload its exact detail.",
            theme,
        );
    };
    PanelContent {
        title: "Summary metadata",
        lines: vec![
            memory_heading(theme),
            Line::raw(format!("Summary ID     {}", summary.summary.summary_id())),
            Line::raw(format!(
                "Version        {}",
                summary.summary.version().get()
            )),
            Line::raw(format!("Sources        {}", summary.source_count)),
            Line::raw(format!("Created ms     {}", summary.created_at_ms)),
            Line::styled(
                "Enter: read exact summary | Esc: memory entries",
                theme.focus,
            ),
        ],
        scroll: 0,
    }
}

fn episodic_counts_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some((_, counts)) = authenticated_episodes(model) else {
        return unavailable_panel(
            "Episodic context",
            "Episodic counts unavailable.",
            "Reload summaries for the exact profile.",
            theme,
        );
    };
    PanelContent {
        title: "Episodic context",
        lines: vec![
            Line::styled("SUMMARY COUNTS", theme.accent),
            Line::raw(format!("Displayed {}", counts.displayed)),
            Line::raw(format!("Omitted {}", counts.omitted)),
            Line::raw(format!("Total {}", counts.total)),
            Line::styled("Summaries are read-only retrieval aids.", theme.muted),
        ],
        scroll: 0,
    }
}

fn episodic_detail_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    let Some(detail) = authenticated_episode_detail(model) else {
        return unavailable_panel(
            "Episodic detail",
            "Episodic detail unavailable.",
            "Select the summary and reload its exact detail.",
            theme,
        );
    };
    let summary = &detail.summary;
    let reference = summary.reference();
    let tags = summary
        .purpose_tags()
        .iter()
        .take(MAX_PURPOSE_TAGS)
        .map(|tag| memory_escape_bounded(tag, TAG_MAX_ESCAPED_BYTES))
        .collect::<Vec<_>>()
        .join(", ");
    PanelContent {
        title: "Episodic detail",
        lines: vec![
            memory_heading(theme),
            Line::raw(format!(
                "Profile        {}",
                reference.profile().profile_id()
            )),
            Line::raw(format!("Summary ID     {}", reference.summary_id())),
            Line::raw(escaped_value_line(
                "Label          ",
                summary.label(),
                EPISODIC_LABEL_MAX_ESCAPED_BYTES,
                width,
            )),
            Line::styled(detail.qualification.label(), theme.warning),
            Line::styled("Read-only | Esc: episodic summaries", theme.focus),
            Line::default(),
            Line::styled("SUMMARY BODY", theme.accent),
            Line::raw(memory_escape_bounded(
                summary.body(),
                EPISODIC_BODY_MAX_ESCAPED_BYTES,
            )),
            Line::raw(format!(
                "Purpose tags   {}",
                if tags.is_empty() { "None" } else { &tags }
            )),
            Line::raw(format!("Sources        {}", summary.sources().len())),
            Line::raw(format!("Created ms     {}", summary.created_at_ms())),
        ],
        scroll: model.agents.memory.detail_scroll,
    }
}

fn episodic_sources_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some(detail) = authenticated_episode_detail(model) else {
        return unavailable_panel(
            "Episodic sources",
            "Episodic sources unavailable.",
            "Reload the exact selected summary.",
            theme,
        );
    };
    let mut lines = vec![
        Line::styled("SOURCE REFERENCES", theme.accent),
        Line::styled("Summary — verify sources", theme.warning),
        Line::styled("Read-only evidence links", theme.muted),
    ];
    for source in detail.summary.sources().iter().take(MAX_EPISODIC_SOURCES) {
        lines.extend([
            Line::raw(format!("Sequence {}", source.sequence())),
            Line::raw(format!(
                "Type {}",
                memory_escape_bounded(source.event_type(), EPISODIC_EVENT_TYPE_MAX_ESCAPED_BYTES)
            )),
            Line::raw(format!("Event {}", source.event_id())),
            Line::raw(format!("Digest {}", source.event_digest())),
        ]);
    }
    PanelContent {
        title: "Episodic sources",
        lines,
        scroll: model.agents.memory.detail_scroll,
    }
}

fn authenticated_episodes(
    model: &TuiModel,
) -> Option<(&crate::app::EpisodicSummariesView, MemoryPageCounts)> {
    model.agents.memory.authenticated_episodes()
}

fn authenticated_selected_episode(
    model: &TuiModel,
) -> Option<&crate::app::EpisodicSummaryListItem> {
    model.agents.memory.authenticated_selected_episode()
}

fn authenticated_episode_detail(model: &TuiModel) -> Option<&crate::app::EpisodicSummaryView> {
    model.agents.memory.authenticated_episode_detail()
}

fn editor_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    let memory = &model.agents.memory;
    if !memory.local_layer_cache_is_authenticated() {
        return unavailable_panel(
            "Memory editor",
            "Memory editor unavailable.",
            "Return to the exact profile and reopen the editor.",
            theme,
        );
    }
    let Some(editor) = memory.editor.as_ref() else {
        return unavailable_panel(
            "Memory editor",
            "Memory editor unavailable.",
            "Return to the exact profile and reopen the editor.",
            theme,
        );
    };
    let Some(profile) = memory.profile.as_ref() else {
        return unavailable_panel(
            "Memory editor",
            "Memory editor unavailable.",
            "Return to the exact profile and reopen the editor.",
            theme,
        );
    };
    let (stage, controls) = match editor.step() {
        MemoryEditorStep::Key => ("Key", "Enter: continue | Esc: cancel"),
        MemoryEditorStep::Value => ("Value", "Enter: continue | Esc: back"),
        MemoryEditorStep::PurposeTags => ("Purpose tags", "Enter: review | Esc: back"),
        MemoryEditorStep::Review => ("Review", "Enter: no change | Esc: revise"),
    };
    let key = if editor.key_input().is_empty() {
        "New entry"
    } else {
        editor.key_input()
    };
    let stable_identity = match memory.editor_origin {
        crate::ui::tui::model::MemoryEditorOrigin::Create => "New entry".to_owned(),
        crate::ui::tui::model::MemoryEditorOrigin::Edit => {
            let Some(seed) = editor.seed() else {
                return unavailable_panel(
                    "Memory editor",
                    "Memory editor unavailable.",
                    "Return to the exact profile and reopen the editor.",
                    theme,
                );
            };
            seed.reference().entry_id().to_string()
        }
    };
    let mut lines = fixed_safety_lines(
        profile.profile.profile_id(),
        &format!("Set/{stage}"),
        &stable_identity,
        key,
        controls,
        width,
        theme,
    );
    lines.push(Line::raw(format!("Stage          {stage}")));
    match editor.step() {
        MemoryEditorStep::Key => {
            lines.push(Line::styled(
                "Enter the key in the Memory input bar.",
                theme.muted,
            ));
        }
        MemoryEditorStep::Value => {
            lines.push(Line::styled(
                "Enter the value in the Memory input bar.",
                theme.muted,
            ));
        }
        MemoryEditorStep::PurposeTags => {
            lines.push(Line::styled(
                "Enter optional comma-separated purpose tags.",
                theme.muted,
            ));
            if !editor.value_input().is_empty() {
                lines.push(Line::raw(format!(
                    "Value          {}",
                    memory_escape_bounded(editor.value_input(), VALUE_MAX_ESCAPED_BYTES)
                )));
            }
        }
        MemoryEditorStep::Review => match editor.preview() {
            Some(MemoryEditPreview::NoChange(reason)) => {
                lines.push(Line::styled("No memory change", theme.accent));
                lines.push(Line::raw(match reason {
                    MemoryNoChange::IdenticalContent => "Identical content",
                    MemoryNoChange::AlreadyAbsent => "Already absent",
                }));
            }
            _ => {
                return unavailable_panel(
                    "Memory editor",
                    "Memory editor review unavailable.",
                    "Request a new exact review.",
                    theme,
                );
            }
        },
    }
    if let Some(code) = editor.safe_error_code() {
        lines.push(Line::styled(
            format!("Validation      {}", memory_escape_bounded(code, 512)),
            theme.warning,
        ));
    }
    PanelContent {
        title: "Memory editor",
        lines,
        scroll: model.agents.memory.detail_scroll,
    }
}

fn editor_context_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    let memory = &model.agents.memory;
    if !memory.local_layer_cache_is_authenticated() {
        return unavailable_panel(
            "Editor context",
            "Memory editor context unavailable.",
            "Return to the exact profile and reopen the editor.",
            theme,
        );
    }
    let Some(profile) = memory.profile.as_ref() else {
        return unavailable_panel(
            "Editor context",
            "Memory editor context unavailable.",
            "Return to the exact profile and reopen the editor.",
            theme,
        );
    };
    let Some(editor) = memory.editor.as_ref() else {
        return unavailable_panel(
            "Editor context",
            "Memory editor context unavailable.",
            "Return to the exact profile and reopen the editor.",
            theme,
        );
    };
    let mut lines = vec![
        Line::styled("EDITOR CONTEXT", theme.accent),
        Line::raw(format!("Profile {}", profile.profile.profile_id())),
    ];
    match memory.editor_origin {
        crate::ui::tui::model::MemoryEditorOrigin::Create => {
            lines.push(Line::raw("Origin Create"));
            lines.push(Line::raw("Entry New entry"));
        }
        crate::ui::tui::model::MemoryEditorOrigin::Edit => {
            let Some(seed) = editor.seed() else {
                return unavailable_panel(
                    "Editor context",
                    "Memory editor context unavailable.",
                    "Return to the exact profile and reopen the editor.",
                    theme,
                );
            };
            let reference = seed.reference();
            lines.extend([
                Line::raw("Origin Edit"),
                Line::raw(format!("Entry ID {}", reference.entry_id())),
                Line::raw(format!("Version {}", reference.version().get())),
                Line::raw(escaped_value_line(
                    "Key ",
                    seed.display_key(),
                    KEY_MAX_ESCAPED_BYTES,
                    width,
                )),
            ]);
        }
    }
    PanelContent {
        title: "Editor context",
        lines,
        scroll: memory.detail_scroll,
    }
}

fn mutation_diff_content(model: &TuiModel, title: &'static str, theme: &Theme) -> PanelContent {
    let memory = &model.agents.memory;
    if memory.edit_review_command().is_none() {
        return unavailable_panel(
            title,
            "Memory review context unavailable.",
            "Reload and request a new exact review.",
            theme,
        );
    }
    let Some(review) = memory.edit_review.as_ref() else {
        return unavailable_panel(
            title,
            "Memory review context unavailable.",
            "Reload and request a new exact review.",
            theme,
        );
    };
    let lines = bounded_mutation_diff_lines(
        review,
        if title == "Retained mutation review" {
            "RETAINED MUTATION REVIEW"
        } else {
            "BOUNDED MUTATION DIFF"
        },
        theme,
    );
    PanelContent {
        title,
        lines,
        scroll: memory.detail_scroll,
    }
}

fn bounded_mutation_diff_lines(
    review: &crate::memory::MemoryEditReview,
    heading: &'static str,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let displayed = review.diff.len().min(LIST_ITEM_CAP);
    let omitted = review.diff.len().saturating_sub(displayed);
    let mut lines = vec![
        Line::styled(heading, theme.accent),
        Line::raw(format!("Displayed diff rows {displayed}")),
        Line::raw(format!("Omitted diff rows {omitted}")),
    ];
    if displayed == 0 {
        lines.push(Line::raw("No field differences returned."));
    } else {
        for diff in review.diff.iter().take(displayed) {
            let field = memory_field_name(diff.field);
            let before = rendered_memory_field_value(diff.field, &diff.before);
            let after = rendered_memory_field_value(diff.field, &diff.after);
            lines.push(Line::raw(format!("{field}: {before} -> {after}")));
        }
    }
    lines
}

fn memory_field_name(field: MemoryField) -> &'static str {
    match field {
        MemoryField::DisplayKey => "Display key",
        MemoryField::State => "State",
        MemoryField::Value => "Value",
        MemoryField::PurposeTags => "Purpose tags",
    }
}

fn rendered_memory_field_value(field: MemoryField, value: &MemoryFieldValue) -> String {
    match value {
        MemoryFieldValue::Missing => "missing".to_owned(),
        MemoryFieldValue::Text(value) => memory_escape_bounded(
            value,
            match field {
                MemoryField::DisplayKey => KEY_MAX_ESCAPED_BYTES,
                MemoryField::PurposeTags => TAG_MAX_ESCAPED_BYTES,
                MemoryField::State => PROFILE_LABEL_MAX_ESCAPED_BYTES,
                MemoryField::Value => VALUE_MAX_ESCAPED_BYTES,
            },
        ),
        MemoryFieldValue::Tags(tags) => tags
            .iter()
            .take(MAX_PURPOSE_TAGS)
            .map(|tag| memory_escape_bounded(tag, TAG_MAX_ESCAPED_BYTES))
            .collect::<Vec<_>>()
            .join(", "),
        MemoryFieldValue::State(state) => match state {
            crate::memory::MemoryEntryState::Present => "Present".to_owned(),
            crate::memory::MemoryEntryState::Deleted => "Deleted".to_owned(),
        },
    }
}

fn approval_status_name(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "Pending",
        ApprovalStatus::Accepted => "Accepted",
        ApprovalStatus::Rejected => "Rejected",
        ApprovalStatus::Expired => "Expired",
        ApprovalStatus::Cancelled => "Cancelled",
    }
}

fn resolution_context_content(
    model: &TuiModel,
    title: &'static str,
    theme: &Theme,
) -> PanelContent {
    let Some(review) = authenticated_resolution_review(model) else {
        return unavailable_panel(
            title,
            "Proposal review context unavailable.",
            "Reload and request a new exact proposal review.",
            theme,
        );
    };
    PanelContent {
        title,
        lines: vec![
            Line::styled(
                if title == "Retained resolution review" {
                    "RETAINED RESOLUTION REVIEW"
                } else {
                    "RESOLUTION TARGET"
                },
                theme.accent,
            ),
            Line::raw(format!(
                "Owner {}",
                review.namespace_owner_identity.profile.profile_id()
            )),
            Line::raw(format!(
                "Proposal ID {}",
                review.proposal.reference().proposal_id()
            )),
            Line::raw(format!("Approval ID {}", review.approval_id)),
            Line::raw(format!("Action {}", resolution_action_name(review.action))),
            Line::raw(format!(
                "Expected approval {}",
                approval_status_name(review.expected_approval_status)
            )),
            Line::raw(format!(
                "Expected entry {}",
                expected_state_name(&review.expected_entry)
            )),
            Line::raw(format!(
                "Proposer {}",
                review.proposer_identity.profile.profile_id()
            )),
            Line::raw(format!(
                "Historical proposer {}",
                review.proposer_is_historical
            )),
        ],
        scroll: model.agents.memory.detail_scroll,
    }
}

fn confirmation_context_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let Some(command) = model.agents.memory.confirmed_command() else {
        return unavailable_panel(
            "Confirmation context",
            "Memory confirmation context unavailable.",
            "Return to review and rebuild the exact confirmation.",
            theme,
        );
    };
    match command {
        ApplicationCommand::SetMemoryEntry { .. }
        | ApplicationCommand::DeleteMemoryEntry { .. } => {
            mutation_diff_content(model, "Retained mutation review", theme)
        }
        ApplicationCommand::ApproveMemoryProposal { .. }
        | ApplicationCommand::RejectMemoryProposal { .. } => {
            resolution_context_content(model, "Retained resolution review", theme)
        }
        _ => unavailable_panel(
            "Confirmation context",
            "Memory confirmation context unavailable.",
            "Return to review and rebuild the exact confirmation.",
            theme,
        ),
    }
}

fn result_context_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let guidance = match model.agents.memory.result_origin {
        crate::ui::tui::model::MemoryResultOrigin::Mutation => {
            "Reload memory entries to view current immutable state."
        }
        crate::ui::tui::model::MemoryResultOrigin::Resolution => {
            "Reload memory proposals to view current resolution state."
        }
    };
    PanelContent {
        title: "Result context",
        lines: vec![
            Line::styled("REFRESH", theme.accent),
            Line::raw(guidance),
            Line::raw("No result identifiers are reconstructed."),
        ],
        scroll: 0,
    }
}

fn mutation_review_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    let memory = &model.agents.memory;
    let Some(command) = memory.edit_review_command() else {
        return unavailable_panel(
            "Mutation review",
            "Memory review unavailable.",
            "Reload and request a new review.",
            theme,
        );
    };
    let (operation, entry_id, identity) = match command {
        ApplicationCommand::SetMemoryEntry {
            expected,
            candidate,
            ..
        } => (
            MemoryMutationKind::Set,
            expected_entry_identity(&expected),
            candidate.display_key().to_owned(),
        ),
        ApplicationCommand::DeleteMemoryEntry { expected, .. } => (
            MemoryMutationKind::Delete,
            expected.entry_id().to_string(),
            expected.normalized_key().as_str().to_owned(),
        ),
        _ => {
            return unavailable_panel(
                "Mutation review",
                "Memory review unavailable.",
                "Reload and request a new review.",
                theme,
            );
        }
    };
    let operation = match operation {
        MemoryMutationKind::Set => "Set",
        MemoryMutationKind::Delete => "Delete",
    };
    let Some(profile) = memory.profile.as_ref() else {
        return unavailable_panel(
            "Mutation review",
            "Memory review unavailable.",
            "Reload and request a new review.",
            theme,
        );
    };
    let mut lines = fixed_safety_lines(
        profile.profile.profile_id(),
        operation,
        &entry_id,
        &identity,
        "Enter: continue | Esc: cancel review",
        width,
        theme,
    );
    lines.extend([
        Line::raw(format!("Profile        {}", profile.profile.profile_id())),
        Line::raw(format!("Operation      {operation}")),
        Line::raw(format!("Entry ID       {entry_id}")),
        Line::raw(escaped_value_line(
            "Key            ",
            &identity,
            KEY_MAX_ESCAPED_BYTES,
            width,
        )),
    ]);
    let Some(review) = memory.edit_review.as_ref() else {
        return unavailable_panel(
            "Mutation review",
            "Memory review unavailable.",
            "Reload and request a new review.",
            theme,
        );
    };
    lines.extend(bounded_mutation_diff_lines(review, "MUTATION DIFF", theme));
    PanelContent {
        title: "Mutation review",
        lines,
        scroll: model.agents.memory.detail_scroll,
    }
}

fn confirmation_content(model: &TuiModel, width: usize, theme: &Theme) -> PanelContent {
    let memory = &model.agents.memory;
    let Some(command) = memory.confirmed_command() else {
        return unavailable_panel(
            "Confirm memory change",
            "Memory confirmation unavailable.",
            "Return to review and rebuild the exact confirmation.",
            theme,
        );
    };
    if let ApplicationCommand::ApproveMemoryProposal { proposal, .. } = &command {
        return proposal_confirmation_content(
            model,
            width,
            theme,
            MemoryResolutionAction::Approve,
            proposal,
        );
    }
    if let ApplicationCommand::RejectMemoryProposal { proposal, .. } = &command {
        return proposal_confirmation_content(
            model,
            width,
            theme,
            MemoryResolutionAction::Reject,
            proposal,
        );
    }
    let Some(profile) = memory.profile.as_ref() else {
        return unavailable_panel(
            "Confirm memory change",
            "Memory confirmation unavailable.",
            "Return to review and rebuild the exact confirmation.",
            theme,
        );
    };
    let (operation, entry_id, key) = match command {
        ApplicationCommand::SetMemoryEntry {
            expected,
            candidate,
            ..
        } => (
            "Set",
            expected_entry_identity(&expected),
            candidate.display_key().to_owned(),
        ),
        ApplicationCommand::DeleteMemoryEntry { expected, .. } => (
            "Delete",
            expected.entry_id().to_string(),
            expected.normalized_key().as_str().to_owned(),
        ),
        _ => {
            return unavailable_panel(
                "Confirm memory change",
                "Memory confirmation unavailable.",
                "Return to review and rebuild the exact confirmation.",
                theme,
            );
        }
    };
    PanelContent {
        title: "Confirm memory change",
        lines: fixed_safety_lines(
            profile.profile.profile_id(),
            operation,
            &entry_id,
            &key,
            "Enter: confirm | Esc: return to review",
            width,
            theme,
        ),
        scroll: model.agents.memory.detail_scroll,
    }
}

fn proposal_confirmation_content(
    model: &TuiModel,
    width: usize,
    theme: &Theme,
    action: MemoryResolutionAction,
    proposal_ref: &crate::memory::MemoryProposalRef,
) -> PanelContent {
    let Some(review) = authenticated_resolution_review(model)
        .filter(|review| review.action == action && review.proposal.reference() == *proposal_ref)
    else {
        return unavailable_panel(
            "Confirm proposal resolution",
            "Proposal confirmation unavailable.",
            "Return to review and rebuild the exact confirmation.",
            theme,
        );
    };
    let action = resolution_action_name(action);
    PanelContent {
        title: "Confirm proposal resolution",
        lines: fixed_safety_lines(
            review.namespace_owner_identity.profile.profile_id(),
            action,
            &review.proposal.reference().proposal_id().to_string(),
            review.proposal.display_key(),
            "Enter: confirm | Esc: return to review",
            width,
            theme,
        ),
        scroll: model.agents.memory.detail_scroll,
    }
}

fn expected_entry_identity(expected: &ExpectedMemoryEntryState) -> String {
    match expected {
        ExpectedMemoryEntryState::Absent => "New entry".to_owned(),
        ExpectedMemoryEntryState::Present(reference)
        | ExpectedMemoryEntryState::Deleted(reference) => reference.entry_id().to_string(),
    }
}

fn result_content(model: &TuiModel, theme: &Theme) -> PanelContent {
    let message = match model.agents.memory.result_origin {
        crate::ui::tui::model::MemoryResultOrigin::Mutation => "Memory change completed.",
        crate::ui::tui::model::MemoryResultOrigin::Resolution => "Proposal resolution completed.",
    };
    PanelContent {
        title: "Memory result",
        lines: vec![
            memory_heading(theme),
            Line::styled(message, theme.success),
            Line::styled("Enter/Esc: return", theme.focus),
        ],
        scroll: model.agents.memory.detail_scroll,
    }
}

fn panel_inner_width(area: Rect) -> usize {
    usize::from(area.width.saturating_sub(2).max(1))
}

fn bounded_ascii_line(value: &str, width: usize) -> String {
    if value.is_ascii() && value.len() <= width {
        value.to_owned()
    } else {
        memory_escape_bounded(value, width)
    }
}

fn escaped_value_line(prefix: &str, value: &str, byte_cap: usize, width: usize) -> String {
    let prefix = bounded_ascii_line(prefix, width);
    let remaining = width.saturating_sub(prefix.len());
    format!(
        "{prefix}{}",
        memory_escape_bounded(value, byte_cap.min(remaining))
    )
}

fn profile_identity_line(model: &TuiModel, width: usize, theme: &Theme) -> Line<'static> {
    model.agents.memory.profile.as_ref().map_or_else(
        || {
            Line::styled(
                bounded_ascii_line("Profile unavailable", width),
                theme.warning,
            )
        },
        |identity| {
            let prefix = bounded_ascii_line("Profile  ", width);
            let remaining = width.saturating_sub(prefix.len());
            Line::from(vec![
                Span::styled(prefix, theme.muted),
                Span::raw(memory_escape_bounded(
                    &identity.display_name,
                    PROFILE_LABEL_MAX_ESCAPED_BYTES.min(remaining),
                )),
            ])
        },
    )
}

fn memory_heading(theme: &Theme) -> Line<'static> {
    Line::styled("Agents / Memory", theme.accent)
}

fn fixed_safety_lines(
    profile_id: crate::domain::AgentProfileId,
    operation: &str,
    stable_identity: &str,
    key: &str,
    controls: &str,
    width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    vec![
        Line::styled(
            escaped_value_line("Agents / Memory ", key, KEY_MAX_ESCAPED_BYTES, width),
            theme.accent,
        ),
        Line::raw(format!("Owner {profile_id}")),
        Line::raw(format!("{operation} {stable_identity}")),
        Line::styled(MEMORY_PLAINTEXT_WARNING, theme.warning),
        Line::styled(controls.to_owned(), theme.focus),
    ]
}

fn first_visible_item(
    requested: usize,
    selected: usize,
    len: usize,
    visible_items: usize,
) -> usize {
    let Some(last) = len.checked_sub(1) else {
        return 0;
    };
    let visible_items = visible_items.max(1);
    let selected_for_viewport = selected.min(last);
    let max_first = len.saturating_sub(visible_items);
    let mut first = requested.min(max_first);
    if selected_for_viewport < first {
        first = selected_for_viewport;
    }
    if selected_for_viewport >= first.saturating_add(visible_items) {
        first = selected_for_viewport
            .saturating_add(1)
            .saturating_sub(visible_items)
            .min(max_first);
    }
    first
}

fn unavailable_panel(
    title: &'static str,
    heading: &'static str,
    guidance: &'static str,
    theme: &Theme,
) -> PanelContent {
    PanelContent {
        title,
        lines: vec![
            memory_heading(theme),
            Line::styled(heading, theme.warning),
            Line::raw(guidance),
        ],
        scroll: 0,
    }
}

fn static_panel(title: &'static str, message: &'static str, theme: &Theme) -> PanelContent {
    PanelContent {
        title,
        lines: vec![memory_heading(theme), Line::default(), Line::raw(message)],
        scroll: 0,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{
        text::Line,
        widgets::{Paragraph, Wrap},
    };
    use uuid::Uuid;

    use super::{
        EPISODIC_BODY_MAX_ESCAPED_BYTES, EPISODIC_EVENT_TYPE_MAX_ESCAPED_BYTES,
        EPISODIC_LABEL_MAX_ESCAPED_BYTES, KEY_MAX_ESCAPED_BYTES, MAX_PURPOSE_TAGS,
        PROFILE_LABEL_MAX_ESCAPED_BYTES, TAG_MAX_ESCAPED_BYTES, VALUE_MAX_ESCAPED_BYTES,
        content_height, escaped_value_line, memory_content_lines, rendered_memory_field_value,
    };
    use crate::{
        agents::{AgentProfileVersion, builtin_profile_templates},
        app::{
            AgentProfilesView, DatabaseReadiness, EpisodicSummariesView, MemoryEntriesView,
            MemoryEntryHistorySummary, MemoryEntryHistoryView, MemoryEntrySummary, MemoryEntryView,
            MemoryProfileIdentityView, MemoryProposalsView, PresentationSnapshot,
            ProcessGuardOwnership,
        },
        domain::{
            Actor, AgentProfileId, AgentProfileVersionId, EventId, InstallationId, MemoryEntryId,
            MemoryEntryVersionId, MemoryNamespaceId, SessionId,
        },
        memory::{
            MemoryEntryDraft, MemoryEntryVersion, MemoryField, MemoryFieldValue,
            MemoryProposalFilter,
        },
        setup::SetupStatus,
        ui::tui::model::{AgentsPane, MemoryPane, TuiModel, View},
    };

    fn profile() -> AgentProfileVersion {
        let template = &builtin_profile_templates()[0];
        AgentProfileVersion::create(
            AgentProfileId::from_uuid(Uuid::from_u128(51_000)),
            AgentProfileVersionId::from_uuid(Uuid::from_u128(51_001)),
            MemoryNamespaceId::from_uuid(Uuid::from_u128(51_002)),
            1_800_000_000_000,
            template.copy_to_draft().expect("profile draft"),
            Some(template.provenance()),
        )
        .expect("profile")
    }

    fn memory_model() -> (TuiModel, AgentProfileVersion) {
        let profile = profile();
        let mut model = TuiModel::new(
            PresentationSnapshot {
                installation_id: InstallationId::from_uuid(Uuid::from_u128(51_010)),
                session_id: SessionId::from_uuid(Uuid::from_u128(51_011)),
                database_readiness: DatabaseReadiness::Ready,
                process_guard_ownership: ProcessGuardOwnership::Held,
                setup_status: SetupStatus::NotStarted,
                recent_audit: Vec::new(),
                agent_profiles: AgentProfilesView {
                    profiles: Vec::new(),
                    total_count: 0,
                    returned_count: 0,
                    truncated: false,
                },
                selected_agent_profile: None,
                selected_agent_profile_history: None,
            },
            false,
        );
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Memory;
        model
            .agents
            .memory
            .bind_profile(
                MemoryProfileIdentityView {
                    profile: profile.reference(),
                    display_name: profile.display_name().to_owned(),
                },
                profile.memory_namespace_id(),
            )
            .expect("memory profile");
        (model, profile)
    }

    #[test]
    fn content_height_measures_the_same_active_layer_as_narrow_rendering() {
        let (mut model, profile) = memory_model();
        model.agents.memory.entries = Some(MemoryEntriesView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            entries: Vec::new(),
            total_count: 0,
            returned_count: 0,
            omitted_count: 0,
        });
        model.agents.memory.proposals = Some(MemoryProposalsView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            filter: MemoryProposalFilter::Pending,
            proposals: Vec::new(),
            total_count: 0,
            returned_count: 0,
            omitted_count: 0,
        });
        model.agents.memory.episodes = Some(EpisodicSummariesView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            summaries: Vec::new(),
            total_count: 0,
            returned_count: 0,
            omitted_count: 0,
        });

        for (pane, expected_height) in [
            (MemoryPane::EntryList, 5),
            (MemoryPane::Proposals, 4),
            (MemoryPane::EpisodicSummaries, 4),
        ] {
            model.agents.memory.pane = pane;
            model.workspace_scroll = u16::MAX;
            assert_eq!(content_height(&model, 60), expected_height, "pane={pane:?}");
        }

        let entry = MemoryEntryVersion::create_present(
            profile.memory_namespace_id(),
            MemoryEntryId::from_uuid(Uuid::from_u128(51_020)),
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(51_021)),
            MemoryEntryDraft::new("history key".to_owned(), "value".to_owned(), Vec::new())
                .expect("entry draft"),
            Actor::Human,
            1_800_000_000_001,
            None,
            EventId::from_uuid(Uuid::from_u128(51_022)),
        )
        .expect("entry");
        model.agents.memory.entries = Some(MemoryEntriesView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            entries: vec![MemoryEntrySummary {
                entry: entry.reference(),
                display_key: entry.display_key().to_owned(),
                purpose_tags: Vec::new(),
                value_bytes: 5,
                created_at_ms: entry.created_at_ms(),
            }],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        });
        model.agents.memory.entry_history = Some(MemoryEntryHistoryView {
            profile: profile.reference(),
            current: entry.reference(),
            versions: vec![MemoryEntryHistorySummary {
                entry: entry.reference(),
                display_key: entry.display_key().to_owned(),
                created_at_ms: entry.created_at_ms(),
                accepted_proposal: None,
            }],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        });
        model.agents.memory.pane = MemoryPane::EntryHistory;
        model.agents.memory.entry_version = None;
        model.workspace_scroll = u16::MAX;
        assert_eq!(content_height(&model, 60), 4);

        let stale = MemoryEntryVersion::create_present(
            profile.memory_namespace_id(),
            MemoryEntryId::from_uuid(Uuid::from_u128(51_030)),
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(51_031)),
            MemoryEntryDraft::new("stale key".to_owned(), "stale".to_owned(), Vec::new())
                .expect("stale draft"),
            Actor::Human,
            1_800_000_000_002,
            None,
            EventId::from_uuid(Uuid::from_u128(51_032)),
        )
        .expect("stale history version");
        model.agents.memory.entry_version = Some(crate::app::MemoryEntryVersionView {
            profile: profile.reference(),
            entry: stale,
        });
        assert_eq!(content_height(&model, 60), 9);
    }

    #[test]
    fn content_height_uses_ratatuis_actual_space_separated_word_wrapping() {
        let (mut model, profile) = memory_model();
        let value = std::iter::repeat_n("abcdefghijklmnopqrstuvw", 40)
            .collect::<Vec<_>>()
            .join(" ");
        let entry = MemoryEntryVersion::create_present(
            profile.memory_namespace_id(),
            MemoryEntryId::from_uuid(Uuid::from_u128(51_040)),
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(51_041)),
            MemoryEntryDraft::new("wrapped height key".to_owned(), value, Vec::new())
                .expect("space-separated entry"),
            Actor::Human,
            1_800_000_000_003,
            None,
            EventId::from_uuid(Uuid::from_u128(51_042)),
        )
        .expect("wrapped entry");
        model.agents.memory.entries = Some(MemoryEntriesView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            entries: vec![MemoryEntrySummary {
                entry: entry.reference(),
                display_key: entry.display_key().to_owned(),
                purpose_tags: Vec::new(),
                value_bytes: u64::try_from(entry.value().expect("present value").len())
                    .expect("value bytes"),
                created_at_ms: entry.created_at_ms(),
            }],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        });
        model.agents.memory.entry_detail = Some(MemoryEntryView {
            profile: profile.reference(),
            entry,
        });
        model.agents.memory.pane = MemoryPane::EntryDetail;

        let width = 47_u16;
        let inner_width = width.saturating_sub(2).max(1);
        let ratatui_height = Paragraph::new(memory_content_lines(&model, width))
            .wrap(Wrap { trim: false })
            .line_count(inner_width);
        assert_eq!(usize::from(content_height(&model, width)), ratatui_height,);
    }

    #[test]
    fn every_untrusted_memory_field_class_is_fragment_safe_and_hard_capped() {
        for (field_class, cap) in [
            ("key", KEY_MAX_ESCAPED_BYTES),
            ("tag", TAG_MAX_ESCAPED_BYTES),
            ("value", VALUE_MAX_ESCAPED_BYTES),
            ("rationale", 2_048),
            ("profile label", PROFILE_LABEL_MAX_ESCAPED_BYTES),
            ("episodic label", EPISODIC_LABEL_MAX_ESCAPED_BYTES),
            ("episodic body", EPISODIC_BODY_MAX_ESCAPED_BYTES),
            ("episodic event type", EPISODIC_EVENT_TYPE_MAX_ESCAPED_BYTES),
        ] {
            let hostile = format!("{field_class}{}", "\u{e9}".repeat(cap));
            let escaped = super::memory_escape_bounded(&hostile, cap);
            assert!(escaped.ends_with("..."), "field={field_class}");
            assert!(escaped.len() <= cap, "field={field_class}");
            assert!(
                Line::from(escaped.clone()).width() <= cap,
                "field={field_class}"
            );
            assert!(!escaped.contains('\u{e9}'), "field={field_class}");
        }

        let hostile_key = "\u{e9}".repeat(KEY_MAX_ESCAPED_BYTES);
        let display_bounded = escaped_value_line("Key ", &hostile_key, KEY_MAX_ESCAPED_BYTES, 32);
        assert!(display_bounded.ends_with("..."));
        assert!(display_bounded.len() <= 32);
        assert!(Line::from(display_bounded).width() <= 32);

        let tags = (0..=MAX_PURPOSE_TAGS)
            .map(|index| format!("{index}{}", "\u{e9}".repeat(TAG_MAX_ESCAPED_BYTES)))
            .collect::<Vec<_>>();
        let rendered =
            rendered_memory_field_value(MemoryField::PurposeTags, &MemoryFieldValue::Tags(tags));
        let retained = rendered.split(", ").collect::<Vec<_>>();
        assert_eq!(retained.len(), MAX_PURPOSE_TAGS);
        assert!(
            retained
                .iter()
                .all(|tag| tag.len() <= TAG_MAX_ESCAPED_BYTES)
        );
        assert!(retained.iter().all(|tag| tag.ends_with("...")));
        assert!(!rendered.contains("8\\u{e9}"));
    }
}
