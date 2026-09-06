use std::{panic::AssertUnwindSafe, time::Duration};

use super::{
    TuiError,
    controller::{ControllerEffect, apply_outcome, handle_event},
    event::{CrosstermEventSource, EventSource, TuiEvent},
    model::{RuntimeStatus, TuiModel},
    terminal::{CrosstermScreen, Screen},
    theme::Theme,
};
use crate::{
    agents::AgentProfileDraft,
    app::{
        AgentSkillAssignmentOperation, AgentSkillAssignmentPreview, AppError, ApplicationCommand,
        CommandOutcome, CommandView, PresentationSnapshot, ShutdownReason,
    },
    panic_boundary::catch_sensitive_unwind,
    runtime::{ApplicationRuntime, PendingOutcome, RuntimeClient, RuntimeError},
    ui::{
        profile_editor::{PreviewEditRequest, ProfileEditor},
        skill_editor::SkillPreviewRequest,
    },
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const STALE_PROFILE_MESSAGE: &str =
    "Profile changed elsewhere. Detail refreshed; reopen Edit to continue.";

#[doc(hidden)]
pub fn execute_agent_effect(
    client: &RuntimeClient,
    model: &mut TuiModel,
    effect: ControllerEffect,
) -> Result<(), RuntimeError> {
    match effect {
        ControllerEffect::LoadAgentProfiles => {
            let outcome =
                submit_agent_command(client, model, ApplicationCommand::ListAgentProfiles)?;
            apply_agent_outcome(model, outcome);
        }
        ControllerEffect::LoadAgentProfile { selected_profile } => {
            load_profile(client, model, selected_profile)?;
        }
        ControllerEffect::LoadAgentProfileHistory { selected_profile } => {
            load_history(client, model, selected_profile)?;
        }
        ControllerEffect::LoadAgentProfileVersion {
            profile_id,
            version,
        } => {
            let outcome = submit_agent_command(
                client,
                model,
                ApplicationCommand::ShowAgentProfileVersion {
                    selector: profile_id.into(),
                    version,
                },
            )?;
            apply_agent_outcome(model, outcome);
        }
        ControllerEffect::StartProfileCreate { template_index } => {
            model.set_command_in_flight(true);
            let templates = client.agent_profile_templates();
            model.set_command_in_flight(false);
            let templates = templates?;
            if model
                .agents
                .start_profile_create(template_index, &templates)
            {
                model.command.clear();
                model.clear_message();
            } else {
                model.agents.pane = super::model::AgentsPane::List;
                model.set_message(
                    super::model::Severity::Warning,
                    "Profile template is unavailable.",
                );
            }
        }
        ControllerEffect::StartProfileEdit { selected_profile } => {
            load_profile(client, model, selected_profile)?;
            start_profile_editor_from_detail(model)?;
        }
        ControllerEffect::StartProfileCreateByTemplate { template_id } => {
            model.set_command_in_flight(true);
            let templates = client.agent_profile_templates();
            model.set_command_in_flight(false);
            let templates = templates?;
            let template_index = templates
                .iter()
                .position(|template| template.id == template_id);
            if template_index
                .is_some_and(|index| model.agents.start_profile_create(index, &templates))
            {
                model.command.clear();
                model.clear_message();
            } else {
                model.agents.pane = super::model::AgentsPane::List;
                model.set_message(
                    super::model::Severity::Warning,
                    "Profile template is unavailable.",
                );
            }
        }
        ControllerEffect::StartProfileEditBySelector { selector } => {
            match submit_agent_command(
                client,
                model,
                ApplicationCommand::ShowAgentProfile { selector },
            ) {
                Ok(outcome) => {
                    apply_agent_outcome(model, outcome);
                    start_profile_editor_from_detail(model)?;
                }
                Err(error @ (RuntimeError::Application(_) | RuntimeError::Backpressure)) => {
                    retain_profile_error(model, &error, super::model::AgentsPane::List);
                }
                Err(error) => return Err(error),
            }
        }
        ControllerEffect::RequestProfilePreview(request) => {
            execute_preview(client, model, request)?;
        }
        ControllerEffect::ExecuteProfile(command) => {
            execute_profile_command(client, model, command)?;
        }
        ControllerEffect::CancelProfileReview => {
            client.cancel_agent_profile_edit()?;
            model.clear_message();
        }
        ControllerEffect::None
        | ControllerEffect::Redraw
        | ControllerEffect::Submit(_)
        | ControllerEffect::RequestShutdown(_)
        | ControllerEffect::LoadSkills
        | ControllerEffect::LoadSkill { .. }
        | ControllerEffect::LoadSkillHistory { .. }
        | ControllerEffect::LoadSkillVersion { .. }
        | ControllerEffect::LoadSkillStarter { .. }
        | ControllerEffect::LoadSkillAgents
        | ControllerEffect::LoadSkillAgent { .. }
        | ControllerEffect::RequestSkillPreview(_)
        | ControllerEffect::RequestSkillAssignmentPreview { .. }
        | ControllerEffect::ExecuteSkill(_)
        | ControllerEffect::CancelSkillReview => {}
    }
    Ok(())
}

#[doc(hidden)]
pub fn execute_skill_effect(
    client: &RuntimeClient,
    model: &mut TuiModel,
    effect: ControllerEffect,
) -> Result<(), RuntimeError> {
    match effect {
        ControllerEffect::LoadSkills => {
            let outcome = submit_agent_command(client, model, ApplicationCommand::ListSkills)?;
            let _ = apply_outcome(model, outcome);
        }
        ControllerEffect::LoadSkill { selected_skill }
        | ControllerEffect::LoadSkillStarter { selected_skill } => {
            let Some(skill_id) = model
                .skills
                .library
                .skills
                .get(selected_skill)
                .map(|summary| summary.skill_ref.skill_id())
            else {
                model.set_message(super::model::Severity::Warning, "No skill is selected.");
                return Ok(());
            };
            let starter = matches!(effect, ControllerEffect::LoadSkillStarter { .. });
            let outcome = submit_agent_command(
                client,
                model,
                ApplicationCommand::ShowSkill {
                    selector: skill_id.into(),
                },
            )?;
            let _ = apply_outcome(model, outcome);
            if starter {
                let seed = model.skills.detail.as_ref().map(|detail| detail.content.clone());
                model.skills.start_create(seed);
                synchronize_host_skill_input(model);
            }
        }
        ControllerEffect::LoadSkillHistory { selected_skill } => {
            let Some(skill_id) = model
                .skills
                .library
                .skills
                .get(selected_skill)
                .map(|summary| summary.skill_ref.skill_id())
            else {
                return Ok(());
            };
            let outcome = submit_agent_command(
                client,
                model,
                ApplicationCommand::ShowSkillHistory {
                    selector: skill_id.into(),
                },
            )?;
            let _ = apply_outcome(model, outcome);
        }
        ControllerEffect::LoadSkillVersion { skill_id, version } => {
            let outcome = submit_agent_command(
                client,
                model,
                ApplicationCommand::ShowSkillVersion {
                    selector: skill_id.into(),
                    version,
                },
            )?;
            let _ = apply_outcome(model, outcome);
        }
        ControllerEffect::LoadSkillAgents => {
            let outcome = submit_agent_command(client, model, ApplicationCommand::ListAgentProfiles)?;
            let _ = apply_outcome(model, outcome);
            model.skills.pane = super::model::SkillsPane::AgentPicker;
        }
        ControllerEffect::LoadSkillAgent { selected_agent } => {
            let Some(profile_id) = model
                .agents
                .profiles
                .profiles
                .get(selected_agent)
                .map(|summary| summary.profile_id)
            else {
                return Ok(());
            };
            let outcome = submit_agent_command(
                client,
                model,
                ApplicationCommand::ShowAgentProfile {
                    selector: profile_id.into(),
                },
            )?;
            let _ = apply_outcome(model, outcome);
        }
        ControllerEffect::RequestSkillPreview(request) => {
            execute_skill_preview(client, model, request)?;
        }
        ControllerEffect::RequestSkillAssignmentPreview {
            profile_id,
            expected_active_profile_version_id,
            target,
            assignment,
        } => {
            let result = match assignment {
                super::model::AssignmentKind::Add => client.preview_agent_skill_assignment(
                    profile_id,
                    expected_active_profile_version_id,
                    target,
                ),
                super::model::AssignmentKind::Upgrade { expected } => client
                    .preview_agent_skill_upgrade(
                        profile_id,
                        expected_active_profile_version_id,
                        expected,
                        target,
                    ),
                super::model::AssignmentKind::Unassign { expected } => client
                    .preview_agent_skill_unassignment(
                        profile_id,
                        expected_active_profile_version_id,
                        expected,
                    ),
                super::model::AssignmentKind::AlreadyAssigned => return Ok(()),
            };
            match result {
                Ok(preview) => install_assignment_preview(model, preview),
                Err(error @ (RuntimeError::Application(_) | RuntimeError::Backpressure)) => {
                    recover_skill_error(client, model, &error)?;
                }
                Err(error) => return Err(error),
            }
        }
        ControllerEffect::ExecuteSkill(command) => {
            match submit_agent_command(client, model, command) {
                Ok(outcome) => {
                    let _ = apply_outcome(model, outcome);
                    model.skills.review_registered = false;
                    model.skills.pending_confirmation = None;
                    model.skills.editor = None;
                    model.skills.pane = super::model::SkillsPane::Result;
                    model.set_message(super::model::Severity::Info, "Skill action completed.");
                }
                Err(error @ (RuntimeError::Application(_) | RuntimeError::Backpressure)) => {
                    recover_skill_error(client, model, &error)?;
                }
                Err(error) => return Err(error),
            }
        }
        ControllerEffect::CancelSkillReview => cancel_skill_review_once(client, model)?,
        _ => {}
    }
    Ok(())
}

fn synchronize_host_skill_input(model: &mut TuiModel) {
    let value = model
        .skills
        .editor
        .as_ref()
        .map(|editor| editor.current_value().to_owned())
        .unwrap_or_default();
    model.command.clear();
    model.command.ingest(&value);
}

fn execute_skill_preview(
    client: &RuntimeClient,
    model: &mut TuiModel,
    request: SkillPreviewRequest,
) -> Result<(), RuntimeError> {
    let generation = request.generation();
    model.set_command_in_flight(true);
    let result = match request {
        SkillPreviewRequest::Create { candidate, .. } => client.preview_skill_creation(candidate),
        SkillPreviewRequest::Version {
            skill_id,
            expected_active_version_id,
            candidate,
            ..
        } => client.preview_skill_version(skill_id, expected_active_version_id, candidate),
    };
    model.set_command_in_flight(false);
    match result {
        Ok(preview) => {
            let installed = model
                .skills
                .editor
                .as_mut()
                .is_some_and(|editor| editor.apply_preview(generation, preview));
            model.skills.review_registered = true;
            if !installed {
                cancel_skill_review_once(client, model)?;
            }
            Ok(())
        }
        Err(error @ (RuntimeError::Application(_) | RuntimeError::Backpressure)) => {
            recover_skill_error(client, model, &error)
        }
        Err(error) => Err(error),
    }
}

fn install_assignment_preview(model: &mut TuiModel, preview: AgentSkillAssignmentPreview) {
    let command = match preview.operation {
        AgentSkillAssignmentOperation::Assign { skill } => ApplicationCommand::AssignAgentSkill {
            profile_id: preview.profile_id,
            expected_active_profile_version_id: preview.expected_active_profile_version_id,
            skill,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
        AgentSkillAssignmentOperation::Upgrade { expected, replacement } => {
            ApplicationCommand::UpgradeAgentSkill {
                profile_id: preview.profile_id,
                expected_active_profile_version_id: preview.expected_active_profile_version_id,
                expected,
                replacement,
                review_token: preview.review_token,
                review_digest: preview.review_digest,
            }
        }
        AgentSkillAssignmentOperation::Unassign { expected } => ApplicationCommand::UnassignAgentSkill {
            profile_id: preview.profile_id,
            expected_active_profile_version_id: preview.expected_active_profile_version_id,
            expected,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    };
    model.skills.review_registered = true;
    model.skills.pending_confirmation = Some(super::model::SkillConfirmation {
        command,
        origin: model.skills.operation_origin,
    });
    model.skills.pane = super::model::SkillsPane::Confirmation;
}

fn cancel_skill_review_once(
    client: &RuntimeClient,
    model: &mut TuiModel,
) -> Result<(), RuntimeError> {
    if std::mem::take(&mut model.skills.review_registered) {
        client.cancel_skill_review()?;
    }
    Ok(())
}

fn recover_skill_error(
    client: &RuntimeClient,
    model: &mut TuiModel,
    error: &RuntimeError,
) -> Result<(), RuntimeError> {
    cancel_skill_review_once(client, model)?;
    model.skills.pending_confirmation = None;
    if let Some(editor) = model.skills.editor.as_mut() {
        editor.clear_review();
        editor.report_error(runtime_error_code(error));
        model.skills.pane = super::model::SkillsPane::Editor;
    } else {
        model.skills.pane = super::model::SkillsPane::Detail;
    }
    model.set_command_in_flight(false);
    model.set_message(super::model::Severity::Error, "Skill action failed; review cleared.");
    Ok(())
}

fn submit_agent_command(
    client: &RuntimeClient,
    model: &mut TuiModel,
    command: ApplicationCommand,
) -> Result<CommandOutcome, RuntimeError> {
    model.set_command_in_flight(true);
    let result = client.submit(command);
    if result.is_err() {
        model.set_command_in_flight(false);
    }
    result
}

fn apply_agent_outcome(model: &mut TuiModel, outcome: CommandOutcome) {
    let view = outcome.view.clone();
    let _ = apply_outcome(model, outcome);
    match view {
        CommandView::AgentProfiles(profiles) => model.agents.replace_profiles(profiles),
        CommandView::AgentProfile(detail) => model.agents.replace_detail(detail),
        CommandView::AgentProfileHistory(history) => model.agents.replace_history(history),
        CommandView::AgentProfileVersion(version) => model.agents.replace_version_detail(version),
        _ => {}
    }
}

fn selected_profile_id(
    model: &mut TuiModel,
    selected_profile: usize,
) -> Option<crate::domain::AgentProfileId> {
    let profile_id = model
        .agents
        .profiles
        .profiles
        .get(selected_profile)
        .map(|summary| summary.profile_id);
    if profile_id.is_none() {
        model.agents.selected_profile = model.agents.profiles.profiles.len().saturating_sub(1);
        model.agents.pane = super::model::AgentsPane::List;
        model.set_message(
            super::model::Severity::Warning,
            "No agent profile is selected.",
        );
    }
    profile_id
}

fn load_profile(
    client: &RuntimeClient,
    model: &mut TuiModel,
    selected_profile: usize,
) -> Result<(), RuntimeError> {
    let Some(profile_id) = selected_profile_id(model, selected_profile) else {
        return Ok(());
    };
    let outcome = submit_agent_command(
        client,
        model,
        ApplicationCommand::ShowAgentProfile {
            selector: profile_id.into(),
        },
    )?;
    apply_agent_outcome(model, outcome);
    Ok(())
}

fn load_history(
    client: &RuntimeClient,
    model: &mut TuiModel,
    selected_profile: usize,
) -> Result<(), RuntimeError> {
    let Some(profile_id) = selected_profile_id(model, selected_profile) else {
        return Ok(());
    };
    let outcome = submit_agent_command(
        client,
        model,
        ApplicationCommand::ShowAgentProfileHistory {
            selector: profile_id.into(),
        },
    )?;
    apply_agent_outcome(model, outcome);
    Ok(())
}

fn execute_preview(
    client: &RuntimeClient,
    model: &mut TuiModel,
    request: PreviewEditRequest,
) -> Result<(), RuntimeError> {
    let PreviewEditRequest {
        generation,
        profile_id,
        expected_active_version_id,
        candidate,
    } = request;
    model.set_command_in_flight(true);
    let result =
        client.preview_agent_profile_edit(profile_id, expected_active_version_id, candidate);
    model.set_command_in_flight(false);
    match result {
        Ok(preview) => {
            if let Some(editor) = model.agents.editor.as_mut() {
                editor.apply_preview(generation, preview);
            }
            model.clear_message();
            Ok(())
        }
        Err(error) if is_stale(&error) => refresh_stale_profile(client, model, profile_id),
        Err(error @ (RuntimeError::Application(_) | RuntimeError::Backpressure)) => {
            if let Some(editor) = model.agents.editor.as_mut() {
                editor.report_error(runtime_error_code(&error));
            }
            retain_profile_error(model, &error, super::model::AgentsPane::Editor);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn execute_profile_command(
    client: &RuntimeClient,
    model: &mut TuiModel,
    command: ApplicationCommand,
) -> Result<(), RuntimeError> {
    let stale_profile_id = match &command {
        ApplicationCommand::ActivateAgentProfileVersion { profile_id, .. } => Some(*profile_id),
        _ => None,
    };
    let outcome = match submit_agent_command(client, model, command) {
        Ok(outcome) => outcome,
        Err(error) if is_stale(&error) => {
            return refresh_stale_profile(
                client,
                model,
                stale_profile_id.expect("stale profile activation carries an ID"),
            );
        }
        Err(error @ (RuntimeError::Application(_) | RuntimeError::Backpressure)) => {
            if let Some(editor) = model.agents.editor.as_mut() {
                editor.report_error(runtime_error_code(&error));
            }
            retain_profile_error(model, &error, super::model::AgentsPane::Confirmation);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let (profile_id, message) = match &outcome.view {
        CommandView::AgentProfileCreated(created) => {
            (Some(created.profile_id), "Agent profile created.")
        }
        CommandView::AgentProfileVersionActivated(activated) => (
            Some(activated.profile_id),
            "Agent profile version activated.",
        ),
        _ => (None, "Agent profile action completed."),
    };
    apply_agent_outcome(model, outcome);
    if let Some(profile_id) = profile_id {
        refresh_profile_state(client, model, profile_id)?;
    }
    model.agents.editor = None;
    model.agents.pending_confirmation = None;
    model.agents.pane = super::model::AgentsPane::Detail;
    model.set_message(super::model::Severity::Info, message);
    Ok(())
}

fn refresh_profile_state(
    client: &RuntimeClient,
    model: &mut TuiModel,
    profile_id: crate::domain::AgentProfileId,
) -> Result<(), RuntimeError> {
    let profiles = submit_agent_command(client, model, ApplicationCommand::ListAgentProfiles)?;
    apply_agent_outcome(model, profiles);
    if let Some(index) = model
        .agents
        .profiles
        .profiles
        .iter()
        .position(|summary| summary.profile_id == profile_id)
    {
        model.agents.selected_profile = index;
    }
    let detail = submit_agent_command(
        client,
        model,
        ApplicationCommand::ShowAgentProfile {
            selector: profile_id.into(),
        },
    )?;
    apply_agent_outcome(model, detail);
    let history = submit_agent_command(
        client,
        model,
        ApplicationCommand::ShowAgentProfileHistory {
            selector: profile_id.into(),
        },
    )?;
    apply_agent_outcome(model, history);
    Ok(())
}

fn refresh_stale_profile(
    client: &RuntimeClient,
    model: &mut TuiModel,
    profile_id: crate::domain::AgentProfileId,
) -> Result<(), RuntimeError> {
    let cancel_failed = client.cancel_agent_profile_edit().is_err();
    let mut refresh_failed = false;
    match submit_agent_command(client, model, ApplicationCommand::ListAgentProfiles) {
        Ok(outcome) => apply_agent_outcome(model, outcome),
        Err(_) => refresh_failed = true,
    }
    match submit_agent_command(
        client,
        model,
        ApplicationCommand::ShowAgentProfile {
            selector: profile_id.into(),
        },
    ) {
        Ok(outcome) => apply_agent_outcome(model, outcome),
        Err(_) => refresh_failed = true,
    }
    model.agents.editor = None;
    model.agents.pending_confirmation = None;
    model.agents.pane = super::model::AgentsPane::Detail;
    let message = match (cancel_failed, refresh_failed) {
        (false, false) => STALE_PROFILE_MESSAGE,
        (true, false) => {
            "Profile changed elsewhere. Detail refreshed, but review cleanup could not be confirmed."
        }
        (false, true) => {
            "Profile changed elsewhere. Review was cancelled, but profile refresh failed."
        }
        (true, true) => {
            "Profile changed elsewhere. Review cleanup and profile refresh both failed."
        }
    };
    model.set_message(super::model::Severity::Warning, message);
    Ok(())
}

fn is_stale(error: &RuntimeError) -> bool {
    matches!(
        error,
        RuntimeError::Application(AppError::StaleAgentProfileVersion)
    )
}

fn start_profile_editor_from_detail(model: &mut TuiModel) -> Result<(), RuntimeError> {
    if let Some(detail) = model.agents.detail.as_ref() {
        let draft = draft_from_profile(&detail.profile)
            .map_err(|error| RuntimeError::Application(error.into()))?;
        model.agents.editor = Some(ProfileEditor::for_edit(
            detail.profile.profile_id(),
            detail.profile.profile_version_id(),
            draft,
        ));
        model.agents.pane = super::model::AgentsPane::Editor;
        model.command.clear();
        model.clear_message();
    }
    Ok(())
}

fn retain_profile_error(
    model: &mut TuiModel,
    error: &RuntimeError,
    pane: super::model::AgentsPane,
) {
    model.set_command_in_flight(false);
    model.agents.pane = pane;
    let message = match error {
        RuntimeError::Backpressure => "Command queue is busy; draft retained.",
        RuntimeError::Application(_) => "Profile action failed; draft retained.",
        _ => "Profile action is unavailable.",
    };
    model.set_message(super::model::Severity::Error, message);
}

fn runtime_error_code(error: &RuntimeError) -> &'static str {
    match error {
        RuntimeError::Application(error) => error.code(),
        RuntimeError::Backpressure => "command_backpressure",
        _ => "runtime_unavailable",
    }
}

fn draft_from_profile(
    profile: &crate::agents::AgentProfileVersion,
) -> Result<AgentProfileDraft, crate::domain::DomainError> {
    AgentProfileDraft::new(
        profile.display_name().to_owned(),
        profile.description().to_owned(),
        profile.role(),
        profile.primary_specialty().to_owned(),
        profile.specialty_tags().to_vec(),
        profile.personality().to_owned(),
        profile.instructions().to_owned(),
        profile.bindings().clone(),
        profile.skill_refs().to_vec(),
        profile.mcp_refs().to_vec(),
    )
}

pub fn run_tui(
    runtime: ApplicationRuntime,
    snapshot: PresentationSnapshot,
    previous_session_interrupted: bool,
) -> Result<(), TuiError> {
    let theme = Theme::from_no_color(std::env::var_os("NO_COLOR").is_some());
    let runner = TuiRunner::new(runtime, snapshot, previous_session_interrupted);
    let mut events = match CrosstermEventSource::new() {
        Ok(events) => events,
        Err(error) => return finish_with_primary(runner, error),
    };
    let mut screen = match CrosstermScreen::new() {
        Ok(screen) => screen,
        Err(error) => return finish_with_primary(runner, error),
    };

    let result = run_with_screen(runner, &mut screen, &mut events, &theme);
    drop(screen);
    result
}

#[doc(hidden)]
pub fn run_tui_with_screen(
    runtime: ApplicationRuntime,
    snapshot: PresentationSnapshot,
    previous_session_interrupted: bool,
    screen: &mut dyn Screen,
    events: &mut dyn EventSource,
    theme: &Theme,
) -> Result<(), TuiError> {
    run_with_screen(
        TuiRunner::new(runtime, snapshot, previous_session_interrupted),
        screen,
        events,
        theme,
    )
}

struct TuiRunner {
    runtime: Option<ApplicationRuntime>,
    client: RuntimeClient,
    model: TuiModel,
    pending: Option<PendingOutcome>,
    queued_shutdown: Option<ShutdownReason>,
}

impl TuiRunner {
    fn new(
        runtime: ApplicationRuntime,
        snapshot: PresentationSnapshot,
        previous_session_interrupted: bool,
    ) -> Self {
        let client = runtime.client();
        Self {
            runtime: Some(runtime),
            client,
            model: TuiModel::new(snapshot, previous_session_interrupted),
            pending: None,
            queued_shutdown: None,
        }
    }

    fn event_loop(
        &mut self,
        screen: &mut dyn Screen,
        events: &mut dyn EventSource,
        theme: &Theme,
    ) -> Result<ShutdownReason, TuiError> {
        self.update_layout(screen)?;
        screen.draw(&self.model, theme)?;

        loop {
            let mut dirty = false;

            if let Some(effect) = self.poll_pending()? {
                if let ControllerEffect::RequestShutdown(reason) = effect {
                    return Ok(reason);
                }
                match self.apply_effect(effect)? {
                    LoopControl::Continue { redraw } => dirty |= redraw,
                    LoopControl::Finish(reason) => return Ok(reason),
                }
            }

            if self.pending.is_none() && self.queued_shutdown.is_some() {
                self.submit_queued_shutdown()?;
                dirty = true;
            }

            dirty |= self.update_layout(screen)?;

            if let Some(event) = events.next_event(EVENT_POLL_INTERVAL)? {
                let effect = handle_event(&mut self.model, event);
                match self.apply_effect(effect)? {
                    LoopControl::Continue { redraw } => dirty |= redraw,
                    LoopControl::Finish(reason) => return Ok(reason),
                }
            }

            if dirty {
                screen.draw(&self.model, theme)?;
            }
        }
    }

    fn poll_pending(&mut self) -> Result<Option<ControllerEffect>, TuiError> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(None);
        };
        let Some(outcome) = pending.try_recv()? else {
            return Ok(None);
        };
        self.pending.take();
        Ok(Some(apply_outcome(&mut self.model, outcome)))
    }

    fn update_layout(&mut self, screen: &dyn Screen) -> Result<bool, TuiError> {
        let area = screen.size()?;
        if (area.width, area.height) == (self.model.terminal_width, self.model.terminal_height) {
            return Ok(false);
        }
        let effect = handle_event(&mut self.model, TuiEvent::Resize(area.width, area.height));
        Ok(matches!(effect, ControllerEffect::Redraw))
    }

    fn apply_effect(&mut self, effect: ControllerEffect) -> Result<LoopControl, TuiError> {
        match effect {
            ControllerEffect::None => Ok(LoopControl::Continue { redraw: false }),
            ControllerEffect::Redraw => Ok(LoopControl::Continue { redraw: true }),
            ControllerEffect::Submit(ApplicationCommand::RequestShutdown) => {
                self.request_auditable_shutdown(ShutdownReason::UserQuit)
            }
            ControllerEffect::Submit(command) => {
                if self.model.runtime_status == RuntimeStatus::Stopping {
                    return Ok(LoopControl::Continue { redraw: false });
                }
                self.submit(command)?;
                Ok(LoopControl::Continue { redraw: true })
            }
            ControllerEffect::RequestShutdown(ShutdownReason::UserQuit) => {
                self.request_auditable_shutdown(ShutdownReason::UserQuit)
            }
            ControllerEffect::RequestShutdown(reason) => {
                self.begin_stopping();
                Ok(LoopControl::Finish(reason))
            }
            effect @ (ControllerEffect::LoadAgentProfiles
            | ControllerEffect::LoadAgentProfile { .. }
            | ControllerEffect::LoadAgentProfileHistory { .. }
            | ControllerEffect::LoadAgentProfileVersion { .. }
            | ControllerEffect::StartProfileCreate { .. }
            | ControllerEffect::StartProfileEdit { .. }
            | ControllerEffect::StartProfileCreateByTemplate { .. }
            | ControllerEffect::StartProfileEditBySelector { .. }
            | ControllerEffect::RequestProfilePreview(_)
            | ControllerEffect::ExecuteProfile(_)
            | ControllerEffect::CancelProfileReview) => {
                execute_agent_effect(&self.client, &mut self.model, effect)?;
                Ok(LoopControl::Continue { redraw: true })
            }
            effect @ (ControllerEffect::LoadSkills
            | ControllerEffect::LoadSkill { .. }
            | ControllerEffect::LoadSkillHistory { .. }
            | ControllerEffect::LoadSkillVersion { .. }
            | ControllerEffect::LoadSkillStarter { .. }
            | ControllerEffect::LoadSkillAgents
            | ControllerEffect::LoadSkillAgent { .. }
            | ControllerEffect::RequestSkillPreview(_)
            | ControllerEffect::RequestSkillAssignmentPreview { .. }
            | ControllerEffect::ExecuteSkill(_)
            | ControllerEffect::CancelSkillReview) => {
                execute_skill_effect(&self.client, &mut self.model, effect)?;
                Ok(LoopControl::Continue { redraw: true })
            }
        }
    }

    fn request_auditable_shutdown(
        &mut self,
        reason: ShutdownReason,
    ) -> Result<LoopControl, TuiError> {
        if self.model.runtime_status == RuntimeStatus::Stopping {
            return Ok(LoopControl::Continue { redraw: false });
        }

        self.begin_stopping();
        if self.pending.is_some() {
            self.queued_shutdown = Some(reason);
        } else {
            self.submit(ApplicationCommand::RequestShutdown)?;
        }
        Ok(LoopControl::Continue { redraw: true })
    }

    fn submit_queued_shutdown(&mut self) -> Result<(), TuiError> {
        let Some(_reason) = self.queued_shutdown.take() else {
            return Ok(());
        };
        self.submit(ApplicationCommand::RequestShutdown)
    }

    fn submit(&mut self, command: ApplicationCommand) -> Result<(), TuiError> {
        debug_assert!(self.pending.is_none());
        self.model.set_command_in_flight(true);
        match self.client.try_submit(command) {
            Ok(pending) => {
                self.pending = Some(pending);
                Ok(())
            }
            Err(error) => {
                self.model.set_command_in_flight(false);
                Err(error.into())
            }
        }
    }

    fn begin_stopping(&mut self) {
        self.model.set_runtime_status(RuntimeStatus::Stopping);
    }

    fn finish(&mut self, reason: ShutdownReason) -> Result<(), RuntimeError> {
        self.begin_stopping();
        let Some(runtime) = self.runtime.take() else {
            return Ok(());
        };
        runtime.finish_and_join(reason)
    }

    fn cancel_active_profile_review(&self) -> Result<(), RuntimeError> {
        if self.model.agents.editor.is_some() || self.model.agents.pending_confirmation.is_some() {
            self.client.cancel_agent_profile_edit()
        } else {
            Ok(())
        }
    }

    fn cancel_active_skill_review(&mut self) -> Result<(), RuntimeError> {
        cancel_skill_review_once(&self.client, &mut self.model)
    }
}

enum LoopControl {
    Continue { redraw: bool },
    Finish(ShutdownReason),
}

fn run_loop(
    runner: &mut TuiRunner,
    screen: &mut dyn Screen,
    events: &mut dyn EventSource,
    theme: &Theme,
) -> Result<ShutdownReason, TuiError> {
    let body = catch_sensitive_unwind(AssertUnwindSafe(|| {
        runner.event_loop(screen, events, theme)
    }));
    match body {
        Ok(result) => result,
        Err(_) => Err(TuiError::Panicked),
    }
}

fn run_with_screen(
    mut runner: TuiRunner,
    screen: &mut dyn Screen,
    events: &mut dyn EventSource,
    theme: &Theme,
) -> Result<(), TuiError> {
    let primary = run_loop(&mut runner, screen, events, theme);
    let finish_reason = match primary {
        Ok(reason) => reason,
        Err(_) => ShutdownReason::ApplicationError,
    };
    runner.begin_stopping();
    let cancellation = runner
        .cancel_active_profile_review()
        .map_err(TuiError::Runtime);
    let skill_cancellation = runner.cancel_active_skill_review().map_err(TuiError::Runtime);
    let restoration = screen.restore();
    let finish = runner.finish(finish_reason).map_err(TuiError::Runtime);
    match primary {
        Err(error) => Err(error),
        Ok(_) => cancellation.and(skill_cancellation).and(restoration).and(finish),
    }
}

fn finish_with_primary(mut runner: TuiRunner, error: TuiError) -> Result<(), TuiError> {
    runner.begin_stopping();
    let _ = runner.finish(ShutdownReason::ApplicationError);
    Err(error)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use crossbeam_channel::{Receiver, Sender, bounded};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;
    use uuid::Uuid;

    use super::{TuiRunner, run_with_screen};
    use crate::{
        app::{
            AppError, ApplicationCommand, CommandOutcome, CommandView, HelpView,
            PresentationSnapshot, ShutdownDisposition, ShutdownReason, ShutdownView, StatusView,
        },
        domain::{CommandId, CorrelationId, InstallationId, SessionId},
        runtime::{ApplicationRuntime, CommandExecutor, RuntimeError},
        setup::SetupStatus,
        ui::tui::{
            EventSource, Screen, TuiError, TuiEvent,
            model::{LayoutMode, RuntimeStatus, TuiModel, View},
            theme::Theme,
        },
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedFrame {
        layout_mode: LayoutMode,
        active_view: View,
        runtime_status: RuntimeStatus,
        command_in_flight: bool,
        history_len: usize,
        message: Option<String>,
    }

    #[derive(Clone)]
    struct RecordingScreen {
        area: Rect,
        frames: Arc<Mutex<Vec<RecordedFrame>>>,
        fail_draw: Arc<AtomicBool>,
        panic_on_draw: Arc<AtomicBool>,
        fail_restore: Arc<AtomicBool>,
        restore_calls: Arc<AtomicUsize>,
        order: Option<Arc<Mutex<Vec<&'static str>>>>,
        restored: Option<Sender<()>>,
    }

    impl RecordingScreen {
        fn new(area: Rect) -> Self {
            Self {
                area,
                frames: Arc::new(Mutex::new(Vec::new())),
                fail_draw: Arc::new(AtomicBool::new(false)),
                panic_on_draw: Arc::new(AtomicBool::new(false)),
                fail_restore: Arc::new(AtomicBool::new(false)),
                restore_calls: Arc::new(AtomicUsize::new(0)),
                order: None,
                restored: None,
            }
        }

        fn failing_draw(area: Rect) -> Self {
            let screen = Self::new(area);
            screen.fail_draw.store(true, Ordering::SeqCst);
            screen
        }

        fn panicking_draw(area: Rect) -> Self {
            let screen = Self::new(area);
            screen.panic_on_draw.store(true, Ordering::SeqCst);
            screen
        }

        fn failing_restore(area: Rect) -> Self {
            let screen = Self::new(area);
            screen.fail_restore.store(true, Ordering::SeqCst);
            screen
        }

        fn observing_restore(
            area: Rect,
            order: Arc<Mutex<Vec<&'static str>>>,
            restored: Sender<()>,
        ) -> Self {
            let mut screen = Self::new(area);
            screen.order = Some(order);
            screen.restored = Some(restored);
            screen
        }

        fn frames(&self) -> Vec<RecordedFrame> {
            self.frames.lock().unwrap().clone()
        }

        fn restore_calls(&self) -> usize {
            self.restore_calls.load(Ordering::SeqCst)
        }
    }

    impl Screen for RecordingScreen {
        fn size(&self) -> Result<Rect, TuiError> {
            Ok(self.area)
        }

        fn draw(&mut self, model: &TuiModel, _theme: &Theme) -> Result<(), TuiError> {
            if self.panic_on_draw.load(Ordering::SeqCst) {
                panic!("injected screen panic");
            }
            if self.fail_draw.load(Ordering::SeqCst) {
                return Err(TuiError::TerminalOutput);
            }
            self.frames.lock().unwrap().push(RecordedFrame {
                layout_mode: model.layout_mode,
                active_view: model.active_view,
                runtime_status: model.runtime_status,
                command_in_flight: model.command_in_flight,
                history_len: model.command.history_len(),
                message: model.message.as_ref().map(|message| message.text.clone()),
            });
            Ok(())
        }

        fn restore(&mut self) -> Result<(), TuiError> {
            if let Some(order) = &self.order {
                order.lock().unwrap().push("restore");
            }
            if let Some(restored) = &self.restored {
                let _ = restored.try_send(());
            }
            self.restore_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_restore.load(Ordering::SeqCst) {
                Err(TuiError::TerminalOutput)
            } else {
                Ok(())
            }
        }
    }

    enum EventStep {
        Event(TuiEvent),
        Idle,
        Release(Sender<()>),
        InputError,
    }

    struct FakeEvents {
        steps: VecDeque<EventStep>,
        timeouts: Arc<Mutex<Vec<Duration>>>,
        exhausted_polls: usize,
    }

    impl FakeEvents {
        fn from(steps: impl IntoIterator<Item = EventStep>) -> Self {
            Self {
                steps: steps.into_iter().collect(),
                timeouts: Arc::new(Mutex::new(Vec::new())),
                exhausted_polls: 0,
            }
        }

        fn timeouts(&self) -> Vec<Duration> {
            self.timeouts.lock().unwrap().clone()
        }
    }

    impl EventSource for FakeEvents {
        fn next_event(&mut self, timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
            self.timeouts.lock().unwrap().push(timeout);
            match self.steps.pop_front() {
                Some(EventStep::Event(event)) => Ok(Some(event)),
                Some(EventStep::Idle) => Ok(None),
                Some(EventStep::Release(sender)) => {
                    sender.send(()).unwrap();
                    thread::yield_now();
                    Ok(None)
                }
                Some(EventStep::InputError) => Err(TuiError::TerminalInput),
                None => {
                    self.exhausted_polls = self.exhausted_polls.saturating_add(1);
                    thread::yield_now();
                    if self.exhausted_polls > 100_000 {
                        Ok(Some(TuiEvent::Interrupt))
                    } else {
                        Ok(None)
                    }
                }
            }
        }
    }

    #[derive(Clone)]
    struct RuntimeObserver {
        commands: Arc<Mutex<Vec<ApplicationCommand>>>,
        finishes: Arc<Mutex<Vec<ShutdownReason>>>,
    }

    impl RuntimeObserver {
        fn commands(&self) -> Vec<ApplicationCommand> {
            self.commands.lock().unwrap().clone()
        }

        fn finishes(&self) -> Vec<ShutdownReason> {
            self.finishes.lock().unwrap().clone()
        }
    }

    struct RecordingExecutor {
        observer: RuntimeObserver,
        release_first: Option<Receiver<()>>,
        fail_execute: bool,
        fail_finish: bool,
    }

    struct OrderingExecutor {
        order: Arc<Mutex<Vec<&'static str>>>,
        started: Sender<()>,
        release: Receiver<()>,
    }

    impl CommandExecutor for OrderingExecutor {
        fn execute_user(
            &mut self,
            command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            self.started.send(()).unwrap();
            self.release.recv().unwrap();
            self.order.lock().unwrap().push("worker_released");
            Ok(outcome(command))
        }

        fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
            self.order.lock().unwrap().push("finish");
            Ok(())
        }
    }

    impl Drop for OrderingExecutor {
        fn drop(&mut self) {
            self.order.lock().unwrap().push("process_guard_drop");
        }
    }

    impl CommandExecutor for RecordingExecutor {
        fn execute_user(
            &mut self,
            command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            self.observer.commands.lock().unwrap().push(command.clone());
            if let Some(release) = self.release_first.take() {
                release.recv().unwrap();
            }
            if self.fail_execute {
                return Err(AppError::LifecycleFinished);
            }
            Ok(outcome(command))
        }

        fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
            self.observer.finishes.lock().unwrap().push(reason);
            if self.fail_finish {
                Err(AppError::LifecycleFinished)
            } else {
                Ok(())
            }
        }
    }

    fn runtime(
        block_first: bool,
        fail_execute: bool,
        fail_finish: bool,
    ) -> (ApplicationRuntime, RuntimeObserver, Option<Sender<()>>) {
        let observer = RuntimeObserver {
            commands: Arc::new(Mutex::new(Vec::new())),
            finishes: Arc::new(Mutex::new(Vec::new())),
        };
        let (release, release_first) = if block_first {
            let (sender, receiver) = bounded(1);
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        let runtime = ApplicationRuntime::spawn(
            RecordingExecutor {
                observer: observer.clone(),
                release_first,
                fail_execute,
                fail_finish,
            },
            1,
        )
        .unwrap();
        (runtime, observer, release)
    }

    fn snapshot() -> PresentationSnapshot {
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
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
        }
    }

    fn outcome(command: ApplicationCommand) -> CommandOutcome {
        let (view, shutdown) = match command {
            ApplicationCommand::ShowStatus => (
                CommandView::Status(StatusView {
                    installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
                    session_id: SessionId::from_uuid(Uuid::from_u128(2)),
                }),
                ShutdownDisposition::Continue,
            ),
            ApplicationCommand::RequestShutdown => (
                CommandView::Shutdown(ShutdownView {
                    disposition: ShutdownDisposition::Requested,
                }),
                ShutdownDisposition::Requested,
            ),
            _ => (CommandView::Help(HelpView), ShutdownDisposition::Continue),
        };
        CommandOutcome {
            command_id: CommandId::from_uuid(Uuid::from_u128(3)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(4)),
            committed_events: Vec::new(),
            view,
            shutdown,
        }
    }

    fn key(character: char) -> EventStep {
        EventStep::Event(TuiEvent::Key(KeyEvent::new(
            KeyCode::Char(character),
            KeyModifiers::NONE,
        )))
    }

    fn special_key(code: KeyCode) -> EventStep {
        EventStep::Event(TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn command_steps(command: &str) -> Vec<EventStep> {
        let mut steps = command.chars().map(key).collect::<Vec<_>>();
        steps.push(special_key(KeyCode::Enter));
        steps
    }

    fn execute(
        runtime: ApplicationRuntime,
        screen: &mut dyn Screen,
        events: &mut dyn EventSource,
    ) -> Result<(), TuiError> {
        run_with_screen(
            TuiRunner::new(runtime, snapshot(), false),
            screen,
            events,
            &Theme::from_no_color(true),
        )
    }

    #[test]
    fn host_draws_processes_resize_and_redraws_only_when_dirty() {
        let (runtime, observer, _) = runtime(false, false, false);
        let mut screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
        let mut steps = vec![
            EventStep::Idle,
            EventStep::Idle,
            EventStep::Event(TuiEvent::Resize(70, 20)),
            key('2'),
            EventStep::Idle,
        ];
        steps.extend(command_steps("/quit"));
        let mut events = FakeEvents::from(steps);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(result.is_ok());
        assert_eq!(observer.finishes(), [ShutdownReason::UserQuit]);
        assert!(
            screen
                .frames()
                .iter()
                .any(|frame| frame.layout_mode == LayoutMode::Narrow)
        );
        assert!(
            screen
                .frames()
                .iter()
                .any(|frame| frame.active_view == View::Setup)
        );
        assert!(
            screen
                .frames()
                .iter()
                .any(|frame| frame.runtime_status == RuntimeStatus::Stopping)
        );
        assert_eq!(screen.frames().len(), 9);
        assert!(!events.timeouts().is_empty());
        assert!(
            events
                .timeouts()
                .iter()
                .all(|timeout| *timeout <= Duration::from_millis(50))
        );
    }

    #[test]
    fn second_submission_is_prevented_until_the_pending_outcome_arrives() {
        let (runtime, observer, release) = runtime(true, false, false);
        let mut steps = command_steps("/status");
        steps.extend(command_steps("/help"));
        steps.push(EventStep::Release(release.unwrap()));
        steps.push(EventStep::Event(TuiEvent::Interrupt));
        let mut screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from(steps);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(result.is_ok());
        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);
        assert_eq!(observer.finishes(), [ShutdownReason::Interrupted]);
    }

    #[test]
    fn quit_command_does_not_bypass_the_pending_command_guard() {
        let (runtime, observer, release) = runtime(true, false, false);
        let mut steps = command_steps("/status");
        steps.push(special_key(KeyCode::Esc));
        steps.extend(command_steps("/quit"));
        steps.push(key('x'));
        steps.push(EventStep::Release(release.unwrap()));
        steps.push(EventStep::Event(TuiEvent::Interrupt));
        let mut screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from(steps);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(result.is_ok());
        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);
        assert_eq!(observer.finishes(), [ShutdownReason::Interrupted]);
        assert!(screen.frames().iter().any(|frame| {
            frame.command_in_flight
                && frame.history_len == 1
                && frame.message.as_deref() == Some("A command is already running.")
        }));
    }

    #[test]
    fn delayed_quit_command_is_in_flight_and_rejects_followup_submission() {
        let (runtime, observer, release) = runtime(true, false, false);
        let mut steps = command_steps("/quit");
        steps.extend(command_steps("/help"));
        steps.push(key('x'));
        steps.push(EventStep::Release(release.unwrap()));
        let mut screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from(steps);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(result.is_ok());
        assert_eq!(observer.commands(), [ApplicationCommand::RequestShutdown]);
        assert!(screen.frames().iter().any(|frame| {
            frame.runtime_status == RuntimeStatus::Stopping
                && frame.command_in_flight
                && frame.history_len == 1
                && frame.message.as_deref() == Some("A command is already running.")
        }));
    }

    #[test]
    fn interrupt_finishes_immediately_without_a_shutdown_command() {
        let (runtime, observer, _) = runtime(false, false, false);
        let mut screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from([EventStep::Event(TuiEvent::Interrupt)]);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(result.is_ok());
        assert!(observer.commands().is_empty());
        assert_eq!(observer.finishes(), [ShutdownReason::Interrupted]);
    }

    #[test]
    fn input_error_is_primary_and_finishes_once_with_application_error() {
        let (runtime, observer, _) = runtime(false, false, true);
        let mut screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from([EventStep::InputError]);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(matches!(result, Err(TuiError::TerminalInput)));
        assert_eq!(observer.finishes(), [ShutdownReason::ApplicationError]);
    }

    #[test]
    fn draw_error_is_primary_even_when_runtime_finish_fails() {
        let (runtime, observer, _) = runtime(false, false, true);
        let mut screen = RecordingScreen::failing_draw(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from([]);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(matches!(result, Err(TuiError::TerminalOutput)));
        assert_eq!(observer.finishes(), [ShutdownReason::ApplicationError]);
    }

    #[test]
    fn body_error_wins_when_restoration_and_runtime_finish_both_fail() {
        let (runtime, observer, _) = runtime(false, false, true);
        let mut screen = RecordingScreen::failing_restore(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from([EventStep::InputError]);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(matches!(result, Err(TuiError::TerminalInput)));
        assert_eq!(observer.finishes(), [ShutdownReason::ApplicationError]);
        assert_eq!(screen.restore_calls(), 1);
    }

    #[test]
    fn restoration_precedes_worker_release_join_and_process_guard_drop() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let (started_sender, started_receiver) = bounded(1);
        let (release_sender, release_receiver) = bounded(1);
        let runtime = ApplicationRuntime::spawn(
            OrderingExecutor {
                order: order.clone(),
                started: started_sender,
                release: release_receiver,
            },
            1,
        )
        .unwrap();
        let (restored_sender, restored_receiver) = bounded(1);
        let screen = RecordingScreen::observing_restore(
            Rect::new(0, 0, 140, 40),
            order.clone(),
            restored_sender,
        );
        let mut steps = command_steps("/status");
        steps.push(EventStep::Event(TuiEvent::Interrupt));
        let events = FakeEvents::from(steps);

        let handle = thread::spawn(move || {
            let mut screen = screen;
            let mut events = events;
            execute(runtime, &mut screen, &mut events)
        });
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker started");
        let restored_before_release = restored_receiver
            .recv_timeout(Duration::from_millis(100))
            .is_ok();
        release_sender.send(()).unwrap();
        let result = handle.join().unwrap();

        assert!(restored_before_release);
        assert!(result.is_ok());
        assert_eq!(
            order.lock().unwrap().as_slice(),
            ["restore", "worker_released", "finish", "process_guard_drop"]
        );
    }

    #[test]
    fn runtime_error_is_primary_and_finishes_once_with_application_error() {
        let (runtime, observer, _) = runtime(false, true, false);
        let mut screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from(command_steps("/status"));

        let result = execute(runtime, &mut screen, &mut events);

        assert!(matches!(
            result,
            Err(TuiError::Runtime(RuntimeError::Application(
                AppError::LifecycleFinished
            )))
        ));
        assert_eq!(observer.finishes(), [ShutdownReason::ApplicationError]);
    }

    #[test]
    fn loop_panic_becomes_safe_error_and_finishes_with_application_error() {
        let (runtime, observer, _) = runtime(false, false, false);
        let mut screen = RecordingScreen::panicking_draw(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from([]);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(matches!(result, Err(TuiError::Panicked)));
        assert_eq!(observer.finishes(), [ShutdownReason::ApplicationError]);
        assert_eq!(screen.restore_calls(), 1);
        assert!(!format!("{result:?}").contains("injected screen panic"));
    }

    #[test]
    fn clean_loop_returns_restore_error_and_restores_once_before_return() {
        let (runtime, observer, _) = runtime(false, false, false);
        let mut screen = RecordingScreen::failing_restore(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from([EventStep::Event(TuiEvent::Interrupt)]);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(matches!(result, Err(TuiError::TerminalOutput)));
        assert_eq!(observer.finishes(), [ShutdownReason::Interrupted]);
        assert_eq!(screen.restore_calls(), 1);
    }

    #[test]
    fn primary_error_wins_over_restore_error_and_restores_once_before_return() {
        let (runtime, observer, _) = runtime(false, false, false);
        let mut screen = RecordingScreen::failing_restore(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from([EventStep::InputError]);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(matches!(result, Err(TuiError::TerminalInput)));
        assert_eq!(observer.finishes(), [ShutdownReason::ApplicationError]);
        assert_eq!(screen.restore_calls(), 1);
    }

    #[test]
    fn clean_loop_restores_exactly_once_before_return() {
        let (runtime, observer, _) = runtime(false, false, false);
        let mut screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from([EventStep::Event(TuiEvent::Interrupt)]);

        let result = execute(runtime, &mut screen, &mut events);

        assert!(result.is_ok());
        assert_eq!(observer.finishes(), [ShutdownReason::Interrupted]);
        assert_eq!(screen.restore_calls(), 1);
    }
}
