use super::{
    TuiEvent,
    model::{
        AgentOutcomeIntent, AgentSkillAction, AgentsPane, AssignmentKind, Focus, LayoutMode,
        ProfileConfirmation, RuntimeStatus, Severity, SkillConfirmation, SkillDetailAction,
        SkillOperationOrigin, SkillWorkspaceOrigin, SkillsPane, TuiModel, View,
    },
    views,
};
use crate::{
    agents::ProfileTemplateId,
    agents::builtin_profile_templates,
    app::{
        AgentProfileSelector, ApplicationCommand, CommandOutcome, CommandView, ShutdownDisposition,
        ShutdownReason,
    },
    audit::AuditEntry,
    ui::command::{AgentWorkflowCommand, ParsedLine, SkillWorkflowCommand, parse_line},
    ui::profile_editor::{
        PreviewEditRequest, ProfileEditorEffect, ProfileEditorMode, ProfileEditorStep,
    },
    ui::skill_editor::{SkillEditorEffect, SkillPreviewRequest},
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

const COMMAND_IN_FLIGHT_MESSAGE: &str = "A command is already running.";
const COMMAND_REJECTED_MESSAGE: &str = "Command rejected. Check the command and try again.";
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerEffect {
    None,
    Redraw,
    Submit(ApplicationCommand),
    RequestShutdown(ShutdownReason),
    LoadAgentProfiles,
    LoadAgentProfile {
        selected_profile: usize,
    },
    LoadAgentProfileHistory {
        selected_profile: usize,
    },
    LoadAgentProfileVersion {
        profile_id: crate::domain::AgentProfileId,
        version: crate::domain::ObjectVersion,
    },
    LoadAgentSkillLibrary,
    StartProfileCreate {
        template_index: usize,
    },
    StartProfileEdit {
        selected_profile: usize,
    },
    StartProfileCreateByTemplate {
        template_id: ProfileTemplateId,
    },
    StartProfileEditBySelector {
        selector: AgentProfileSelector,
    },
    StartSkillWorkflow(SkillWorkflowCommand),
    RequestProfilePreview(PreviewEditRequest),
    ExecuteProfile(ApplicationCommand),
    CancelProfileReview,
    LoadSkills,
    LoadSkill { selected_skill: usize },
    LoadSkillHistory { skill_id: crate::domain::SkillId },
    LoadSkillVersion {
        skill_id: crate::domain::SkillId,
        version: crate::domain::ObjectVersion,
    },
    LoadSkillStarter { selected_skill: usize },
    LoadSkillAgents,
    LoadSkillAgent { profile_id: crate::domain::AgentProfileId },
    RequestSkillPreview(SkillPreviewRequest),
    RequestSkillAssignmentPreview {
        profile_id: crate::domain::AgentProfileId,
        expected_active_profile_version_id: crate::domain::AgentProfileVersionId,
        target: crate::skills::SkillVersionRef,
        assignment: AssignmentKind,
    },
    ExecuteSkill(ApplicationCommand),
    CancelSkillReview,
}

pub fn handle_event(model: &mut TuiModel, event: TuiEvent) -> ControllerEffect {
    let effect = match event {
        TuiEvent::Interrupt => ControllerEffect::RequestShutdown(ShutdownReason::Interrupted),
        TuiEvent::Resize(width, height) => {
            model.set_terminal_size(width, height);
            if model.focus != Focus::Command {
                normalize_focus(model);
            }
            ControllerEffect::Redraw
        }
        TuiEvent::Paste(text) => handle_paste(model, &text),
        TuiEvent::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            handle_key(model, key)
        }
        TuiEvent::Key(_) => ControllerEffect::None,
    };
    clamp_workspace_scroll(model);
    effect
}

pub fn apply_outcome(model: &mut TuiModel, outcome: CommandOutcome) -> ControllerEffect {
    model.set_command_in_flight(false);

    let CommandOutcome {
        committed_events,
        view,
        shutdown,
        ..
    } = outcome;
    let committed_audit = committed_events
        .iter()
        .map(AuditEntry::from_event)
        .collect::<Vec<_>>();
    let mut follow_up = ControllerEffect::Redraw;

    let view_shutdown = match view {
        CommandView::Help(_) => {
            select_workspace_view(model, View::Help);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::Status(status) => {
            model.installation_id = status.installation_id;
            model.session_id = status.session_id;
            select_workspace_view(model, View::Overview);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::SetupStatus(setup) => {
            model.setup_status = setup.status;
            select_workspace_view(model, View::Setup);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::AuditTail(audit) => {
            model.replace_audit(audit.entries);
            select_workspace_view(model, View::Audit);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::InputRejected(_) => {
            model.set_focus(Focus::Command);
            model.set_message(Severity::Error, COMMAND_REJECTED_MESSAGE);
            ShutdownDisposition::Continue
        }
        CommandView::Shutdown(shutdown) => {
            model.set_runtime_status(RuntimeStatus::Stopping);
            shutdown.disposition
        }
        CommandView::AgentProfiles(profiles) => {
            let intent = model
                .pending_agent_outcome
                .take()
                .unwrap_or(AgentOutcomeIntent::AgentsWorkspace);
            model.agents.replace_profiles(profiles);
            match intent {
                AgentOutcomeIntent::AgentsWorkspace => {
                    model.agents.pane = AgentsPane::List;
                    select_workspace_view(model, View::Agents);
                }
                AgentOutcomeIntent::SkillAssignment => model.skills.active = true,
            }
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::AgentProfile(profile) => {
            let intent = model
                .pending_agent_outcome
                .take()
                .unwrap_or(AgentOutcomeIntent::AgentsWorkspace);
            let needs_skill_library = intent == AgentOutcomeIntent::AgentsWorkspace
                && !model.skills.library_loaded
                && !profile.profile.skill_refs().is_empty();
            match intent {
                AgentOutcomeIntent::SkillAssignment => {
                    let target = model.skills.selected_skill_ref().cloned();
                    let current = target.as_ref().and_then(|target| {
                        profile.profile.skill_refs().iter().find(|current| {
                            current.skill_id() == target.skill_id()
                        })
                    });
                    model.skills.assignment = target
                        .as_ref()
                        .map(|target| AssignmentKind::classify(target, current));
                    model.skills.selected_agent_detail = Some(profile);
                    model.skills.active = true;
                    model.skills.pane = SkillsPane::AssignmentReview;
                }
                AgentOutcomeIntent::AgentsWorkspace => {
                    model.agents.replace_detail(profile);
                    model.agents.pane = AgentsPane::Detail;
                    select_workspace_view(model, View::Agents);
                }
            }
            if needs_skill_library {
                follow_up = ControllerEffect::LoadAgentSkillLibrary;
            }
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::AgentProfileHistory(history) => {
            model.agents.replace_history(history);
            model.agents.pane = AgentsPane::History;
            select_workspace_view(model, View::Agents);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::AgentProfileVersion(version) => {
            model.agents.replace_version_detail(version);
            model.agents.pane = AgentsPane::History;
            select_workspace_view(model, View::Agents);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::AgentProfileCreated(_) | CommandView::AgentProfileVersionActivated(_) => {
            ShutdownDisposition::Continue
        }
        CommandView::Skills(skills) => {
            model.skills.replace_skills(skills);
            model.skills.active = true;
            model.skills.pane = SkillsPane::List;
            if let Some(skill_id) = model.skills.pending_active_skill {
                follow_up = ControllerEffect::Submit(ApplicationCommand::ShowSkill {
                    selector: skill_id.into(),
                });
            }
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::Skill(skill) => {
            let skill_id = skill.skill_ref.skill_id();
            model.skills.replace_detail(skill);
            if model.skills.pending_active_skill == Some(skill_id) {
                model.skills.pending_active_skill = None;
            }
            model.skills.active = true;
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::SkillHistory(history) => {
            model.skills.replace_history(history);
            model.skills.active = true;
            model.skills.pane = SkillsPane::History;
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::SkillVersion(version) => {
            model.skills.replace_version_detail(version);
            model.skills.active = true;
            model.skills.pane = SkillsPane::Detail;
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::SkillCreated(created) => {
            model.skills.pending_active_skill = Some(created.skill_id);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::SkillVersionActivated(activated) => {
            model.skills.pending_active_skill = Some(activated.skill_id);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::AgentSkillAssigned(_)
        | CommandView::AgentSkillUpgraded(_)
        | CommandView::AgentSkillUnassigned(_) => {
            model.clear_message();
            ShutdownDisposition::Continue
        }
    };

    merge_committed_audit(model, committed_audit);
    clamp_workspace_scroll(model);

    if requests_shutdown(shutdown) || requests_shutdown(view_shutdown) {
        model.set_runtime_status(RuntimeStatus::Stopping);
        ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
    } else {
        follow_up
    }
}

fn handle_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    if is_ctrl_c(key) {
        return ControllerEffect::RequestShutdown(ShutdownReason::Interrupted);
    }

    if model.layout_mode == LayoutMode::TooSmall {
        return handle_too_small_key(model, key);
    }

    if active_confirmation(model) {
        return handle_confirmation_key(model, key);
    }

    if active_profile_editor(model) {
        return handle_profile_editor_key(model, key);
    }

    if model.focus == Focus::Command {
        return handle_command_key(model, key);
    }

    if is_plain_char(key, '/') {
        model.command.clear();
        model.command.insert('/');
        model.set_focus(Focus::Command);
        return ControllerEffect::Redraw;
    }

    if active_skill_confirmation(model) {
        return handle_skill_confirmation_key(model, key);
    }

    if active_skill_editor(model) {
        return handle_skill_editor_key(model, key);
    }

    if model.skills.active {
        return handle_skills_key(model, key).unwrap_or(ControllerEffect::None);
    }

    if model.active_view == View::Agents
        && let Some(effect) = handle_agents_key(model, key)
    {
        return effect;
    }

    match key.code {
        KeyCode::Char('1') if no_modifiers(key.modifiers) => select_view(model, View::Overview),
        KeyCode::Char('2') if no_modifiers(key.modifiers) => select_view(model, View::Setup),
        KeyCode::Char('3') if no_modifiers(key.modifiers) => select_view(model, View::Audit),
        KeyCode::Char('4') if no_modifiers(key.modifiers) => select_view(model, View::Help),
        KeyCode::Char('a') if no_modifiers(key.modifiers) => open_agents(model),
        KeyCode::Char('s') if no_modifiers(key.modifiers) => open_skills(model),
        KeyCode::Char('?') if text_modifiers(key.modifiers) => select_view(model, View::Help),
        KeyCode::Char('/') if no_modifiers(key.modifiers) => {
            model.command.clear();
            model.command.insert('/');
            model.set_focus(Focus::Command);
            ControllerEffect::Redraw
        }
        KeyCode::Char('i') if no_modifiers(key.modifiers) => toggle_or_focus_inspector(model),
        KeyCode::Tab if no_modifiers(key.modifiers) => cycle_focus(model, true),
        KeyCode::BackTab if backtab_modifiers(key.modifiers) => cycle_focus(model, false),
        KeyCode::Esc if no_modifiers(key.modifiers) => dismiss(model),
        KeyCode::Up | KeyCode::Left if no_modifiers(key.modifiers) => {
            move_focused(model, false, false)
        }
        KeyCode::Down | KeyCode::Right if no_modifiers(key.modifiers) => {
            move_focused(model, true, false)
        }
        KeyCode::PageUp if no_modifiers(key.modifiers) => move_focused(model, false, true),
        KeyCode::PageDown if no_modifiers(key.modifiers) => move_focused(model, true, true),
        KeyCode::Home if no_modifiers(key.modifiers) => move_to_bound(model, false),
        KeyCode::End if no_modifiers(key.modifiers) => move_to_bound(model, true),
        _ => ControllerEffect::None,
    }
}

fn handle_too_small_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    if model.focus == Focus::Command {
        return handle_command_key(model, key);
    }

    if is_plain_char(key, '/') {
        model.command.clear();
        model.command.insert('/');
        model.set_focus(Focus::Command);
        ControllerEffect::Redraw
    } else {
        ControllerEffect::None
    }
}

fn handle_command_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    match key.code {
        KeyCode::Esc if no_modifiers(key.modifiers) => {
            model.command.clear();
            model.set_focus(Focus::Workspace);
            ControllerEffect::Redraw
        }
        KeyCode::Enter if no_modifiers(key.modifiers) => submit_command(model),
        KeyCode::Left if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_left())
        }
        KeyCode::Right if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_right())
        }
        KeyCode::Home if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_home())
        }
        KeyCode::End if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_end())
        }
        KeyCode::Backspace if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.backspace())
        }
        KeyCode::Delete if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.delete())
        }
        KeyCode::Up if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.history_previous())
        }
        KeyCode::Down if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.history_next())
        }
        KeyCode::Tab if no_modifiers(key.modifiers) => cycle_focus(model, true),
        KeyCode::BackTab if backtab_modifiers(key.modifiers) => cycle_focus(model, false),
        KeyCode::Char(character) if text_modifiers(key.modifiers) => {
            model.command.insert(character);
            ControllerEffect::Redraw
        }
        _ => ControllerEffect::None,
    }
}

