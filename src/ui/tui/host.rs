use std::{collections::VecDeque, panic::AssertUnwindSafe, time::Duration};

use super::{
    TuiError,
    controller::{ControllerEffect, apply_outcome, handle_event},
    event::{CrosstermEventSource, EventSource, TuiEvent},
    model::{
        AgentOutcomeIntent, AgentProfileRead, AgentProfileTarget, MemoryOutcomeIntent, MemoryPane,
        MemoryResultOrigin, MemoryViewState, NavigationTab, RuntimeStatus, SkillOperationOrigin,
        SkillWorkspaceOrigin, TuiModel, absent_set_diff_matches, canonical_memory_edit_diff,
        delete_diff_matches, deleted_set_diff_matches, expected_state_matches_namespace_key,
        seeded_present_set_diff,
    },
    terminal::{CrosstermScreen, Screen},
    theme::Theme,
};
use crate::{
    agents::AgentProfileDraft,
    app::{
        AgentProfileSelector, AgentSkillAssignmentOperation, AgentSkillAssignmentPreview, AppError,
        ApplicationCommand, CommandOutcome, CommandView, MemoryEditPreview, PresentationSnapshot,
        ShutdownReason,
    },
    memory::{
        ExpectedMemoryEntryState, MemoryEntryDraft, MemoryEntryState, MemoryEntryVersion,
        MemoryMutationKind, MemoryNoChange, MemoryPlaintextAcknowledgement,
        MemoryProposalOperation, MemoryProposalRef, MemoryResolutionAction,
    },
    panic_boundary::catch_sensitive_unwind,
    runtime::{ApplicationRuntime, PendingOutcome, RuntimeClient, RuntimeError},
    ui::{
        command::SkillWorkflowCommand,
        memory_editor::MemoryPreviewRequest,
        profile_editor::{PreviewEditRequest, ProfileEditor},
        skill_editor::SkillPreviewRequest,
    },
};

const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const STALE_PROFILE_MESSAGE: &str =
    "Profile changed elsewhere. Detail refreshed; reopen Edit to continue.";
const PENDING_INTERACTION_MESSAGE: &str =
    "Another command is running; this action remains available to retry.";

fn memory_mutation_intent(command: &ApplicationCommand) -> Result<MemoryOutcomeIntent, TuiError> {
    match command {
        ApplicationCommand::SetMemoryEntry { .. }
        | ApplicationCommand::DeleteMemoryEntry { .. } => Ok(MemoryOutcomeIntent::Mutation),
        ApplicationCommand::ApproveMemoryProposal { .. }
        | ApplicationCommand::RejectMemoryProposal { .. } => Ok(MemoryOutcomeIntent::Resolution),
        _ => Err(TuiError::UnexpectedControllerEffect),
    }
}

fn apply_memory_outcome(
    state: &mut super::model::MemoryViewState,
    intent: &MemoryOutcomeIntent,
    generation: u64,
    view: CommandView,
) -> bool {
    if state.generation != generation || state.pending_intent.as_ref() != Some(intent) {
        return false;
    }
    state.apply_matching_view(intent, view)
}

fn cancel_tui_memory_review_once(
    client: &RuntimeClient,
    state: &mut MemoryViewState,
) -> Result<(), RuntimeError> {
    if std::mem::take(&mut state.review_registered) {
        client.cancel_memory_review()?;
    }
    Ok(())
}

fn synchronize_memory_editor_input(model: &mut TuiModel) {
    let (step, input) = model
        .agents
        .memory
        .editor
        .as_ref()
        .map(|editor| {
            let input = match editor.step() {
                crate::ui::memory_editor::MemoryEditorStep::Key => editor.key_input(),
                crate::ui::memory_editor::MemoryEditorStep::Value => editor.value_input(),
                crate::ui::memory_editor::MemoryEditorStep::PurposeTags => editor.tags_input(),
                crate::ui::memory_editor::MemoryEditorStep::Review => "",
            };
            (editor.step(), input.to_owned())
        })
        .unwrap_or((
            crate::ui::memory_editor::MemoryEditorStep::Key,
            String::new(),
        ));
    if step == crate::ui::memory_editor::MemoryEditorStep::Value {
        model.command.replace_with_memory_value(&input);
    } else {
        model.command.clear();
        model.command.ingest(&input);
    }
}

#[derive(Debug)]
enum MemoryPreviewFailure {
    Passive(TuiError),
    ReviewCancellation(TuiError),
}

impl From<RuntimeError> for MemoryPreviewFailure {
    fn from(error: RuntimeError) -> Self {
        Self::Passive(error.into())
    }
}

impl From<TuiError> for MemoryPreviewFailure {
    fn from(error: TuiError) -> Self {
        Self::Passive(error)
    }
}

impl MemoryPreviewFailure {
    fn into_tui_error(self) -> TuiError {
        match self {
            Self::Passive(error) | Self::ReviewCancellation(error) => error,
        }
    }
}

type MemoryPreviewInstallResult = Result<(), MemoryPreviewFailure>;

fn cancel_returned_memory_review(
    client: &RuntimeClient,
    memory: &mut MemoryViewState,
) -> MemoryPreviewInstallResult {
    cancel_tui_memory_review_once(client, memory)
        .map_err(|error| MemoryPreviewFailure::ReviewCancellation(error.into()))
}

fn install_set_preview_for_host(
    client: &RuntimeClient,
    model: &mut TuiModel,
    request: MemoryPreviewRequest,
) -> MemoryPreviewInstallResult {
    let preview = client.preview_memory_set(request.selector.clone(), request.candidate.clone())?;
    let memory = &mut model.agents.memory;
    let request_is_current = memory_set_request_is_current(memory, &request);
    let retained_seed = retained_set_seed(memory).cloned();

    match preview {
        MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent) => {
            if !request_is_current
                || !memory_identical_content_matches_request(
                    retained_seed.as_ref(),
                    &request.candidate,
                )
            {
                return Err(TuiError::UnexpectedControllerEffect.into());
            }
            let installed = memory.editor.as_mut().is_some_and(|editor| {
                editor.apply_preview(
                    request.generation,
                    MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent),
                )
            });
            if !installed {
                return Err(TuiError::UnexpectedControllerEffect.into());
            }
            memory.review_registered = false;
            model.set_message(super::model::Severity::Info, "No change");
            Ok(())
        }
        MemoryEditPreview::Review(review) => {
            // The passive API has already registered this review; own it before
            // checking its payload so malformed responses are still cancelled.
            memory.review_registered = true;
            if !request_is_current
                || !memory_set_review_matches_request(
                    memory,
                    &review,
                    &request,
                    retained_seed.as_ref(),
                )
            {
                cancel_returned_memory_review(client, memory)?;
                return Err(TuiError::UnexpectedControllerEffect.into());
            }
            if !memory.editor.as_mut().is_some_and(|editor| {
                editor.apply_preview(
                    request.generation,
                    MemoryEditPreview::Review(review.clone()),
                )
            }) {
                cancel_returned_memory_review(client, memory)?;
                return Err(TuiError::UnexpectedControllerEffect.into());
            }
            memory.edit_review = Some(review);
            memory.resolution_review = None;
            memory.confirmation = None;
            memory.transition_to(MemoryPane::MutationReview);
            model.clear_message();
            Ok(())
        }
        MemoryEditPreview::NoChange(_) => Err(TuiError::UnexpectedControllerEffect.into()),
    }
}

fn install_delete_preview_for_host(
    client: &RuntimeClient,
    model: &mut TuiModel,
    selector: AgentProfileSelector,
    key: String,
    generation: u64,
) -> MemoryPreviewInstallResult {
    let preview = client.preview_memory_delete(selector.clone(), key.clone())?;
    let memory = &mut model.agents.memory;
    let requested_entry = memory_delete_request_entry(memory, &selector, &key, generation).cloned();
    match preview {
        MemoryEditPreview::NoChange(MemoryNoChange::AlreadyAbsent) => {
            if requested_entry.is_none() {
                return Err(TuiError::UnexpectedControllerEffect.into());
            }
            memory.review_registered = false;
            memory.transition_to(MemoryPane::EntryDetail);
            model.set_message(super::model::Severity::Info, "No change");
            Ok(())
        }
        MemoryEditPreview::Review(review) => {
            memory.review_registered = true;
            if !requested_entry.as_ref().is_some_and(|entry| {
                memory_delete_review_matches_request(memory, &review, &selector, &key, entry)
            }) {
                cancel_returned_memory_review(client, memory)?;
                return Err(TuiError::UnexpectedControllerEffect.into());
            }
            memory.edit_review = Some(review);
            memory.resolution_review = None;
            memory.confirmation = None;
            memory.transition_to(MemoryPane::MutationReview);
            model.clear_message();
            Ok(())
        }
        MemoryEditPreview::NoChange(_) => Err(TuiError::UnexpectedControllerEffect.into()),
    }
}

fn install_resolution_preview_for_host(
    client: &RuntimeClient,
    model: &mut TuiModel,
    proposal: MemoryProposalRef,
    action: MemoryResolutionAction,
    generation: u64,
) -> MemoryPreviewInstallResult {
    let review = match action {
        MemoryResolutionAction::Approve => {
            client.preview_memory_proposal_approval(proposal.clone())?
        }
        MemoryResolutionAction::Reject => {
            client.preview_memory_proposal_rejection(proposal.clone())?
        }
    };
    let memory = &mut model.agents.memory;
    memory.review_registered = true;
    let valid =
        memory_resolution_review_matches_request(memory, &review, &proposal, action, generation);
    if !valid {
        cancel_returned_memory_review(client, memory)?;
        return Err(TuiError::UnexpectedControllerEffect.into());
    }
    memory.edit_review = None;
    memory.resolution_review = Some(review);
    memory.confirmation = None;
    memory.transition_to(MemoryPane::ProposalResolutionReview);
    model.clear_message();
    Ok(())
}

#[cfg(test)]
fn install_set_preview(
    client: &RuntimeClient,
    model: &mut TuiModel,
    request: MemoryPreviewRequest,
) -> Result<(), TuiError> {
    install_set_preview_for_host(client, model, request)
        .map_err(MemoryPreviewFailure::into_tui_error)
}

#[cfg(test)]
fn install_delete_preview(
    client: &RuntimeClient,
    model: &mut TuiModel,
    selector: AgentProfileSelector,
    key: String,
    generation: u64,
) -> Result<(), TuiError> {
    install_delete_preview_for_host(client, model, selector, key, generation)
        .map_err(MemoryPreviewFailure::into_tui_error)
}

#[cfg(test)]
fn install_resolution_preview(
    client: &RuntimeClient,
    model: &mut TuiModel,
    proposal: MemoryProposalRef,
    action: MemoryResolutionAction,
    generation: u64,
) -> Result<(), TuiError> {
    install_resolution_preview_for_host(client, model, proposal, action, generation)
        .map_err(MemoryPreviewFailure::into_tui_error)
}

fn memory_set_request_is_current(memory: &MemoryViewState, request: &MemoryPreviewRequest) -> bool {
    memory.editor_is_authenticated()
        && memory.profile_selector().ok().as_ref() == Some(&request.selector)
        && memory.editor.as_ref().is_some_and(|editor| {
            editor.selector() == &request.selector
                && editor.preview_request().ok().as_ref() == Some(request)
        })
}

fn retained_set_seed(memory: &MemoryViewState) -> Option<&MemoryEntryVersion> {
    memory.editor.as_ref().and_then(|editor| editor.seed())
}

fn memory_set_review_matches_request(
    memory: &MemoryViewState,
    review: &crate::memory::MemoryEditReview,
    request: &MemoryPreviewRequest,
    seed: Option<&MemoryEntryVersion>,
) -> bool {
    if review.operation != MemoryMutationKind::Set
        || review.candidate.as_ref() != Some(&request.candidate)
        || memory.profile.as_ref().map(|identity| &identity.profile) != Some(&review.profile)
        || memory.namespace_id != Some(review.namespace_id)
        || review.plaintext_acknowledgement
            != MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1
        || !expected_state_matches_namespace_key(
            &review.expected,
            review.namespace_id,
            &request.candidate.normalized_key(),
        )
        || !canonical_memory_edit_diff(&review.diff)
    {
        return false;
    }
    match (seed, &review.expected) {
        (Some(seed), ExpectedMemoryEntryState::Present(entry)) => {
            *entry == seed.reference()
                && entry.namespace_id() == review.namespace_id
                && seeded_present_set_diff(seed, &request.candidate)
                    .is_some_and(|expected| review.diff == expected)
        }
        (Some(_), _) | (None, ExpectedMemoryEntryState::Present(_)) => false,
        (None, ExpectedMemoryEntryState::Absent) => {
            absent_set_diff_matches(&review.diff, &request.candidate)
        }
        (None, ExpectedMemoryEntryState::Deleted(entry)) => {
            entry.namespace_id() == review.namespace_id
                && entry.state() == MemoryEntryState::Deleted
                && entry.normalized_key() == &request.candidate.normalized_key()
                && deleted_set_diff_matches(&review.diff, &request.candidate)
        }
    }
}

fn memory_delete_request_entry<'a>(
    memory: &'a MemoryViewState,
    selector: &AgentProfileSelector,
    key: &str,
    generation: u64,
) -> Option<&'a MemoryEntryVersion> {
    if memory.generation != generation || memory.profile_selector().ok().as_ref() != Some(selector)
    {
        return None;
    }
    let detail = memory.entry_detail.as_ref()?;
    let identity = memory.profile.as_ref()?;
    let normalized = crate::memory::normalize_memory_key(key).ok()?;
    (detail.profile == identity.profile
        && detail.entry.reference().namespace_id() == memory.namespace_id?
        && detail.entry.reference().state() == MemoryEntryState::Present
        && detail.entry.reference().normalized_key() == &normalized)
        .then_some(&detail.entry)
}

fn memory_delete_review_matches_request(
    memory: &MemoryViewState,
    review: &crate::memory::MemoryEditReview,
    selector: &AgentProfileSelector,
    key: &str,
    requested_entry: &MemoryEntryVersion,
) -> bool {
    review.operation == MemoryMutationKind::Delete
        && review.candidate.is_none()
        && memory.profile_selector().ok().as_ref() == Some(selector)
        && memory.profile.as_ref().map(|identity| &identity.profile) == Some(&review.profile)
        && memory.namespace_id == Some(review.namespace_id)
        && review.plaintext_acknowledgement
            == MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1
        && expected_state_matches_namespace_key(
            &review.expected,
            review.namespace_id,
            requested_entry.reference().normalized_key(),
        )
        && canonical_memory_edit_diff(&review.diff)
        && matches!(
            &review.expected,
            ExpectedMemoryEntryState::Present(entry)
                if *entry == requested_entry.reference()
                    && entry.namespace_id() == review.namespace_id
                    && crate::memory::normalize_memory_key(key).ok().as_ref()
                        == Some(entry.normalized_key())
        )
        && delete_diff_matches(&review.diff, requested_entry)
}

fn memory_resolution_review_matches_request(
    memory: &MemoryViewState,
    review: &crate::app::MemoryProposalResolutionReview,
    requested_proposal: &MemoryProposalRef,
    requested_action: MemoryResolutionAction,
    generation: u64,
) -> bool {
    let Some(detail) = memory.proposal_detail.as_ref() else {
        return false;
    };
    let selected_action = match memory.selected_proposal_detail_action {
        super::model::MemoryProposalDetailAction::Approve => MemoryResolutionAction::Approve,
        super::model::MemoryProposalDetailAction::Reject => MemoryResolutionAction::Reject,
    };
    memory.generation == generation
        && selected_action == requested_action
        && detail.proposal.reference() == *requested_proposal
        && detail.status == crate::memory::MemoryProposalStatus::Pending
        && detail.resolution.is_none()
        && review.action == requested_action
        && review.proposal == detail.proposal
        && review.proposal.reference() == *requested_proposal
        && review.approval_id == detail.proposal.approval_id()
        && review.expected_approval_status == crate::policy::ApprovalStatus::Pending
        && review.expected_entry == detail.current_entry
        && review.proposer_is_historical == detail.proposer_is_historical
        && review.proposer_identity == detail.proposer_identity
        && review.namespace_owner_identity == detail.namespace_owner_identity
        && review.plaintext_acknowledgement
            == MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1
        && memory.profile.as_ref().map(|identity| &identity.profile)
            == Some(&review.namespace_owner_identity.profile)
        && memory.namespace_id == Some(review.proposal.namespace_id())
}

fn memory_resolution_result_shape_matches(
    review: &crate::app::MemoryProposalResolutionReview,
    result: &crate::app::MemoryProposalResolutionView,
) -> bool {
    match review.action {
        MemoryResolutionAction::Approve => result
            .entry
            .as_ref()
            .is_some_and(|entry| memory_accepted_proposal_entry_matches(review, entry)),
        MemoryResolutionAction::Reject => result.entry.is_none(),
    }
}

fn memory_accepted_proposal_entry_matches(
    review: &crate::app::MemoryProposalResolutionReview,
    entry: &crate::memory::MemoryEntryRef,
) -> bool {
    if entry.namespace_id() != review.proposal.namespace_id()
        || entry.normalized_key() != review.proposal.normalized_key()
        || !match review.proposal.expected() {
            ExpectedMemoryEntryState::Absent => true,
            ExpectedMemoryEntryState::Present(expected)
            | ExpectedMemoryEntryState::Deleted(expected) => {
                entry.entry_id() == expected.entry_id()
            }
        }
    {
        return false;
    }
    match review.proposal.operation() {
        MemoryProposalOperation::Set { candidate } => {
            entry.state() == MemoryEntryState::Present
                && entry.normalized_key() == &candidate.normalized_key()
        }
        MemoryProposalOperation::Delete => entry.state() == MemoryEntryState::Deleted,
    }
}

fn memory_identical_content_matches_request(
    seed: Option<&MemoryEntryVersion>,
    candidate: &MemoryEntryDraft,
) -> bool {
    seed.and_then(|seed| seeded_present_set_diff(seed, candidate))
        .is_some_and(|diff| diff.is_empty())
}