fn submit_command(model: &mut TuiModel) -> ControllerEffect {
    if model.command_in_flight {
        model.set_message(Severity::Warning, COMMAND_IN_FLIGHT_MESSAGE);
        return ControllerEffect::None;
    }

    let input = model.command.take_text();
    match parse_line(input.as_bytes()) {
        ParsedLine::Ignored => ControllerEffect::None,
        ParsedLine::Command(command) => {
            if !matches!(command, ApplicationCommand::RejectInput(_)) {
                model.command.remember(input);
            }
            model.clear_message();
            model.set_command_in_flight(true);
            if matches!(
                command,
                ApplicationCommand::ListAgentProfiles
                    | ApplicationCommand::ShowAgentProfile { .. }
            ) {
                model.pending_agent_outcome = Some(AgentOutcomeIntent::AgentsWorkspace);
            }
            ControllerEffect::Submit(command)
        }
        ParsedLine::AgentWorkflow(workflow) => {
            model.command.remember(input);
            model.clear_message();
            model.select_view(View::Agents);
            model.set_focus(Focus::Workspace);
            match workflow {
                AgentWorkflowCommand::SelectCreateTemplate => {
                    ControllerEffect::StartProfileCreate {
                        template_index: model.agents.selected_template,
                    }
                }
                AgentWorkflowCommand::Create { template_id } => {
                    ControllerEffect::StartProfileCreateByTemplate { template_id }
                }
                AgentWorkflowCommand::Edit { selector } => {
                    ControllerEffect::StartProfileEditBySelector { selector }
                }
            }
        }
        ParsedLine::SkillWorkflow(workflow) => {
            model.command.remember(input);
            model.clear_message();
            if !model.skills.active {
                model.skills.workspace_origin = Some(SkillWorkspaceOrigin::Cockpit(model.active_view));
            }
            model.skills.clear_skill_context();
            model.skills.active = true;
            model.skills.operation_origin = SkillOperationOrigin::Skills(SkillsPane::Detail);
            model.set_focus(Focus::Workspace);
            ControllerEffect::StartSkillWorkflow(workflow)
        }
    }
}

fn handle_paste(model: &mut TuiModel, text: &str) -> ControllerEffect {
    if !active_profile_editor(model) && !active_skill_editor(model) && model.focus != Focus::Command {
        return ControllerEffect::None;
    }
    let before = model.command.text().len();
    model.command.ingest(text);
    if model.command.text().len() == before {
        ControllerEffect::None
    } else {
        ControllerEffect::Redraw
    }
}

fn open_agents(model: &mut TuiModel) -> ControllerEffect {
    model.skills.active = false;
    model.select_view(View::Agents);
    model.set_focus(Focus::Workspace);
    model.agents.pane = AgentsPane::List;
    ControllerEffect::LoadAgentProfiles
}

fn open_skills(model: &mut TuiModel) -> ControllerEffect {
    model.skills.workspace_origin = Some(
        if model.active_view == View::Agents && model.agents.skill_panel_open {
            model
                .agents
                .detail
                .as_ref()
                .map(|detail| SkillWorkspaceOrigin::AgentSkills {
                    profile_id: detail.profile.profile_id(),
                })
                .unwrap_or(SkillWorkspaceOrigin::Cockpit(model.active_view))
        } else {
            SkillWorkspaceOrigin::Cockpit(model.active_view)
        },
    );
    model.skills.active = true;
    model.skills.pane = SkillsPane::List;
    model.set_focus(Focus::Workspace);
    ControllerEffect::LoadSkills
}

fn active_skill_confirmation(model: &TuiModel) -> bool {
    model.skills.pane == SkillsPane::Confirmation
        && model.skills.pending_confirmation.is_some()
}

fn active_skill_editor(model: &TuiModel) -> bool {
    model.skills.active
        && model.skills.pane == SkillsPane::Editor
        && model.skills.editor.is_some()
}

fn handle_skill_confirmation_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    match key.code {
        KeyCode::Enter if key.kind == KeyEventKind::Press && no_modifiers(key.modifiers) => model
            .skills
            .pending_confirmation
            .as_ref()
            .map(|confirmation| ControllerEffect::ExecuteSkill(confirmation.command.clone()))
            .unwrap_or(ControllerEffect::None),
        KeyCode::Esc if no_modifiers(key.modifiers) => {
            let origin = model
                .skills
                .pending_confirmation
                .take()
                .map(|confirmation| confirmation.origin)
                .unwrap_or(SkillOperationOrigin::Skills(SkillsPane::Detail));
            if origin == SkillOperationOrigin::Skills(SkillsPane::Editor)
                && let Some(editor) = model.skills.editor.as_mut()
            {
                editor.clear_review();
            }
            restore_skill_origin(model, origin);
            synchronize_skill_editor_input(model);
            ControllerEffect::CancelSkillReview
        }
        _ => ControllerEffect::None,
    }
}

fn handle_skill_editor_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    match key.code {
        KeyCode::Esc if no_modifiers(key.modifiers) => {
            if model
                .skills
                .editor
                .as_mut()
                .is_some_and(|editor| editor.cancel_reference_interaction())
            {
                synchronize_skill_editor_input(model);
                return ControllerEffect::Redraw;
            }
            let effect = model
                .skills
                .editor
                .as_mut()
                .map(|editor| editor.back())
                .unwrap_or(SkillEditorEffect::Cancelled);
            let keep_editor = !matches!(effect, SkillEditorEffect::Cancelled);
            let controller_effect = apply_skill_editor_effect(model, effect);
            if keep_editor {
                synchronize_skill_editor_input(model);
            } else {
                model.command.clear();
            }
            controller_effect
        }
        KeyCode::Enter if no_modifiers(key.modifiers) => {
            if model
                .skills
                .editor
                .as_mut()
                .is_some_and(|editor| editor.begin_edit_selected_reference())
            {
                synchronize_skill_editor_input(model);
                return ControllerEffect::Redraw;
            }
            let input = model.command.text().to_owned();
            let effect = model
                .skills
                .editor
                .as_mut()
                .map(|editor| editor.submit_keyboard_line(&input))
                .unwrap_or(SkillEditorEffect::None);
            let valid = model
                .skills
                .editor
                .as_ref()
                .is_some_and(|editor| editor.local_error().is_none());
            let controller_effect = apply_skill_editor_effect(model, effect);
            if valid {
                synchronize_skill_editor_input(model);
            }
            controller_effect
        }
        KeyCode::Up | KeyCode::Down if no_modifiers(key.modifiers) => {
            let reference_picker = model
                .skills
                .editor
                .as_ref()
                .is_some_and(|editor| {
                    editor.field() == crate::ui::skill_editor::SkillEditorField::ReferenceName
                        && model.command.text().is_empty()
                });
            if !reference_picker {
                return ControllerEffect::None;
            }
            if let Some(editor) = model.skills.editor.as_mut() {
                editor.select_reference(matches!(key.code, KeyCode::Down));
            }
            ControllerEffect::Redraw
        }
        KeyCode::Char(character) if text_modifiers(key.modifiers) => {
            if let Some(editor) = model.skills.editor.as_mut() {
                editor.clear_reference_selection();
            }
            model.command.insert(character);
            ControllerEffect::Redraw
        }
        KeyCode::Backspace if no_modifiers(key.modifiers) => edit(model, |model| model.command.backspace()),
        KeyCode::Delete if no_modifiers(key.modifiers) => {
            if model
                .skills
                .editor
                .as_mut()
                .is_some_and(|editor| editor.remove_selected_reference())
            {
                synchronize_skill_editor_input(model);
                ControllerEffect::Redraw
            } else {
                edit(model, |model| model.command.delete())
            }
        }
        KeyCode::Left if no_modifiers(key.modifiers) => edit(model, |model| model.command.move_left()),
        KeyCode::Right if no_modifiers(key.modifiers) => edit(model, |model| model.command.move_right()),
        KeyCode::Home if no_modifiers(key.modifiers) => edit(model, |model| model.command.move_home()),
        KeyCode::End if no_modifiers(key.modifiers) => edit(model, |model| model.command.move_end()),
        _ => ControllerEffect::None,
    }
}

fn apply_skill_editor_effect(model: &mut TuiModel, effect: SkillEditorEffect) -> ControllerEffect {
    match effect {
        SkillEditorEffect::None => ControllerEffect::Redraw,
        SkillEditorEffect::Preview(request) => ControllerEffect::RequestSkillPreview(request),
        SkillEditorEffect::Execute(command) => {
            model.command.clear();
            model.skills.operation_origin = SkillOperationOrigin::Skills(SkillsPane::Editor);
            model.skills.pending_confirmation = Some(SkillConfirmation {
                command,
                origin: model.skills.operation_origin,
            });
            model.skills.pane = SkillsPane::Confirmation;
            ControllerEffect::Redraw
        }
        SkillEditorEffect::CancelReview => ControllerEffect::CancelSkillReview,
        SkillEditorEffect::Cancelled => {
            let origin = model.skills.editor_origin;
            model.skills.editor = None;
            model.skills.pane = origin;
            ControllerEffect::Redraw
        }
    }
}

fn handle_skills_key(model: &mut TuiModel, key: KeyEvent) -> Option<ControllerEffect> {
    let effect = match (model.skills.pane, key.code) {
        (_, KeyCode::Char('/')) if no_modifiers(key.modifiers) => {
            model.command.clear();
            model.command.insert('/');
            model.set_focus(Focus::Command);
            ControllerEffect::Redraw
        }
        (SkillsPane::List, KeyCode::Down) if no_modifiers(key.modifiers) => {
            let last = model.skills.library.skills.len().saturating_sub(1);
            model.skills.selected_skill = model.skills.selected_skill.saturating_add(1).min(last);
            ControllerEffect::Redraw
        }
        (SkillsPane::List, KeyCode::Up) if no_modifiers(key.modifiers) => {
            model.skills.selected_skill = model.skills.selected_skill.saturating_sub(1);
            ControllerEffect::Redraw
        }
        (SkillsPane::List, KeyCode::Enter) if no_modifiers(key.modifiers) => {
            ControllerEffect::LoadSkill { selected_skill: model.skills.selected_skill }
        }
        (SkillsPane::List, KeyCode::Char('c')) if no_modifiers(key.modifiers) => {
            model.skills.pane = SkillsPane::CreateSource;
            ControllerEffect::Redraw
        }
        (SkillsPane::CreateSource, KeyCode::Down) if no_modifiers(key.modifiers) => {
            model.skills.selected_create_source = model
                .skills
                .selected_create_source
                .saturating_add(1)
                .min(model.skills.library.skills.len());
            ControllerEffect::Redraw
        }
        (SkillsPane::CreateSource, KeyCode::Up) if no_modifiers(key.modifiers) => {
            model.skills.selected_create_source = model.skills.selected_create_source.saturating_sub(1);
            ControllerEffect::Redraw
        }
        (SkillsPane::CreateSource, KeyCode::Enter) if no_modifiers(key.modifiers) => {
            if model.skills.selected_create_source == 0 {
                model.skills.start_create(None);
                synchronize_skill_editor_input(model);
                ControllerEffect::Redraw
            } else {
                ControllerEffect::LoadSkillStarter {
                    selected_skill: model.skills.selected_create_source - 1,
                }
            }
        }
        (SkillsPane::Detail, KeyCode::Down | KeyCode::Right) if no_modifiers(key.modifiers) => {
            let last = model.skills.available_detail_actions().len().saturating_sub(1);
            model.skills.selected_action_index = model.skills.selected_action_index.saturating_add(1).min(last);
            ControllerEffect::Redraw
        }
        (SkillsPane::Detail, KeyCode::Up | KeyCode::Left) if no_modifiers(key.modifiers) => {
            model.skills.selected_action_index = model.skills.selected_action_index.saturating_sub(1);
            ControllerEffect::Redraw
        }
        (SkillsPane::Detail, KeyCode::Enter) if no_modifiers(key.modifiers) => {
            match model.skills.selected_action() {
                SkillDetailAction::Assign => {
                    model.skills.pane = SkillsPane::AgentPicker;
                    model.pending_agent_outcome = Some(AgentOutcomeIntent::SkillAssignment);
                    match model.skills.agent_origin_profile_id() {
                        Some(profile_id) => ControllerEffect::LoadSkillAgent { profile_id },
                        None => ControllerEffect::LoadSkillAgents,
                    }
                }
                SkillDetailAction::CreateVersion => {
                    model.skills.start_version();
                    synchronize_skill_editor_input(model);
                    ControllerEffect::Redraw
                }
                SkillDetailAction::History => ControllerEffect::LoadSkillHistory {
                    skill_id: model.skills.current_skill_id()?,
                },
            }
        }
        (SkillsPane::History, KeyCode::Down) if no_modifiers(key.modifiers) => {
            let last = model.skills.history.as_ref().map(|history| history.versions.len().saturating_sub(1)).unwrap_or(0);
            model.skills.selected_history_version = model.skills.selected_history_version.saturating_add(1).min(last);
            ControllerEffect::Redraw
        }
        (SkillsPane::History, KeyCode::Up) if no_modifiers(key.modifiers) => {
            model.skills.selected_history_version = model.skills.selected_history_version.saturating_sub(1);
            ControllerEffect::Redraw
        }
        (SkillsPane::History, KeyCode::Enter) if no_modifiers(key.modifiers) => {
            let Some(history) = model.skills.history.as_ref() else { return Some(ControllerEffect::Redraw); };
            let Some(entry) = history.versions.get(model.skills.selected_history_version) else { return Some(ControllerEffect::Redraw); };
            ControllerEffect::LoadSkillVersion {
                skill_id: history.skill_id,
                version: entry.skill_ref.version(),
            }
        }
        (SkillsPane::AgentPicker, KeyCode::Down) if no_modifiers(key.modifiers) => {
            let last = model.agents.profiles.profiles.len().saturating_sub(1);
            model.skills.selected_agent = model.skills.selected_agent.saturating_add(1).min(last);
            ControllerEffect::Redraw
        }
        (SkillsPane::AgentPicker, KeyCode::Up) if no_modifiers(key.modifiers) => {
            model.skills.selected_agent = model.skills.selected_agent.saturating_sub(1);
            ControllerEffect::Redraw
        }
        (SkillsPane::AgentPicker, KeyCode::Enter) if no_modifiers(key.modifiers) => {
            let profile_id = model
                .agents
                .profiles
                .profiles
                .get(model.skills.selected_agent)?
                .profile_id;
            model.pending_agent_outcome = Some(AgentOutcomeIntent::SkillAssignment);
            ControllerEffect::LoadSkillAgent { profile_id }
        }
        (SkillsPane::AssignmentReview, KeyCode::Enter) if no_modifiers(key.modifiers) => {
            let detail = model.skills.selected_agent_detail.as_ref()?;
            let target = model.skills.selected_skill_ref()?.clone();
            let assignment = model.skills.assignment.clone()?;
            if assignment == AssignmentKind::AlreadyAssigned {
                model.set_message(Severity::Warning, "Agent already has this exact skill version.");
                ControllerEffect::Redraw
            } else {
                model.skills.operation_origin = model
                    .skills
                    .agent_origin_profile_id()
                    .map(|profile_id| SkillOperationOrigin::AgentSkills { profile_id })
                    .unwrap_or(SkillOperationOrigin::Skills(SkillsPane::AssignmentReview));
                ControllerEffect::RequestSkillAssignmentPreview {
                    profile_id: detail.profile.profile_id(),
                    expected_active_profile_version_id: detail.profile.profile_version_id(),
                    target,
                    assignment,
                }
            }
        }
        (SkillsPane::Result, KeyCode::Enter | KeyCode::Esc) if no_modifiers(key.modifiers) => {
            if model.skills.pending_active_skill.is_some() {
                ControllerEffect::Submit(ApplicationCommand::ListSkills)
            } else {
                model.skills.pane = SkillsPane::Detail;
                ControllerEffect::Redraw
            }
        }
        (_, KeyCode::Esc) if no_modifiers(key.modifiers) => return Some(unwind_skills(model)),
        _ => return None,
    };
    Some(effect)
}