#[doc(hidden)]
pub fn execute_agent_effect(
    client: &RuntimeClient,
    model: &mut TuiModel,
    effect: ControllerEffect,
) -> Result<(), RuntimeError> {
    match effect {
        ControllerEffect::LoadSelectedAgentProfile { target, read } => {
            if model.agents.profile_target().as_ref() == Some(&target) {
                let outcome = client.submit(profile_read_command(&target, read))?;
                if profile_read_matches(read, &outcome.view) {
                    model.agents.install_profile_result(&target, outcome.view);
                }
            }
        }
        ControllerEffect::StartSelectedProfileEdit { target } => {
            if model.agents.profile_target().as_ref() == Some(&target)
                && model.agents.matching_detail().is_some()
            {
                start_profile_editor_from_detail(model)?;
            }
        }
        ControllerEffect::LoadAgentProfiles => {
            refresh_agent_navigation_data(client, model)?;
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
        ControllerEffect::LoadAgentSkillLibrary => {
            load_agent_skill_library(client, model)?;
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
                model.set_focus(super::model::Focus::Workspace);
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
                model.set_focus(super::model::Focus::Workspace);
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
        ControllerEffect::LoadAgentMemory(_)
        | ControllerEffect::LoadMemoryEntry { .. }
        | ControllerEffect::LoadMemoryEntryHistory { .. }
        | ControllerEffect::LoadMemoryEntryVersion { .. }
        | ControllerEffect::RequestMemorySetPreview(_)
        | ControllerEffect::RequestMemoryDeletePreview { .. }
        | ControllerEffect::LoadMemoryProposals { .. }
        | ControllerEffect::LoadMemoryProposal(_)
        | ControllerEffect::RequestMemoryProposalResolutionPreview { .. }
        | ControllerEffect::LoadEpisodicSummaries(_)
        | ControllerEffect::LoadEpisodicSummary(_)
        | ControllerEffect::ExecuteMemory(_)
        | ControllerEffect::CancelMemoryReview => {
            return Err(RuntimeError::Application(
                AppError::WrongMemoryCommandDispatcher,
            ));
        }
        ControllerEffect::None
        | ControllerEffect::Redraw
        | ControllerEffect::Submit(_)
        | ControllerEffect::SubmitPreservingNavigation(_)
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
        | ControllerEffect::StartSkillWorkflow(_)
        | ControllerEffect::ExecuteSkill(_)
        | ControllerEffect::CancelSkillReview => {}
    }
    Ok(())
}

fn refresh_agent_navigation_data(
    client: &RuntimeClient,
    model: &mut TuiModel,
) -> Result<(), RuntimeError> {
    let navigation = model.navigation_state_snapshot();
    model.pending_agent_outcome = None;

    let result = (|| {
        let skills = submit_agent_command(client, model, ApplicationCommand::ListSkills)?;
        let _ = apply_outcome(model, skills);
        let outcome = submit_agent_command(client, model, ApplicationCommand::ListAgentProfiles)?;
        apply_agent_outcome(model, outcome);
        Ok(())
    })();

    model.restore_navigation_state(navigation);
    result
}

#[doc(hidden)]
pub fn execute_skill_effect(
    client: &RuntimeClient,
    model: &mut TuiModel,
    effect: ControllerEffect,
) -> Result<(), RuntimeError> {
    match effect {
        ControllerEffect::StartSkillWorkflow(workflow) => {
            if let Err(error) = execute_typed_skill_workflow(client, model, workflow) {
                match error {
                    error @ (RuntimeError::Application(_) | RuntimeError::Backpressure) => {
                        model.set_command_in_flight(false);
                        model.skills.pane = super::model::SkillsPane::List;
                        model.set_message(
                            super::model::Severity::Error,
                            match error {
                                RuntimeError::Backpressure => {
                                    "Command queue is busy; skill workflow was not started."
                                }
                                _ => "Skill workflow could not be started.",
                            },
                        );
                    }
                    error => return Err(error),
                }
            }
        }
        ControllerEffect::LoadSkills => {
            let navigation = model.navigation_state_snapshot();
            let result = submit_agent_command(client, model, ApplicationCommand::ListSkills).map(
                |outcome| {
                    let _ = apply_outcome(model, outcome);
                },
            );
            model.restore_navigation_state(navigation);
            result?;
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
                let seed = model
                    .skills
                    .detail
                    .as_ref()
                    .map(|detail| detail.content.clone());
                model.skills.start_create(seed);
                synchronize_host_skill_input(model);
            }
        }
        ControllerEffect::LoadSkillHistory { skill_id } => {
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
            let outcome = submit_agent_command_for(
                client,
                model,
                AgentOutcomeIntent::SkillAssignment,
                ApplicationCommand::ListAgentProfiles,
            )?;
            let _ = apply_outcome(model, outcome);
            model.skills.pane = super::model::SkillsPane::AgentPicker;
        }
        ControllerEffect::LoadSkillAgent { profile_id } => {
            let outcome = submit_agent_command_for(
                client,
                model,
                AgentOutcomeIntent::SkillAssignment,
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
                super::model::AssignmentKind::Reassign { expected } => client
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
            let affected_profile_id = assignment_profile_id(&command);
            match submit_protected_skill_command(client, model, command) {
                Ok(outcome) => {
                    let _ = apply_outcome(model, outcome);
                    model.skills.review_registered = false;
                    model.skills.pending_confirmation = None;
                    model.skills.editor = None;
                    model.skills.assignment = None;
                    if let Some(profile_id) = affected_profile_id {
                        refresh_agent_skill_assignment_state(client, model, profile_id)?;
                    }
                    model.skills.pane = super::model::SkillsPane::Result;
                    model.set_message(super::model::Severity::Info, "Skill action completed.");
                }
                Err(RuntimeError::Backpressure) => {
                    model.set_command_in_flight(false);
                    model.set_message(
                        super::model::Severity::Error,
                        "Command queue is busy; review retained for retry.",
                    );
                }
                Err(error @ RuntimeError::Application(_)) => {
                    recover_skill_error(client, model, &error)?;
                }
                Err(error) => return Err(error),
            }
        }
        ControllerEffect::CancelSkillReview => cancel_skill_review_once(client, model)?,
        ControllerEffect::LoadAgentMemory(_)
        | ControllerEffect::LoadMemoryEntry { .. }
        | ControllerEffect::LoadMemoryEntryHistory { .. }
        | ControllerEffect::LoadMemoryEntryVersion { .. }
        | ControllerEffect::RequestMemorySetPreview(_)
        | ControllerEffect::RequestMemoryDeletePreview { .. }
        | ControllerEffect::LoadMemoryProposals { .. }
        | ControllerEffect::LoadMemoryProposal(_)
        | ControllerEffect::RequestMemoryProposalResolutionPreview { .. }
        | ControllerEffect::LoadEpisodicSummaries(_)
        | ControllerEffect::LoadEpisodicSummary(_)
        | ControllerEffect::ExecuteMemory(_)
        | ControllerEffect::CancelMemoryReview => {
            return Err(RuntimeError::Application(
                AppError::WrongMemoryCommandDispatcher,
            ));
        }
        _ => {}
    }
    Ok(())
}

fn execute_typed_skill_workflow(
    client: &RuntimeClient,
    model: &mut TuiModel,
    workflow: SkillWorkflowCommand,
) -> Result<(), RuntimeError> {
    match workflow {
        SkillWorkflowCommand::Add => {
            model.skills.start_create(None);
            synchronize_host_skill_input(model);
            model.clear_message();
        }
        SkillWorkflowCommand::Assign {
            skill,
            agent,
            version,
        } => {
            let command = match version {
                None => ApplicationCommand::ShowSkill { selector: skill },
                Some(version) => ApplicationCommand::ShowSkillVersion {
                    selector: skill,
                    version,
                },
            };
            let skill = submit_agent_command(client, model, command)?;
            let _ = apply_outcome(model, skill);
            let agent = submit_agent_command_for(
                client,
                model,
                AgentOutcomeIntent::SkillAssignment,
                ApplicationCommand::ShowAgentProfile { selector: agent },
            )?;
            let _ = apply_outcome(model, agent);
        }
        SkillWorkflowCommand::Unassign { skill, agent } => {
            let skill = submit_agent_command(
                client,
                model,
                ApplicationCommand::ShowSkill { selector: skill },
            )?;
            let _ = apply_outcome(model, skill);
            let Some(skill_id) = model.skills.current_skill_id() else {
                return Ok(());
            };
            let agent = submit_agent_command_for(
                client,
                model,
                AgentOutcomeIntent::SkillAssignment,
                ApplicationCommand::ShowAgentProfile { selector: agent },
            )?;
            let _ = apply_outcome(model, agent);
            let Some(agent_detail) = model.skills.selected_agent_detail.clone() else {
                return Ok(());
            };
            let Some(expected) = agent_detail
                .profile
                .skill_refs()
                .iter()
                .find(|reference| reference.skill_id() == skill_id)
                .cloned()
            else {
                model.skills.pane = super::model::SkillsPane::Detail;
                model.set_message(
                    super::model::Severity::Warning,
                    "Agent does not have this skill assigned.",
                );
                return Ok(());
            };
            let exact = submit_agent_command(
                client,
                model,
                ApplicationCommand::ShowSkillVersion {
                    selector: skill_id.into(),
                    version: expected.version(),
                },
            )?;
            let _ = apply_outcome(model, exact);
            model.skills.selected_agent_detail = Some(agent_detail);
            model.skills.assignment = Some(super::model::AssignmentKind::Unassign { expected });
            model.skills.pane = super::model::SkillsPane::AssignmentReview;
        }
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
        AgentSkillAssignmentOperation::Upgrade {
            expected,
            replacement,
        } => ApplicationCommand::UpgradeAgentSkill {
            profile_id: preview.profile_id,
            expected_active_profile_version_id: preview.expected_active_profile_version_id,
            expected,
            replacement,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
        AgentSkillAssignmentOperation::Unassign { expected } => {
            ApplicationCommand::UnassignAgentSkill {
                profile_id: preview.profile_id,
                expected_active_profile_version_id: preview.expected_active_profile_version_id,
                expected,
                review_token: preview.review_token,
                review_digest: preview.review_digest,
            }
        }
    };
    model.skills.review_registered = true;
    if let SkillOperationOrigin::AgentSkills { profile_id } = model.skills.operation_origin {
        model.skills.workspace_origin = Some(SkillWorkspaceOrigin::AgentSkills { profile_id });
        model.switch_tab(NavigationTab::Skills);
    }
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
    if let SkillOperationOrigin::AgentSkills { profile_id } = model.skills.operation_origin {
        model.skills.workspace_origin = None;
        model.select_view(super::model::View::Agents);
        model.agents.select_profile_id(profile_id);
        model.agents.skill_panel_open = true;
    }
    model.set_command_in_flight(false);
    model.set_message(
        super::model::Severity::Error,
        "Skill action failed; review cleared.",
    );
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

fn submit_agent_command_for(
    client: &RuntimeClient,
    model: &mut TuiModel,
    intent: AgentOutcomeIntent,
    command: ApplicationCommand,
) -> Result<CommandOutcome, RuntimeError> {
    model.pending_agent_outcome = Some(intent);
    let result = submit_agent_command(client, model, command);
    if result.is_err() {
        model.pending_agent_outcome = None;
    }
    result
}

fn submit_protected_skill_command(
    client: &RuntimeClient,
    model: &mut TuiModel,
    command: ApplicationCommand,
) -> Result<CommandOutcome, RuntimeError> {
    model.set_command_in_flight(true);
    let result = client
        .try_submit(command)
        .and_then(|pending| pending.recv());
    if result.is_err() {
        model.set_command_in_flight(false);
    }
    result
}

fn apply_agent_outcome(model: &mut TuiModel, outcome: CommandOutcome) {
    let view = outcome.view.clone();
    let _ = apply_outcome(model, outcome);
    match view {
        CommandView::AgentProfiles(profiles) => {
            model.agents.replace_profiles(profiles);
        }
        CommandView::AgentProfile(detail) => {
            model.agents.replace_detail(detail);
        }
        CommandView::AgentProfileHistory(history) => {
            model.agents.replace_history(history);
        }
        CommandView::AgentProfileVersion(version) => {
            model.agents.replace_version_detail(version);
        }
        _ => {}
    }
}

fn assignment_profile_id(command: &ApplicationCommand) -> Option<crate::domain::AgentProfileId> {
    match command {
        ApplicationCommand::AssignAgentSkill { profile_id, .. }
        | ApplicationCommand::UpgradeAgentSkill { profile_id, .. }
        | ApplicationCommand::UnassignAgentSkill { profile_id, .. } => Some(*profile_id),
        _ => None,
    }
}

fn needs_agent_skill_library(model: &TuiModel) -> bool {
    !model.skills.library_loaded
        && model
            .agents
            .detail
            .as_ref()
            .is_some_and(|detail| !detail.profile.skill_refs().is_empty())
}

fn load_agent_skill_library(
    client: &RuntimeClient,
    model: &mut TuiModel,
) -> Result<(), RuntimeError> {
    let navigation = model.navigation_state_snapshot();
    let result =
        submit_agent_command(client, model, ApplicationCommand::ListSkills).map(|outcome| {
            let _ = apply_outcome(model, outcome);
        });
    model.restore_navigation_state(navigation);
    result
}

fn ensure_agent_skill_library(
    client: &RuntimeClient,
    model: &mut TuiModel,
) -> Result<(), RuntimeError> {
    if needs_agent_skill_library(model) {
        load_agent_skill_library(client, model)?;
    }
    Ok(())
}

fn refresh_agent_skill_assignment_state(
    client: &RuntimeClient,
    model: &mut TuiModel,
    profile_id: crate::domain::AgentProfileId,
) -> Result<(), RuntimeError> {
    let profiles = submit_agent_command_for(
        client,
        model,
        AgentOutcomeIntent::SkillAssignment,
        ApplicationCommand::ListAgentProfiles,
    )?;
    apply_agent_outcome(model, profiles);
    model.agents.select_profile_id(profile_id);

    let outcome = submit_agent_command_for(
        client,
        model,
        AgentOutcomeIntent::SkillAssignment,
        ApplicationCommand::ShowAgentProfile {
            selector: profile_id.into(),
        },
    )?;
    let refreshed = match &outcome.view {
        CommandView::AgentProfile(detail) => Some(detail.clone()),
        _ => None,
    };
    let _ = apply_outcome(model, outcome);
    if let Some(detail) = refreshed {
        model.agents.replace_detail(detail.clone());
        model.skills.selected_agent_detail = Some(detail);
    }
    ensure_agent_skill_library(client, model)
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
    ensure_agent_skill_library(client, model)?;
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
        model.agents.select_profile_index(index);
    }
    let detail = submit_agent_command(
        client,
        model,
        ApplicationCommand::ShowAgentProfile {
            selector: profile_id.into(),
        },
    )?;
    apply_agent_outcome(model, detail);
    ensure_agent_skill_library(client, model)?;
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
    if ensure_agent_skill_library(client, model).is_err() {
        refresh_failed = true;
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
        model.agents.synchronize_field_input();
        model.set_focus(super::model::Focus::Workspace);
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingMemoryRequest {
    intent: MemoryOutcomeIntent,
    generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingProfileRequest {
    target: AgentProfileTarget,
    read: AgentProfileRead,
}

fn profile_read_command(target: &AgentProfileTarget, read: AgentProfileRead) -> ApplicationCommand {
    let selector = target.profile_id.into();
    match read {
        AgentProfileRead::Detail => ApplicationCommand::ShowAgentProfile { selector },
        AgentProfileRead::History => ApplicationCommand::ShowAgentProfileHistory { selector },
        AgentProfileRead::Version(version) => {
            ApplicationCommand::ShowAgentProfileVersion { selector, version }
        }
    }
}

fn profile_read_matches(read: AgentProfileRead, view: &CommandView) -> bool {
    match (read, view) {
        (AgentProfileRead::Detail, CommandView::AgentProfile(_))
        | (AgentProfileRead::History, CommandView::AgentProfileHistory(_)) => true,
        (AgentProfileRead::Version(expected), CommandView::AgentProfileVersion(version)) => {
            version.profile.version() == expected
        }
        _ => false,
    }
}

struct TuiRunner {
    runtime: Option<ApplicationRuntime>,
    client: RuntimeClient,
    model: TuiModel,
    pending: Option<PendingOutcome>,
    pending_memory: Option<PendingMemoryRequest>,
    pending_profile: Option<PendingProfileRequest>,
    deferred_profile_refresh: Option<PendingProfileRequest>,
    queued_shutdown: Option<ShutdownReason>,
    deferred_navigation_refresh: Option<ControllerEffect>,
    deferred_memory_refreshes: VecDeque<ControllerEffect>,
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
            pending_memory: None,
            pending_profile: None,
            deferred_profile_refresh: None,
            queued_shutdown: None,
            deferred_navigation_refresh: None,
            deferred_memory_refreshes: VecDeque::new(),
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

            if self.pending.is_none()
                && self.queued_shutdown.is_none()
                && let Some(effect) = self.deferred_navigation_refresh.take()
            {
                match self.apply_effect(effect)? {
                    LoopControl::Continue { redraw } => dirty |= redraw,
                    LoopControl::Finish(reason) => return Ok(reason),
                }
            }

            if self.pending.is_none()
                && self.queued_shutdown.is_none()
                && let Some(effect) = self.deferred_memory_refreshes.pop_front()
            {
                match self.apply_effect(effect)? {
                    LoopControl::Continue { redraw } => dirty |= redraw,
                    LoopControl::Finish(reason) => return Ok(reason),
                }
            }

            if self.pending.is_none()
                && self.queued_shutdown.is_none()
                && let Some(request) = self.deferred_profile_refresh.take()
            {
                self.apply_effect(ControllerEffect::LoadSelectedAgentProfile {
                    target: request.target,
                    read: request.read,
                })?;
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
        let outcome = match pending.try_recv() {
            Ok(Some(outcome)) => outcome,
            Ok(None) => return Ok(None),
            Err(error) => {
                self.pending.take();
                if let Some(request) = self.pending_profile.take() {
                    self.model.set_command_in_flight(false);
                    if self.model.agents.profile_target().as_ref() == Some(&request.target) {
                        self.model.set_message(
                            super::model::Severity::Warning,
                            "Agent details are unavailable. Select the agent again to retry.",
                        );
                    }
                    if matches!(error, RuntimeError::Application(_)) {
                        return Ok(Some(ControllerEffect::Redraw));
                    }
                }
                if let Some(request) = self.pending_memory.take() {
                    if matches!(error, RuntimeError::Application(_)) {
                        self.recover_memory_pending_error(&request, &error)?;
                        return Ok(Some(ControllerEffect::Redraw));
                    }
                    self.model
                        .agents
                        .memory
                        .clear_pending_if(&request.intent, request.generation);
                }
                self.model.set_command_in_flight(false);
                return Err(error.into());
            }
        };
        self.pending.take();
        if let Some(request) = self.pending_profile.take() {
            self.model.set_command_in_flight(false);
            if profile_read_matches(request.read, &outcome.view) {
                self.model
                    .agents
                    .install_profile_result(&request.target, outcome.view);
            }
            return Ok(Some(ControllerEffect::Redraw));
        }
        let memory_request = self.pending_memory.take();
        if let Some(request) = memory_request {
            let applied = apply_memory_outcome(
                &mut self.model.agents.memory,
                &request.intent,
                request.generation,
                outcome.view.clone(),
            );
            if applied {
                self.complete_memory_outcome(&request.intent, &outcome.view)?;
            } else {
                let malformed_current_request = self
                    .model
                    .agents
                    .memory
                    .clear_pending_if(&request.intent, request.generation);
                self.model.set_command_in_flight(false);
                if malformed_current_request {
                    return Err(TuiError::UnexpectedControllerEffect);
                }
                return Ok(Some(ControllerEffect::Redraw));
            }
        }
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
        if let ControllerEffect::LoadSelectedAgentProfile { target, read } = &effect {
            if self.model.agents.profile_target().as_ref() != Some(target) {
                return Ok(LoopControl::Continue { redraw: true });
            }
            let request = PendingProfileRequest {
                target: target.clone(),
                read: *read,
            };
            if self.pending.is_some() {
                if self.pending_profile.as_ref() != Some(&request) {
                    self.deferred_profile_refresh = Some(request);
                }
                return Ok(LoopControl::Continue { redraw: true });
            }
        }
        if self.pending.is_some() {
            if effect.is_navigation_refresh() {
                self.defer_navigation_refresh(effect);
                return Ok(LoopControl::Continue { redraw: true });
            }
            if effect.blocked_while_command_in_flight() {
                self.model
                    .set_message(super::model::Severity::Warning, PENDING_INTERACTION_MESSAGE);
                return Ok(LoopControl::Continue { redraw: true });
            }
        }

        match effect {
            ControllerEffect::LoadSelectedAgentProfile { target, read } => {
                if self.model.runtime_status == RuntimeStatus::Stopping {
                    return Ok(LoopControl::Continue { redraw: false });
                }
                self.submit_preserving_navigation(profile_read_command(&target, read))?;
                self.pending_profile = Some(PendingProfileRequest { target, read });
                Ok(LoopControl::Continue { redraw: true })
            }
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
            ControllerEffect::SubmitPreservingNavigation(command) => {
                if self.model.runtime_status == RuntimeStatus::Stopping {
                    return Ok(LoopControl::Continue { redraw: false });
                }
                self.submit_preserving_navigation(command)?;
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
            | ControllerEffect::StartSelectedProfileEdit { .. }
            | ControllerEffect::LoadAgentProfile { .. }
            | ControllerEffect::LoadAgentProfileHistory { .. }
            | ControllerEffect::LoadAgentProfileVersion { .. }
            | ControllerEffect::LoadAgentSkillLibrary
            | ControllerEffect::StartProfileCreate { .. }
            | ControllerEffect::StartProfileEdit { .. }
            | ControllerEffect::StartProfileCreateByTemplate { .. }
            | ControllerEffect::StartProfileEditBySelector { .. }
            | ControllerEffect::RequestProfilePreview(_)
            | ControllerEffect::ExecuteProfile(_)
            | ControllerEffect::CancelProfileReview) => {
                execute_agent_effect(&self.client, &mut self.model, effect)?;
                if self.model.agents.matching_detail().is_none()
                    && let Some(target) = self.model.agents.profile_target()
                {
                    self.deferred_profile_refresh = Some(PendingProfileRequest {
                        target,
                        read: AgentProfileRead::Detail,
                    });
                }
                Ok(LoopControl::Continue { redraw: true })
            }
            effect @ (ControllerEffect::LoadSkills
            | ControllerEffect::StartSkillWorkflow(_)
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
            effect @ (ControllerEffect::LoadAgentMemory(_)
            | ControllerEffect::LoadMemoryEntry { .. }
            | ControllerEffect::LoadMemoryEntryHistory { .. }
            | ControllerEffect::LoadMemoryEntryVersion { .. }
            | ControllerEffect::RequestMemorySetPreview(_)
            | ControllerEffect::RequestMemoryDeletePreview { .. }
            | ControllerEffect::LoadMemoryProposals { .. }
            | ControllerEffect::LoadMemoryProposal(_)
            | ControllerEffect::RequestMemoryProposalResolutionPreview { .. }
            | ControllerEffect::LoadEpisodicSummaries(_)
            | ControllerEffect::LoadEpisodicSummary(_)
            | ControllerEffect::ExecuteMemory(_)
            | ControllerEffect::CancelMemoryReview) => self.execute_memory_effect(effect),
        }
    }

    fn queue_memory_command(
        &mut self,
        command: ApplicationCommand,
        intent: MemoryOutcomeIntent,
    ) -> Result<LoopControl, TuiError> {
        debug_assert!(self.pending.is_none());
        if matches!(
            intent,
            MemoryOutcomeIntent::Mutation | MemoryOutcomeIntent::Resolution
        ) {
            let confirmed = self
                .model
                .agents
                .memory
                .confirmed_command()
                .ok_or(TuiError::UnexpectedControllerEffect)?;
            if confirmed != command {
                return Err(TuiError::UnexpectedControllerEffect);
            }
        }
        let generation = self.model.agents.memory.begin_pending(intent.clone())?;
        if matches!(
            intent,
            MemoryOutcomeIntent::Mutation | MemoryOutcomeIntent::Resolution
        ) {
            self.model
                .agents
                .memory
                .confirmation
                .as_mut()
                .ok_or(TuiError::UnexpectedControllerEffect)?
                .generation = generation;
        }
        self.model.set_command_in_flight_preserving_navigation();
        match self.client.try_submit(command) {
            Ok(pending) => {
                self.pending = Some(pending);
                self.pending_memory = Some(PendingMemoryRequest { intent, generation });
                Ok(LoopControl::Continue { redraw: true })
            }
            Err(RuntimeError::Backpressure) => {
                self.model.set_command_in_flight(false);
                self.model
                    .agents
                    .memory
                    .clear_pending_if(&intent, generation);
                self.model.set_message(
                    super::model::Severity::Warning,
                    "Another command is running; this memory action remains available to retry.",
                );
                Ok(LoopControl::Continue { redraw: true })
            }
            Err(error) => {
                self.model.set_command_in_flight(false);
                self.model
                    .agents
                    .memory
                    .clear_pending_if(&intent, generation);
                Err(error.into())
            }
        }
    }

    fn execute_memory_effect(&mut self, effect: ControllerEffect) -> Result<LoopControl, TuiError> {
        match effect {
            ControllerEffect::LoadAgentMemory(selector) => self.queue_memory_command(
                ApplicationCommand::ListMemoryEntries { selector },
                MemoryOutcomeIntent::Entries,
            ),
            ControllerEffect::LoadMemoryEntry { selector, key } => self.queue_memory_command(
                ApplicationCommand::ShowMemoryEntry {
                    selector,
                    display_key: key,
                },
                MemoryOutcomeIntent::EntryDetail(self.model.agents.memory.selected_entry_id()?),
            ),
            ControllerEffect::LoadMemoryEntryHistory { selector, key } => self
                .queue_memory_command(
                    ApplicationCommand::ShowMemoryEntryHistory {
                        selector,
                        display_key: key,
                    },
                    MemoryOutcomeIntent::EntryHistory(
                        self.model.agents.memory.selected_entry_id()?,
                    ),
                ),
            ControllerEffect::LoadMemoryEntryVersion {
                selector,
                key,
                version,
                expected_entry_version_id,
            } => self.queue_memory_command(
                ApplicationCommand::ShowMemoryEntryVersion {
                    selector: selector.clone(),
                    display_key: key.clone(),
                    version,
                },
                MemoryOutcomeIntent::EntryVersion {
                    selector,
                    key,
                    version,
                    entry_version_id: expected_entry_version_id,
                },
            ),
            ControllerEffect::LoadMemoryProposals { selector, filter } => self
                .queue_memory_command(
                    ApplicationCommand::ListMemoryProposals { selector, filter },
                    MemoryOutcomeIntent::Proposals,
                ),
            ControllerEffect::LoadMemoryProposal(proposal_id) => self.queue_memory_command(
                ApplicationCommand::ShowMemoryProposal { proposal_id },
                MemoryOutcomeIntent::ProposalDetail(proposal_id),
            ),
            ControllerEffect::LoadEpisodicSummaries(selector) => self.queue_memory_command(
                ApplicationCommand::ListEpisodicSummaries { selector },
                MemoryOutcomeIntent::Episodes,
            ),
            ControllerEffect::LoadEpisodicSummary(summary_id) => self.queue_memory_command(
                ApplicationCommand::ShowEpisodicSummary { summary_id },
                MemoryOutcomeIntent::EpisodeDetail(summary_id),
            ),
            ControllerEffect::ExecuteMemory(command) => {
                self.queue_memory_command(command.clone(), memory_mutation_intent(&command)?)
            }
            ControllerEffect::RequestMemorySetPreview(request) => {
                let result = install_set_preview_for_host(&self.client, &mut self.model, request);
                self.apply_memory_preview_result(result)
            }
            ControllerEffect::RequestMemoryDeletePreview {
                selector,
                key,
                generation,
            } => {
                let result = install_delete_preview_for_host(
                    &self.client,
                    &mut self.model,
                    selector,
                    key,
                    generation,
                );
                self.apply_memory_preview_result(result)
            }
            ControllerEffect::RequestMemoryProposalResolutionPreview {
                proposal,
                action,
                generation,
            } => {
                let result = install_resolution_preview_for_host(
                    &self.client,
                    &mut self.model,
                    proposal,
                    action,
                    generation,
                );
                self.apply_memory_preview_result(result)
            }
            ControllerEffect::CancelMemoryReview => {
                cancel_tui_memory_review_once(&self.client, &mut self.model.agents.memory)?;
                self.unwind_memory_review();
                Ok(LoopControl::Continue { redraw: true })
            }
            _ => Err(TuiError::UnexpectedControllerEffect),
        }
    }

    fn apply_memory_preview_result(
        &mut self,
        result: MemoryPreviewInstallResult,
    ) -> Result<LoopControl, TuiError> {
        match result {
            Ok(()) => Ok(LoopControl::Continue { redraw: true }),
            Err(MemoryPreviewFailure::Passive(
                TuiError::Runtime(RuntimeError::Application(_))
                | TuiError::Runtime(RuntimeError::Backpressure),
            )) => {
                self.model.set_message(
                    super::model::Severity::Error,
                    "Memory preview could not be completed.",
                );
                Ok(LoopControl::Continue { redraw: true })
            }
            Err(error) => Err(error.into_tui_error()),
        }
    }

    fn unwind_memory_review(&mut self) {
        let memory = &mut self.model.agents.memory;
        memory.confirmation = None;
        if memory.edit_review.take().is_some() {
            let pane = if memory.editor.is_some() {
                MemoryPane::Editor
            } else {
                MemoryPane::EntryDetail
            };
            memory.transition_to(pane);
            if let Some(editor) = memory.editor.as_mut() {
                editor.clear_review();
            }
        } else if memory.resolution_review.take().is_some() {
            memory.transition_to(MemoryPane::ProposalDetail);
        }
        synchronize_memory_editor_input(&mut self.model);
    }

    fn complete_memory_outcome(
        &mut self,
        intent: &MemoryOutcomeIntent,
        view: &CommandView,
    ) -> Result<(), TuiError> {
        if !matches!(
            intent,
            MemoryOutcomeIntent::Mutation | MemoryOutcomeIntent::Resolution
        ) {
            return Ok(());
        }
        let result_origin = match (intent, view) {
            (MemoryOutcomeIntent::Mutation, CommandView::MemoryEntryMutation(_)) => {
                MemoryResultOrigin::Mutation
            }
            (MemoryOutcomeIntent::Resolution, CommandView::MemoryProposalResolution(value)) => {
                let review = self
                    .model
                    .agents
                    .memory
                    .resolution_review
                    .as_ref()
                    .ok_or(TuiError::UnexpectedControllerEffect)?;
                if !memory_resolution_result_shape_matches(review, value) {
                    return Err(TuiError::UnexpectedControllerEffect);
                }
                MemoryResultOrigin::Resolution
            }
            _ => return Err(TuiError::UnexpectedControllerEffect),
        };
        let selector = self.model.agents.memory.profile_selector()?;
        let had_proposals = self.model.agents.memory.proposals.is_some();
        {
            let memory = &mut self.model.agents.memory;
            memory.review_registered = false;
            memory.edit_review = None;
            memory.resolution_review = None;
            memory.confirmation = None;
            memory.editor = None;
            memory.result_origin = result_origin;
            memory.transition_to(MemoryPane::Result);
        }
        match (intent, view) {
            (MemoryOutcomeIntent::Mutation, CommandView::MemoryEntryMutation(_)) => {
                self.deferred_memory_refreshes
                    .push_back(ControllerEffect::LoadAgentMemory(selector.clone()));
                if had_proposals {
                    self.deferred_memory_refreshes.push_back(
                        ControllerEffect::LoadMemoryProposals {
                            selector,
                            filter: crate::memory::MemoryProposalFilter::Pending,
                        },
                    );
                }
            }
            (MemoryOutcomeIntent::Resolution, CommandView::MemoryProposalResolution(value)) => {
                if value.entry.is_some() {
                    self.deferred_memory_refreshes
                        .push_back(ControllerEffect::LoadAgentMemory(selector.clone()));
                }
                self.deferred_memory_refreshes
                    .push_back(ControllerEffect::LoadMemoryProposals {
                        selector,
                        filter: crate::memory::MemoryProposalFilter::Pending,
                    });
            }
            _ => {}
        }
        Ok(())
    }

    fn recover_memory_pending_error(
        &mut self,
        request: &PendingMemoryRequest,
        error: &RuntimeError,
    ) -> Result<(), TuiError> {
        let request_is_current = self
            .model
            .agents
            .memory
            .clear_pending_if(&request.intent, request.generation);
        self.model.set_command_in_flight(false);
        if !request_is_current {
            return Ok(());
        }
        if !matches!(
            request.intent,
            MemoryOutcomeIntent::Mutation | MemoryOutcomeIntent::Resolution
        ) {
            self.model.set_message(
                super::model::Severity::Error,
                "Memory action could not be completed.",
            );
            return Ok(());
        }
        if memory_submission_is_retryable(error) {
            self.model.set_message(
                super::model::Severity::Warning,
                "Memory action could not be completed; review retained for retry.",
            );
            return Ok(());
        }

        cancel_tui_memory_review_once(&self.client, &mut self.model.agents.memory)?;
        let restore_editor_input = {
            let memory = &mut self.model.agents.memory;
            let had_edit_review = memory.edit_review.take().is_some();
            let had_resolution_review = memory.resolution_review.take().is_some();
            memory.confirmation = None;
            if had_edit_review && memory.editor.is_some() {
                if let Some(editor) = memory.editor.as_mut() {
                    editor.clear_review();
                }
                memory.transition_to(MemoryPane::Editor);
                true
            } else {
                if had_edit_review {
                    memory.transition_to(MemoryPane::EntryDetail);
                } else if had_resolution_review {
                    memory.transition_to(MemoryPane::ProposalDetail);
                }
                false
            }
        };
        if restore_editor_input {
            synchronize_memory_editor_input(&mut self.model);
        }
        self.model.set_message(
            super::model::Severity::Error,
            "Memory review was cleared; request a fresh review.",
        );
        Ok(())
    }

    fn defer_navigation_refresh(&mut self, effect: ControllerEffect) {
        if matches!(
            self.deferred_navigation_refresh.as_ref(),
            Some(ControllerEffect::LoadAgentProfiles)
        ) {
            return;
        }
        if effect == ControllerEffect::LoadAgentProfiles
            || self.deferred_navigation_refresh.is_none()
        {
            self.deferred_navigation_refresh = Some(effect);
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
                self.model.pending_agent_outcome = None;
                Err(error.into())
            }
        }
    }

    fn submit_preserving_navigation(
        &mut self,
        command: ApplicationCommand,
    ) -> Result<(), TuiError> {
        debug_assert!(self.pending.is_none());
        self.model.set_command_in_flight_preserving_navigation();
        match self.client.try_submit(command) {
            Ok(pending) => {
                self.pending = Some(pending);
                Ok(())
            }
            Err(error) => {
                self.model.set_command_in_flight(false);
                self.model.pending_agent_outcome = None;
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

    fn cancel_active_memory_review(&mut self) -> Result<(), RuntimeError> {
        cancel_tui_memory_review_once(&self.client, &mut self.model.agents.memory)
    }
}

fn memory_submission_is_retryable(error: &RuntimeError) -> bool {
    matches!(
        error,
        RuntimeError::Backpressure
            | RuntimeError::Application(AppError::Persistence(
                crate::persistence::PersistenceError::Contention
                    | crate::persistence::PersistenceError::Capacity
                    | crate::persistence::PersistenceError::QueryFailed
            ))
    )
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
    let skill_cancellation = runner
        .cancel_active_skill_review()
        .map_err(TuiError::Runtime);
    let memory_cancellation = runner
        .cancel_active_memory_review()
        .map_err(TuiError::Runtime);
    let restoration = screen.restore();
    let finish = runner.finish(finish_reason).map_err(TuiError::Runtime);
    match primary {
        Err(error) => Err(error),
        Ok(_) => cancellation
            .and(skill_cancellation)
            .and(memory_cancellation)
            .and(restoration)
            .and(finish),
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

    use super::{
        PendingMemoryRequest, TuiRunner, apply_memory_outcome, cancel_tui_memory_review_once,
        execute_agent_effect, install_delete_preview, install_resolution_preview,
        install_set_preview, memory_mutation_intent, memory_submission_is_retryable,
        run_with_screen,
    };
    use crate::{
        agents::{AgentProfileVersion, AgentReadiness, builtin_profile_templates},
        app::{
            AgentProfileCreatedView, AgentProfileHistoryEntry, AgentProfileHistoryView,
            AgentProfileSelector, AgentProfileSummary, AgentProfileView, AgentProfilesView,
            AppError, ApplicationCommand, CommandOutcome, CommandView, EpisodicSummariesView,
            EpisodicSummaryListItem, EpisodicSummaryView, HelpView, MemoryEditPreview,
            MemoryEntriesView, MemoryEntryHistorySummary, MemoryEntryHistoryView,
            MemoryEntryMutationView, MemoryEntrySummary, MemoryEntryVersionView, MemoryEntryView,
            MemoryProfileIdentityView, MemoryProposalResolutionReview,
            MemoryProposalResolutionView, MemoryProposalSummary, MemoryProposalView,
            MemoryProposalsView, PresentationSnapshot, ShutdownDisposition, ShutdownReason,
            ShutdownView, StatusView,
        },
        domain::{
            Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CommandId, CorrelationId,
            EpisodicSummaryId, EventId, InstallationId, MemoryEntryId, MemoryEntryVersionId,
            MemoryNamespaceId, MemoryProposalId, MemoryReviewToken, SessionId, SkillId, sha256,
        },
        memory::{
            EpisodicQualification, EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState,
            MemoryEditReview, MemoryEntryDraft, MemoryEntryState, MemoryEntryVersion, MemoryField,
            MemoryFieldDiff, MemoryFieldValue, MemoryMutationKind, MemoryNoChange,
            MemoryPlaintextAcknowledgement, MemoryProposal, MemoryProposalFilter,
            MemoryProposalOperation, MemoryProposalOperationKind, MemoryProposalResolution,
            MemoryProposalStatus, MemoryResolutionAction,
        },
        persistence::PersistenceError,
        policy::ApprovalStatus,
        runtime::{ApplicationRuntime, CommandExecutor, RuntimeError},
        setup::SetupStatus,
        ui::tui::{
            ControllerEffect, EventSource, MemoryOutcomeIntent, MemoryPane, MemoryViewState,
            Screen, TuiError, TuiEvent, handle_event,
            model::{
                AgentDetailAction, AgentsPane, LayoutMode, MemoryEditorOrigin, RuntimeStatus,
                SkillConfirmation, SkillOperationOrigin, SkillsPane, TuiModel, View,
            },
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

    enum MemoryReadResponse {
        View(Box<CommandView>),
        Error(AppError),
    }

    type MemoryReadRuntime = (
        ApplicationRuntime,
        Arc<Mutex<Vec<ApplicationCommand>>>,
        Option<Sender<()>>,
    );

    struct MemoryReadExecutor {
        expected: ApplicationCommand,
        response: Option<MemoryReadResponse>,
        calls: Arc<Mutex<Vec<ApplicationCommand>>>,
        release: Option<Receiver<()>>,
    }

    impl CommandExecutor for MemoryReadExecutor {
        fn execute_user(
            &mut self,
            command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            self.calls.lock().unwrap().push(command.clone());
            assert_eq!(command, self.expected);
            if let Some(release) = self.release.take() {
                release.recv().expect("release memory read");
            }
            match self.response.take().expect("one memory response") {
                MemoryReadResponse::View(view) => Ok(CommandOutcome {
                    command_id: CommandId::from_uuid(Uuid::from_u128(78_001)),
                    correlation_id: CorrelationId::from_uuid(Uuid::from_u128(78_002)),
                    committed_events: Vec::new(),
                    view: *view,
                    shutdown: ShutdownDisposition::Continue,
                }),
                MemoryReadResponse::Error(error) => Err(error),
            }
        }

        fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
            Ok(())
        }
    }

    fn memory_read_runtime(
        expected: ApplicationCommand,
        response: MemoryReadResponse,
        blocked: bool,
    ) -> MemoryReadRuntime {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (release_sender, release) = if blocked {
            let (sender, receiver) = bounded(1);
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        let runtime = ApplicationRuntime::spawn(
            MemoryReadExecutor {
                expected,
                response: Some(response),
                calls: Arc::clone(&calls),
                release,
            },
            1,
        )
        .expect("memory read runtime");
        (runtime, calls, release_sender)
    }

    fn poll_memory_completion(runner: &mut TuiRunner) -> Result<ControllerEffect, TuiError> {
        for _ in 0..100_000 {
            if let Some(effect) = runner.poll_pending()? {
                return Ok(effect);
            }
            thread::yield_now();
        }
        panic!("memory outcome did not arrive")
    }

    struct MemoryActionExecutor {
        responses: VecDeque<Result<CommandView, AppError>>,
        calls: Arc<Mutex<Vec<ApplicationCommand>>>,
        cancellations: Arc<AtomicUsize>,
    }

    impl CommandExecutor for MemoryActionExecutor {
        fn execute_user(
            &mut self,
            command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            self.calls.lock().unwrap().push(command);
            self.responses
                .pop_front()
                .expect("memory action response")
                .map(|view| CommandOutcome {
                    command_id: CommandId::from_uuid(Uuid::from_u128(78_101)),
                    correlation_id: CorrelationId::from_uuid(Uuid::from_u128(78_102)),
                    committed_events: Vec::new(),
                    view,
                    shutdown: ShutdownDisposition::Continue,
                })
        }

        fn cancel_memory_review(&mut self) -> Result<(), AppError> {
            self.cancellations.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
            Ok(())
        }
    }

    fn memory_action_runtime(
        responses: impl IntoIterator<Item = Result<CommandView, AppError>>,
    ) -> (
        ApplicationRuntime,
        Arc<Mutex<Vec<ApplicationCommand>>>,
        Arc<AtomicUsize>,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let cancellations = Arc::new(AtomicUsize::new(0));
        let runtime = ApplicationRuntime::spawn(
            MemoryActionExecutor {
                responses: responses.into_iter().collect(),
                calls: Arc::clone(&calls),
                cancellations: Arc::clone(&cancellations),
            },
            2,
        )
        .expect("memory action runtime");
        (runtime, calls, cancellations)
    }

    struct QueueBlockerExecutor {
        started: Option<Sender<()>>,
        release: Receiver<()>,
        calls: Arc<Mutex<Vec<ApplicationCommand>>>,
        cancellations: Arc<AtomicUsize>,
    }

    impl CommandExecutor for QueueBlockerExecutor {
        fn execute_user(
            &mut self,
            command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            self.calls.lock().unwrap().push(command.clone());
            if let Some(started) = self.started.take() {
                started.send(()).expect("announce queue blocker");
                self.release.recv().expect("release queue blocker");
            }
            Ok(outcome(command))
        }

        fn cancel_memory_review(&mut self) -> Result<(), AppError> {
            self.cancellations.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct TeardownObserver {
        profile_cancellations: Arc<AtomicUsize>,
        skill_cancellations: Arc<AtomicUsize>,
        memory_cancellations: Arc<AtomicUsize>,
        finishes: Arc<Mutex<Vec<ShutdownReason>>>,
    }

    struct TeardownExecutor {
        observer: TeardownObserver,
        fail_execute: bool,
        fail_profile_cancel: bool,
        fail_skill_cancel: bool,
        fail_memory_cancel: bool,
    }

    impl CommandExecutor for TeardownExecutor {
        fn execute_user(
            &mut self,
            command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            if self.fail_execute {
                Err(AppError::LifecycleFinished)
            } else {
                Ok(outcome(command))
            }
        }

        fn cancel_agent_profile_edit(&mut self) -> Result<(), AppError> {
            self.observer
                .profile_cancellations
                .fetch_add(1, Ordering::SeqCst);
            if self.fail_profile_cancel {
                Err(AppError::LifecycleFinished)
            } else {
                Ok(())
            }
        }

        fn cancel_skill_review(&mut self) -> Result<(), AppError> {
            self.observer
                .skill_cancellations
                .fetch_add(1, Ordering::SeqCst);
            if self.fail_skill_cancel {
                Err(AppError::LifecycleFinished)
            } else {
                Ok(())
            }
        }

        fn cancel_memory_review(&mut self) -> Result<(), AppError> {
            self.observer
                .memory_cancellations
                .fetch_add(1, Ordering::SeqCst);
            if self.fail_memory_cancel {
                Err(AppError::LifecycleFinished)
            } else {
                Ok(())
            }
        }

        fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
            self.observer.finishes.lock().unwrap().push(reason);
            Ok(())
        }
    }

    fn teardown_runtime(
        fail_execute: bool,
        fail_profile_cancel: bool,
        fail_skill_cancel: bool,
        fail_memory_cancel: bool,
    ) -> (ApplicationRuntime, TeardownObserver) {
        let observer = TeardownObserver {
            profile_cancellations: Arc::new(AtomicUsize::new(0)),
            skill_cancellations: Arc::new(AtomicUsize::new(0)),
            memory_cancellations: Arc::new(AtomicUsize::new(0)),
            finishes: Arc::new(Mutex::new(Vec::new())),
        };
        let runtime = ApplicationRuntime::spawn(
            TeardownExecutor {
                observer: observer.clone(),
                fail_execute,
                fail_profile_cancel,
                fail_skill_cancel,
                fail_memory_cancel,
            },
            2,
        )
        .expect("teardown runtime");
        (runtime, observer)
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

    fn refresh_profile(seed: u128, template_index: usize) -> AgentProfileVersion {
        let template = &builtin_profile_templates()[template_index];
        AgentProfileVersion::create(
            AgentProfileId::from_uuid(Uuid::from_u128(seed)),
            AgentProfileVersionId::from_uuid(Uuid::from_u128(seed + 1)),
            MemoryNamespaceId::from_uuid(Uuid::from_u128(seed + 2)),
            1_800_000_000_000,
            template.copy_to_draft().expect("profile draft"),
            Some(template.provenance()),
        )
        .expect("profile")
    }

    fn refresh_profile_summary(profile: &AgentProfileVersion) -> AgentProfileSummary {
        AgentProfileSummary {
            profile_id: profile.profile_id(),
            profile_version_id: profile.profile_version_id(),
            version: profile.version(),
            display_name: profile.display_name().to_owned(),
            role: profile.role(),
            primary_specialty: profile.primary_specialty().to_owned(),
            readiness: AgentReadiness::Unbound,
            content_digest: profile.content_digest().clone(),
        }
    }

    fn refresh_profile_history(profile: &AgentProfileVersion) -> AgentProfileHistoryView {
        AgentProfileHistoryView {
            profile_id: profile.profile_id(),
            active_version_id: profile.profile_version_id(),
            versions: vec![AgentProfileHistoryEntry {
                profile_version_id: profile.profile_version_id(),
                version: profile.version(),
                supersedes: profile.supersedes(),
                created_at_ms: profile.created_at_ms(),
                readiness: AgentReadiness::Unbound,
                content_digest: profile.content_digest().clone(),
            }],
            total_count: 1,
            returned_count: 1,
            truncated: false,
        }
    }

    fn memory_identity(profile: &AgentProfileVersion) -> MemoryProfileIdentityView {
        MemoryProfileIdentityView {
            profile: profile.reference(),
            display_name: profile.display_name().to_owned(),
        }
    }

    fn memory_entry(
        profile: &AgentProfileVersion,
        seed: u128,
        candidate: MemoryEntryDraft,
    ) -> MemoryEntryVersion {
        MemoryEntryVersion::create_present(
            profile.memory_namespace_id(),
            MemoryEntryId::from_uuid(Uuid::from_u128(seed)),
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(seed + 1)),
            candidate,
            Actor::Human,
            1_800_000_000_000,
            None,
            EventId::from_uuid(Uuid::from_u128(seed + 2)),
        )
        .expect("memory entry")
    }

    fn deleted_memory_entry(
        profile: &AgentProfileVersion,
        seed: u128,
        candidate: MemoryEntryDraft,
    ) -> MemoryEntryVersion {
        memory_entry(profile, seed, candidate)
            .next_deleted(
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(seed + 3)),
                Actor::Human,
                1_800_000_000_001,
                None,
                EventId::from_uuid(Uuid::from_u128(seed + 4)),
            )
            .expect("deleted memory entry")
    }

    fn bind_memory(model: &mut TuiModel, profile: &AgentProfileVersion) {
        model
            .agents
            .memory
            .bind_profile(memory_identity(profile), profile.memory_namespace_id())
            .expect("memory identity");
    }

    fn memory_entry_summary(entry: &MemoryEntryVersion) -> MemoryEntrySummary {
        MemoryEntrySummary {
            entry: entry.reference(),
            display_key: entry.display_key().to_owned(),
            purpose_tags: entry.purpose_tags().to_vec(),
            value_bytes: u64::try_from(entry.value().expect("present entry").len())
                .expect("entry value length"),
            created_at_ms: entry.created_at_ms(),
        }
    }

    fn memory_entries_view(
        profile: &AgentProfileVersion,
        entries: &[MemoryEntryVersion],
    ) -> MemoryEntriesView {
        MemoryEntriesView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            entries: entries.iter().map(memory_entry_summary).collect(),
            total_count: u64::try_from(entries.len()).expect("entry count"),
            returned_count: u64::try_from(entries.len()).expect("entry count"),
            omitted_count: 0,
        }
    }

    fn memory_entry_history_view(
        profile: &AgentProfileVersion,
        entry: &MemoryEntryVersion,
    ) -> MemoryEntryHistoryView {
        MemoryEntryHistoryView {
            profile: profile.reference(),
            current: entry.reference(),
            versions: vec![MemoryEntryHistorySummary {
                entry: entry.reference(),
                display_key: entry.display_key().to_owned(),
                created_at_ms: entry.created_at_ms(),
                accepted_proposal: entry.accepted_proposal().cloned(),
            }],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        }
    }

    fn memory_proposal_summary(proposal: &MemoryProposal) -> MemoryProposalSummary {
        MemoryProposalSummary {
            proposal: proposal.reference(),
            namespace_id: proposal.namespace_id(),
            proposer: proposal.proposer().clone(),
            operation: MemoryProposalOperationKind::Set,
            display_key: proposal.display_key().to_owned(),
            status: MemoryProposalStatus::Pending,
            created_at_ms: proposal.created_at_ms(),
        }
    }

    fn memory_proposals_view(
        profile: &AgentProfileVersion,
        proposals: &[MemoryProposal],
    ) -> MemoryProposalsView {
        MemoryProposalsView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            filter: MemoryProposalFilter::Pending,
            proposals: proposals.iter().map(memory_proposal_summary).collect(),
            total_count: u64::try_from(proposals.len()).expect("proposal count"),
            returned_count: u64::try_from(proposals.len()).expect("proposal count"),
            omitted_count: 0,
        }
    }

    fn episodic_summary(profile: &AgentProfileVersion, seed: u128) -> EpisodicSummary {
        let source = EpisodicSourceRef::new(
            u64::try_from(seed).expect("source sequence"),
            EventId::from_uuid(Uuid::from_u128(seed + 1)),
            "help_viewed".to_owned(),
            sha256(format!("episodic-source-{seed}").as_bytes()),
        )
        .expect("episodic source");
        EpisodicSummary::new(
            EpisodicSummaryId::from_uuid(Uuid::from_u128(seed)),
            profile,
            "Episode label".to_owned(),
            "Episode body".to_owned(),
            Vec::new(),
            vec![source],
            1_800_000_000_000,
            u64::try_from(seed + 2).expect("last source sequence"),
            EventId::from_uuid(Uuid::from_u128(seed + 3)),
        )
        .expect("episodic summary")
    }

    fn episodic_summaries_view(
        profile: &AgentProfileVersion,
        summaries: &[EpisodicSummary],
    ) -> EpisodicSummariesView {
        EpisodicSummariesView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            summaries: summaries
                .iter()
                .map(|summary| EpisodicSummaryListItem {
                    summary: summary.reference(),
                    label: summary.label().to_owned(),
                    purpose_tags: summary.purpose_tags().to_vec(),
                    source_count: u64::try_from(summary.sources().len()).expect("source count"),
                    created_at_ms: summary.created_at_ms(),
                })
                .collect(),
            total_count: u64::try_from(summaries.len()).expect("summary count"),
            returned_count: u64::try_from(summaries.len()).expect("summary count"),
            omitted_count: 0,
        }
    }

    #[derive(Clone)]
    struct MemoryReadCase {
        name: &'static str,
        pane: MemoryPane,
        resets_detail_scroll: bool,
        effect: ControllerEffect,
        command: ApplicationCommand,
        intent: MemoryOutcomeIntent,
        view: CommandView,
    }

    fn memory_read_cases(
        profile: &AgentProfileVersion,
        entry: &MemoryEntryVersion,
        proposal: &MemoryProposal,
        episode: &EpisodicSummary,
    ) -> Vec<MemoryReadCase> {
        let selector: AgentProfileSelector = profile.profile_id().into();
        vec![
            MemoryReadCase {
                name: "entries",
                pane: MemoryPane::EntryList,
                resets_detail_scroll: false,
                effect: ControllerEffect::LoadAgentMemory(selector.clone()),
                command: ApplicationCommand::ListMemoryEntries {
                    selector: selector.clone(),
                },
                intent: MemoryOutcomeIntent::Entries,
                view: CommandView::MemoryEntries(memory_entries_view(
                    profile,
                    std::slice::from_ref(entry),
                )),
            },
            MemoryReadCase {
                name: "entry detail",
                pane: MemoryPane::EntryDetail,
                resets_detail_scroll: true,
                effect: ControllerEffect::LoadMemoryEntry {
                    selector: selector.clone(),
                    key: entry.display_key().to_owned(),
                },
                command: ApplicationCommand::ShowMemoryEntry {
                    selector: selector.clone(),
                    display_key: entry.display_key().to_owned(),
                },
                intent: MemoryOutcomeIntent::EntryDetail(entry.reference().entry_id()),
                view: CommandView::MemoryEntry(MemoryEntryView {
                    profile: profile.reference(),
                    entry: entry.clone(),
                }),
            },
            MemoryReadCase {
                name: "entry history",
                pane: MemoryPane::EntryHistory,
                resets_detail_scroll: false,
                effect: ControllerEffect::LoadMemoryEntryHistory {
                    selector: selector.clone(),
                    key: entry.display_key().to_owned(),
                },
                command: ApplicationCommand::ShowMemoryEntryHistory {
                    selector: selector.clone(),
                    display_key: entry.display_key().to_owned(),
                },
                intent: MemoryOutcomeIntent::EntryHistory(entry.reference().entry_id()),
                view: CommandView::MemoryEntryHistory(memory_entry_history_view(profile, entry)),
            },
            MemoryReadCase {
                name: "entry version",
                pane: MemoryPane::EntryHistory,
                resets_detail_scroll: true,
                effect: ControllerEffect::LoadMemoryEntryVersion {
                    selector: selector.clone(),
                    key: entry.display_key().to_owned(),
                    version: entry.reference().version(),
                    expected_entry_version_id: entry.reference().entry_version_id(),
                },
                command: ApplicationCommand::ShowMemoryEntryVersion {
                    selector: selector.clone(),
                    display_key: entry.display_key().to_owned(),
                    version: entry.reference().version(),
                },
                intent: MemoryOutcomeIntent::EntryVersion {
                    selector: selector.clone(),
                    key: entry.display_key().to_owned(),
                    version: entry.reference().version(),
                    entry_version_id: entry.reference().entry_version_id(),
                },
                view: CommandView::MemoryEntryVersion(MemoryEntryVersionView {
                    profile: profile.reference(),
                    entry: entry.clone(),
                }),
            },
            MemoryReadCase {
                name: "proposals",
                pane: MemoryPane::Proposals,
                resets_detail_scroll: false,
                effect: ControllerEffect::LoadMemoryProposals {
                    selector: selector.clone(),
                    filter: MemoryProposalFilter::Pending,
                },
                command: ApplicationCommand::ListMemoryProposals {
                    selector: selector.clone(),
                    filter: MemoryProposalFilter::Pending,
                },
                intent: MemoryOutcomeIntent::Proposals,
                view: CommandView::MemoryProposals(memory_proposals_view(
                    profile,
                    std::slice::from_ref(proposal),
                )),
            },
            MemoryReadCase {
                name: "proposal detail",
                pane: MemoryPane::ProposalDetail,
                resets_detail_scroll: true,
                effect: ControllerEffect::LoadMemoryProposal(proposal.reference().proposal_id()),
                command: ApplicationCommand::ShowMemoryProposal {
                    proposal_id: proposal.reference().proposal_id(),
                },
                intent: MemoryOutcomeIntent::ProposalDetail(proposal.reference().proposal_id()),
                view: CommandView::MemoryProposal(proposal_view(profile, proposal)),
            },
            MemoryReadCase {
                name: "episodes",
                pane: MemoryPane::EpisodicSummaries,
                resets_detail_scroll: false,
                effect: ControllerEffect::LoadEpisodicSummaries(selector.clone()),
                command: ApplicationCommand::ListEpisodicSummaries {
                    selector: selector.clone(),
                },
                intent: MemoryOutcomeIntent::Episodes,
                view: CommandView::EpisodicSummaries(episodic_summaries_view(
                    profile,
                    std::slice::from_ref(episode),
                )),
            },
            MemoryReadCase {
                name: "episode detail",
                pane: MemoryPane::EpisodicDetail,
                resets_detail_scroll: true,
                effect: ControllerEffect::LoadEpisodicSummary(episode.reference().summary_id()),
                command: ApplicationCommand::ShowEpisodicSummary {
                    summary_id: episode.reference().summary_id(),
                },
                intent: MemoryOutcomeIntent::EpisodeDetail(episode.reference().summary_id()),
                view: CommandView::EpisodicSummary(EpisodicSummaryView {
                    summary: episode.clone(),
                    qualification: EpisodicQualification::SummaryVerifySources,
                }),
            },
        ]
    }

    fn assert_read_view_installed(memory: &MemoryViewState, expected: &CommandView) {
        match expected {
            CommandView::MemoryEntries(value) => assert_eq!(memory.entries.as_ref(), Some(value)),
            CommandView::MemoryEntry(value) => {
                assert_eq!(memory.entry_detail.as_ref(), Some(value));
            }
            CommandView::MemoryEntryHistory(value) => {
                assert_eq!(memory.entry_history.as_ref(), Some(value));
            }
            CommandView::MemoryEntryVersion(value) => {
                assert_eq!(memory.entry_version.as_ref(), Some(value));
            }
            CommandView::MemoryProposals(value) => {
                assert_eq!(memory.proposals.as_ref(), Some(value));
            }
            CommandView::MemoryProposal(value) => {
                assert_eq!(memory.proposal_detail.as_ref(), Some(value));
            }
            CommandView::EpisodicSummaries(value) => {
                assert_eq!(memory.episodes.as_ref(), Some(value));
            }
            CommandView::EpisodicSummary(value) => {
                assert_eq!(memory.episode_detail.as_ref(), Some(value));
            }
            _ => panic!("non-read fixture"),
        }
    }

    fn absent_set_diff(candidate: &MemoryEntryDraft) -> Vec<MemoryFieldDiff> {
        vec![
            MemoryFieldDiff {
                field: MemoryField::DisplayKey,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::Text(candidate.display_key().to_owned()),
            },
            MemoryFieldDiff {
                field: MemoryField::State,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::State(MemoryEntryState::Present),
            },
            MemoryFieldDiff {
                field: MemoryField::Value,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::Text(candidate.value().to_owned()),
            },
            MemoryFieldDiff {
                field: MemoryField::PurposeTags,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
            },
        ]
    }

    fn deleted_set_diff(candidate: &MemoryEntryDraft) -> Vec<MemoryFieldDiff> {
        vec![
            MemoryFieldDiff {
                field: MemoryField::State,
                before: MemoryFieldValue::State(MemoryEntryState::Deleted),
                after: MemoryFieldValue::State(MemoryEntryState::Present),
            },
            MemoryFieldDiff {
                field: MemoryField::Value,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::Text(candidate.value().to_owned()),
            },
            MemoryFieldDiff {
                field: MemoryField::PurposeTags,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
            },
        ]
    }

    fn deleted_set_diff_with_prior_key(
        candidate: &MemoryEntryDraft,
        prior_key: &str,
    ) -> Vec<MemoryFieldDiff> {
        let mut diff = deleted_set_diff(candidate);
        diff.insert(
            0,
            MemoryFieldDiff {
                field: MemoryField::DisplayKey,
                before: MemoryFieldValue::Text(prior_key.to_owned()),
                after: MemoryFieldValue::Text(candidate.display_key().to_owned()),
            },
        );
        diff
    }

    fn present_set_diff(
        seed: &MemoryEntryVersion,
        candidate: &MemoryEntryDraft,
    ) -> Vec<MemoryFieldDiff> {
        let before = [
            MemoryFieldValue::Text(seed.display_key().to_owned()),
            MemoryFieldValue::State(MemoryEntryState::Present),
            MemoryFieldValue::Text(seed.value().expect("present value").to_owned()),
            MemoryFieldValue::Tags(seed.purpose_tags().to_vec()),
        ];
        let after = [
            MemoryFieldValue::Text(candidate.display_key().to_owned()),
            MemoryFieldValue::State(MemoryEntryState::Present),
            MemoryFieldValue::Text(candidate.value().to_owned()),
            MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
        ];
        [
            MemoryField::DisplayKey,
            MemoryField::State,
            MemoryField::Value,
            MemoryField::PurposeTags,
        ]
        .into_iter()
        .zip(before)
        .zip(after)
        .filter_map(|((field, before), after)| {
            (before != after).then_some(MemoryFieldDiff {
                field,
                before,
                after,
            })
        })
        .collect()
    }

    fn delete_diff(entry: &MemoryEntryVersion) -> Vec<MemoryFieldDiff> {
        vec![
            MemoryFieldDiff {
                field: MemoryField::State,
                before: MemoryFieldValue::State(MemoryEntryState::Present),
                after: MemoryFieldValue::State(MemoryEntryState::Deleted),
            },
            MemoryFieldDiff {
                field: MemoryField::Value,
                before: MemoryFieldValue::Text(entry.value().expect("present value").to_owned()),
                after: MemoryFieldValue::Missing,
            },
            MemoryFieldDiff {
                field: MemoryField::PurposeTags,
                before: MemoryFieldValue::Tags(entry.purpose_tags().to_vec()),
                after: MemoryFieldValue::Missing,
            },
        ]
    }

    fn set_review(
        profile: &AgentProfileVersion,
        expected: ExpectedMemoryEntryState,
        candidate: MemoryEntryDraft,
        diff: Vec<MemoryFieldDiff>,
        seed: u128,
    ) -> MemoryEditReview {
        MemoryEditReview {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            expected,
            operation: MemoryMutationKind::Set,
            candidate: Some(candidate),
            diff,
            plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed)),
            review_digest: sha256(format!("set-review-{seed}").as_bytes()),
        }
    }

    fn install_confirmed_set(
        model: &mut TuiModel,
        profile: &AgentProfileVersion,
        seed: u128,
    ) -> (ApplicationCommand, MemoryEntryVersion) {
        bind_memory(model, profile);
        let candidate = MemoryEntryDraft::new(
            format!("Confirmed key {seed}"),
            format!("Confirmed value {seed}"),
            vec!["confirmed".to_owned()],
        )
        .expect("confirmed candidate");
        model
            .agents
            .memory
            .open_create_editor(profile.profile_id().into())
            .expect("confirmed editor");
        let editor = model.agents.memory.editor.as_mut().expect("editor");
        editor
            .submit_line(candidate.display_key().to_owned())
            .expect("candidate key");
        editor
            .submit_line(candidate.value().to_owned())
            .expect("candidate value");
        let crate::ui::memory_editor::MemoryEditorEffect::Preview(request) = editor
            .submit_line("confirmed".to_owned())
            .expect("candidate tags")
        else {
            panic!("preview request")
        };
        model
            .agents
            .memory
            .begin_review_request()
            .expect("outer review generation");
        let review = set_review(
            profile,
            ExpectedMemoryEntryState::Absent,
            candidate.clone(),
            absent_set_diff(&candidate),
            seed + 10,
        );
        assert!(
            model
                .agents
                .memory
                .editor
                .as_mut()
                .expect("editor")
                .apply_preview(
                    request.generation,
                    MemoryEditPreview::Review(review.clone())
                )
        );
        model.agents.memory.edit_review = Some(review.clone());
        model.agents.memory.review_registered = true;
        model.agents.memory.pane = MemoryPane::Confirmation;
        let command = ApplicationCommand::SetMemoryEntry {
            profile: review.profile,
            expected: review.expected,
            candidate: review.candidate.expect("set candidate"),
            review_token: review.review_token,
            review_digest: review.review_digest,
        };
        model.agents.memory.confirmation = Some(super::super::model::MemoryConfirmation {
            command: command.clone(),
            generation: model.agents.memory.generation,
        });
        let committed = memory_entry(profile, seed + 20, candidate);
        (command, committed)
    }

    fn delete_review(
        profile: &AgentProfileVersion,
        entry: &MemoryEntryVersion,
        seed: u128,
    ) -> MemoryEditReview {
        MemoryEditReview {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            expected: ExpectedMemoryEntryState::Present(entry.reference()),
            operation: MemoryMutationKind::Delete,
            candidate: None,
            diff: delete_diff(entry),
            plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed)),
            review_digest: sha256(format!("delete-review-{seed}").as_bytes()),
        }
    }

    fn install_confirmed_delete(
        model: &mut TuiModel,
        profile: &AgentProfileVersion,
        entry: &MemoryEntryVersion,
        seed: u128,
    ) -> (ApplicationCommand, MemoryEntryVersion) {
        bind_memory(model, profile);
        model.agents.memory.entries =
            Some(memory_entries_view(profile, std::slice::from_ref(entry)));
        model.agents.memory.selected_entry = 0;
        model.agents.memory.entry_detail = Some(MemoryEntryView {
            profile: profile.reference(),
            entry: entry.clone(),
        });
        model
            .agents
            .memory
            .begin_review_request()
            .expect("delete review generation");
        let review = delete_review(profile, entry, seed);
        let expected = match &review.expected {
            ExpectedMemoryEntryState::Present(expected) => expected.clone(),
            _ => panic!("present delete expectation"),
        };
        let command = ApplicationCommand::DeleteMemoryEntry {
            profile: review.profile.clone(),
            expected,
            review_token: review.review_token,
            review_digest: review.review_digest.clone(),
        };
        model.agents.memory.edit_review = Some(review);
        model.agents.memory.review_registered = true;
        model.agents.memory.pane = MemoryPane::Confirmation;
        model.agents.memory.confirmation = Some(super::super::model::MemoryConfirmation {
            command: command.clone(),
            generation: model.agents.memory.generation,
        });
        assert!(
            model.agents.memory.confirmed_command().is_some(),
            "confirmed delete fixture must be authenticated",
        );
        let deleted = entry
            .next_deleted(
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(seed + 1)),
                Actor::Human,
                1_800_000_000_100,
                None,
                EventId::from_uuid(Uuid::from_u128(seed + 2)),
            )
            .expect("deleted result");
        (command, deleted)
    }

    fn memory_proposal(profile: &AgentProfileVersion, seed: u128) -> MemoryProposal {
        let candidate = MemoryEntryDraft::new(
            "Proposal key".to_owned(),
            "Proposal value".to_owned(),
            Vec::new(),
        )
        .expect("proposal candidate");
        MemoryProposal::new(
            MemoryProposalId::from_uuid(Uuid::from_u128(seed)),
            profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Set { candidate },
            "Proposal key".to_owned(),
            ExpectedMemoryEntryState::Absent,
            "Proposal rationale".to_owned(),
            1_800_000_000_000,
            EventId::from_uuid(Uuid::from_u128(seed + 1)),
            ApprovalId::from_uuid(Uuid::from_u128(seed + 2)),
        )
        .expect("proposal")
    }

    fn proposal_view(
        profile: &AgentProfileVersion,
        proposal: &MemoryProposal,
    ) -> MemoryProposalView {
        MemoryProposalView {
            proposal: proposal.clone(),
            status: MemoryProposalStatus::Pending,
            resolution: None,
            current_entry: ExpectedMemoryEntryState::Absent,
            proposer_is_historical: false,
            proposer_identity: memory_identity(profile),
            namespace_owner_identity: memory_identity(profile),
        }
    }

    fn resolution_review(
        profile: &AgentProfileVersion,
        proposal: &MemoryProposal,
        action: MemoryResolutionAction,
        seed: u128,
    ) -> MemoryProposalResolutionReview {
        MemoryProposalResolutionReview {
            action,
            proposal: proposal.clone(),
            approval_id: proposal.approval_id(),
            expected_approval_status: ApprovalStatus::Pending,
            expected_entry: ExpectedMemoryEntryState::Absent,
            proposer_is_historical: false,
            proposer_identity: memory_identity(profile),
            namespace_owner_identity: memory_identity(profile),
            plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed)),
            review_digest: sha256(format!("resolution-review-{seed}").as_bytes()),
        }
    }

    fn install_confirmed_resolution(
        model: &mut TuiModel,
        profile: &AgentProfileVersion,
        proposal: &MemoryProposal,
        action: MemoryResolutionAction,
        seed: u128,
    ) -> ApplicationCommand {
        bind_memory(model, profile);
        model.agents.memory.proposal_detail = Some(proposal_view(profile, proposal));
        model.agents.memory.selected_proposal_detail_action = match action {
            MemoryResolutionAction::Approve => {
                super::super::model::MemoryProposalDetailAction::Approve
            }
            MemoryResolutionAction::Reject => {
                super::super::model::MemoryProposalDetailAction::Reject
            }
        };
        model
            .agents
            .memory
            .begin_review_request()
            .expect("resolution review generation");
        let review = resolution_review(profile, proposal, action, seed);
        model.agents.memory.resolution_review = Some(review.clone());
        model.agents.memory.review_registered = true;
        model.agents.memory.pane = MemoryPane::Confirmation;
        let fields = (
            review.proposal.reference(),
            review.approval_id,
            review.expected_approval_status,
            review.expected_entry,
            review.review_token,
            review.review_digest,
        );
        let command = match action {
            MemoryResolutionAction::Approve => ApplicationCommand::ApproveMemoryProposal {
                proposal: fields.0,
                approval_id: fields.1,
                expected_approval_status: fields.2,
                expected_entry: fields.3,
                review_token: fields.4,
                review_digest: fields.5,
            },
            MemoryResolutionAction::Reject => ApplicationCommand::RejectMemoryProposal {
                proposal: fields.0,
                approval_id: fields.1,
                expected_approval_status: fields.2,
                expected_entry: fields.3,
                review_token: fields.4,
                review_digest: fields.5,
            },
        };
        model.agents.memory.confirmation = Some(super::super::model::MemoryConfirmation {
            command: command.clone(),
            generation: model.agents.memory.generation,
        });
        command
    }

    fn proposal_resolution_view(
        proposal: &MemoryProposal,
        action: MemoryResolutionAction,
        entry: Option<crate::memory::MemoryEntryRef>,
        seed: u128,
    ) -> MemoryProposalResolutionView {
        let status = match action {
            MemoryResolutionAction::Approve => MemoryProposalStatus::Accepted,
            MemoryResolutionAction::Reject => MemoryProposalStatus::Rejected,
        };
        MemoryProposalResolutionView {
            resolution: MemoryProposalResolution::new(
                proposal.reference(),
                status,
                proposal.approval_id(),
                Actor::Human,
                1_800_000_000_100,
                EventId::from_uuid(Uuid::from_u128(seed)),
            )
            .expect("proposal resolution"),
            entry,
            expired_proposals: Vec::new(),
        }
    }

    enum MemoryPreviewResponse {
        Set(MemoryEditPreview),
        Delete(MemoryEditPreview),
        Approve(MemoryProposalResolutionReview),
        Reject(MemoryProposalResolutionReview),
    }

    struct MemoryPreviewExecutor {
        response: Option<MemoryPreviewResponse>,
        cancellations: Arc<AtomicUsize>,
        fail_cancellation: bool,
    }

    impl CommandExecutor for MemoryPreviewExecutor {
        fn execute_user(
            &mut self,
            _command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            Err(AppError::LifecycleFinished)
        }

        fn preview_memory_set(
            &mut self,
            _selector: AgentProfileSelector,
            _candidate: MemoryEntryDraft,
        ) -> Result<MemoryEditPreview, AppError> {
            match self.response.take().expect("set preview response") {
                MemoryPreviewResponse::Set(preview) => Ok(preview),
                _ => panic!("wrong preview route"),
            }
        }

        fn preview_memory_delete(
            &mut self,
            _selector: AgentProfileSelector,
            _display_key: String,
        ) -> Result<MemoryEditPreview, AppError> {
            match self.response.take().expect("delete preview response") {
                MemoryPreviewResponse::Delete(preview) => Ok(preview),
                _ => panic!("wrong preview route"),
            }
        }

        fn preview_memory_proposal_approval(
            &mut self,
            _proposal: crate::memory::MemoryProposalRef,
        ) -> Result<MemoryProposalResolutionReview, AppError> {
            match self.response.take().expect("approval preview response") {
                MemoryPreviewResponse::Approve(review) => Ok(review),
                _ => panic!("wrong preview route"),
            }
        }

        fn preview_memory_proposal_rejection(
            &mut self,
            _proposal: crate::memory::MemoryProposalRef,
        ) -> Result<MemoryProposalResolutionReview, AppError> {
            match self.response.take().expect("rejection preview response") {
                MemoryPreviewResponse::Reject(review) => Ok(review),
                _ => panic!("wrong preview route"),
            }
        }

        fn cancel_memory_review(&mut self) -> Result<(), AppError> {
            self.cancellations.fetch_add(1, Ordering::SeqCst);
            if self.fail_cancellation {
                Err(AppError::LifecycleFinished)
            } else {
                Ok(())
            }
        }

        fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
            Ok(())
        }
    }

    fn preview_runtime(
        response: MemoryPreviewResponse,
        fail_cancellation: bool,
    ) -> (ApplicationRuntime, Arc<AtomicUsize>) {
        let cancellations = Arc::new(AtomicUsize::new(0));
        let runtime = ApplicationRuntime::spawn(
            MemoryPreviewExecutor {
                response: Some(response),
                cancellations: cancellations.clone(),
                fail_cancellation,
            },
            2,
        )
        .expect("preview runtime");
        (runtime, cancellations)
    }

    fn begin_create_set_request(
        model: &mut TuiModel,
        profile: &AgentProfileVersion,
        candidate: &MemoryEntryDraft,
    ) -> crate::ui::memory_editor::MemoryPreviewRequest {
        bind_memory(model, profile);
        model
            .agents
            .memory
            .open_create_editor(profile.profile_id().into())
            .expect("create editor");
        let editor = model.agents.memory.editor.as_mut().expect("editor");
        editor
            .submit_line(candidate.display_key().to_owned())
            .expect("candidate key");
        editor
            .submit_line(candidate.value().to_owned())
            .expect("candidate value");
        let crate::ui::memory_editor::MemoryEditorEffect::Preview(request) = editor
            .submit_line(candidate.purpose_tags().join(","))
            .expect("candidate tags")
        else {
            panic!("create editor must produce a preview request")
        };
        model
            .agents
            .memory
            .begin_review_request()
            .expect("outer review generation");
        request
    }

    fn begin_seeded_set_request(
        model: &mut TuiModel,
        profile: &AgentProfileVersion,
        seed: MemoryEntryVersion,
        value: &str,
        tags: &str,
    ) -> crate::ui::memory_editor::MemoryPreviewRequest {
        bind_memory(model, profile);
        model
            .agents
            .memory
            .open_edit_editor(profile.profile_id().into(), seed)
            .expect("seeded editor");
        let editor = model.agents.memory.editor.as_mut().expect("editor");
        editor
            .submit_line(value.to_owned())
            .expect("candidate value");
        let crate::ui::memory_editor::MemoryEditorEffect::Preview(request) =
            editor.submit_line(tags.to_owned()).expect("candidate tags")
        else {
            panic!("seeded editor must produce a preview request")
        };
        model
            .agents
            .memory
            .begin_review_request()
            .expect("outer review generation");
        request
    }

    fn install_returned_set_review(
        model: &mut TuiModel,
        request: crate::ui::memory_editor::MemoryPreviewRequest,
        review: MemoryEditReview,
    ) -> (Result<(), TuiError>, usize) {
        let (runtime, cancellations) = preview_runtime(
            MemoryPreviewResponse::Set(MemoryEditPreview::Review(review)),
            false,
        );
        let result = install_set_preview(&runtime.client(), model, request);
        let cancellation_count = cancellations.load(Ordering::SeqCst);
        runtime
            .finish_and_join(ShutdownReason::Interrupted)
            .expect("finish preview runtime");
        (result, cancellation_count)
    }

    #[test]
    fn memory_command_classifier_accepts_only_confirmable_mutations_and_resolutions() {
        let profile = refresh_profile(79_000, 0);
        let set = ApplicationCommand::SetMemoryEntry {
            profile: profile.reference(),
            expected: ExpectedMemoryEntryState::Absent,
            candidate: MemoryEntryDraft::new(
                "Classifier key".to_owned(),
                "Classifier value".to_owned(),
                Vec::new(),
            )
            .expect("candidate"),
            review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(79_010)),
            review_digest: sha256(b"classifier review"),
        };

        assert!(matches!(
            memory_mutation_intent(&set),
            Ok(crate::ui::tui::MemoryOutcomeIntent::Mutation)
        ));
        assert!(memory_mutation_intent(&ApplicationCommand::ShowHelp).is_err());
    }

    #[test]
    fn all_thirteen_memory_effect_variants_are_explicitly_enumerated_by_the_host_contract() {
        let profile = refresh_profile(79_020, 0);
        let entry = memory_entry(
            &profile,
            79_030,
            MemoryEntryDraft::new(
                "Effect key".to_owned(),
                "Effect value".to_owned(),
                Vec::new(),
            )
            .expect("effect entry"),
        );
        let proposal = memory_proposal(&profile, 79_040);
        let episode = episodic_summary(&profile, 79_050);
        let selector: AgentProfileSelector = profile.profile_id().into();
        let preview = crate::ui::memory_editor::MemoryPreviewRequest {
            generation: 1,
            selector: selector.clone(),
            candidate: MemoryEntryDraft::new(
                "Effect preview".to_owned(),
                "Effect preview value".to_owned(),
                Vec::new(),
            )
            .expect("effect preview"),
        };
        let execute = ApplicationCommand::DeleteMemoryEntry {
            profile: profile.reference(),
            expected: entry.reference(),
            review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(79_060)),
            review_digest: sha256(b"effect delete"),
        };
        let effects = vec![
            ControllerEffect::LoadAgentMemory(selector.clone()),
            ControllerEffect::LoadMemoryEntry {
                selector: selector.clone(),
                key: entry.display_key().to_owned(),
            },
            ControllerEffect::LoadMemoryEntryHistory {
                selector: selector.clone(),
                key: entry.display_key().to_owned(),
            },
            ControllerEffect::LoadMemoryEntryVersion {
                selector: selector.clone(),
                key: entry.display_key().to_owned(),
                version: entry.reference().version(),
                expected_entry_version_id: entry.reference().entry_version_id(),
            },
            ControllerEffect::RequestMemorySetPreview(preview),
            ControllerEffect::RequestMemoryDeletePreview {
                selector: selector.clone(),
                key: entry.display_key().to_owned(),
                generation: 2,
            },
            ControllerEffect::LoadMemoryProposals {
                selector: selector.clone(),
                filter: MemoryProposalFilter::Pending,
            },
            ControllerEffect::LoadMemoryProposal(proposal.reference().proposal_id()),
            ControllerEffect::RequestMemoryProposalResolutionPreview {
                proposal: proposal.reference(),
                action: MemoryResolutionAction::Approve,
                generation: 3,
            },
            ControllerEffect::LoadEpisodicSummaries(selector),
            ControllerEffect::LoadEpisodicSummary(episode.reference().summary_id()),
            ControllerEffect::ExecuteMemory(execute),
            ControllerEffect::CancelMemoryReview,
        ];
        let route = |effect: &ControllerEffect| match effect {
            ControllerEffect::LoadAgentMemory(_) => 0,
            ControllerEffect::LoadMemoryEntry { .. } => 1,
            ControllerEffect::LoadMemoryEntryHistory { .. } => 2,
            ControllerEffect::LoadMemoryEntryVersion { .. } => 3,
            ControllerEffect::RequestMemorySetPreview(_) => 4,
            ControllerEffect::RequestMemoryDeletePreview { .. } => 5,
            ControllerEffect::LoadMemoryProposals { .. } => 6,
            ControllerEffect::LoadMemoryProposal(_) => 7,
            ControllerEffect::RequestMemoryProposalResolutionPreview { .. } => 8,
            ControllerEffect::LoadEpisodicSummaries(_) => 9,
            ControllerEffect::LoadEpisodicSummary(_) => 10,
            ControllerEffect::ExecuteMemory(_) => 11,
            ControllerEffect::CancelMemoryReview => 12,
            _ => panic!("non-Memory effect in exhaustive Memory matrix"),
        };

        assert_eq!(effects.len(), 13);
        assert_eq!(
            effects.iter().map(route).collect::<Vec<_>>(),
            (0..13).collect::<Vec<_>>()
        );
        assert!(
            effects
                .iter()
                .all(ControllerEffect::blocked_while_command_in_flight)
        );
    }

    #[test]
    fn delayed_memory_outcome_cannot_close_a_newer_editor() {
        let mut memory = crate::ui::tui::MemoryViewState::default();
        let intent = MemoryOutcomeIntent::Entries;
        let generation = memory
            .begin_pending(intent.clone())
            .expect("pending request");
        memory
            .open_create_editor(AgentProfileId::from_uuid(Uuid::from_u128(79_100)).into())
            .expect("new editor");

        assert!(!apply_memory_outcome(
            &mut memory,
            &intent,
            generation,
            CommandView::Help(HelpView),
        ));
        assert_eq!(memory.pane, MemoryPane::Editor);
        assert_eq!(memory.pending_intent, None);
    }

    #[test]
    fn memory_read_is_submitted_once_with_immutable_runner_provenance() {
        let profile = refresh_profile(79_200, 0);
        let (runtime, observer, release) = runtime(true, false, false);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        runner
            .model
            .agents
            .memory
            .bind_profile(
                MemoryProfileIdentityView {
                    profile: profile.reference(),
                    display_name: profile.display_name().to_owned(),
                },
                profile.memory_namespace_id(),
            )
            .expect("memory identity");

        let control = runner
            .apply_effect(ControllerEffect::LoadAgentMemory(
                profile.profile_id().into(),
            ))
            .expect("queue memory read");

        assert!(matches!(
            control,
            super::LoopControl::Continue { redraw: true }
        ));
        assert_eq!(
            runner.pending_memory,
            Some(super::PendingMemoryRequest {
                intent: MemoryOutcomeIntent::Entries,
                generation: 1,
            })
        );
        assert_eq!(
            runner.model.agents.memory.pending_intent,
            Some(MemoryOutcomeIntent::Entries)
        );
        for _ in 0..100_000 {
            if matches!(
                observer.commands().as_slice(),
                [ApplicationCommand::ListMemoryEntries { .. }]
            ) {
                break;
            }
            thread::yield_now();
        }
        assert!(matches!(
            observer.commands().as_slice(),
            [ApplicationCommand::ListMemoryEntries { selector }]
                if *selector == profile.profile_id().into()
        ));

        release.expect("release worker").send(()).expect("release");
        runner.finish(ShutdownReason::Interrupted).expect("finish");
    }

    #[test]
    fn all_eight_memory_reads_route_exact_commands_with_immutable_provenance() {
        let profile = refresh_profile(79_220, 0);
        let entry = memory_entry(
            &profile,
            79_230,
            MemoryEntryDraft::new("Read key".to_owned(), "Read value".to_owned(), Vec::new())
                .expect("entry draft"),
        );
        let proposal = memory_proposal(&profile, 79_240);
        let episode = episodic_summary(&profile, 79_250);

        for case in memory_read_cases(&profile, &entry, &proposal, &episode) {
            let (runtime, calls, _) = memory_read_runtime(
                case.command.clone(),
                MemoryReadResponse::View(Box::new(case.view.clone())),
                false,
            );
            let mut runner = TuiRunner::new(runtime, snapshot(), false);
            bind_memory(&mut runner.model, &profile);
            runner.model.active_view = View::Agents;
            runner.model.agents.pane = AgentsPane::Memory;
            runner.model.agents.memory.entries =
                Some(memory_entries_view(&profile, std::slice::from_ref(&entry)));
            runner.model.agents.memory.pane = case.pane;
            runner.model.agents.memory.detail_scroll = usize::MAX;

            assert!(
                matches!(
                    runner.apply_effect(case.effect.clone()),
                    Ok(super::LoopControl::Continue { redraw: true })
                ),
                "{}",
                case.name
            );
            let generation = runner.model.agents.memory.generation;
            assert_eq!(
                runner.pending_memory,
                Some(PendingMemoryRequest {
                    intent: case.intent.clone(),
                    generation,
                }),
                "{}",
                case.name,
            );
            assert_eq!(
                runner.model.agents.memory.pending_intent,
                Some(case.intent.clone()),
                "{}",
                case.name,
            );

            assert_eq!(
                poll_memory_completion(&mut runner).expect("matching memory outcome"),
                ControllerEffect::Redraw,
                "{}",
                case.name,
            );
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                [case.command],
                "{}",
                case.name
            );
            assert_read_view_installed(&runner.model.agents.memory, &case.view);
            assert_eq!(
                runner.model.agents.memory.detail_scroll,
                if case.resets_detail_scroll {
                    0
                } else {
                    usize::MAX
                },
                "{}",
                case.name
            );
            assert!(runner.pending.is_none(), "{}", case.name);
            assert!(runner.pending_memory.is_none(), "{}", case.name);
            assert_eq!(
                runner.model.agents.memory.pending_intent, None,
                "{}",
                case.name
            );
            assert_eq!(runner.model.active_view, View::Agents, "{}", case.name);
            assert_eq!(
                runner.model.agents.pane,
                AgentsPane::Memory,
                "{}",
                case.name
            );
            runner
                .finish(ShutdownReason::Interrupted)
                .expect("finish read runtime");
        }
    }

    #[test]
    fn all_eight_malformed_memory_read_outcomes_fail_closed_and_consume_their_tags() {
        let profile = refresh_profile(79_260, 0);
        let entry = memory_entry(
            &profile,
            79_270,
            MemoryEntryDraft::new(
                "Malformed key".to_owned(),
                "Malformed value".to_owned(),
                Vec::new(),
            )
            .expect("entry draft"),
        );
        let proposal = memory_proposal(&profile, 79_280);
        let episode = episodic_summary(&profile, 79_290);

        for case in memory_read_cases(&profile, &entry, &proposal, &episode) {
            let (runtime, _, _) = memory_read_runtime(
                case.command.clone(),
                MemoryReadResponse::View(Box::new(CommandView::Help(HelpView))),
                false,
            );
            let mut runner = TuiRunner::new(runtime, snapshot(), false);
            bind_memory(&mut runner.model, &profile);
            runner.model.active_view = View::Agents;
            runner.model.agents.pane = AgentsPane::Memory;
            runner.model.agents.memory.entries =
                Some(memory_entries_view(&profile, std::slice::from_ref(&entry)));
            runner.model.agents.memory.pane = case.pane;
            runner.model.agents.memory.detail_scroll = 91;

            runner
                .apply_effect(case.effect)
                .expect("queue malformed memory response");
            let mut expected_memory = runner.model.agents.memory.clone();
            expected_memory.pending_intent = None;

            assert!(
                matches!(
                    poll_memory_completion(&mut runner),
                    Err(TuiError::UnexpectedControllerEffect)
                ),
                "{}",
                case.name
            );
            assert_eq!(runner.model.agents.memory, expected_memory, "{}", case.name);
            assert_eq!(runner.model.active_view, View::Agents, "{}", case.name);
            assert_eq!(
                runner.model.agents.pane,
                AgentsPane::Memory,
                "{}",
                case.name
            );
            assert!(!runner.model.command_in_flight, "{}", case.name);
            assert!(runner.pending.is_none(), "{}", case.name);
            assert!(runner.pending_memory.is_none(), "{}", case.name);
            runner
                .finish(ShutdownReason::ApplicationError)
                .expect("finish malformed runtime");
        }
    }

    #[test]
    fn delayed_successes_and_errors_for_all_eight_reads_preserve_a_newer_editor() {
        let profile = refresh_profile(79_600, 0);
        let entry = memory_entry(
            &profile,
            79_610,
            MemoryEntryDraft::new(
                "Delayed key".to_owned(),
                "Delayed value".to_owned(),
                Vec::new(),
            )
            .expect("entry draft"),
        );
        let proposal = memory_proposal(&profile, 79_620);
        let episode = episodic_summary(&profile, 79_630);

        for returns_error in [false, true] {
            for case in memory_read_cases(&profile, &entry, &proposal, &episode) {
                let response = if returns_error {
                    MemoryReadResponse::Error(AppError::MemoryEntryNotFound)
                } else {
                    MemoryReadResponse::View(Box::new(case.view.clone()))
                };
                let (runtime, _, release) =
                    memory_read_runtime(case.command.clone(), response, true);
                let mut runner = TuiRunner::new(runtime, snapshot(), false);
                bind_memory(&mut runner.model, &profile);
                runner.model.active_view = View::Agents;
                runner.model.agents.pane = AgentsPane::Memory;
                runner.model.agents.memory.entries =
                    Some(memory_entries_view(&profile, std::slice::from_ref(&entry)));
                runner
                    .apply_effect(case.effect)
                    .expect("queue delayed read");
                runner
                    .model
                    .agents
                    .memory
                    .open_create_editor(profile.profile_id().into())
                    .expect("superseding editor");
                runner.model.command.ingest("newer local draft");
                let protected_memory = runner.model.agents.memory.clone();
                let protected_command = runner.model.command.clone();

                release
                    .expect("blocked read release")
                    .send(())
                    .expect("release read");
                assert_eq!(
                    poll_memory_completion(&mut runner).expect("stale response consumed"),
                    ControllerEffect::Redraw,
                    "{} error={returns_error}",
                    case.name,
                );
                assert_eq!(
                    runner.model.agents.memory, protected_memory,
                    "{} error={returns_error}",
                    case.name,
                );
                assert_eq!(
                    runner.model.command, protected_command,
                    "{} error={returns_error}",
                    case.name,
                );
                assert_eq!(runner.model.active_view, View::Agents, "{}", case.name);
                assert_eq!(
                    runner.model.agents.pane,
                    AgentsPane::Memory,
                    "{}",
                    case.name
                );
                assert!(runner.pending.is_none(), "{}", case.name);
                assert!(runner.pending_memory.is_none(), "{}", case.name);
                runner
                    .finish(ShutdownReason::Interrupted)
                    .expect("finish delayed runtime");
            }
        }
    }

    #[test]
    fn set_identical_content_keeps_no_runtime_review_registered() {
        let profile = refresh_profile(79_300, 0);
        let candidate = MemoryEntryDraft::new(
            "No change key".to_owned(),
            "No change value".to_owned(),
            Vec::new(),
        )
        .expect("candidate");
        let entry = memory_entry(&profile, 79_310, candidate.clone());
        let runtime = ApplicationRuntime::spawn(NoChangeMemoryPreviewExecutor, 1).expect("runtime");
        let client = runtime.client();
        let mut model = TuiModel::new(snapshot(), false);
        let selector: crate::app::AgentProfileSelector = profile.profile_id().into();
        bind_memory(&mut model, &profile);
        model.agents.memory.entry_detail = Some(MemoryEntryView {
            profile: profile.reference(),
            entry: entry.clone(),
        });
        model
            .agents
            .memory
            .open_edit_editor(selector, entry)
            .expect("editor");
        let editor = model.agents.memory.editor.as_mut().expect("editor state");
        editor
            .submit_line(candidate.value().to_owned())
            .expect("value");
        let crate::ui::memory_editor::MemoryEditorEffect::Preview(request) =
            editor.submit_line(String::new()).expect("tags")
        else {
            panic!("editor must produce preview request");
        };
        model
            .agents
            .memory
            .begin_review_request()
            .expect("review generation");

        install_set_preview(&client, &mut model, request).expect("no-change preview");

        assert!(!model.agents.memory.review_registered);
        assert_eq!(model.agents.memory.pane, MemoryPane::Editor);
        assert_eq!(
            model.message.as_ref().map(|message| message.text.as_str()),
            Some("No change")
        );
        runtime
            .finish_and_join(ShutdownReason::Interrupted)
            .expect("finish");
    }

    #[test]
    fn set_identical_content_rejects_create_without_a_retained_present_seed() {
        let profile = refresh_profile(79_400, 0);
        let runtime = ApplicationRuntime::spawn(NoChangeMemoryPreviewExecutor, 1).expect("runtime");
        let client = runtime.client();
        let mut model = TuiModel::new(snapshot(), false);
        let selector: crate::app::AgentProfileSelector = profile.profile_id().into();
        bind_memory(&mut model, &profile);
        model
            .agents
            .memory
            .open_create_editor(selector.clone())
            .expect("editor");
        let editor = model.agents.memory.editor.as_mut().expect("editor");
        editor
            .submit_line("Create key".to_owned())
            .expect("create key");
        editor
            .submit_line("Create value".to_owned())
            .expect("create value");
        let crate::ui::memory_editor::MemoryEditorEffect::Preview(request) =
            editor.submit_line(String::new()).expect("create tags")
        else {
            panic!("preview request")
        };
        model
            .agents
            .memory
            .begin_review_request()
            .expect("review generation");

        assert!(matches!(
            install_set_preview(&client, &mut model, request),
            Err(TuiError::UnexpectedControllerEffect)
        ));
        assert!(!model.agents.memory.review_registered);
        runtime
            .finish_and_join(ShutdownReason::Interrupted)
            .expect("finish");
    }

    #[test]
    fn returned_set_and_delete_reviews_are_cancelled_even_when_request_provenance_is_stale() {
        let profile = refresh_profile(79_500, 0);
        let other = refresh_profile(79_600, 1);
        let candidate =
            MemoryEntryDraft::new("Stale set".to_owned(), "Stale value".to_owned(), Vec::new())
                .expect("candidate");
        let review = set_review(
            &profile,
            ExpectedMemoryEntryState::Absent,
            candidate.clone(),
            absent_set_diff(&candidate),
            79_510,
        );
        let (runtime, cancellations) = preview_runtime(
            MemoryPreviewResponse::Set(MemoryEditPreview::Review(review)),
            false,
        );
        let mut model = TuiModel::new(snapshot(), false);
        bind_memory(&mut model, &profile);
        model
            .agents
            .memory
            .open_create_editor(profile.profile_id().into())
            .expect("editor");
        let editor = model.agents.memory.editor.as_mut().expect("editor");
        editor
            .submit_line(candidate.display_key().to_owned())
            .expect("key");
        editor
            .submit_line(candidate.value().to_owned())
            .expect("value");
        let crate::ui::memory_editor::MemoryEditorEffect::Preview(mut request) =
            editor.submit_line(String::new()).expect("tags")
        else {
            panic!("preview request")
        };
        request.selector = other.profile_id().into();

        assert!(matches!(
            install_set_preview(&runtime.client(), &mut model, request),
            Err(TuiError::UnexpectedControllerEffect)
        ));
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        assert!(!model.agents.memory.review_registered);
        runtime
            .finish_and_join(ShutdownReason::Interrupted)
            .expect("finish set runtime");

        let entry = memory_entry(
            &profile,
            79_700,
            MemoryEntryDraft::new(
                "Stale delete".to_owned(),
                "Delete value".to_owned(),
                Vec::new(),
            )
            .expect("delete entry"),
        );
        let review = delete_review(&profile, &entry, 79_710);
        let (runtime, cancellations) = preview_runtime(
            MemoryPreviewResponse::Delete(MemoryEditPreview::Review(review)),
            false,
        );
        let mut model = TuiModel::new(snapshot(), false);
        bind_memory(&mut model, &profile);
        model.agents.memory.entry_detail = Some(MemoryEntryView {
            profile: profile.reference(),
            entry: entry.clone(),
        });
        let generation = model
            .agents
            .memory
            .begin_review_request()
            .expect("delete generation");

        assert!(matches!(
            install_delete_preview(
                &runtime.client(),
                &mut model,
                profile.profile_id().into(),
                entry.display_key().to_owned(),
                generation + 1,
            ),
            Err(TuiError::UnexpectedControllerEffect)
        ));
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        assert!(!model.agents.memory.review_registered);
        runtime
            .finish_and_join(ShutdownReason::Interrupted)
            .expect("finish delete runtime");
    }

    #[test]
    fn set_review_authenticates_canonical_diff_seed_and_deleted_recreation() {
        let profile = refresh_profile(79_800, 0);
        let candidate = MemoryEntryDraft::new(
            "Recreated key".to_owned(),
            "Recreated value".to_owned(),
            vec!["purpose".to_owned()],
        )
        .expect("candidate");
        let tombstone = deleted_memory_entry(&profile, 79_810, candidate.clone());
        let review = set_review(
            &profile,
            ExpectedMemoryEntryState::Deleted(tombstone.reference()),
            candidate.clone(),
            deleted_set_diff(&candidate),
            79_820,
        );
        let (runtime, cancellations) = preview_runtime(
            MemoryPreviewResponse::Set(MemoryEditPreview::Review(review.clone())),
            false,
        );
        let mut model = TuiModel::new(snapshot(), false);
        bind_memory(&mut model, &profile);
        model
            .agents
            .memory
            .open_create_editor(profile.profile_id().into())
            .expect("create editor");
        let editor = model.agents.memory.editor.as_mut().expect("editor");
        editor
            .submit_line(candidate.display_key().to_owned())
            .expect("key");
        editor
            .submit_line(candidate.value().to_owned())
            .expect("value");
        let crate::ui::memory_editor::MemoryEditorEffect::Preview(request) = editor
            .submit_line("purpose".to_owned())
            .expect("purpose tags")
        else {
            panic!("preview request")
        };
        model
            .agents
            .memory
            .begin_review_request()
            .expect("outer generation");
        model.agents.memory.detail_scroll = usize::MAX;

        install_set_preview(&runtime.client(), &mut model, request)
            .expect("deleted recreation review");
        assert_eq!(model.agents.memory.edit_review, Some(review));
        assert!(model.agents.memory.review_registered);
        assert_eq!(model.agents.memory.detail_scroll, 0);
        assert_eq!(cancellations.load(Ordering::SeqCst), 0);
        runtime
            .finish_and_join(ShutdownReason::Interrupted)
            .expect("finish valid runtime");

        let mut malformed = set_review(
            &profile,
            ExpectedMemoryEntryState::Absent,
            candidate.clone(),
            absent_set_diff(&candidate),
            79_830,
        );
        malformed.diff.swap(0, 1);
        let (runtime, cancellations) = preview_runtime(
            MemoryPreviewResponse::Set(MemoryEditPreview::Review(malformed)),
            false,
        );
        let mut model = TuiModel::new(snapshot(), false);
        bind_memory(&mut model, &profile);
        model
            .agents
            .memory
            .open_create_editor(profile.profile_id().into())
            .expect("create editor");
        let editor = model.agents.memory.editor.as_mut().expect("editor");
        editor
            .submit_line(candidate.display_key().to_owned())
            .expect("key");
        editor
            .submit_line(candidate.value().to_owned())
            .expect("value");
        let crate::ui::memory_editor::MemoryEditorEffect::Preview(request) = editor
            .submit_line("purpose".to_owned())
            .expect("purpose tags")
        else {
            panic!("preview request")
        };
        model
            .agents
            .memory
            .begin_review_request()
            .expect("outer generation");
        model.agents.memory.detail_scroll = 73;

        assert!(matches!(
            install_set_preview(&runtime.client(), &mut model, request),
            Err(TuiError::UnexpectedControllerEffect)
        ));
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        assert!(!model.agents.memory.review_registered);
        assert_eq!(model.agents.memory.detail_scroll, 73);
        runtime
            .finish_and_join(ShutdownReason::Interrupted)
            .expect("finish malformed runtime");
    }

    #[test]
    fn deleted_recreation_prior_display_key_is_authenticated_before_host_installation() {
        let profile = refresh_profile(79_835, 0);
        let tombstone = deleted_memory_entry(
            &profile,
            79_836,
            MemoryEntryDraft::new(
                "Recreated Key".to_owned(),
                "Prior deleted value".to_owned(),
                vec!["prior".to_owned()],
            )
            .expect("prior entry"),
        );
        let candidate = MemoryEntryDraft::new(
            "recreated key".to_owned(),
            "Recreated value".to_owned(),
            vec!["recreated".to_owned()],
        )
        .expect("recreated candidate");

        for (index, (name, prior_key, valid)) in [
            ("canonical same-normalized prior key", "Recreated Key", true),
            ("foreign normalized prior key", "FOREIGN_PRIOR_KEY", false),
            ("unsafe prior key", "UNSAFE_PRIOR\u{202e}_KEY", false),
            ("noncanonical prior key", "recreated  key", false),
        ]
        .into_iter()
        .enumerate()
        {
            let mut model = TuiModel::new(snapshot(), false);
            let request = begin_create_set_request(&mut model, &profile, &candidate);
            let review = set_review(
                &profile,
                ExpectedMemoryEntryState::Deleted(tombstone.reference()),
                candidate.clone(),
                deleted_set_diff_with_prior_key(&candidate, prior_key),
                79_837 + u128::try_from(index).expect("case index"),
            );

            let (result, cancellations) =
                install_returned_set_review(&mut model, request, review.clone());

            if valid {
                assert!(result.is_ok(), "case={name}: {result:?}");
                assert_eq!(cancellations, 0, "case={name}");
                assert_eq!(model.agents.memory.edit_review, Some(review), "case={name}");
                assert!(
                    model.agents.memory.edit_review_command().is_some(),
                    "case={name}"
                );
            } else {
                assert!(
                    matches!(result, Err(TuiError::UnexpectedControllerEffect)),
                    "case={name}: {result:?}",
                );
                assert_eq!(cancellations, 1, "case={name}");
                assert!(!model.agents.memory.review_registered, "case={name}");
                assert!(model.agents.memory.edit_review.is_none(), "case={name}");
                assert!(
                    model.agents.memory.edit_review_command().is_none(),
                    "case={name}"
                );
            }
        }
    }

    #[test]
    fn seeded_set_review_uses_the_immutable_editor_seed_when_detail_cache_changes() {
        let profile = refresh_profile(79_840, 0);
        let foreign = refresh_profile(79_850, 1);
        let seed = memory_entry(
            &profile,
            79_860,
            MemoryEntryDraft::new(
                "Retained seed key".to_owned(),
                "Retained seed value".to_owned(),
                vec!["retained".to_owned()],
            )
            .expect("retained seed"),
        );
        let same_profile_replacement = memory_entry(
            &profile,
            79_870,
            MemoryEntryDraft::new(
                "Different cached key".to_owned(),
                "Different cached value".to_owned(),
                Vec::new(),
            )
            .expect("same-profile replacement"),
        );
        let foreign_replacement = memory_entry(
            &foreign,
            79_880,
            MemoryEntryDraft::new(
                "Foreign cached key".to_owned(),
                "Foreign cached value".to_owned(),
                Vec::new(),
            )
            .expect("foreign replacement"),
        );

        for (name, cached_detail) in [
            ("absent", None),
            (
                "different same-profile entry",
                Some(MemoryEntryView {
                    profile: profile.reference(),
                    entry: same_profile_replacement.clone(),
                }),
            ),
            (
                "foreign entry",
                Some(MemoryEntryView {
                    profile: foreign.reference(),
                    entry: foreign_replacement.clone(),
                }),
            ),
        ] {
            let mut model = TuiModel::new(snapshot(), false);
            let request = begin_seeded_set_request(
                &mut model,
                &profile,
                seed.clone(),
                "Updated retained value",
                "updated",
            );
            let candidate = request.candidate.clone();
            model.agents.memory.entry_detail = cached_detail;
            let review = set_review(
                &profile,
                ExpectedMemoryEntryState::Present(seed.reference()),
                candidate.clone(),
                present_set_diff(&seed, &candidate),
                79_890,
            );

            let (result, cancellations) =
                install_returned_set_review(&mut model, request, review.clone());

            assert!(result.is_ok(), "cache case={name}: {result:?}");
            assert_eq!(cancellations, 0, "cache case={name}");
            assert_eq!(model.agents.memory.edit_review, Some(review), "case={name}");
            assert_eq!(model.agents.memory.pane, MemoryPane::MutationReview);
            assert!(
                model.agents.memory.edit_review_command().is_some(),
                "case={name}"
            );
        }
    }

    #[test]
    fn seeded_set_review_rejects_cache_inferred_absent_deleted_and_present_states_once() {
        let profile = refresh_profile(79_900, 0);
        let seed = memory_entry(
            &profile,
            79_910,
            MemoryEntryDraft::new(
                "Protected seed key".to_owned(),
                "Protected seed value".to_owned(),
                vec!["protected".to_owned()],
            )
            .expect("protected seed"),
        );

        for name in [
            "absent without cache",
            "deleted without cache",
            "replacement cache",
        ] {
            let mut model = TuiModel::new(snapshot(), false);
            let request = begin_seeded_set_request(
                &mut model,
                &profile,
                seed.clone(),
                "Updated protected value",
                "updated",
            );
            let candidate = request.candidate.clone();
            let review = match name {
                "absent without cache" => set_review(
                    &profile,
                    ExpectedMemoryEntryState::Absent,
                    candidate.clone(),
                    absent_set_diff(&candidate),
                    79_920,
                ),
                "deleted without cache" => {
                    let tombstone = deleted_memory_entry(&profile, 79_930, candidate.clone());
                    set_review(
                        &profile,
                        ExpectedMemoryEntryState::Deleted(tombstone.reference()),
                        candidate.clone(),
                        deleted_set_diff(&candidate),
                        79_940,
                    )
                }
                "replacement cache" => {
                    let replacement = memory_entry(
                        &profile,
                        79_950,
                        MemoryEntryDraft::new(
                            seed.display_key().to_owned(),
                            "Cached replacement value".to_owned(),
                            vec!["cached".to_owned()],
                        )
                        .expect("cached replacement"),
                    );
                    let diff = present_set_diff(&replacement, &candidate);
                    model.agents.memory.entry_detail = Some(MemoryEntryView {
                        profile: profile.reference(),
                        entry: replacement.clone(),
                    });
                    set_review(
                        &profile,
                        ExpectedMemoryEntryState::Present(replacement.reference()),
                        candidate.clone(),
                        diff,
                        79_960,
                    )
                }
                _ => unreachable!("fixed cache case"),
            };

            let (result, cancellations) = install_returned_set_review(&mut model, request, review);

            assert!(
                matches!(result, Err(TuiError::UnexpectedControllerEffect)),
                "cache case={name}: {result:?}",
            );
            assert_eq!(cancellations, 1, "cache case={name}");
            assert!(!model.agents.memory.review_registered, "case={name}");
            assert!(model.agents.memory.edit_review.is_none(), "case={name}");
            assert!(
                model.agents.memory.edit_review_command().is_none(),
                "case={name}"
            );
        }
    }

    #[test]
    fn create_set_review_ignores_unrelated_detail_cache_but_keeps_create_state_exact() {
        let profile = refresh_profile(79_970, 0);
        let foreign = refresh_profile(79_980, 1);
        let unrelated = memory_entry(
            &foreign,
            79_990,
            MemoryEntryDraft::new(
                "Unrelated detail key".to_owned(),
                "Unrelated detail value".to_owned(),
                Vec::new(),
            )
            .expect("unrelated entry"),
        );

        for (name, expected_valid) in [
            ("absent create", true),
            ("deleted recreation", true),
            ("present cache substitution", false),
        ] {
            let candidate = MemoryEntryDraft::new(
                format!("Create cache key {name}"),
                "Create cache value".to_owned(),
                vec!["create".to_owned()],
            )
            .expect("create candidate");
            let mut model = TuiModel::new(snapshot(), false);
            let request = begin_create_set_request(&mut model, &profile, &candidate);
            model.agents.memory.entry_detail = Some(MemoryEntryView {
                profile: foreign.reference(),
                entry: unrelated.clone(),
            });
            let review = match name {
                "absent create" => set_review(
                    &profile,
                    ExpectedMemoryEntryState::Absent,
                    candidate.clone(),
                    absent_set_diff(&candidate),
                    80_000,
                ),
                "deleted recreation" => {
                    let tombstone = deleted_memory_entry(&profile, 80_010, candidate.clone());
                    set_review(
                        &profile,
                        ExpectedMemoryEntryState::Deleted(tombstone.reference()),
                        candidate.clone(),
                        deleted_set_diff(&candidate),
                        80_020,
                    )
                }
                "present cache substitution" => set_review(
                    &profile,
                    ExpectedMemoryEntryState::Present(unrelated.reference()),
                    candidate.clone(),
                    present_set_diff(&unrelated, &candidate),
                    80_030,
                ),
                _ => unreachable!("fixed create case"),
            };

            let (result, cancellations) = install_returned_set_review(&mut model, request, review);

            if expected_valid {
                assert!(result.is_ok(), "create case={name}: {result:?}");
                assert_eq!(cancellations, 0, "create case={name}");
                assert!(
                    model.agents.memory.edit_review_command().is_some(),
                    "case={name}"
                );
            } else {
                assert!(
                    matches!(result, Err(TuiError::UnexpectedControllerEffect)),
                    "create case={name}: {result:?}",
                );
                assert_eq!(cancellations, 1, "create case={name}");
                assert!(
                    model.agents.memory.edit_review_command().is_none(),
                    "case={name}"
                );
            }
        }
    }

    #[test]
    fn malformed_editor_provenance_cannot_install_or_execute_a_returned_set_review() {
        let profile = refresh_profile(80_040, 0);
        let foreign = refresh_profile(80_050, 1);

        let seed = memory_entry(
            &profile,
            80_060,
            MemoryEntryDraft::new(
                "Malformed provenance key".to_owned(),
                "Original value".to_owned(),
                Vec::new(),
            )
            .expect("seed"),
        );
        let mut seeded_wrong_origin = TuiModel::new(snapshot(), false);
        let request = begin_seeded_set_request(
            &mut seeded_wrong_origin,
            &profile,
            seed.clone(),
            "Updated value",
            "updated",
        );
        let candidate = request.candidate.clone();
        seeded_wrong_origin.agents.memory.editor_origin = MemoryEditorOrigin::Create;
        seeded_wrong_origin.agents.memory.entry_detail = Some(MemoryEntryView {
            profile: profile.reference(),
            entry: seed.clone(),
        });
        let review = set_review(
            &profile,
            ExpectedMemoryEntryState::Present(seed.reference()),
            candidate.clone(),
            present_set_diff(&seed, &candidate),
            80_070,
        );
        let (result, cancellations) =
            install_returned_set_review(&mut seeded_wrong_origin, request, review);
        assert!(matches!(result, Err(TuiError::UnexpectedControllerEffect)));
        assert_eq!(cancellations, 1);
        assert!(
            seeded_wrong_origin
                .agents
                .memory
                .edit_review_command()
                .is_none()
        );

        let cached_seed = memory_entry(
            &profile,
            80_080,
            MemoryEntryDraft::new(
                "Seedless editor key".to_owned(),
                "Cached value".to_owned(),
                Vec::new(),
            )
            .expect("cached seed"),
        );
        let mut seedless_edit = TuiModel::new(snapshot(), false);
        let create_candidate = MemoryEntryDraft::new(
            cached_seed.display_key().to_owned(),
            "Seedless updated value".to_owned(),
            vec!["updated".to_owned()],
        )
        .expect("seedless candidate");
        let request = begin_create_set_request(&mut seedless_edit, &profile, &create_candidate);
        seedless_edit.agents.memory.editor_origin = MemoryEditorOrigin::Edit;
        seedless_edit.agents.memory.entry_detail = Some(MemoryEntryView {
            profile: profile.reference(),
            entry: cached_seed.clone(),
        });
        let review = set_review(
            &profile,
            ExpectedMemoryEntryState::Present(cached_seed.reference()),
            create_candidate.clone(),
            present_set_diff(&cached_seed, &create_candidate),
            80_090,
        );
        let (result, cancellations) =
            install_returned_set_review(&mut seedless_edit, request, review);
        assert!(matches!(result, Err(TuiError::UnexpectedControllerEffect)));
        assert_eq!(cancellations, 1);
        assert!(seedless_edit.agents.memory.edit_review_command().is_none());

        let foreign_seed = memory_entry(
            &foreign,
            80_100,
            MemoryEntryDraft::new(
                "Foreign seed key".to_owned(),
                "Foreign seed value".to_owned(),
                Vec::new(),
            )
            .expect("foreign seed"),
        );
        let mut foreign_seed_editor = TuiModel::new(snapshot(), false);
        let request = begin_seeded_set_request(
            &mut foreign_seed_editor,
            &profile,
            foreign_seed.clone(),
            "Foreign updated value",
            "updated",
        );
        let candidate = request.candidate.clone();
        foreign_seed_editor.agents.memory.entry_detail = Some(MemoryEntryView {
            profile: foreign.reference(),
            entry: foreign_seed.clone(),
        });
        let review = set_review(
            &profile,
            ExpectedMemoryEntryState::Present(foreign_seed.reference()),
            candidate.clone(),
            present_set_diff(&foreign_seed, &candidate),
            80_110,
        );
        let (result, cancellations) =
            install_returned_set_review(&mut foreign_seed_editor, request, review);
        assert!(matches!(result, Err(TuiError::UnexpectedControllerEffect)));
        assert_eq!(cancellations, 1);
        assert!(
            foreign_seed_editor
                .agents
                .memory
                .edit_review_command()
                .is_none()
        );
    }

    #[test]
    fn delete_review_requires_exact_selected_detail_and_canonical_diff() {
        let profile = refresh_profile(80_000, 0);
        let entry = memory_entry(
            &profile,
            80_010,
            MemoryEntryDraft::new(
                "Delete exact".to_owned(),
                "Delete value".to_owned(),
                vec!["purpose".to_owned()],
            )
            .expect("entry"),
        );
        let mut malformed = delete_review(&profile, &entry, 80_020);
        malformed.diff.swap(0, 1);
        for (name, selected_detail, review) in [
            (
                "missing selected detail",
                None,
                delete_review(&profile, &entry, 80_021),
            ),
            (
                "noncanonical diff",
                Some(MemoryEntryView {
                    profile: profile.reference(),
                    entry: entry.clone(),
                }),
                malformed,
            ),
        ] {
            let (runtime, cancellations) = preview_runtime(
                MemoryPreviewResponse::Delete(MemoryEditPreview::Review(review)),
                false,
            );
            let mut model = TuiModel::new(snapshot(), false);
            bind_memory(&mut model, &profile);
            model.agents.memory.entry_detail = selected_detail;
            let generation = model
                .agents
                .memory
                .begin_review_request()
                .expect("delete generation");

            assert!(
                matches!(
                    install_delete_preview(
                        &runtime.client(),
                        &mut model,
                        profile.profile_id().into(),
                        entry.display_key().to_owned(),
                        generation,
                    ),
                    Err(TuiError::UnexpectedControllerEffect)
                ),
                "{name}",
            );
            assert_eq!(cancellations.load(Ordering::SeqCst), 1, "{name}");
            assert!(!model.agents.memory.review_registered, "{name}");
            runtime
                .finish_and_join(ShutdownReason::Interrupted)
                .expect("finish runtime");
        }
    }

    #[test]
    fn delete_review_rejects_substituted_selected_value_and_tags() {
        let profile = refresh_profile(80_040, 0);
        let entry = memory_entry(
            &profile,
            80_050,
            MemoryEntryDraft::new(
                "Delete protected".to_owned(),
                "Exact selected value".to_owned(),
                vec!["exact-tag".to_owned()],
            )
            .expect("entry"),
        );
        let mut wrong_value = delete_review(&profile, &entry, 80_060);
        wrong_value.diff[1].before = MemoryFieldValue::Text("substituted value".to_owned());
        let mut wrong_tags = delete_review(&profile, &entry, 80_061);
        wrong_tags.diff[2].before = MemoryFieldValue::Tags(vec!["substituted-tag".to_owned()]);

        for (name, review) in [("value", wrong_value), ("tags", wrong_tags)] {
            let (runtime, cancellations) = preview_runtime(
                MemoryPreviewResponse::Delete(MemoryEditPreview::Review(review)),
                false,
            );
            let mut model = TuiModel::new(snapshot(), false);
            bind_memory(&mut model, &profile);
            model.agents.memory.entry_detail = Some(MemoryEntryView {
                profile: profile.reference(),
                entry: entry.clone(),
            });
            let generation = model
                .agents
                .memory
                .begin_review_request()
                .expect("delete generation");

            assert!(
                matches!(
                    install_delete_preview(
                        &runtime.client(),
                        &mut model,
                        profile.profile_id().into(),
                        entry.display_key().to_owned(),
                        generation,
                    ),
                    Err(TuiError::UnexpectedControllerEffect)
                ),
                "{name}"
            );
            assert_eq!(cancellations.load(Ordering::SeqCst), 1, "{name}");
            assert!(!model.agents.memory.review_registered, "{name}");
            runtime
                .finish_and_join(ShutdownReason::Interrupted)
                .expect("finish runtime");
        }
    }

    #[test]
    fn delete_preview_installs_exact_review_and_authenticates_no_change_kind() {
        let profile = refresh_profile(80_070, 0);
        let entry = memory_entry(
            &profile,
            80_080,
            MemoryEntryDraft::new(
                "Delete exact success".to_owned(),
                "Delete exact value".to_owned(),
                vec!["purpose".to_owned()],
            )
            .expect("entry"),
        );
        let exact_review = delete_review(&profile, &entry, 80_090);
        let (runtime, cancellations) = preview_runtime(
            MemoryPreviewResponse::Delete(MemoryEditPreview::Review(exact_review.clone())),
            false,
        );
        let mut model = TuiModel::new(snapshot(), false);
        bind_memory(&mut model, &profile);
        model.agents.memory.entry_detail = Some(MemoryEntryView {
            profile: profile.reference(),
            entry: entry.clone(),
        });
        model.agents.memory.pane = MemoryPane::EntryDetail;
        let generation = model
            .agents
            .memory
            .begin_review_request()
            .expect("delete generation");
        model.agents.memory.detail_scroll = usize::MAX;

        install_delete_preview(
            &runtime.client(),
            &mut model,
            profile.profile_id().into(),
            entry.display_key().to_owned(),
            generation,
        )
        .expect("exact delete review");
        assert_eq!(model.agents.memory.edit_review, Some(exact_review));
        assert_eq!(model.agents.memory.pane, MemoryPane::MutationReview);
        assert!(model.agents.memory.review_registered);
        assert_eq!(model.agents.memory.detail_scroll, 0);
        cancel_tui_memory_review_once(&runtime.client(), &mut model.agents.memory)
            .expect("cancel exact review");
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        runtime
            .finish_and_join(ShutdownReason::Interrupted)
            .expect("finish exact runtime");

        for (name, no_change, accepted) in [
            ("already absent", MemoryNoChange::AlreadyAbsent, true),
            ("identical content", MemoryNoChange::IdenticalContent, false),
        ] {
            let (runtime, cancellations) = preview_runtime(
                MemoryPreviewResponse::Delete(MemoryEditPreview::NoChange(no_change)),
                false,
            );
            let mut model = TuiModel::new(snapshot(), false);
            bind_memory(&mut model, &profile);
            model.agents.memory.entry_detail = Some(MemoryEntryView {
                profile: profile.reference(),
                entry: entry.clone(),
            });
            model.agents.memory.pane = MemoryPane::EntryDetail;
            let generation = model
                .agents
                .memory
                .begin_review_request()
                .expect("delete generation");
            model.agents.memory.detail_scroll = 71;
            let result = install_delete_preview(
                &runtime.client(),
                &mut model,
                profile.profile_id().into(),
                entry.display_key().to_owned(),
                generation,
            );
            assert_eq!(result.is_ok(), accepted, "{name}");
            assert!(!model.agents.memory.review_registered, "{name}");
            assert_eq!(model.agents.memory.detail_scroll, 71, "{name}");
            assert_eq!(cancellations.load(Ordering::SeqCst), 0, "{name}");
            runtime
                .finish_and_join(ShutdownReason::Interrupted)
                .expect("finish no-change runtime");
        }
    }

    #[test]
    fn approve_and_reject_previews_use_their_exact_passive_routes() {
        let profile = refresh_profile(80_095, 0);
        let proposal = memory_proposal(&profile, 80_096);

        for action in [
            MemoryResolutionAction::Approve,
            MemoryResolutionAction::Reject,
        ] {
            let exact_review = resolution_review(&profile, &proposal, action, 80_097);
            let response = match action {
                MemoryResolutionAction::Approve => {
                    MemoryPreviewResponse::Approve(exact_review.clone())
                }
                MemoryResolutionAction::Reject => {
                    MemoryPreviewResponse::Reject(exact_review.clone())
                }
            };
            let (runtime, cancellations) = preview_runtime(response, false);
            let mut model = TuiModel::new(snapshot(), false);
            bind_memory(&mut model, &profile);
            model.agents.memory.proposal_detail = Some(proposal_view(&profile, &proposal));
            model.agents.memory.selected_proposal_detail_action = match action {
                MemoryResolutionAction::Approve => {
                    super::super::model::MemoryProposalDetailAction::Approve
                }
                MemoryResolutionAction::Reject => {
                    super::super::model::MemoryProposalDetailAction::Reject
                }
            };
            model.agents.memory.pane = MemoryPane::ProposalDetail;
            let generation = model
                .agents
                .memory
                .begin_review_request()
                .expect("resolution generation");
            model.agents.memory.detail_scroll = usize::MAX;

            install_resolution_preview(
                &runtime.client(),
                &mut model,
                proposal.reference(),
                action,
                generation,
            )
            .expect("exact resolution review");
            assert_eq!(model.agents.memory.resolution_review, Some(exact_review));
            assert_eq!(
                model.agents.memory.pane,
                MemoryPane::ProposalResolutionReview
            );
            assert!(model.agents.memory.review_registered);
            assert_eq!(model.agents.memory.detail_scroll, 0);
            cancel_tui_memory_review_once(&runtime.client(), &mut model.agents.memory)
                .expect("cancel resolution review");
            assert_eq!(cancellations.load(Ordering::SeqCst), 1);
            runtime
                .finish_and_join(ShutdownReason::Interrupted)
                .expect("finish resolution runtime");
        }
    }

    #[test]
    fn memory_retry_classifier_matches_only_backpressure_and_transient_persistence() {
        for retryable in [
            RuntimeError::Backpressure,
            RuntimeError::Application(AppError::Persistence(PersistenceError::Contention)),
            RuntimeError::Application(AppError::Persistence(PersistenceError::Capacity)),
            RuntimeError::Application(AppError::Persistence(PersistenceError::QueryFailed)),
        ] {
            assert!(memory_submission_is_retryable(&retryable), "{retryable:?}");
        }
        for terminal in [
            RuntimeError::Application(AppError::MemoryEntryNotFound),
            RuntimeError::Application(AppError::Persistence(
                PersistenceError::ProjectionStateConflict,
            )),
            RuntimeError::Closed,
            RuntimeError::WorkerExited,
            RuntimeError::WorkerPanicked,
        ] {
            assert!(!memory_submission_is_retryable(&terminal), "{terminal:?}");
        }
    }

    #[test]
    fn retryable_submission_error_retains_exact_review_and_advances_both_generations() {
        let profile = refresh_profile(80_400, 0);
        let (runtime, calls, cancellations) = memory_action_runtime([
            Err(AppError::Persistence(PersistenceError::Contention)),
            Ok(CommandView::MemoryEntryMutation(MemoryEntryMutationView {
                entry: memory_entry(
                    &profile,
                    80_420,
                    MemoryEntryDraft::new(
                        "Confirmed key 80410".to_owned(),
                        "Confirmed value 80410".to_owned(),
                        vec!["confirmed".to_owned()],
                    )
                    .expect("committed draft"),
                )
                .reference(),
                expired_proposals: Vec::new(),
            })),
        ]);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        let (command, _) = install_confirmed_set(&mut runner.model, &profile, 80_410);
        runner.model.agents.memory.detail_scroll = 83;
        let initial_generation = runner.model.agents.memory.generation;
        let retained_review = runner.model.agents.memory.edit_review.clone();
        let retained_draft = runner
            .model
            .agents
            .memory
            .editor
            .as_ref()
            .expect("editor")
            .draft()
            .expect("draft");

        runner
            .apply_effect(ControllerEffect::ExecuteMemory(command.clone()))
            .expect("first submission");
        let first_generation = initial_generation + 1;
        assert_eq!(runner.model.agents.memory.generation, first_generation);
        assert_eq!(
            runner
                .model
                .agents
                .memory
                .confirmation
                .as_ref()
                .map(|confirmation| confirmation.generation),
            Some(first_generation),
        );
        assert_eq!(
            runner.pending_memory,
            Some(PendingMemoryRequest {
                intent: MemoryOutcomeIntent::Mutation,
                generation: first_generation,
            })
        );
        assert_eq!(
            poll_memory_completion(&mut runner).expect("retryable result"),
            ControllerEffect::Redraw,
        );
        assert!(runner.model.agents.memory.review_registered);
        assert_eq!(runner.model.agents.memory.edit_review, retained_review);
        assert_eq!(
            runner
                .model
                .agents
                .memory
                .editor
                .as_ref()
                .expect("retained editor")
                .draft()
                .expect("retained draft"),
            retained_draft,
        );
        assert!(runner.model.agents.memory.confirmed_command().is_some());
        assert_eq!(runner.model.agents.memory.detail_scroll, 83);
        assert_eq!(cancellations.load(Ordering::SeqCst), 0);
        assert!(runner.pending.is_none());
        assert!(runner.pending_memory.is_none());
        assert_eq!(runner.model.agents.memory.pending_intent, None);

        runner
            .apply_effect(ControllerEffect::ExecuteMemory(command.clone()))
            .expect("retry submission");
        let retry_generation = first_generation + 1;
        assert_eq!(runner.model.agents.memory.generation, retry_generation);
        assert_eq!(
            runner
                .model
                .agents
                .memory
                .confirmation
                .as_ref()
                .map(|confirmation| confirmation.generation),
            Some(retry_generation),
        );
        assert_eq!(
            poll_memory_completion(&mut runner).expect("successful retry"),
            ControllerEffect::Redraw,
        );
        assert_eq!(calls.lock().unwrap().as_slice(), [command.clone(), command]);
        assert_eq!(cancellations.load(Ordering::SeqCst), 0);
        assert!(!runner.model.agents.memory.review_registered);
        assert!(runner.model.agents.memory.edit_review.is_none());
        assert!(runner.model.agents.memory.confirmation.is_none());
        assert!(runner.model.agents.memory.editor.is_none());
        assert_eq!(runner.model.agents.memory.pane, MemoryPane::Result);
        assert_eq!(runner.model.agents.memory.detail_scroll, 0);
        assert_eq!(runner.deferred_memory_refreshes.len(), 1);
        runner
            .finish(ShutdownReason::Interrupted)
            .expect("finish retry runtime");
    }

    #[test]
    fn nonretryable_application_error_cancels_once_and_preserves_set_draft_for_fresh_preview() {
        let profile = refresh_profile(80_500, 0);
        let (runtime, calls, cancellations) =
            memory_action_runtime([Err(AppError::MemoryEntryNotFound)]);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        let (command, _) = install_confirmed_set(&mut runner.model, &profile, 80_510);
        runner.model.agents.memory.detail_scroll = usize::MAX;
        let retained_draft = runner
            .model
            .agents
            .memory
            .editor
            .as_ref()
            .expect("editor")
            .draft()
            .expect("draft");

        runner
            .apply_effect(ControllerEffect::ExecuteMemory(command.clone()))
            .expect("submission");
        assert_eq!(
            poll_memory_completion(&mut runner).expect("application error recovered"),
            ControllerEffect::Redraw,
        );

        assert_eq!(calls.lock().unwrap().as_slice(), [command]);
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        assert!(!runner.model.agents.memory.review_registered);
        assert!(runner.model.agents.memory.edit_review.is_none());
        assert!(runner.model.agents.memory.resolution_review.is_none());
        assert!(runner.model.agents.memory.confirmation.is_none());
        assert_eq!(runner.model.agents.memory.pane, MemoryPane::Editor);
        assert_eq!(runner.model.agents.memory.detail_scroll, 0);
        assert_eq!(
            runner
                .model
                .agents
                .memory
                .editor
                .as_ref()
                .expect("preserved editor")
                .draft()
                .expect("preserved draft"),
            retained_draft,
        );
        assert!(runner.model.agents.memory.confirmed_command().is_none());
        assert!(runner.pending.is_none());
        assert!(runner.pending_memory.is_none());
        assert_eq!(runner.model.agents.memory.pending_intent, None);
        runner
            .finish(ShutdownReason::Interrupted)
            .expect("finish nonretryable runtime");
    }

    #[test]
    fn memory_confirmation_mismatch_and_generation_overflow_are_nonmutating() {
        let profile = refresh_profile(80_600, 0);
        let (runtime, calls, _) = memory_action_runtime([]);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        let (command, _) = install_confirmed_set(&mut runner.model, &profile, 80_610);

        let before_mismatch = runner.model.agents.memory.clone();
        assert!(matches!(
            runner.queue_memory_command(
                ApplicationCommand::RequestShutdown,
                MemoryOutcomeIntent::Mutation,
            ),
            Err(TuiError::UnexpectedControllerEffect)
        ));
        assert_eq!(runner.model.agents.memory, before_mismatch);
        assert!(runner.pending.is_none());

        runner.model.agents.memory.generation = u64::MAX;
        runner
            .model
            .agents
            .memory
            .confirmation
            .as_mut()
            .expect("confirmation")
            .generation = u64::MAX;
        let before_overflow = runner.model.agents.memory.clone();
        assert!(matches!(
            runner.queue_memory_command(command, MemoryOutcomeIntent::Mutation),
            Err(TuiError::MemoryState(
                crate::domain::DomainError::MemoryGenerationOverflow
            ))
        ));
        assert_eq!(runner.model.agents.memory, before_overflow);
        assert!(runner.pending.is_none());
        assert!(calls.lock().unwrap().is_empty());
        runner
            .finish(ShutdownReason::Interrupted)
            .expect("finish overflow runtime");
    }

    #[test]
    fn admission_backpressure_retains_exact_review_and_synchronized_confirmation() {
        let profile = refresh_profile(80_700, 0);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let cancellations = Arc::new(AtomicUsize::new(0));
        let (started_sender, started) = bounded(1);
        let (release, release_receiver) = bounded(1);
        let runtime = ApplicationRuntime::spawn(
            QueueBlockerExecutor {
                started: Some(started_sender),
                release: release_receiver,
                calls: Arc::clone(&calls),
                cancellations: Arc::clone(&cancellations),
            },
            1,
        )
        .expect("queue blocker runtime");
        let client = runtime.client();
        let first = client
            .try_submit(ApplicationCommand::ShowStatus)
            .expect("blocking command");
        started.recv().expect("worker started");
        let second = client
            .try_submit(ApplicationCommand::ShowStatus)
            .expect("fill command queue");
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        let (command, _) = install_confirmed_set(&mut runner.model, &profile, 80_710);
        let prior_generation = runner.model.agents.memory.generation;
        let retained_review = runner.model.agents.memory.edit_review.clone();

        assert!(matches!(
            runner.apply_effect(ControllerEffect::ExecuteMemory(command.clone())),
            Ok(super::LoopControl::Continue { redraw: true })
        ));
        let attempted_generation = prior_generation + 1;
        assert_eq!(runner.model.agents.memory.generation, attempted_generation);
        assert_eq!(
            runner.model.agents.memory.confirmation.as_ref(),
            Some(&super::super::model::MemoryConfirmation {
                command,
                generation: attempted_generation,
            }),
        );
        assert_eq!(runner.model.agents.memory.edit_review, retained_review);
        assert!(runner.model.agents.memory.review_registered);
        assert_eq!(runner.model.agents.memory.pending_intent, None);
        assert!(runner.pending.is_none());
        assert!(runner.pending_memory.is_none());
        assert!(!runner.model.command_in_flight);
        assert_eq!(cancellations.load(Ordering::SeqCst), 0);

        release.send(()).expect("release queue");
        first.recv().expect("first completion");
        second.recv().expect("second completion");
        runner
            .cancel_active_memory_review()
            .expect("cancel retained review");
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        runner
            .finish(ShutdownReason::Interrupted)
            .expect("finish backpressure runtime");
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                ApplicationCommand::ShowStatus,
                ApplicationCommand::ShowStatus
            ]
        );
    }

    #[test]
    fn resolution_success_requires_action_specific_entry_shape_before_cleanup() {
        let profile = refresh_profile(80_800, 0);
        let proposal = memory_proposal(&profile, 80_810);
        let MemoryProposalOperation::Set { candidate } = proposal.operation() else {
            panic!("set proposal")
        };
        let accepted_entry = memory_entry(&profile, 80_820, candidate.clone()).reference();

        for (name, action, entry) in [
            (
                "approve without entry",
                MemoryResolutionAction::Approve,
                None,
            ),
            (
                "reject with entry",
                MemoryResolutionAction::Reject,
                Some(accepted_entry.clone()),
            ),
        ] {
            let view = proposal_resolution_view(&proposal, action, entry, 80_830);
            let (runtime, calls, cancellations) =
                memory_action_runtime([Ok(CommandView::MemoryProposalResolution(view))]);
            let mut runner = TuiRunner::new(runtime, snapshot(), false);
            let command = install_confirmed_resolution(
                &mut runner.model,
                &profile,
                &proposal,
                action,
                80_840,
            );
            runner.model.agents.memory.detail_scroll = usize::MAX;
            runner
                .apply_effect(ControllerEffect::ExecuteMemory(command.clone()))
                .expect("resolution submission");

            assert!(
                matches!(
                    poll_memory_completion(&mut runner),
                    Err(TuiError::UnexpectedControllerEffect)
                ),
                "{name}"
            );
            assert_eq!(calls.lock().unwrap().as_slice(), [command], "{name}");
            assert!(runner.model.agents.memory.review_registered, "{name}");
            assert!(
                runner.model.agents.memory.resolution_review.is_some(),
                "{name}"
            );
            assert!(runner.model.agents.memory.confirmation.is_some(), "{name}");
            assert_eq!(
                runner.model.agents.memory.pane,
                MemoryPane::Confirmation,
                "{name}"
            );
            assert_eq!(runner.model.agents.memory.pending_intent, None, "{name}");
            assert!(runner.deferred_memory_refreshes.is_empty(), "{name}");
            assert_eq!(
                runner.model.agents.memory.detail_scroll,
                usize::MAX,
                "{name}"
            );
            runner
                .cancel_active_memory_review()
                .expect("cancel malformed result review");
            assert_eq!(cancellations.load(Ordering::SeqCst), 1, "{name}");
            runner
                .finish(ShutdownReason::ApplicationError)
                .expect("finish malformed resolution runtime");
        }
    }

    #[test]
    fn successful_resolution_sets_origin_and_exact_refresh_plan_for_both_actions() {
        let profile = refresh_profile(80_900, 0);
        let proposal = memory_proposal(&profile, 80_910);
        let MemoryProposalOperation::Set { candidate } = proposal.operation() else {
            panic!("set proposal")
        };
        let accepted_entry = memory_entry(&profile, 80_920, candidate.clone()).reference();
        let selector: AgentProfileSelector = profile.profile_id().into();

        for (action, entry, expected_refreshes) in [
            (
                MemoryResolutionAction::Approve,
                Some(accepted_entry.clone()),
                VecDeque::from([
                    ControllerEffect::LoadAgentMemory(selector.clone()),
                    ControllerEffect::LoadMemoryProposals {
                        selector: selector.clone(),
                        filter: MemoryProposalFilter::Pending,
                    },
                ]),
            ),
            (
                MemoryResolutionAction::Reject,
                None,
                VecDeque::from([ControllerEffect::LoadMemoryProposals {
                    selector: selector.clone(),
                    filter: MemoryProposalFilter::Pending,
                }]),
            ),
        ] {
            let view = proposal_resolution_view(&proposal, action, entry, 80_930);
            let (runtime, _, cancellations) =
                memory_action_runtime([Ok(CommandView::MemoryProposalResolution(view))]);
            let mut runner = TuiRunner::new(runtime, snapshot(), false);
            let command = install_confirmed_resolution(
                &mut runner.model,
                &profile,
                &proposal,
                action,
                80_940,
            );
            runner.model.agents.memory.detail_scroll = usize::MAX;
            runner
                .apply_effect(ControllerEffect::ExecuteMemory(command))
                .expect("resolution submission");
            assert_eq!(
                poll_memory_completion(&mut runner).expect("resolution completion"),
                ControllerEffect::Redraw,
            );

            assert_eq!(
                runner.model.agents.memory.result_origin,
                super::super::model::MemoryResultOrigin::Resolution,
                "{action:?}",
            );
            assert_eq!(
                runner.model.agents.memory.pane,
                MemoryPane::Result,
                "{action:?}"
            );
            assert_eq!(runner.model.agents.memory.detail_scroll, 0, "{action:?}");
            assert!(!runner.model.agents.memory.review_registered, "{action:?}");
            assert!(
                runner.model.agents.memory.resolution_review.is_none(),
                "{action:?}"
            );
            assert!(
                runner.model.agents.memory.confirmation.is_none(),
                "{action:?}"
            );
            assert_eq!(
                runner.deferred_memory_refreshes, expected_refreshes,
                "{action:?}"
            );
            assert_eq!(cancellations.load(Ordering::SeqCst), 0, "{action:?}");
            runner
                .finish(ShutdownReason::Interrupted)
                .expect("finish resolution runtime");
        }
    }

    #[test]
    fn direct_set_and_delete_success_cleanup_use_exact_conditional_refresh_plans() {
        let profile = refresh_profile(81_000, 0);
        let selected_entry = memory_entry(
            &profile,
            81_010,
            MemoryEntryDraft::new(
                "Delete plan".to_owned(),
                "Delete plan value".to_owned(),
                Vec::new(),
            )
            .expect("delete entry"),
        );
        let selector: AgentProfileSelector = profile.profile_id().into();

        for has_proposals in [false, true] {
            for delete in [false, true] {
                let (runtime, _, _) = memory_action_runtime([]);
                let mut runner = TuiRunner::new(runtime, snapshot(), false);
                let (command, result) = if delete {
                    install_confirmed_delete(&mut runner.model, &profile, &selected_entry, 81_020)
                } else {
                    install_confirmed_set(&mut runner.model, &profile, 81_030)
                };
                assert_eq!(
                    runner
                        .model
                        .agents
                        .memory
                        .confirmation
                        .as_ref()
                        .map(|confirmation| &confirmation.command),
                    Some(&command),
                );
                if has_proposals {
                    runner.model.agents.memory.proposals = Some(memory_proposals_view(
                        &profile,
                        &[memory_proposal(&profile, 81_040)],
                    ));
                }
                runner.model.agents.memory.detail_scroll = usize::MAX;

                runner
                    .complete_memory_outcome(
                        &MemoryOutcomeIntent::Mutation,
                        &CommandView::MemoryEntryMutation(MemoryEntryMutationView {
                            entry: result.reference(),
                            expired_proposals: Vec::new(),
                        }),
                    )
                    .expect("mutation cleanup");

                let mut expected =
                    VecDeque::from([ControllerEffect::LoadAgentMemory(selector.clone())]);
                if has_proposals {
                    expected.push_back(ControllerEffect::LoadMemoryProposals {
                        selector: selector.clone(),
                        filter: MemoryProposalFilter::Pending,
                    });
                }
                assert_eq!(runner.deferred_memory_refreshes, expected);
                assert_eq!(
                    runner.model.agents.memory.result_origin,
                    super::super::model::MemoryResultOrigin::Mutation,
                );
                assert_eq!(runner.model.agents.memory.pane, MemoryPane::Result);
                assert_eq!(runner.model.agents.memory.detail_scroll, 0);
                assert!(!runner.model.agents.memory.review_registered);
                assert!(runner.model.agents.memory.edit_review.is_none());
                assert!(runner.model.agents.memory.confirmation.is_none());
                assert!(runner.model.agents.memory.editor.is_none());
                runner
                    .finish(ShutdownReason::Interrupted)
                    .expect("finish direct mutation runtime");
            }
        }
    }

    #[test]
    fn post_commit_refreshes_run_fifo_through_the_single_receiver_and_keep_result_pane() {
        let profile = refresh_profile(81_100, 0);
        let existing_entry = memory_entry(
            &profile,
            81_110,
            MemoryEntryDraft::new(
                "Existing selection".to_owned(),
                "Existing value".to_owned(),
                Vec::new(),
            )
            .expect("existing entry"),
        );
        let committed = memory_entry(
            &profile,
            81_140,
            MemoryEntryDraft::new(
                "Confirmed key 81120".to_owned(),
                "Confirmed value 81120".to_owned(),
                vec!["confirmed".to_owned()],
            )
            .expect("committed entry"),
        );
        let selected_proposal = memory_proposal(&profile, 81_150);
        let responses = [
            Ok(CommandView::MemoryEntryMutation(MemoryEntryMutationView {
                entry: committed.reference(),
                expired_proposals: Vec::new(),
            })),
            Ok(CommandView::MemoryEntries(memory_entries_view(
                &profile,
                &[committed.clone(), existing_entry.clone()],
            ))),
            Ok(CommandView::MemoryProposals(memory_proposals_view(
                &profile,
                std::slice::from_ref(&selected_proposal),
            ))),
        ];
        let (runtime, calls, cancellations) = memory_action_runtime(responses);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        let (command, _) = install_confirmed_set(&mut runner.model, &profile, 81_120);
        runner.model.agents.memory.entries = Some(memory_entries_view(
            &profile,
            std::slice::from_ref(&existing_entry),
        ));
        runner.model.agents.memory.proposals = Some(memory_proposals_view(
            &profile,
            std::slice::from_ref(&selected_proposal),
        ));
        runner.model.agents.memory.selected_entry = 0;
        runner.model.agents.memory.selected_proposal = 0;
        runner.model.agents.memory.entry_scroll = 0;
        runner.model.agents.memory.proposal_scroll = 0;

        runner
            .apply_effect(ControllerEffect::ExecuteMemory(command.clone()))
            .expect("mutation submission");
        poll_memory_completion(&mut runner).expect("mutation completion");
        assert_eq!(runner.deferred_memory_refreshes.len(), 2);
        assert!(runner.pending.is_none());
        assert_eq!(runner.model.agents.memory.pane, MemoryPane::Result);

        let entries_effect = runner
            .deferred_memory_refreshes
            .pop_front()
            .expect("entries refresh");
        assert!(matches!(
            entries_effect,
            ControllerEffect::LoadAgentMemory(_)
        ));
        runner
            .apply_effect(entries_effect)
            .expect("queue entries refresh");
        assert!(runner.pending.is_some());
        assert_eq!(runner.deferred_memory_refreshes.len(), 1);
        assert!(matches!(
            runner.pending_memory,
            Some(PendingMemoryRequest {
                intent: MemoryOutcomeIntent::Entries,
                ..
            })
        ));
        poll_memory_completion(&mut runner).expect("entries refresh completion");
        assert!(runner.pending.is_none());
        assert_eq!(runner.model.agents.memory.pane, MemoryPane::Result);
        assert_eq!(
            runner.model.agents.memory.selected_entry_id(),
            Ok(existing_entry.reference().entry_id()),
        );

        let proposals_effect = runner
            .deferred_memory_refreshes
            .pop_front()
            .expect("proposals refresh");
        assert!(matches!(
            proposals_effect,
            ControllerEffect::LoadMemoryProposals { .. }
        ));
        runner
            .apply_effect(proposals_effect)
            .expect("queue proposals refresh");
        assert!(runner.pending.is_some());
        assert!(runner.deferred_memory_refreshes.is_empty());
        assert!(matches!(
            runner.pending_memory,
            Some(PendingMemoryRequest {
                intent: MemoryOutcomeIntent::Proposals,
                ..
            })
        ));
        poll_memory_completion(&mut runner).expect("proposals refresh completion");
        assert!(runner.pending.is_none());
        assert!(runner.pending_memory.is_none());
        assert_eq!(runner.model.agents.memory.pane, MemoryPane::Result);
        assert_eq!(runner.model.agents.memory.selected_entry, 1);
        assert_eq!(runner.model.agents.memory.selected_proposal, 0);
        assert_eq!(
            (
                runner.model.agents.memory.entry_scroll,
                runner.model.agents.memory.proposal_scroll
            ),
            (0, 0)
        );
        assert_eq!(cancellations.load(Ordering::SeqCst), 0);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                command,
                ApplicationCommand::ListMemoryEntries {
                    selector: profile.profile_id().into(),
                },
                ApplicationCommand::ListMemoryProposals {
                    selector: profile.profile_id().into(),
                    filter: MemoryProposalFilter::Pending,
                },
            ],
        );
        runner
            .finish(ShutdownReason::Interrupted)
            .expect("finish FIFO runtime");
    }

    #[test]
    fn cancel_memory_review_unwinds_each_origin_only_after_one_successful_runtime_cancel() {
        let profile = refresh_profile(81_300, 0);
        let delete_entry = memory_entry(
            &profile,
            81_310,
            MemoryEntryDraft::new(
                "Cancel delete".to_owned(),
                "Cancel delete value".to_owned(),
                Vec::new(),
            )
            .expect("delete entry"),
        );
        let proposal = memory_proposal(&profile, 81_320);

        for origin in ["set", "delete", "resolution"] {
            let (runtime, cancellations) = preview_runtime(
                MemoryPreviewResponse::Set(MemoryEditPreview::NoChange(
                    MemoryNoChange::AlreadyAbsent,
                )),
                false,
            );
            let mut runner = TuiRunner::new(runtime, snapshot(), false);
            let retained_draft = match origin {
                "set" => {
                    install_confirmed_set(&mut runner.model, &profile, 81_330);
                    runner.model.agents.memory.pane = MemoryPane::MutationReview;
                    Some(
                        runner
                            .model
                            .agents
                            .memory
                            .editor
                            .as_ref()
                            .expect("set editor")
                            .draft()
                            .expect("set draft"),
                    )
                }
                "delete" => {
                    install_confirmed_delete(&mut runner.model, &profile, &delete_entry, 81_340);
                    runner.model.agents.memory.pane = MemoryPane::MutationReview;
                    None
                }
                "resolution" => {
                    install_confirmed_resolution(
                        &mut runner.model,
                        &profile,
                        &proposal,
                        MemoryResolutionAction::Approve,
                        81_350,
                    );
                    runner.model.agents.memory.pane = MemoryPane::ProposalResolutionReview;
                    None
                }
                _ => unreachable!(),
            };
            runner.model.agents.memory.detail_scroll = usize::MAX;

            assert!(
                matches!(
                    runner.apply_effect(ControllerEffect::CancelMemoryReview),
                    Ok(super::LoopControl::Continue { redraw: true })
                ),
                "{origin}"
            );
            assert_eq!(cancellations.load(Ordering::SeqCst), 1, "{origin}");
            assert!(!runner.model.agents.memory.review_registered, "{origin}");
            assert!(runner.model.agents.memory.edit_review.is_none(), "{origin}");
            assert!(
                runner.model.agents.memory.resolution_review.is_none(),
                "{origin}"
            );
            assert!(
                runner.model.agents.memory.confirmation.is_none(),
                "{origin}"
            );
            assert_eq!(runner.model.agents.memory.detail_scroll, 0, "{origin}");
            match origin {
                "set" => {
                    assert_eq!(runner.model.agents.memory.pane, MemoryPane::Editor);
                    let editor = runner.model.agents.memory.editor.as_ref().expect("editor");
                    assert_eq!(
                        editor.step(),
                        crate::ui::memory_editor::MemoryEditorStep::PurposeTags
                    );
                    assert_eq!(
                        editor.draft().expect("restored draft"),
                        retained_draft.unwrap()
                    );
                    assert_eq!(runner.model.command.text(), "confirmed");
                }
                "delete" => {
                    assert_eq!(runner.model.agents.memory.pane, MemoryPane::EntryDetail);
                    assert!(runner.model.agents.memory.editor.is_none());
                }
                "resolution" => {
                    assert_eq!(runner.model.agents.memory.pane, MemoryPane::ProposalDetail);
                    assert!(runner.model.agents.memory.editor.is_none());
                }
                _ => unreachable!(),
            }

            runner
                .apply_effect(ControllerEffect::CancelMemoryReview)
                .expect("idempotent local cancel");
            assert_eq!(cancellations.load(Ordering::SeqCst), 1, "{origin}");
            runner
                .finish(ShutdownReason::Interrupted)
                .expect("finish cancel runtime");
        }
    }

    #[test]
    fn failed_memory_cancel_clears_only_ownership_and_teardown_does_not_retry() {
        let profile = refresh_profile(81_400, 0);
        let (runtime, cancellations) = preview_runtime(
            MemoryPreviewResponse::Set(MemoryEditPreview::NoChange(MemoryNoChange::AlreadyAbsent)),
            true,
        );
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        install_confirmed_set(&mut runner.model, &profile, 81_410);
        runner.model.agents.memory.pane = MemoryPane::MutationReview;
        runner.model.agents.memory.detail_scroll = usize::MAX;
        runner
            .model
            .command
            .replace_with_memory_value("protected visible input");
        let protected_model = runner.model.clone();

        assert!(matches!(
            runner.apply_effect(ControllerEffect::CancelMemoryReview),
            Err(TuiError::Runtime(RuntimeError::Application(
                AppError::LifecycleFinished
            )))
        ));
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        let mut expected_model = protected_model;
        expected_model.agents.memory.review_registered = false;
        assert_eq!(runner.model, expected_model);

        runner
            .cancel_active_memory_review()
            .expect("teardown sees ownership already consumed");
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        runner
            .finish(ShutdownReason::ApplicationError)
            .expect("finish failed-cancel runtime");
    }

    #[derive(Clone, Copy, Debug)]
    enum MemoryTeardownExit {
        UserQuit,
        Interrupt,
        InputError,
        DrawError,
        RuntimeError,
        Panic,
    }

    #[test]
    fn active_memory_review_is_cancelled_once_on_every_host_exit_path() {
        for exit in [
            MemoryTeardownExit::UserQuit,
            MemoryTeardownExit::Interrupt,
            MemoryTeardownExit::InputError,
            MemoryTeardownExit::DrawError,
            MemoryTeardownExit::RuntimeError,
            MemoryTeardownExit::Panic,
        ] {
            let (runtime, observer) = teardown_runtime(
                matches!(exit, MemoryTeardownExit::RuntimeError),
                false,
                false,
                false,
            );
            let mut runner = TuiRunner::new(runtime, snapshot(), false);
            runner.model.agents.memory.review_registered = true;
            let mut screen = match exit {
                MemoryTeardownExit::DrawError => {
                    RecordingScreen::failing_draw(Rect::new(0, 0, 140, 40))
                }
                MemoryTeardownExit::Panic => {
                    RecordingScreen::panicking_draw(Rect::new(0, 0, 140, 40))
                }
                _ => RecordingScreen::new(Rect::new(0, 0, 140, 40)),
            };
            let steps = match exit {
                MemoryTeardownExit::UserQuit => command_steps("/quit"),
                MemoryTeardownExit::Interrupt => {
                    vec![EventStep::Event(TuiEvent::Interrupt)]
                }
                MemoryTeardownExit::InputError => vec![EventStep::InputError],
                MemoryTeardownExit::RuntimeError => command_steps("/status"),
                MemoryTeardownExit::DrawError | MemoryTeardownExit::Panic => Vec::new(),
            };
            let mut events = FakeEvents::from(steps);

            let result = run_with_screen(
                runner,
                &mut screen,
                &mut events,
                &Theme::from_no_color(true),
            );

            match exit {
                MemoryTeardownExit::UserQuit | MemoryTeardownExit::Interrupt => {
                    assert!(result.is_ok(), "{exit:?}: {result:?}");
                }
                MemoryTeardownExit::InputError => {
                    assert!(matches!(result, Err(TuiError::TerminalInput)), "{exit:?}");
                }
                MemoryTeardownExit::DrawError => {
                    assert!(matches!(result, Err(TuiError::TerminalOutput)), "{exit:?}");
                }
                MemoryTeardownExit::RuntimeError => assert!(matches!(
                    result,
                    Err(TuiError::Runtime(RuntimeError::Application(
                        AppError::LifecycleFinished
                    )))
                )),
                MemoryTeardownExit::Panic => {
                    assert!(matches!(result, Err(TuiError::Panicked)), "{exit:?}");
                }
            }
            assert_eq!(
                observer.memory_cancellations.load(Ordering::SeqCst),
                1,
                "{exit:?}",
            );
            assert_eq!(screen.restore_calls(), 1, "{exit:?}");
            assert_eq!(observer.finishes.lock().unwrap().len(), 1, "{exit:?}");
            assert_eq!(
                observer.finishes.lock().unwrap()[0],
                match exit {
                    MemoryTeardownExit::UserQuit => ShutdownReason::UserQuit,
                    MemoryTeardownExit::Interrupt => ShutdownReason::Interrupted,
                    MemoryTeardownExit::InputError
                    | MemoryTeardownExit::DrawError
                    | MemoryTeardownExit::RuntimeError
                    | MemoryTeardownExit::Panic => ShutdownReason::ApplicationError,
                },
                "{exit:?}",
            );
        }
    }

    #[test]
    fn profile_and_skill_cleanup_failures_cannot_skip_memory_cleanup_restore_or_finish() {
        let (runtime, observer) = teardown_runtime(false, true, true, false);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        runner.model.agents.pending_confirmation = Some(super::super::model::ProfileConfirmation {
            command: ApplicationCommand::RequestShutdown,
        });
        runner.model.skills.review_registered = true;
        runner.model.agents.memory.review_registered = true;
        let mut screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
        let mut events = FakeEvents::from([EventStep::Event(TuiEvent::Interrupt)]);

        let result = run_with_screen(
            runner,
            &mut screen,
            &mut events,
            &Theme::from_no_color(true),
        );

        assert!(matches!(
            result,
            Err(TuiError::Runtime(RuntimeError::Application(
                AppError::LifecycleFinished
            )))
        ));
        assert_eq!(observer.profile_cancellations.load(Ordering::SeqCst), 1);
        assert_eq!(observer.skill_cancellations.load(Ordering::SeqCst), 1);
        assert_eq!(observer.memory_cancellations.load(Ordering::SeqCst), 1);
        assert_eq!(screen.restore_calls(), 1);
        assert_eq!(
            observer.finishes.lock().unwrap().as_slice(),
            [ShutdownReason::Interrupted]
        );
    }

    #[test]
    fn resolution_review_authenticates_cached_proposal_and_selected_action() {
        let profile = refresh_profile(80_100, 0);
        let proposal = memory_proposal(&profile, 80_110);
        let exact_detail = proposal_view(&profile, &proposal);
        let exact_review =
            resolution_review(&profile, &proposal, MemoryResolutionAction::Approve, 80_120);
        for (name, detail, reject_selected, mut review) in [
            ("missing proposal detail", None, false, exact_review.clone()),
            (
                "opposite selected action",
                Some(exact_detail.clone()),
                true,
                exact_review.clone(),
            ),
            (
                "wrong expected current entry",
                Some(exact_detail.clone()),
                false,
                exact_review.clone(),
            ),
        ] {
            if name == "wrong expected current entry" {
                review.expected_entry = ExpectedMemoryEntryState::Deleted(
                    deleted_memory_entry(
                        &profile,
                        80_130,
                        MemoryEntryDraft::new(
                            "Proposal key".to_owned(),
                            "old".to_owned(),
                            Vec::new(),
                        )
                        .expect("old entry"),
                    )
                    .reference(),
                );
            }
            let (runtime, cancellations) =
                preview_runtime(MemoryPreviewResponse::Approve(review), false);
            let mut model = TuiModel::new(snapshot(), false);
            bind_memory(&mut model, &profile);
            model.agents.memory.proposal_detail = detail;
            if reject_selected {
                model.agents.memory.selected_proposal_detail_action =
                    super::super::model::MemoryProposalDetailAction::Reject;
            }
            let generation = model
                .agents
                .memory
                .begin_review_request()
                .expect("resolution generation");

            assert!(
                matches!(
                    install_resolution_preview(
                        &runtime.client(),
                        &mut model,
                        proposal.reference(),
                        MemoryResolutionAction::Approve,
                        generation,
                    ),
                    Err(TuiError::UnexpectedControllerEffect)
                ),
                "{name}",
            );
            assert_eq!(cancellations.load(Ordering::SeqCst), 1, "{name}");
            assert!(!model.agents.memory.review_registered, "{name}");
            runtime
                .finish_and_join(ShutdownReason::Interrupted)
                .expect("finish runtime");
        }
    }

    #[test]
    fn stale_tagged_application_error_does_not_cancel_a_newer_memory_review() {
        let profile = refresh_profile(80_200, 0);
        let candidate = MemoryEntryDraft::new(
            "Newer review".to_owned(),
            "Newer value".to_owned(),
            Vec::new(),
        )
        .expect("candidate");
        let (runtime, cancellations) = preview_runtime(
            MemoryPreviewResponse::Set(MemoryEditPreview::NoChange(MemoryNoChange::AlreadyAbsent)),
            false,
        );
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        bind_memory(&mut runner.model, &profile);
        runner.model.agents.memory.generation = 2;
        runner.model.agents.memory.edit_review = Some(set_review(
            &profile,
            ExpectedMemoryEntryState::Absent,
            candidate.clone(),
            absent_set_diff(&candidate),
            80_210,
        ));
        runner.model.agents.memory.review_registered = true;
        runner.model.agents.memory.pane = MemoryPane::MutationReview;
        let protected_before = runner.model.agents.memory.clone();

        runner
            .recover_memory_pending_error(
                &PendingMemoryRequest {
                    intent: MemoryOutcomeIntent::Mutation,
                    generation: 1,
                },
                &RuntimeError::Application(AppError::MemoryEntryNotFound),
            )
            .expect("stale error ignored");

        assert_eq!(cancellations.load(Ordering::SeqCst), 0);
        assert_eq!(runner.model.agents.memory, protected_before);
        runner
            .finish(ShutdownReason::Interrupted)
            .expect("finish runtime");
    }

    #[test]
    fn malformed_review_cancellation_failure_is_not_swallowed_as_a_preview_error() {
        let profile = refresh_profile(80_300, 0);
        let candidate = MemoryEntryDraft::new(
            "Malformed review".to_owned(),
            "Malformed value".to_owned(),
            Vec::new(),
        )
        .expect("candidate");
        let mut malformed = set_review(
            &profile,
            ExpectedMemoryEntryState::Absent,
            candidate.clone(),
            absent_set_diff(&candidate),
            80_310,
        );
        malformed.operation = MemoryMutationKind::Delete;
        let (runtime, cancellations) = preview_runtime(
            MemoryPreviewResponse::Set(MemoryEditPreview::Review(malformed)),
            true,
        );
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        bind_memory(&mut runner.model, &profile);
        runner
            .model
            .agents
            .memory
            .open_create_editor(profile.profile_id().into())
            .expect("editor");
        let editor = runner.model.agents.memory.editor.as_mut().expect("editor");
        editor
            .submit_line(candidate.display_key().to_owned())
            .expect("key");
        editor
            .submit_line(candidate.value().to_owned())
            .expect("value");
        let crate::ui::memory_editor::MemoryEditorEffect::Preview(request) =
            editor.submit_line(String::new()).expect("tags")
        else {
            panic!("preview request")
        };
        runner
            .model
            .agents
            .memory
            .begin_review_request()
            .expect("outer generation");

        assert!(matches!(
            runner.apply_effect(ControllerEffect::RequestMemorySetPreview(request)),
            Err(TuiError::Runtime(RuntimeError::Application(
                AppError::LifecycleFinished
            )))
        ));
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        assert!(!runner.model.agents.memory.review_registered);
        runner
            .cancel_active_memory_review()
            .expect("teardown sees malformed-review ownership consumed");
        assert_eq!(cancellations.load(Ordering::SeqCst), 1);
        runner
            .finish(ShutdownReason::ApplicationError)
            .expect("finish runtime");
    }

    struct ProfileCreationRefreshExecutor {
        first: AgentProfileVersion,
        created: AgentProfileVersion,
        commands: Arc<Mutex<Vec<&'static str>>>,
    }

    struct NoChangeMemoryPreviewExecutor;

    impl CommandExecutor for NoChangeMemoryPreviewExecutor {
        fn execute_user(
            &mut self,
            command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            Ok(outcome(command))
        }

        fn preview_memory_set(
            &mut self,
            _selector: crate::app::AgentProfileSelector,
            _candidate: MemoryEntryDraft,
        ) -> Result<crate::app::MemoryEditPreview, AppError> {
            Ok(crate::app::MemoryEditPreview::NoChange(
                MemoryNoChange::IdenticalContent,
            ))
        }

        fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
            Ok(())
        }
    }

    impl CommandExecutor for ProfileCreationRefreshExecutor {
        fn execute_user(
            &mut self,
            command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            let (label, view) = match command {
                ApplicationCommand::CreateAgentProfile { .. } => (
                    "create",
                    CommandView::AgentProfileCreated(AgentProfileCreatedView {
                        profile_id: self.created.profile_id(),
                        profile_version_id: self.created.profile_version_id(),
                        version: self.created.version(),
                        readiness: AgentReadiness::Unbound,
                    }),
                ),
                ApplicationCommand::ListAgentProfiles => (
                    "list",
                    CommandView::AgentProfiles(AgentProfilesView {
                        profiles: vec![
                            refresh_profile_summary(&self.first),
                            refresh_profile_summary(&self.created),
                        ],
                        total_count: 2,
                        returned_count: 2,
                        truncated: false,
                    }),
                ),
                ApplicationCommand::ShowAgentProfile { selector }
                    if selector == self.created.profile_id().into() =>
                {
                    (
                        "detail",
                        CommandView::AgentProfile(AgentProfileView {
                            profile: self.created.clone(),
                            readiness: AgentReadiness::Unbound,
                        }),
                    )
                }
                ApplicationCommand::ShowAgentProfileHistory { selector }
                    if selector == self.created.profile_id().into() =>
                {
                    (
                        "history",
                        CommandView::AgentProfileHistory(refresh_profile_history(&self.created)),
                    )
                }
                _ => return Err(AppError::AgentProfileNotFound),
            };
            self.commands.lock().unwrap().push(label);
            Ok(CommandOutcome {
                command_id: CommandId::from_uuid(Uuid::from_u128(80_000)),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(80_001)),
                committed_events: Vec::new(),
                view,
                shutdown: ShutdownDisposition::Continue,
            })
        }

        fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[test]
    fn created_profile_refresh_uses_identity_transition_from_memory_action() {
        let first = refresh_profile(81_000, 0);
        let created = refresh_profile(82_000, 1);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let runtime = ApplicationRuntime::spawn(
            ProfileCreationRefreshExecutor {
                first: first.clone(),
                created: created.clone(),
                commands: commands.clone(),
            },
            4,
        )
        .expect("runtime");
        let mut model = TuiModel::new(snapshot(), false);
        model.select_view(View::Agents);
        model.skills.library_loaded = true;
        model.agents.profiles = AgentProfilesView {
            profiles: vec![refresh_profile_summary(&first)],
            total_count: 1,
            returned_count: 1,
            truncated: false,
        };
        model.agents.detail = Some(AgentProfileView {
            profile: first.clone(),
            readiness: AgentReadiness::Unbound,
        });
        model.agents.pane = AgentsPane::Memory;
        model.agents.selected_detail_action = AgentDetailAction::Memory;
        model
            .agents
            .memory
            .bind_profile(
                MemoryProfileIdentityView {
                    profile: first.reference(),
                    display_name: first.display_name().to_owned(),
                },
                first.memory_namespace_id(),
            )
            .expect("bind first profile memory");

        execute_agent_effect(
            &runtime.client(),
            &mut model,
            ControllerEffect::ExecuteProfile(ApplicationCommand::CreateAgentProfile {
                draft: builtin_profile_templates()[1]
                    .copy_to_draft()
                    .expect("create draft"),
                template_provenance: Some(builtin_profile_templates()[1].provenance()),
            }),
        )
        .expect("create and refresh");

        assert_eq!(
            commands.lock().unwrap().as_slice(),
            ["create", "list", "detail", "history"],
        );
        assert_eq!(model.agents.selected_profile, 1);
        assert_eq!(
            model
                .agents
                .selected_summary()
                .map(|summary| summary.profile_id),
            Some(created.profile_id()),
        );
        assert_eq!(
            model
                .agents
                .detail
                .as_ref()
                .map(|detail| detail.profile.profile_id()),
            Some(created.profile_id()),
        );
        assert_eq!(
            model.agents.selected_detail_action,
            AgentDetailAction::Profile,
        );
        assert!(model.agents.memory.profile.is_none());
        assert_eq!(model.agents.pane, AgentsPane::Detail);
        runtime
            .finish_and_join(ShutdownReason::UserQuit)
            .expect("finish runtime");
    }

    fn key(character: char) -> EventStep {
        EventStep::Event(TuiEvent::Key(KeyEvent::new(
            KeyCode::Char(character),
            KeyModifiers::NONE,
        )))
    }

    fn navigation_key(character: char) -> EventStep {
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
            navigation_key('7'),
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
    fn tab_hydration_is_deferred_while_an_async_command_is_pending() {
        let (runtime, observer, release) = runtime(true, false, false);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        runner.submit(ApplicationCommand::ShowStatus).unwrap();
        for _ in 0..100_000 {
            if observer.commands() == [ApplicationCommand::ShowStatus] {
                break;
            }
            thread::yield_now();
        }
        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);

        let effect = handle_event(
            &mut runner.model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE)),
        );
        assert_eq!(effect, ControllerEffect::LoadSkills);
        assert!(runner.model.skills.active);
        let super::LoopControl::Continue { redraw } = runner.apply_effect(effect).unwrap() else {
            panic!("navigation refresh cannot finish the loop")
        };
        assert!(redraw);

        assert_eq!(
            runner.deferred_navigation_refresh,
            Some(ControllerEffect::LoadSkills)
        );
        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);

        release.unwrap().send(()).unwrap();
        let completed = loop {
            if let Some(effect) = runner.poll_pending().unwrap() {
                break effect;
            }
            thread::yield_now();
        };
        assert!(runner.model.skills.active);
        runner.apply_effect(completed).unwrap();
        let refresh = runner
            .deferred_navigation_refresh
            .take()
            .expect("deferred tab hydration");
        runner.apply_effect(refresh).unwrap();

        assert_eq!(
            observer.commands(),
            [
                ApplicationCommand::ShowStatus,
                ApplicationCommand::ListSkills
            ]
        );
        assert!(runner.model.skills.active);
        runner.finish(ShutdownReason::Interrupted).unwrap();
    }

    #[test]
    fn background_follow_up_hydrates_without_presenting_its_result() {
        let (runtime, observer, _) = runtime(false, false, false);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        runner.model.select_view(View::Setup);

        let super::LoopControl::Continue { redraw } = runner
            .apply_effect(ControllerEffect::SubmitPreservingNavigation(
                ApplicationCommand::ShowStatus,
            ))
            .unwrap()
        else {
            panic!("background follow-up cannot finish the loop")
        };
        assert!(redraw);
        let completed = loop {
            if let Some(effect) = runner.poll_pending().unwrap() {
                break effect;
            }
            thread::yield_now();
        };
        runner.apply_effect(completed).unwrap();

        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);
        assert_eq!(runner.model.active_view, View::Setup);
        assert!(!runner.model.command_in_flight);
        runner.finish(ShutdownReason::Interrupted).unwrap();
    }

    #[test]
    fn restored_confirmation_cannot_block_behind_an_async_command() {
        let (runtime, observer, release) = runtime(true, false, false);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        runner.model.skills.active = true;
        runner.model.skills.library_loaded = true;
        runner.model.skills.pane = SkillsPane::Confirmation;
        runner.model.skills.pending_confirmation = Some(SkillConfirmation {
            command: ApplicationCommand::ShowHelp,
            origin: SkillOperationOrigin::Skills(SkillsPane::Detail),
        });
        runner.model.skills.review_registered = true;
        let expected_confirmation = runner.model.skills.pending_confirmation.clone();
        runner.model.select_view(View::Overview);
        runner.submit(ApplicationCommand::ShowStatus).unwrap();
        for _ in 0..100_000 {
            if observer.commands() == [ApplicationCommand::ShowStatus] {
                break;
            }
            thread::yield_now();
        }
        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);

        assert_eq!(
            handle_event(
                &mut runner.model,
                TuiEvent::Key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE,)),
            ),
            ControllerEffect::Redraw
        );
        assert_eq!(runner.model.skills.pane, SkillsPane::Confirmation);
        let execute = handle_event(
            &mut runner.model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(execute, ControllerEffect::Redraw);
        let super::LoopControl::Continue { redraw } = runner.apply_effect(execute).unwrap() else {
            panic!("a staged confirmation cannot finish the loop")
        };

        assert!(redraw);
        assert_eq!(
            runner.model.skills.pending_confirmation,
            expected_confirmation
        );
        assert!(runner.model.skills.review_registered);
        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);
        assert_eq!(
            runner
                .model
                .message
                .as_ref()
                .map(|message| message.text.as_str()),
            Some("A command is already running.")
        );

        runner.model.clear_message();
        assert_eq!(
            handle_event(
                &mut runner.model,
                TuiEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            ),
            ControllerEffect::Redraw
        );
        assert!(runner.model.skills.active);
        assert_eq!(runner.model.skills.pane, SkillsPane::Confirmation);
        assert_eq!(
            runner.model.skills.pending_confirmation,
            expected_confirmation
        );
        assert!(runner.model.skills.review_registered);

        release.unwrap().send(()).unwrap();
        runner.finish(ShutdownReason::Interrupted).unwrap();
    }

    #[test]
    fn deferred_agent_hydration_is_not_downgraded_by_a_later_skills_switch() {
        let (runtime, observer, release) = runtime(true, false, false);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        runner.submit(ApplicationCommand::ShowStatus).unwrap();
        for _ in 0..100_000 {
            if observer.commands() == [ApplicationCommand::ShowStatus] {
                break;
            }
            thread::yield_now();
        }
        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);

        runner
            .apply_effect(ControllerEffect::LoadAgentProfiles)
            .unwrap();
        runner.apply_effect(ControllerEffect::LoadSkills).unwrap();

        assert_eq!(
            runner.deferred_navigation_refresh,
            Some(ControllerEffect::LoadAgentProfiles)
        );
        release.unwrap().send(()).unwrap();
        runner.finish(ShutdownReason::Interrupted).unwrap();
    }

    #[test]
    fn runner_profile_reads_coalesce_and_never_install_a_late_identity() {
        use super::super::model::{AgentProfileRead, Focus};
        let first = refresh_profile(91_000, 0);
        let second = refresh_profile(92_000, 1);
        let (runtime, calls, _) = memory_action_runtime([
            Ok(CommandView::AgentProfile(AgentProfileView {
                profile: first.clone(),
                readiness: AgentReadiness::Unbound,
            })),
            Ok(CommandView::AgentProfile(AgentProfileView {
                profile: second.clone(),
                readiness: AgentReadiness::Unbound,
            })),
        ]);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        runner.model.select_view(View::Agents);
        runner.model.agents.profiles.profiles = vec![
            refresh_profile_summary(&first),
            refresh_profile_summary(&second),
        ];
        runner.model.set_focus(Focus::List);
        let target = runner.model.agents.profile_target().unwrap();
        runner
            .apply_effect(ControllerEffect::LoadSelectedAgentProfile {
                target,
                read: AgentProfileRead::Detail,
            })
            .unwrap();
        for code in [KeyCode::Down, KeyCode::Up, KeyCode::Down, KeyCode::Down] {
            let effect = handle_event(
                &mut runner.model,
                TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
            );
            runner.apply_effect(effect).unwrap();
        }
        assert_eq!(runner.model.agents.selected_profile, 1);
        poll_memory_completion(&mut runner).unwrap();
        assert!(runner.model.agents.detail.is_none());
        assert_eq!(runner.model.focus, Focus::List);
        let queued = runner
            .deferred_profile_refresh
            .take()
            .expect("latest passive selection");
        assert_eq!(queued.target.profile_id, second.profile_id());
        runner
            .apply_effect(ControllerEffect::LoadSelectedAgentProfile {
                target: queued.target,
                read: queued.read,
            })
            .unwrap();
        poll_memory_completion(&mut runner).unwrap();
        assert_eq!(
            runner.model.agents.matching_detail().unwrap().profile,
            second
        );
        assert_eq!(runner.model.focus, Focus::List);
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                ApplicationCommand::ShowAgentProfile {
                    selector: first.profile_id().into()
                },
                ApplicationCommand::ShowAgentProfile {
                    selector: second.profile_id().into()
                },
            ]
        );
        runner.finish(ShutdownReason::Interrupted).unwrap();
    }

    #[test]
    fn stable_edit_target_cannot_follow_a_mutated_row_index() {
        let first = refresh_profile(93_000, 0);
        let second = refresh_profile(94_000, 1);
        let (runtime, calls, _) = memory_action_runtime([]);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        runner.model.agents.profiles.profiles = vec![
            refresh_profile_summary(&first),
            refresh_profile_summary(&second),
        ];
        let target = runner.model.agents.profile_target().unwrap();
        runner.model.agents.select_profile_index(1);
        runner.model.agents.detail = Some(AgentProfileView {
            profile: second,
            readiness: AgentReadiness::Unbound,
        });
        runner
            .apply_effect(ControllerEffect::StartSelectedProfileEdit { target })
            .unwrap();
        assert!(runner.model.agents.editor.is_none());
        assert!(calls.lock().unwrap().is_empty());
        runner.finish(ShutdownReason::Interrupted).unwrap();
    }

    #[test]
    fn restored_skills_result_cannot_submit_behind_an_async_command() {
        let (runtime, observer, release) = runtime(true, false, false);
        let mut runner = TuiRunner::new(runtime, snapshot(), false);
        runner.model.skills.active = true;
        runner.model.skills.library_loaded = true;
        runner.model.skills.pane = SkillsPane::Result;
        runner.model.skills.pending_active_skill = Some(SkillId::from_uuid(Uuid::from_u128(77)));
        runner.model.select_view(View::Overview);
        runner.submit(ApplicationCommand::ShowStatus).unwrap();
        for _ in 0..100_000 {
            if observer.commands() == [ApplicationCommand::ShowStatus] {
                break;
            }
            thread::yield_now();
        }
        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);

        assert_eq!(
            handle_event(
                &mut runner.model,
                TuiEvent::Key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE,)),
            ),
            ControllerEffect::Redraw
        );
        let submit = handle_event(
            &mut runner.model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(submit, ControllerEffect::Redraw);
        let super::LoopControl::Continue { redraw } = runner.apply_effect(submit).unwrap() else {
            panic!("a staged result cannot finish the loop")
        };

        assert!(redraw);
        assert_eq!(runner.model.skills.pane, SkillsPane::Result);
        assert_eq!(observer.commands(), [ApplicationCommand::ShowStatus]);
        assert_eq!(
            runner
                .model
                .message
                .as_ref()
                .map(|message| message.text.as_str()),
            Some("A command is already running.")
        );

        release.unwrap().send(()).unwrap();
        runner.finish(ShutdownReason::Interrupted).unwrap();
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