fn synchronize_skill_editor_input(model: &mut TuiModel) {
    let value = model
        .skills
        .editor
        .as_ref()
        .map(|editor| editor.current_value().to_owned())
        .unwrap_or_default();
    model.command.clear();
    model.command.ingest(&value);
}

fn restore_skill_origin(model: &mut TuiModel, origin: SkillOperationOrigin) {
    match origin {
        SkillOperationOrigin::Skills(pane) => {
            model.skills.active = true;
            model.skills.pane = pane;
        }
        SkillOperationOrigin::AgentSkills { profile_id } => {
            model.skills.active = false;
            model.skills.pane = SkillsPane::Detail;
            model.agents.select_profile_id(profile_id);
            model.agents.skill_panel_open = true;
            model.active_view = View::Agents;
        }
    }
}

fn unwind_skills(model: &mut TuiModel) -> ControllerEffect {
    match model.skills.pane {
        SkillsPane::List => {
            model.skills.active = false;
            match model.skills.workspace_origin.take() {
                Some(SkillWorkspaceOrigin::Cockpit(view)) => model.active_view = view,
                Some(SkillWorkspaceOrigin::AgentSkills { profile_id }) => {
                    model.active_view = View::Agents;
                    model.agents.select_profile_id(profile_id);
                    model.agents.skill_panel_open = true;
                }
                None => {}
            }
        }
        SkillsPane::Detail
            if matches!(
                model.skills.workspace_origin,
                Some(SkillWorkspaceOrigin::AgentSkills { .. })
            ) =>
        {
            let profile_id = model.skills.agent_origin_profile_id();
            model.skills.active = false;
            model.skills.workspace_origin = None;
            model.active_view = View::Agents;
            if let Some(profile_id) = profile_id {
                model.agents.select_profile_id(profile_id);
            }
            model.agents.skill_panel_open = true;
        }
        SkillsPane::CreateSource | SkillsPane::Detail => model.skills.pane = SkillsPane::List,
        SkillsPane::History | SkillsPane::AgentPicker | SkillsPane::Result => model.skills.pane = SkillsPane::Detail,
        SkillsPane::AssignmentReview => model.skills.pane = SkillsPane::AgentPicker,
        SkillsPane::Editor => {
            model.skills.editor = None;
            model.skills.pane = SkillsPane::Detail;
        }
        SkillsPane::Confirmation => {
            model.skills.pending_confirmation = None;
            model.skills.pane = SkillsPane::Detail;
        }
    }
    ControllerEffect::Redraw
}

fn active_confirmation(model: &TuiModel) -> bool {
    model.active_view == View::Agents
        && model.agents.pane == AgentsPane::Confirmation
        && model.agents.pending_confirmation.is_some()
}

fn active_profile_editor(model: &TuiModel) -> bool {
    model.active_view == View::Agents
        && model.agents.pane == AgentsPane::Editor
        && model.agents.editor.is_some()
}

fn handle_confirmation_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    match key.code {
        KeyCode::Enter if key.kind == KeyEventKind::Press && no_modifiers(key.modifiers) => {
            let Some(command) = model
                .agents
                .pending_confirmation
                .as_ref()
                .map(|confirmation| confirmation.command.clone())
            else {
                return ControllerEffect::None;
            };
            model.command.clear();
            model.clear_message();
            ControllerEffect::ExecuteProfile(command)
        }
        KeyCode::Esc if no_modifiers(key.modifiers) => {
            model.command.clear();
            model.agents.pending_confirmation = None;
            model.agents.pane = AgentsPane::Editor;
            ControllerEffect::Redraw
        }
        _ => ControllerEffect::None,
    }
}

fn handle_profile_editor_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    match key.code {
        KeyCode::Esc if no_modifiers(key.modifiers) => {
            let at_first_step = model
                .agents
                .editor
                .as_ref()
                .is_none_or(|editor| editor.step() == ProfileEditorStep::Template);
            if at_first_step {
                model.command.clear();
                model.agents.editor = None;
                model.agents.pane = AgentsPane::Detail;
                ControllerEffect::CancelProfileReview
            } else {
                let effect = model
                    .agents
                    .editor
                    .as_mut()
                    .map(|editor| editor.submit_line(":back"))
                    .unwrap_or(ProfileEditorEffect::None);
                apply_profile_editor_effect(model, effect)
            }
        }
        KeyCode::Up if no_modifiers(key.modifiers) => cycle_profile_template(model, false),
        KeyCode::Down if no_modifiers(key.modifiers) => cycle_profile_template(model, true),
        KeyCode::Enter => submit_profile_editor_enter(model, key),
        KeyCode::Char(character) if text_modifiers(key.modifiers) => {
            model.command.insert(character);
            ControllerEffect::Redraw
        }
        KeyCode::Backspace if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.backspace())
        }
        KeyCode::Delete if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.delete())
        }
        KeyCode::Left if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_left())
        }
        KeyCode::Right if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_right())
        }
        KeyCode::Home if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_home())
        }
        KeyCode::End if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_end())
        }
        _ => ControllerEffect::None,
    }
}

fn cycle_profile_template(model: &mut TuiModel, forward: bool) -> ControllerEffect {
    let Some(editor) = model.agents.editor.as_mut() else {
        return ControllerEffect::None;
    };
    if editor.step() != ProfileEditorStep::Template {
        return ControllerEffect::None;
    }
    if !matches!(editor.mode(), ProfileEditorMode::Create { .. }) {
        return ControllerEffect::None;
    }
    let templates = builtin_profile_templates();
    if templates.is_empty() {
        return ControllerEffect::None;
    }
    let current = templates
        .iter()
        .position(|template| template.role == editor.draft().role)
        .unwrap_or(0);
    let selected = if forward {
        (current + 1) % templates.len()
    } else {
        (current + templates.len() - 1) % templates.len()
    };
    if editor.select_template(&templates[selected]) {
        model.agents.selected_template = selected;
    }
    ControllerEffect::Redraw
}

fn submit_profile_editor_enter(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    let on_review = model
        .agents
        .editor
        .as_ref()
        .is_some_and(|editor| editor.step() == ProfileEditorStep::Review);
    if on_review && key.kind != KeyEventKind::Press {
        return ControllerEffect::None;
    }

    let input = model.command.take_text();
    let effect = model
        .agents
        .editor
        .as_mut()
        .map(|editor| {
            if editor.step() == ProfileEditorStep::Review && input.is_empty() {
                let control = match editor.mode() {
                    ProfileEditorMode::Create { .. } => ":create",
                    ProfileEditorMode::Edit { .. } if editor.review().is_some() => ":activate",
                    ProfileEditorMode::Edit { .. } => ":review",
                };
                editor.submit_line(control)
            } else {
                editor.submit_keyboard_line(&input)
            }
        })
        .unwrap_or(ProfileEditorEffect::None);
    apply_profile_editor_effect(model, effect)
}

fn apply_profile_editor_effect(
    model: &mut TuiModel,
    effect: ProfileEditorEffect,
) -> ControllerEffect {
    match effect {
        ProfileEditorEffect::None => ControllerEffect::Redraw,
        ProfileEditorEffect::PreviewEdit(request) => {
            ControllerEffect::RequestProfilePreview(request)
        }
        ProfileEditorEffect::Execute(command) => {
            model.command.clear();
            model.agents.pending_confirmation = Some(ProfileConfirmation { command });
            model.agents.pane = AgentsPane::Confirmation;
            ControllerEffect::Redraw
        }
        ProfileEditorEffect::Cancelled => {
            model.agents.editor = None;
            model.agents.pane = AgentsPane::Detail;
            ControllerEffect::CancelProfileReview
        }
    }
}

fn handle_agents_key(model: &mut TuiModel, key: KeyEvent) -> Option<ControllerEffect> {
    if model.agents.pane == AgentsPane::Detail && model.agents.skill_panel_open {
        return handle_agent_skills_key(model, key);
    }
    let effect = match (model.agents.pane, key.code) {
        (AgentsPane::List, KeyCode::Down) if no_modifiers(key.modifiers) => {
            let last = model.agents.profiles.profiles.len().saturating_sub(1);
            model.agents.selected_profile =
                model.agents.selected_profile.saturating_add(1).min(last);
            model.agents.list_scroll = model.agents.selected_profile;
            ControllerEffect::Redraw
        }
        (AgentsPane::List, KeyCode::Up) if no_modifiers(key.modifiers) => {
            model.agents.selected_profile = model.agents.selected_profile.saturating_sub(1);
            model.agents.list_scroll = model.agents.selected_profile;
            ControllerEffect::Redraw
        }
        (AgentsPane::List, KeyCode::Enter) if no_modifiers(key.modifiers) => {
            model.agents.pane = AgentsPane::Detail;
            ControllerEffect::LoadAgentProfile {
                selected_profile: model.agents.selected_profile,
            }
        }
        (AgentsPane::List | AgentsPane::Detail, KeyCode::Char('c'))
            if no_modifiers(key.modifiers) =>
        {
            let template_index = model.agents.selected_template;
            ControllerEffect::StartProfileCreate { template_index }
        }
        (AgentsPane::List, KeyCode::Char('e')) if no_modifiers(key.modifiers) => {
            model.agents.pane = AgentsPane::Editor;
            ControllerEffect::StartProfileEdit {
                selected_profile: model.agents.selected_profile,
            }
        }
        (AgentsPane::Detail, KeyCode::Char('h')) if no_modifiers(key.modifiers) => {
            model.agents.pane = AgentsPane::History;
            ControllerEffect::LoadAgentProfileHistory {
                selected_profile: model.agents.selected_profile,
            }
        }
        (AgentsPane::Detail, KeyCode::Enter) if no_modifiers(key.modifiers) => {
            if model
                .agents
                .detail
                .as_ref()
                .is_some_and(|detail| !detail.profile.skill_refs().is_empty())
            {
                model.agents.skill_panel_open = true;
                model.agents.selected_assigned_skill = 0;
                model.agents.selected_skill_action_index = 0;
            }
            ControllerEffect::Redraw
        }
        (AgentsPane::Detail, KeyCode::Down) if no_modifiers(key.modifiers) => {
            model.agents.detail_scroll = model.agents.detail_scroll.saturating_add(1);
            ControllerEffect::Redraw
        }
        (AgentsPane::Detail, KeyCode::Up) if no_modifiers(key.modifiers) => {
            model.agents.detail_scroll = model.agents.detail_scroll.saturating_sub(1);
            ControllerEffect::Redraw
        }
        (AgentsPane::Detail, KeyCode::Char('e')) if no_modifiers(key.modifiers) => {
            model.agents.pane = AgentsPane::Editor;
            ControllerEffect::StartProfileEdit {
                selected_profile: model.agents.selected_profile,
            }
        }
        (AgentsPane::History, KeyCode::Down) if no_modifiers(key.modifiers) => {
            let last = model
                .agents
                .history
                .as_ref()
                .map(|history| history.versions.len().saturating_sub(1))
                .unwrap_or(0);
            model.agents.selected_history_version = model
                .agents
                .selected_history_version
                .saturating_add(1)
                .min(last);
            model.agents.history_scroll = model.agents.selected_history_version;
            ControllerEffect::Redraw
        }
        (AgentsPane::History, KeyCode::Up) if no_modifiers(key.modifiers) => {
            model.agents.selected_history_version =
                model.agents.selected_history_version.saturating_sub(1);
            model.agents.history_scroll = model.agents.selected_history_version;
            ControllerEffect::Redraw
        }
        (AgentsPane::History, KeyCode::Enter) if no_modifiers(key.modifiers) => {
            let selection = model.agents.history.as_ref().and_then(|history| {
                history
                    .versions
                    .get(model.agents.selected_history_version)
                    .map(|entry| (history.profile_id, entry.version))
            });
            let Some((profile_id, version)) = selection else {
                return Some(ControllerEffect::Redraw);
            };
            ControllerEffect::LoadAgentProfileVersion {
                profile_id,
                version,
            }
        }
        (_, KeyCode::Esc) if no_modifiers(key.modifiers) => return Some(unwind_agents(model)),
        _ => return None,
    };
    Some(effect)
}

fn handle_agent_skills_key(model: &mut TuiModel, key: KeyEvent) -> Option<ControllerEffect> {
    let effect = match key.code {
        KeyCode::Down if no_modifiers(key.modifiers) => {
            let previous_action = model.selected_available_agent_skill_action();
            let last = model
                .agents
                .detail
                .as_ref()
                .map(|detail| detail.profile.skill_refs().len().saturating_sub(1))
                .unwrap_or(0);
            model.agents.selected_assigned_skill = model
                .agents
                .selected_assigned_skill
                .saturating_add(1)
                .min(last);
            model.select_available_agent_skill_action(previous_action);
            ControllerEffect::Redraw
        }
        KeyCode::Up if no_modifiers(key.modifiers) => {
            let previous_action = model.selected_available_agent_skill_action();
            model.agents.selected_assigned_skill =
                model.agents.selected_assigned_skill.saturating_sub(1);
            model.select_available_agent_skill_action(previous_action);
            ControllerEffect::Redraw
        }
        KeyCode::Right if no_modifiers(key.modifiers) => {
            move_agent_skill_action(model, true);
            ControllerEffect::Redraw
        }
        KeyCode::Left if no_modifiers(key.modifiers) => {
            move_agent_skill_action(model, false);
            ControllerEffect::Redraw
        }
        KeyCode::Enter if no_modifiers(key.modifiers) => {
            let detail = model.agents.detail.as_ref()?;
            let selected = detail
                .profile
                .skill_refs()
                .get(model.agents.selected_assigned_skill)?
                .clone();
            match model.selected_available_agent_skill_action() {
                AgentSkillAction::View => {
                    model.skills.workspace_origin = Some(SkillWorkspaceOrigin::AgentSkills {
                        profile_id: detail.profile.profile_id(),
                    });
                    model.skills.clear_skill_context();
                    model.skills.active = true;
                    ControllerEffect::LoadSkillVersion {
                        skill_id: selected.skill_id(),
                        version: selected.version(),
                    }
                }
                AgentSkillAction::Upgrade => {
                    let super::model::AgentSkillUpgradeAvailability::Available(replacement) =
                        model.agent_skill_upgrade_availability()
                    else {
                        model.select_available_agent_skill_action(AgentSkillAction::View);
                        return Some(ControllerEffect::Redraw);
                    };
                    model.skills.operation_origin = SkillOperationOrigin::AgentSkills {
                        profile_id: detail.profile.profile_id(),
                    };
                    ControllerEffect::RequestSkillAssignmentPreview {
                        profile_id: detail.profile.profile_id(),
                        expected_active_profile_version_id: detail.profile.profile_version_id(),
                        target: replacement,
                        assignment: AssignmentKind::Upgrade { expected: selected },
                    }
                }
                AgentSkillAction::Unassign => {
                    model.skills.operation_origin = SkillOperationOrigin::AgentSkills {
                        profile_id: detail.profile.profile_id(),
                    };
                    ControllerEffect::RequestSkillAssignmentPreview {
                        profile_id: detail.profile.profile_id(),
                        expected_active_profile_version_id: detail.profile.profile_version_id(),
                        target: selected.clone(),
                        assignment: AssignmentKind::Unassign { expected: selected },
                    }
                }
            }
        }
        KeyCode::Esc if no_modifiers(key.modifiers) => {
            model.agents.skill_panel_open = false;
            ControllerEffect::Redraw
        }
        _ => return None,
    };
    Some(effect)
}

fn move_agent_skill_action(model: &mut TuiModel, forward: bool) {
    let actions = model.available_agent_skill_actions();
    let current = model.selected_available_agent_skill_action();
    let current_index = actions
        .iter()
        .position(|action| *action == current)
        .unwrap_or(0);
    let next_index = if forward {
        current_index.saturating_add(1).min(actions.len().saturating_sub(1))
    } else {
        current_index.saturating_sub(1)
    };
    model.select_available_agent_skill_action(actions[next_index]);
}

fn unwind_agents(model: &mut TuiModel) -> ControllerEffect {
    match model.agents.pane {
        AgentsPane::List => {
            model.select_view(View::Overview);
            ControllerEffect::Redraw
        }
        AgentsPane::Detail => {
            model.agents.skill_panel_open = false;
            model.agents.pane = AgentsPane::List;
            ControllerEffect::Redraw
        }
        AgentsPane::History => {
            model.agents.pane = AgentsPane::Detail;
            ControllerEffect::Redraw
        }
        AgentsPane::Editor => {
            model.agents.pane = AgentsPane::Detail;
            ControllerEffect::Redraw
        }
        AgentsPane::Confirmation => {
            model.agents.pending_confirmation = None;
            model.agents.pane = AgentsPane::Editor;
            ControllerEffect::Redraw
        }
    }
}

fn select_view(model: &mut TuiModel, view: View) -> ControllerEffect {
    select_workspace_view(model, view);
    ControllerEffect::Redraw
}

fn select_workspace_view(model: &mut TuiModel, view: View) {
    model.skills.active = false;
    model.select_view(view);
    model.set_focus(Focus::Workspace);
    model.scroll_home();
}

fn toggle_or_focus_inspector(model: &mut TuiModel) -> ControllerEffect {
    if model.focus == Focus::Inspector {
        model.inspector_open = false;
        model.set_focus(Focus::Workspace);
    } else {
        model.inspector_open = true;
        model.set_focus(Focus::Inspector);
    }
    ControllerEffect::Redraw
}

fn dismiss(model: &mut TuiModel) -> ControllerEffect {
    if model.focus == Focus::Inspector || model.inspector_open {
        model.inspector_open = false;
        model.set_focus(Focus::Workspace);
        ControllerEffect::Redraw
    } else if model.message.is_some() {
        model.clear_message();
        ControllerEffect::Redraw
    } else {
        ControllerEffect::None
    }
}

fn cycle_focus(model: &mut TuiModel, forward: bool) -> ControllerEffect {
    let order = visible_focus_order(model);
    let current = order
        .iter()
        .position(|focus| *focus == model.focus)
        .unwrap_or(0);
    let next = if forward {
        current.saturating_add(1) % order.len()
    } else if current == 0 {
        order.len().saturating_sub(1)
    } else {
        current - 1
    };
    model.set_focus(order[next]);
    ControllerEffect::Redraw
}

fn visible_focus_order(model: &TuiModel) -> &'static [Focus] {
    const WIDE: &[Focus] = &[
        Focus::Navigation,
        Focus::Workspace,
        Focus::Inspector,
        Focus::Command,
    ];
    const MEDIUM: &[Focus] = &[Focus::Navigation, Focus::Workspace, Focus::Command];
    const MEDIUM_INSPECTOR: &[Focus] = &[
        Focus::Navigation,
        Focus::Workspace,
        Focus::Inspector,
        Focus::Command,
    ];
    const NARROW: &[Focus] = &[Focus::Workspace, Focus::Command];
    const NARROW_INSPECTOR: &[Focus] = &[Focus::Workspace, Focus::Inspector, Focus::Command];

    match (model.layout_mode, model.inspector_open) {
        (LayoutMode::Wide, _) => WIDE,
        (LayoutMode::Medium, true) => MEDIUM_INSPECTOR,
        (LayoutMode::Medium, false) => MEDIUM,
        (LayoutMode::Narrow, true) => NARROW_INSPECTOR,
        (LayoutMode::Narrow, false) => NARROW,
        (LayoutMode::TooSmall, _) => &[Focus::Workspace],
    }
}

fn normalize_focus(model: &mut TuiModel) {
    if !visible_focus_order(model).contains(&model.focus) {
        model.set_focus(Focus::Workspace);
    }
}

fn move_focused(model: &mut TuiModel, forward: bool, page: bool) -> ControllerEffect {
    if model.focus == Focus::Navigation {
        model.select_view(adjacent_view(model.active_view, forward));
        model.scroll_home();
        return ControllerEffect::Redraw;
    }

    if model.active_view == View::Audit {
        let count = if page {
            usize::from(model.workspace_body_height.max(1))
        } else {
            1
        };
        for _ in 0..count {
            if forward {
                model.select_next_audit();
            } else {
                model.select_previous_audit();
            }
        }
    } else if forward {
        model.scroll_down(if page {
            model.workspace_body_height.max(1)
        } else {
            1
        });
    } else {
        model.scroll_up(if page {
            model.workspace_body_height.max(1)
        } else {
            1
        });
    }
    ControllerEffect::Redraw
}

fn move_to_bound(model: &mut TuiModel, end: bool) -> ControllerEffect {
    if model.focus == Focus::Navigation {
        model.select_view(if end { View::Help } else { View::Overview });
    } else if model.active_view == View::Audit {
        if end {
            model.select_last_audit();
        } else {
            model.select_first_audit();
        }
    } else if end {
        model.workspace_scroll = workspace_max_scroll(model);
    } else {
        model.scroll_home();
    }
    ControllerEffect::Redraw
}

fn workspace_max_scroll(model: &TuiModel) -> u16 {
    if model.active_view == View::Audit {
        return 0;
    }
    views::workspace_content_height(model, model.workspace_body_width)
        .saturating_sub(model.workspace_body_height)
}

fn clamp_workspace_scroll(model: &mut TuiModel) {
    model.workspace_scroll = model.workspace_scroll.min(workspace_max_scroll(model));
}

fn adjacent_view(view: View, forward: bool) -> View {
    match (view, forward) {
        (View::Overview, true) | (View::Audit, false) => View::Setup,
        (View::Setup, true) | (View::Help, false) => View::Audit,
        (View::Audit, true) | (View::Overview, false) => View::Help,
        (View::Help, true) | (View::Setup, false) => View::Overview,
        (View::Agents, _) => View::Overview,
    }
}

fn merge_committed_audit(model: &mut TuiModel, committed: Vec<AuditEntry>) {
    let mut entries = std::mem::take(&mut model.audit_entries);
    for entry in committed {
        if let Some(existing) = entries
            .iter_mut()
            .find(|existing| existing.sequence == entry.sequence)
        {
            *existing = entry;
        } else {
            entries.push(entry);
        }
    }
    entries.sort_by_key(|entry| entry.sequence);
    model.replace_audit(entries);
}

fn edit(model: &mut TuiModel, operation: impl FnOnce(&mut TuiModel)) -> ControllerEffect {
    operation(model);
    ControllerEffect::Redraw
}

fn requests_shutdown(disposition: ShutdownDisposition) -> bool {
    match disposition {
        ShutdownDisposition::Continue => false,
        ShutdownDisposition::Requested => true,
    }
}

fn is_ctrl_c(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL
}

fn is_plain_char(key: KeyEvent, expected: char) -> bool {
    key.code == KeyCode::Char(expected) && no_modifiers(key.modifiers)
}

fn no_modifiers(modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::NONE
}

fn backtab_modifiers(modifiers: KeyModifiers) -> bool {
    matches!(modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
}

fn text_modifiers(modifiers: KeyModifiers) -> bool {
    matches!(modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use uuid::Uuid;

    use super::{ControllerEffect, apply_outcome, handle_event};
    use crate::{
        app::{
            ApplicationCommand, ApplicationEvent, AuditLimit, AuditTailView, CommandOutcome,
            CommandView, EventEnvelope, HelpView, InputRejectedView, InputRejection,
            InputRejectionCategory, MAX_INPUT_BYTES, PresentationSnapshot, SetupStatusView,
            ShutdownDisposition, ShutdownReason, ShutdownView, StatusView,
        },
        audit::AuditEntry,
        domain::{
            Actor, CommandId, ConfigurationVersionId, CorrelationId, EventId, InstallationId,
            SessionId, sha256,
        },
        setup::SetupStatus,
        ui::tui::{
            TuiEvent,
            model::{Focus, LayoutMode, Severity, TuiModel, View},
        },
    };

    fn model() -> TuiModel {
        TuiModel::new(
            PresentationSnapshot {
                installation_id: installation_id(1),
                session_id: session_id(2),
                database_readiness: crate::app::DatabaseReadiness::Ready,
                process_guard_ownership: crate::app::ProcessGuardOwnership::Held,
                setup_status: SetupStatus::NotStarted,
                recent_audit: Vec::new(),
                agent_profiles: crate::app::AgentProfilesView {
                    profiles: Vec::new(),
                    total_count: 0,
                    returned_count: 0,
                    truncated: false,
                },
                selected_agent_profile: None,
                selected_agent_profile_history: None,
            },
            false,
        )
    }

    fn command_model(text: &str) -> TuiModel {
        let mut model = model();
        model.focus = Focus::Command;
        for character in text.chars() {
            model.command.insert(character);
        }
        model
    }

    fn key(character: char) -> TuiEvent {
        key_code(KeyCode::Char(character), KeyModifiers::NONE)
    }

    fn key_code(code: KeyCode, modifiers: KeyModifiers) -> TuiEvent {
        TuiEvent::Key(KeyEvent::new(code, modifiers))
    }

    fn enter() -> TuiEvent {
        key_code(KeyCode::Enter, KeyModifiers::NONE)
    }

    fn assert_redraw_and_view(model: &mut TuiModel, event: TuiEvent, view: View) {
        assert_eq!(handle_event(model, event), ControllerEffect::Redraw);
        assert_eq!(model.active_view, view);
    }

    fn installation_id(value: u128) -> InstallationId {
        InstallationId::from_uuid(Uuid::from_u128(value))
    }

    fn session_id(value: u128) -> SessionId {
        SessionId::from_uuid(Uuid::from_u128(value))
    }

    fn audit_entry(sequence: u64) -> AuditEntry {
        AuditEntry {
            sequence,
            occurred_at_ms: sequence as i64,
            actor: Actor::System,
            kind: "help_viewed".to_owned(),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(sequence as u128 + 100)),
            summary: "help viewed".to_owned(),
        }
    }

    fn envelope(sequence: u64, event: ApplicationEvent) -> EventEnvelope {
        EventEnvelope {
            sequence,
            event_id: EventId::from_uuid(Uuid::from_u128(sequence as u128 + 200)),
            event_schema_version: crate::app::EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: sequence as i64,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(sequence as u128 + 300)),
            causation_id: None,
            object: None,
            event,
            previous_event_digest: (sequence > 1).then(|| sha256(b"previous")),
            event_digest: sha256(format!("event-{sequence}").as_bytes()),
        }
    }

    fn outcome(
        view: CommandView,
        committed_events: Vec<EventEnvelope>,
        shutdown: ShutdownDisposition,
    ) -> CommandOutcome {
        CommandOutcome {
            command_id: CommandId::from_uuid(Uuid::from_u128(400)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(401)),
            committed_events,
            view,
            shutdown,
        }
    }

    #[test]
    fn global_keys_switch_views_focus_inspector_and_leave_bare_q_inert() {
        let mut model = model();
        assert_redraw_and_view(&mut model, key('2'), View::Setup);
        assert_redraw_and_view(&mut model, key('3'), View::Audit);
        assert_redraw_and_view(&mut model, key('4'), View::Help);
        assert_eq!(handle_event(&mut model, key('i')), ControllerEffect::Redraw);
        assert!(model.inspector_open);
        assert_eq!(model.focus, Focus::Inspector);
        assert_eq!(handle_event(&mut model, key('/')), ControllerEffect::Redraw);
        assert_eq!(model.focus, Focus::Command);
        assert_eq!(model.command.text(), "/");
        assert_eq!(handle_event(&mut model, key('q')), ControllerEffect::Redraw);
        assert_eq!(model.command.text(), "/q");
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Esc, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        let before_q = model.clone();
        assert_eq!(handle_event(&mut model, key('q')), ControllerEffect::None);
        assert_eq!(model, before_q);
    }

    #[test]
    fn ctrl_c_and_interrupt_request_the_canonical_interrupted_shutdown_globally() {
        for mut model in [command_model("q"), model()] {
            model.layout_mode = LayoutMode::TooSmall;
            assert_eq!(
                handle_event(
                    &mut model,
                    key_code(KeyCode::Char('c'), KeyModifiers::CONTROL),
                ),
                ControllerEffect::RequestShutdown(ShutdownReason::Interrupted)
            );
        }
        assert_eq!(
            handle_event(&mut model(), TuiEvent::Interrupt),
            ControllerEffect::RequestShutdown(ShutdownReason::Interrupted)
        );
    }

    #[test]
    fn unsupported_modified_chords_are_ignored_without_changing_model_state() {
        let mut navigation = model();
        navigation.focus = Focus::Navigation;
        navigation.active_view = View::Setup;

        let cases = [
            (
                model(),
                key_code(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                ),
            ),
            (
                model(),
                key_code(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL | KeyModifiers::ALT,
                ),
            ),
            (model(), key_code(KeyCode::Tab, KeyModifiers::CONTROL)),
            (
                command_model("/status"),
                key_code(KeyCode::Enter, KeyModifiers::ALT),
            ),
            (
                command_model("abc"),
                key_code(KeyCode::Left, KeyModifiers::ALT),
            ),
            (
                command_model("abc"),
                key_code(KeyCode::Delete, KeyModifiers::SUPER),
            ),
            (
                command_model("abc"),
                key_code(KeyCode::Char('x'), KeyModifiers::META),
            ),
            (navigation, key_code(KeyCode::Down, KeyModifiers::ALT)),
            (model(), key_code(KeyCode::Char('q'), KeyModifiers::SHIFT)),
        ];

        for (mut model, event) in cases {
            let before = model.clone();
            assert_eq!(handle_event(&mut model, event), ControllerEffect::None);
            assert_eq!(model, before);
        }
    }

    #[test]
    fn hyper_modified_enter_is_ignored_without_submitting_or_editing() {
        let mut model = command_model("/status");
        let before = model.clone();

        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Enter, KeyModifiers::HYPER),),
            ControllerEffect::None
        );
        assert_eq!(model, before);
    }

    #[test]
    fn shifted_question_mark_opens_help_outside_command_focus() {
        let mut model = model();
        model.active_view = View::Setup;

        assert_eq!(
            handle_event(
                &mut model,
                key_code(KeyCode::Char('?'), KeyModifiers::SHIFT),
            ),
            ControllerEffect::Redraw
        );
        assert_eq!(model.active_view, View::Help);
        assert_eq!(model.focus, Focus::Workspace);
    }

    #[test]
    fn exact_shift_bindings_enter_text_and_move_focus_backward() {
        let mut command = command_model("a");
        assert_eq!(
            handle_event(
                &mut command,
                key_code(KeyCode::Char('B'), KeyModifiers::SHIFT),
            ),
            ControllerEffect::Redraw
        );
        assert_eq!(command.command.text(), "aB");

        let mut workspace = model();
        assert_eq!(
            handle_event(
                &mut workspace,
                key_code(KeyCode::BackTab, KeyModifiers::SHIFT),
            ),
            ControllerEffect::Redraw
        );
        assert_eq!(workspace.focus, Focus::Navigation);
    }

    #[test]
    fn enter_uses_the_authoritative_parser_and_marks_one_command_in_flight() {
        let mut model = command_model("/status");
        let effect = handle_event(&mut model, enter());
        assert_eq!(
            effect,
            ControllerEffect::Submit(ApplicationCommand::ShowStatus)
        );
        assert!(model.command_in_flight);
        assert_eq!(model.command.text(), "");
        assert_eq!(model.command.history_back(), Some("/status"));
    }

    #[test]
    fn enter_during_an_in_flight_command_does_not_submit_again() {
        let mut model = command_model("/audit 5");
        model.command_in_flight = true;
        assert_eq!(handle_event(&mut model, enter()), ControllerEffect::None);
        assert_eq!(model.message.as_ref().unwrap().severity, Severity::Warning);
        assert_eq!(model.command.text(), "/audit 5");
    }

    #[test]
    fn ignored_and_rejected_input_do_not_leave_raw_text_in_the_model() {
        let mut blank = command_model("   ");
        assert_eq!(handle_event(&mut blank, enter()), ControllerEffect::None);
        assert_eq!(blank.focus, Focus::Command);
        assert_eq!(blank.command.text(), "");

        let secret = "/unknown credential=top-secret";
        let mut rejected = command_model(secret);
        assert!(matches!(
            handle_event(&mut rejected, enter()),
            ControllerEffect::Submit(ApplicationCommand::RejectInput(_))
        ));
        assert_eq!(rejected.command.text(), "");
        assert_eq!(rejected.command.history_len(), 0);
        assert!(!format!("{rejected:?}").contains("top-secret"));
    }

    #[test]
    fn command_focus_edits_text_and_traverses_memory_only_history() {
        let mut model = command_model("ac");
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Left, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(handle_event(&mut model, key('b')), ControllerEffect::Redraw);
        assert_eq!(model.command.text(), "abc");
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Backspace, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.command.text(), "ac");
        model.command.remember("/help".to_owned());
        model.command.remember("/status".to_owned());
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Up, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.command.text(), "/status");
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Down, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.command.text(), "");
    }

    #[test]
    fn bare_q_never_requests_shutdown_in_normal_or_too_small_layout() {
        let mut normal = model();
        let normal_before_q = normal.clone();
        assert_eq!(handle_event(&mut normal, key('q')), ControllerEffect::None);
        assert_eq!(normal, normal_before_q);

        let mut tiny = model();
        tiny.layout_mode = LayoutMode::TooSmall;
        let before = tiny.clone();
        for event in [
            key('1'),
            key('i'),
            key('?'),
            key_code(KeyCode::Tab, KeyModifiers::NONE),
            key_code(KeyCode::Enter, KeyModifiers::NONE),
        ] {
            assert_eq!(handle_event(&mut tiny, event), ControllerEffect::None);
            assert_eq!(tiny, before);
        }
        assert_eq!(handle_event(&mut tiny, key('q')), ControllerEffect::None);
        assert_eq!(tiny, before);

        let mut command_tiny = command_model("");
        command_tiny.layout_mode = LayoutMode::TooSmall;
        assert_eq!(
            handle_event(&mut command_tiny, key('q')),
            ControllerEffect::Redraw
        );
        assert_eq!(command_tiny.command.text(), "q");
    }

    #[test]
    fn resize_focus_and_navigation_keys_update_only_model_state() {
        let mut model = model();
        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(59, 18)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.layout_mode, LayoutMode::TooSmall);
        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(120, 30)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.layout_mode, LayoutMode::Wide);

        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Tab, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.focus, Focus::Inspector);
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::BackTab, KeyModifiers::SHIFT)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.focus, Focus::Workspace);

        model.active_view = View::Audit;
        model.replace_audit(vec![audit_entry(1), audit_entry(2), audit_entry(3)]);
        model.audit_selection = Some(1);
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Up, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.audit_selection, Some(0));
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::End, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.audit_selection, Some(2));
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Home, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.audit_selection, Some(0));
    }

    #[test]
    fn outcomes_map_every_native_view_clear_the_gate_and_keep_only_safe_state() {
        let mut model = model();
        model.command_in_flight = true;
        assert_eq!(
            apply_outcome(
                &mut model,
                outcome(
                    CommandView::Help(HelpView),
                    vec![envelope(1, ApplicationEvent::HelpViewed)],
                    ShutdownDisposition::Continue,
                ),
            ),
            ControllerEffect::Redraw
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.active_view, View::Help);
        assert_eq!(
            model.audit_entries,
            vec![audit_entry_from_event(1, ApplicationEvent::HelpViewed)]
        );

        let next_installation = installation_id(11);
        let next_session = session_id(12);
        model.command_in_flight = true;
        assert_eq!(
            apply_outcome(
                &mut model,
                outcome(
                    CommandView::Status(StatusView {
                        installation_id: next_installation,
                        session_id: next_session,
                    }),
                    Vec::new(),
                    ShutdownDisposition::Continue,
                ),
            ),
            ControllerEffect::Redraw
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.active_view, View::Overview);
        assert_eq!(model.installation_id, next_installation);
        assert_eq!(model.session_id, next_session);

        let setup_status = SetupStatus::Applied {
            configuration_id: ConfigurationVersionId::from_uuid(Uuid::from_u128(13)),
        };
        model.command_in_flight = true;
        apply_outcome(
            &mut model,
            outcome(
                CommandView::SetupStatus(SetupStatusView {
                    status: setup_status.clone(),
                }),
                Vec::new(),
                ShutdownDisposition::Continue,
            ),
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.active_view, View::Setup);
        assert_eq!(model.setup_status, setup_status);

        model.command_in_flight = true;
        apply_outcome(
            &mut model,
            outcome(
                CommandView::AuditTail(AuditTailView {
                    limit: AuditLimit::new(100).unwrap(),
                    entries: (1..=105).map(audit_entry).collect(),
                }),
                Vec::new(),
                ShutdownDisposition::Continue,
            ),
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.active_view, View::Audit);
        assert_eq!(model.audit_entries.len(), 100);
        assert_eq!(model.audit_entries.first().unwrap().sequence, 6);

        let secret = b"/unknown credential=top-secret";
        model.command_in_flight = true;
        apply_outcome(
            &mut model,
            outcome(
                CommandView::InputRejected(InputRejectedView {
                    rejection: InputRejection::from_input(
                        InputRejectionCategory::Unknown,
                        None,
                        secret,
                    ),
                }),
                Vec::new(),
                ShutdownDisposition::Continue,
            ),
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.message.as_ref().unwrap().severity, Severity::Error);
        assert!(!format!("{model:?}").contains("top-secret"));

        model.command_in_flight = true;
        assert_eq!(
            apply_outcome(
                &mut model,
                outcome(
                    CommandView::Shutdown(ShutdownView {
                        disposition: ShutdownDisposition::Requested,
                    }),
                    Vec::new(),
                    ShutdownDisposition::Requested,
                ),
            ),
            ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
        );
        assert!(!model.command_in_flight);
    }

    #[test]
    fn committed_events_append_as_the_newest_one_hundred_audit_entries() {
        let mut model = model();
        model.replace_audit((1..=75).map(audit_entry).collect());
        let committed = (76..=125)
            .map(|sequence| envelope(sequence, ApplicationEvent::StatusViewed))
            .collect();
        apply_outcome(
            &mut model,
            outcome(
                CommandView::Status(StatusView {
                    installation_id: installation_id(1),
                    session_id: session_id(2),
                }),
                committed,
                ShutdownDisposition::Continue,
            ),
        );
        assert_eq!(model.audit_entries.len(), 100);
        assert_eq!(model.audit_entries.first().unwrap().sequence, 26);
        assert_eq!(model.audit_entries.last().unwrap().sequence, 125);
        assert!(
            model.audit_entries[75..]
                .iter()
                .all(|entry| entry.kind == "status_viewed")
        );
    }

    #[test]
    fn too_small_resize_preserves_command_text_and_accepts_quit_completion() {
        let mut model = model();
        assert_eq!(handle_event(&mut model, key('/')), ControllerEffect::Redraw);
        assert_eq!(model.focus, Focus::Command);
        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(59, 17)),
            ControllerEffect::Redraw
        );

        for character in "quit".chars() {
            assert_eq!(
                handle_event(&mut model, key(character)),
                ControllerEffect::Redraw
            );
        }
        assert_eq!(model.command.text(), "/quit");
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Enter, KeyModifiers::NONE)),
            ControllerEffect::Submit(ApplicationCommand::RequestShutdown)
        );
    }

    #[test]
    fn paste_is_ignored_outside_command_focus_and_bounded_and_sanitized_inside() {
        let mut model = model();
        assert_eq!(
            handle_event(&mut model, TuiEvent::Paste("/status".to_owned())),
            ControllerEffect::None
        );
        assert!(model.command.text().is_empty());

        model.set_focus(Focus::Command);
        let pasted = format!("{}\n\t界", "a".repeat(MAX_INPUT_BYTES - 1));
        assert_eq!(
            handle_event(&mut model, TuiEvent::Paste(pasted)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.command.text().len(), MAX_INPUT_BYTES - 1);
        assert!(!model.command.text().chars().any(char::is_control));
        assert!(
            model
                .command
                .text()
                .is_char_boundary(model.command.text().len())
        );
    }

    #[test]
    fn audit_end_and_page_navigation_remain_selection_based() {
        let mut model = model();
        model.select_view(View::Audit);
        model.replace_audit((1..=30).map(audit_entry).collect());
        handle_event(&mut model, TuiEvent::Resize(60, 18));

        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::End, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.audit_selection, Some(29));
        assert_eq!(model.workspace_scroll, 0);

        handle_event(&mut model, key_code(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(model.audit_selection, Some(29));
        assert_eq!(model.workspace_scroll, 0);
    }

    fn audit_entry_from_event(sequence: u64, event: ApplicationEvent) -> AuditEntry {
        AuditEntry::from_event(&envelope(sequence, event))
    }
}
