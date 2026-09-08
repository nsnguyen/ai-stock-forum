mod support;

use std::{
    collections::VecDeque,
    io::{self, BufRead, Cursor, Read, Write},
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};

use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole, builtin_profile_templates,
    },
    app::{
        AgentProfileSelector, AgentProfilesView, AppError, ApplicationCommand, CommandOutcome,
        CommandView, DatabaseReadiness, EpisodicSummariesView, EpisodicSummaryListItem,
        EpisodicSummaryView, HelpView, InputRejectedView, InputRejectionCategory,
        MemoryEditPreview, MemoryEntriesView, MemoryEntryHistorySummary, MemoryEntryHistoryView,
        MemoryEntryMutationView, MemoryEntrySummary, MemoryEntryVersionView, MemoryEntryView,
        MemoryProfileIdentityView, MemoryProposalCreatedView, MemoryProposalResolutionReview,
        MemoryProposalResolutionView, MemoryProposalSummary, MemoryProposalView,
        MemoryProposalsView, PresentationSnapshot, ProcessGuardOwnership, ShutdownDisposition,
        ShutdownReason, ShutdownView,
    },
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, DomainError, EpisodicSummaryId,
        EventId, InstallationId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId,
        MemoryProposalId, MemoryReviewToken, ObjectVersion, SessionId, sha256,
    },
    memory::{
        EpisodicQualification, EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState,
        MEMORY_PLAINTEXT_WARNING, MemoryEditReview, MemoryEntryDraft, MemoryEntryVersion,
        MemoryField, MemoryFieldDiff, MemoryFieldValue, MemoryMutationKind, MemoryNoChange,
        MemoryPlaintextAcknowledgement, MemoryProposal, MemoryProposalFilter,
        MemoryProposalOperation, MemoryProposalOperationKind, MemoryProposalResolution,
        MemoryProposalStatus, MemoryResolutionAction,
    },
    persistence::PersistenceError,
    policy::{ApprovalStatus, Capability, PolicyDecision},
    runtime::{ApplicationRuntime, CommandExecutor, PendingOutcome, RuntimeClient, RuntimeError},
    setup::SetupStatus,
    ui::{
        command::{
            FallbackRunner, MemoryWorkflowCommand, ParsedLine, TextRenderer, UiError, parse_line,
        },
        tui::{
            ControllerEffect, TuiEvent, handle_event,
            model::{AgentsPane, Focus, Severity, TuiModel, UiMessage, View},
        },
    },
};
use crossbeam_channel::{Receiver, Sender, bounded};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

#[derive(Default)]
struct WorkflowExecutorState {
    commands: Vec<ApplicationCommand>,
    preview_count: usize,
    cancel_count: usize,
    cancel_results: VecDeque<Result<(), AppError>>,
    mutation_results: VecDeque<Result<CommandOutcome, AppError>>,
}

struct WorkflowExecutor {
    state: Arc<Mutex<WorkflowExecutorState>>,
    entry: Option<MemoryEntryVersion>,
    proposal: Option<MemoryProposalView>,
    edit_preview: Option<Result<MemoryEditPreview, AppError>>,
    resolution_preview: Option<Result<MemoryProposalResolutionReview, AppError>>,
    execution_gate: Option<(
        crossbeam_channel::Sender<()>,
        crossbeam_channel::Receiver<()>,
    )>,
    panic_on_mutation: bool,
}

impl CommandExecutor for WorkflowExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        self.state.lock().unwrap().commands.push(command.clone());
        if self.panic_on_mutation
            && matches!(
                command,
                ApplicationCommand::SetMemoryEntry { .. }
                    | ApplicationCommand::DeleteMemoryEntry { .. }
                    | ApplicationCommand::ApproveMemoryProposal { .. }
                    | ApplicationCommand::RejectMemoryProposal { .. }
            )
        {
            panic!("private worker failure");
        }
        if matches!(command, ApplicationCommand::ShowStatus)
            && let Some((entered, release)) = &self.execution_gate
        {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
        match command {
            ApplicationCommand::RequestShutdown => Ok(workflow_outcome_with_shutdown(
                CommandView::Shutdown(ShutdownView {
                    disposition: ShutdownDisposition::Requested,
                }),
                ShutdownDisposition::Requested,
            )),
            ApplicationCommand::ShowMemoryEntry { .. } => self.entry.clone().map_or_else(
                || Err(AppError::MemoryEntryNotFound),
                |entry| {
                    Ok(workflow_outcome(CommandView::MemoryEntry(
                        MemoryEntryView {
                            profile: profile().reference(),
                            entry,
                        },
                    )))
                },
            ),
            ApplicationCommand::ShowMemoryProposal { .. } => self
                .proposal
                .clone()
                .map(|view| workflow_outcome(CommandView::MemoryProposal(view)))
                .ok_or(AppError::MemoryProposalNotFound),
            _ => self
                .state
                .lock()
                .unwrap()
                .mutation_results
                .pop_front()
                .unwrap_or_else(|| Ok(workflow_outcome(CommandView::Help(HelpView)))),
        }
    }

    fn preview_memory_set(
        &mut self,
        _selector: AgentProfileSelector,
        _candidate: MemoryEntryDraft,
    ) -> Result<MemoryEditPreview, AppError> {
        let result = self
            .edit_preview
            .clone()
            .unwrap_or(Err(AppError::WrongMemoryCommandDispatcher));
        if matches!(&result, Ok(MemoryEditPreview::Review(_))) {
            self.state.lock().unwrap().preview_count += 1;
        }
        result
    }

    fn preview_memory_delete(
        &mut self,
        _selector: AgentProfileSelector,
        _display_key: String,
    ) -> Result<MemoryEditPreview, AppError> {
        let result = self
            .edit_preview
            .clone()
            .unwrap_or(Err(AppError::WrongMemoryCommandDispatcher));
        if matches!(&result, Ok(MemoryEditPreview::Review(_))) {
            self.state.lock().unwrap().preview_count += 1;
        }
        result
    }

    fn preview_memory_proposal_approval(
        &mut self,
        _proposal: ai_stock_forum::memory::MemoryProposalRef,
    ) -> Result<MemoryProposalResolutionReview, AppError> {
        let result = self
            .resolution_preview
            .clone()
            .unwrap_or(Err(AppError::WrongMemoryCommandDispatcher));
        if result.is_ok() {
            self.state.lock().unwrap().preview_count += 1;
        }
        result
    }

    fn preview_memory_proposal_rejection(
        &mut self,
        _proposal: ai_stock_forum::memory::MemoryProposalRef,
    ) -> Result<MemoryProposalResolutionReview, AppError> {
        let result = self
            .resolution_preview
            .clone()
            .unwrap_or(Err(AppError::WrongMemoryCommandDispatcher));
        if result.is_ok() {
            self.state.lock().unwrap().preview_count += 1;
        }
        result
    }

    fn cancel_memory_review(&mut self) -> Result<(), AppError> {
        let mut state = self.state.lock().unwrap();
        state.cancel_count += 1;
        state.cancel_results.pop_front().unwrap_or(Ok(()))
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

fn workflow_outcome(view: CommandView) -> CommandOutcome {
    workflow_outcome_with_shutdown(view, ShutdownDisposition::Continue)
}

fn workflow_outcome_with_shutdown(
    view: CommandView,
    shutdown: ShutdownDisposition,
) -> CommandOutcome {
    CommandOutcome {
        command_id: ai_stock_forum::domain::CommandId::from_uuid(Uuid::from_u128(91_001)),
        correlation_id: ai_stock_forum::domain::CorrelationId::from_uuid(Uuid::from_u128(91_002)),
        committed_events: Vec::new(),
        view,
        shutdown,
    }
}

fn edit_review(
    operation: MemoryMutationKind,
    expected: ExpectedMemoryEntryState,
    candidate: Option<MemoryEntryDraft>,
    digest_seed: &[u8],
) -> MemoryEditReview {
    let profile = profile();
    let diff = match (&operation, &expected, &candidate) {
        (MemoryMutationKind::Set, ExpectedMemoryEntryState::Absent, Some(candidate)) => vec![
            memory_diff(
                MemoryField::DisplayKey,
                MemoryFieldValue::Missing,
                MemoryFieldValue::Text(candidate.display_key().to_owned()),
            ),
            memory_diff(
                MemoryField::State,
                MemoryFieldValue::Missing,
                MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Present),
            ),
            memory_diff(
                MemoryField::Value,
                MemoryFieldValue::Missing,
                MemoryFieldValue::Text(candidate.value().to_owned()),
            ),
            memory_diff(
                MemoryField::PurposeTags,
                MemoryFieldValue::Missing,
                MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
            ),
        ],
        (MemoryMutationKind::Set, ExpectedMemoryEntryState::Present(_), Some(candidate)) => vec![
            memory_diff(
                MemoryField::Value,
                MemoryFieldValue::Text("private thesis\nsecond line".into()),
                MemoryFieldValue::Text(candidate.value().to_owned()),
            ),
            memory_diff(
                MemoryField::PurposeTags,
                MemoryFieldValue::Tags(vec!["Catalyst".into()]),
                MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
            ),
        ],
        (MemoryMutationKind::Set, ExpectedMemoryEntryState::Deleted(_), Some(candidate)) => vec![
            memory_diff(
                MemoryField::State,
                MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Deleted),
                MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Present),
            ),
            memory_diff(
                MemoryField::Value,
                MemoryFieldValue::Missing,
                MemoryFieldValue::Text(candidate.value().to_owned()),
            ),
            memory_diff(
                MemoryField::PurposeTags,
                MemoryFieldValue::Missing,
                MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
            ),
        ],
        (MemoryMutationKind::Delete, _, None) => vec![
            memory_diff(
                MemoryField::State,
                MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Present),
                MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Deleted),
            ),
            memory_diff(
                MemoryField::Value,
                MemoryFieldValue::Text("private thesis\nsecond line".into()),
                MemoryFieldValue::Missing,
            ),
            memory_diff(
                MemoryField::PurposeTags,
                MemoryFieldValue::Tags(vec!["Catalyst".into()]),
                MemoryFieldValue::Missing,
            ),
        ],
        _ => Vec::new(),
    };
    MemoryEditReview {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        expected,
        operation,
        candidate,
        diff,
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(91_003)),
        review_digest: sha256(digest_seed),
    }
}

fn memory_diff(
    field: MemoryField,
    before: MemoryFieldValue,
    after: MemoryFieldValue,
) -> MemoryFieldDiff {
    MemoryFieldDiff {
        field,
        before,
        after,
    }
}

fn proposal_view(proposal: MemoryProposal) -> MemoryProposalView {
    let owner = profile();
    MemoryProposalView {
        proposal,
        status: MemoryProposalStatus::Pending,
        resolution: None,
        current_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: false,
        proposer_identity: MemoryProfileIdentityView {
            profile: owner.reference(),
            display_name: owner.display_name().to_owned(),
        },
        namespace_owner_identity: MemoryProfileIdentityView {
            profile: owner.reference(),
            display_name: owner.display_name().to_owned(),
        },
    }
}

fn resolution_review(
    action: MemoryResolutionAction,
    digest_seed: &[u8],
) -> MemoryProposalResolutionReview {
    let proposer = profile();
    MemoryProposalResolutionReview {
        action,
        proposal: proposal(&proposer),
        approval_id: ApprovalId::from_uuid(Uuid::from_u128(91_004)),
        expected_approval_status: ApprovalStatus::Pending,
        expected_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: false,
        proposer_identity: MemoryProfileIdentityView {
            profile: proposer.reference(),
            display_name: proposer.display_name().to_owned(),
        },
        namespace_owner_identity: MemoryProfileIdentityView {
            profile: proposer.reference(),
            display_name: proposer.display_name().to_owned(),
        },
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(91_005)),
        review_digest: sha256(digest_seed),
    }
}

fn workflow_runtime(
    entry: Option<MemoryEntryVersion>,
    proposal: Option<MemoryProposalView>,
    edit_preview: Option<Result<MemoryEditPreview, AppError>>,
    resolution_preview: Option<Result<MemoryProposalResolutionReview, AppError>>,
) -> (ApplicationRuntime, Arc<Mutex<WorkflowExecutorState>>) {
    let state = Arc::new(Mutex::new(WorkflowExecutorState::default()));
    let runtime = ApplicationRuntime::spawn(
        WorkflowExecutor {
            state: state.clone(),
            entry,
            proposal,
            edit_preview,
            resolution_preview,
            execution_gate: None,
            panic_on_mutation: false,
        },
        8,
    )
    .unwrap();
    (runtime, state)
}

fn mutation_commands(state: &Arc<Mutex<WorkflowExecutorState>>) -> Vec<ApplicationCommand> {
    state
        .lock()
        .unwrap()
        .commands
        .iter()
        .filter(|command| {
            matches!(
                command,
                ApplicationCommand::SetMemoryEntry { .. }
                    | ApplicationCommand::DeleteMemoryEntry { .. }
                    | ApplicationCommand::ApproveMemoryProposal { .. }
                    | ApplicationCommand::RejectMemoryProposal { .. }
            )
        })
        .cloned()
        .collect()
}

fn assert_rejected_edit_preview(
    current: Option<MemoryEntryVersion>,
    review: MemoryEditReview,
    input: Vec<u8>,
) {
    let (runtime, state) = workflow_runtime(
        current,
        None,
        Some(Ok(MemoryEditPreview::Review(review))),
        None,
    );
    let runner = FallbackRunner::new(runtime.client(), false);
    let mut output = Vec::new();
    let error = runner.run(Cursor::new(input), &mut output).unwrap_err();

    assert!(matches!(error, UiError::Panicked));
    assert!(mutation_commands(&state).is_empty());
    assert_eq!(state.lock().unwrap().preview_count, 1);
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains("Memory set review"));
    assert!(!output.contains("Memory delete review"));

    assert_eq!(
        runner
            .run(Cursor::new(b":cancel\n".to_vec()), Vec::new())
            .unwrap(),
        ShutdownReason::InputClosed
    );
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

fn command(input: &[u8]) -> ApplicationCommand {
    match parse_line(input) {
        ParsedLine::Command(command) => command,
        ParsedLine::AgentWorkflow(_) => panic!("expected a direct command"),
        ParsedLine::SkillWorkflow(_) => panic!("expected a direct command"),
        ParsedLine::MemoryWorkflow(_) => panic!("expected a direct command"),
        ParsedLine::Ignored => panic!("expected a command"),
    }
}

fn malformed(input: &[u8]) {
    let ApplicationCommand::RejectInput(rejection) = command(input) else {
        panic!("expected a typed rejection for {input:?}");
    };
    assert_eq!(rejection.category, InputRejectionCategory::Malformed);
    assert_eq!(rejection.safe_token.as_deref(), Some("/memory"));
    let encoded = serde_json::to_string(&rejection).unwrap();
    assert!(!encoded.contains("raw_input"));
}

#[test]
fn fallback_set_opens_a_seeded_editor_for_an_absent_key() {
    let fixture = support::runtime();
    let client = fixture.client();
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().unwrap();
    draft.display_name = "Fallback Memory Agent".to_owned();
    client
        .submit(ApplicationCommand::CreateAgentProfile {
            draft,
            template_provenance: Some(template.provenance()),
        })
        .unwrap();

    let mut output = Vec::new();
    let reason = FallbackRunner::new(client, false)
        .run(
            Cursor::new(b"/memory set \"Fallback Memory Agent\" thesis\n:cancel\n".to_vec()),
            &mut output,
        )
        .unwrap();

    assert_eq!(reason, ai_stock_forum::app::ShutdownReason::InputClosed);
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Memory editor [Value]"));
    assert!(output.contains("Plaintext local memory"));
    fixture.finish_and_join(reason);
}

#[test]
fn fallback_recreates_from_a_tombstone_and_delete_tombstone_is_passive_no_change() {
    let fixture = support::runtime();
    let client = fixture.client();
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().unwrap();
    draft.display_name = "Fallback Tombstone Agent".to_owned();
    client
        .submit(ApplicationCommand::CreateAgentProfile {
            draft,
            template_provenance: Some(template.provenance()),
        })
        .unwrap();
    let candidate = MemoryEntryDraft::new(
        "Thesis".into(),
        "original value".into(),
        vec!["evidence".into()],
    )
    .unwrap();
    let set_review = match client
        .preview_memory_set(
            AgentProfileSelector::Name("Fallback Tombstone Agent".into()),
            candidate,
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected set review, got {other:?}"),
    };
    client
        .submit(ApplicationCommand::SetMemoryEntry {
            profile: set_review.profile,
            expected: set_review.expected,
            candidate: set_review.candidate.unwrap(),
            review_token: set_review.review_token,
            review_digest: set_review.review_digest,
        })
        .unwrap();
    let delete_review = match client
        .preview_memory_delete(
            AgentProfileSelector::Name("Fallback Tombstone Agent".into()),
            "Thesis".into(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected delete review, got {other:?}"),
    };
    let ExpectedMemoryEntryState::Present(expected) = delete_review.expected else {
        panic!("delete must bind a present entry");
    };
    let deleted = client
        .submit(ApplicationCommand::DeleteMemoryEntry {
            profile: delete_review.profile,
            expected,
            review_token: delete_review.review_token,
            review_digest: delete_review.review_digest,
        })
        .unwrap();
    let CommandView::MemoryEntryMutation(deleted) = deleted.view else {
        panic!("expected deleted memory entry view");
    };
    assert_eq!(
        deleted.entry.state(),
        ai_stock_forum::memory::MemoryEntryState::Deleted
    );

    let replacement = MemoryEntryDraft::new(
        "Thesis".into(),
        "restored value".into(),
        vec!["reopened".into(), "verified".into()],
    )
    .unwrap();
    let recreate_review = match client
        .preview_memory_set(
            AgentProfileSelector::Name("Fallback Tombstone Agent".into()),
            replacement.clone(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected recreate review, got {other:?}"),
    };
    assert_eq!(
        recreate_review.expected,
        ExpectedMemoryEntryState::Deleted(deleted.entry.clone())
    );
    assert_eq!(recreate_review.operation, MemoryMutationKind::Set);
    assert_eq!(recreate_review.candidate.as_ref(), Some(&replacement));
    let recreate_phrase = format!("set {}", recreate_review.review_digest);
    client.cancel_memory_review().unwrap();

    let mut output = Vec::new();
    let reason = FallbackRunner::new(client.clone(), false)
        .run(
            Cursor::new(
                format!(
                    "/memory delete \"Fallback Tombstone Agent\" thesis\n/memory set \"Fallback Tombstone Agent\" \"Thesis\"\nrestored value\nreopened, verified\n{recreate_phrase}\n"
                )
                .into_bytes(),
            ),
            &mut output,
        )
        .unwrap();

    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Memory editor [Value]"));
    assert!(output.contains("Memory edit has no effect: AlreadyAbsent."));
    assert!(output.contains("Memory set review"));
    assert!(output.contains(&format!(
        "Expected entry: {} version {} version-id {} state Deleted digest {}",
        deleted.entry.entry_id(),
        deleted.entry.version().get(),
        deleted.entry.entry_version_id(),
        deleted.entry.content_digest(),
    )));
    assert!(output.contains(&format!("Type exactly: {recreate_phrase}")));

    let current = client
        .submit(ApplicationCommand::ShowMemoryEntry {
            selector: AgentProfileSelector::Name("Fallback Tombstone Agent".into()),
            display_key: "Thesis".into(),
        })
        .unwrap();
    let CommandView::MemoryEntry(current) = current.view else {
        panic!("expected current memory entry");
    };
    assert_eq!(
        current.entry.reference().state(),
        ai_stock_forum::memory::MemoryEntryState::Present
    );
    assert_eq!(
        current.entry.reference().entry_id(),
        deleted.entry.entry_id()
    );
    assert_eq!(
        current.entry.predecessor_version_id(),
        Some(deleted.entry.entry_version_id())
    );
    assert_eq!(current.entry.value(), Some("restored value"));
    assert_eq!(
        current.entry.purpose_tags(),
        &["reopened".to_owned(), "verified".to_owned()]
    );
    fixture.finish_and_join(reason);
}

#[test]
fn fallback_set_from_absent_and_current_submits_the_exact_review_owned_command() {
    for current in [None, Some(entry(&profile()))] {
        let (display_key, expected, input_prefix) = match current.as_ref() {
            Some(current) => (
                current.display_key().to_owned(),
                ExpectedMemoryEntryState::Present(current.reference()),
                format!(
                    "/memory set {} \"{}\"\nreplacement value\nfresh\n",
                    profile().profile_id(),
                    current.display_key()
                ),
            ),
            None => (
                "Fresh Key".to_owned(),
                ExpectedMemoryEntryState::Absent,
                format!(
                    "/memory set {} \"Fresh Key\"\nreplacement value\nfresh\n",
                    profile().profile_id()
                ),
            ),
        };
        let candidate = MemoryEntryDraft::new(
            display_key,
            "replacement value".into(),
            vec!["fresh".into()],
        )
        .unwrap();
        let review = edit_review(
            MemoryMutationKind::Set,
            expected,
            Some(candidate.clone()),
            if current.is_some() {
                b"current"
            } else {
                b"absent"
            },
        );
        let exact = format!("set {}", review.review_digest);
        let expected_command = ApplicationCommand::SetMemoryEntry {
            profile: review.profile.clone(),
            expected: review.expected.clone(),
            candidate,
            review_token: review.review_token,
            review_digest: review.review_digest.clone(),
        };
        let (runtime, state) = workflow_runtime(
            current,
            None,
            Some(Ok(MemoryEditPreview::Review(review))),
            None,
        );
        let mut output = Vec::new();
        let reason = FallbackRunner::new(runtime.client(), false)
            .run(
                Cursor::new(format!("{input_prefix}{exact}\n").into_bytes()),
                &mut output,
            )
            .unwrap();

        assert_eq!(mutation_commands(&state), vec![expected_command]);
        assert_eq!(state.lock().unwrap().cancel_count, 0);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Memory editor [Value]"));
        assert!(output.contains("Memory set review"));
        assert!(output.matches(MEMORY_PLAINTEXT_WARNING).count() >= 2);
        runtime.finish_and_join(reason).unwrap();
    }
}

#[test]
fn fallback_no_change_previews_are_passive_and_never_register_or_submit() {
    for (input, preview) in [
        (
            format!(
                "/memory set {} \"Earnings Thesis\"\nprivate thesis\nCatalyst\n",
                profile().profile_id()
            ),
            MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent),
        ),
        (
            format!(
                "/memory delete {} \"Missing Key\"\n",
                profile().profile_id()
            ),
            MemoryEditPreview::NoChange(MemoryNoChange::AlreadyAbsent),
        ),
    ] {
        let current = matches!(
            preview,
            MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent)
        )
        .then(|| entry(&profile()));
        let (runtime, state) = workflow_runtime(current, None, Some(Ok(preview)), None);
        let mut output = Vec::new();
        let reason = FallbackRunner::new(runtime.client(), false)
            .run(Cursor::new(input.into_bytes()), &mut output)
            .unwrap();

        assert!(mutation_commands(&state).is_empty());
        assert_eq!(state.lock().unwrap().cancel_count, 0);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Memory edit has no effect:"));
        runtime.finish_and_join(reason).unwrap();
    }
}

#[test]
fn fallback_delete_review_shows_detail_and_submits_the_exact_bound_command() {
    let current = entry(&profile());
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(current.reference()),
        None,
        b"delete",
    );
    let exact = format!("delete {}", review.review_digest);
    let expected_command = ApplicationCommand::DeleteMemoryEntry {
        profile: review.profile.clone(),
        expected: current.reference(),
        review_token: review.review_token,
        review_digest: review.review_digest.clone(),
    };
    let (runtime, state) = workflow_runtime(
        Some(current.clone()),
        None,
        Some(Ok(MemoryEditPreview::Review(review))),
        None,
    );
    let mut output = Vec::new();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(
            Cursor::new(
                format!(
                    "/memory delete {} \"{}\"\n{exact}\n",
                    profile().profile_id(),
                    current.display_key()
                )
                .into_bytes(),
            ),
            &mut output,
        )
        .unwrap();

    assert_eq!(mutation_commands(&state), vec![expected_command]);
    assert_eq!(state.lock().unwrap().cancel_count, 0);
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Memory delete review"));
    assert!(output.contains(current.reference().normalized_key().as_str()));
    assert!(output.contains(&current.reference().entry_version_id().to_string()));
    let state_diff = output.find("State: Present -> Deleted").unwrap();
    let value_diff = output
        .find("Value: private thesis\\nsecond line -> missing")
        .unwrap();
    let tags_diff = output.find("Purpose tags: Catalyst -> missing").unwrap();
    assert!(state_diff < value_diff && value_diff < tags_diff);
    assert!(output.contains(MEMORY_PLAINTEXT_WARNING));
    runtime.finish_and_join(reason).unwrap();
}

#[test]
fn fallback_delete_rejects_a_cross_wired_set_preview_before_render_or_submission() {
    let candidate = MemoryEntryDraft::new(
        "Fresh Key".into(),
        "replacement value".into(),
        vec!["fresh".into()],
    )
    .unwrap();
    let review = edit_review(
        MemoryMutationKind::Set,
        ExpectedMemoryEntryState::Absent,
        Some(candidate),
        b"delete-received-set",
    );
    let (runtime, state) = workflow_runtime(
        None,
        None,
        Some(Ok(MemoryEditPreview::Review(review))),
        None,
    );
    let mut output = Vec::new();
    let error = FallbackRunner::new(runtime.client(), false)
        .run(
            Cursor::new(
                format!("/memory delete {} \"Fresh Key\"\n", profile().profile_id()).into_bytes(),
            ),
            &mut output,
        )
        .unwrap_err();

    assert!(matches!(error, UiError::Panicked));
    assert!(mutation_commands(&state).is_empty());
    assert_eq!(state.lock().unwrap().preview_count, 1);
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains("Memory set review"));
    assert!(!output.contains("Memory delete review"));
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn fallback_set_rejects_a_cross_wired_delete_preview_before_render_or_submission() {
    let current = entry(&profile());
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(current.reference()),
        None,
        b"set-received-delete",
    );
    let (runtime, state) = workflow_runtime(
        Some(current.clone()),
        None,
        Some(Ok(MemoryEditPreview::Review(review))),
        None,
    );
    let mut output = Vec::new();
    let error = FallbackRunner::new(runtime.client(), false)
        .run(
            Cursor::new(
                format!(
                    "/memory set {} \"{}\"\nreplacement value\nfresh\n",
                    profile().profile_id(),
                    current.display_key(),
                )
                .into_bytes(),
            ),
            &mut output,
        )
        .unwrap_err();

    assert!(matches!(error, UiError::Panicked));
    assert!(mutation_commands(&state).is_empty());
    assert_eq!(state.lock().unwrap().preview_count, 1);
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Memory editor [Value]"));
    assert!(!output.contains("Memory set review"));
    assert!(!output.contains("Memory delete review"));
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn fallback_set_rejects_a_same_operation_candidate_cross_wire() {
    let requested = MemoryEntryDraft::new(
        "Requested Key".into(),
        "requested value".into(),
        vec!["requested".into()],
    )
    .unwrap();
    let substituted = MemoryEntryDraft::new(
        "Different Key".into(),
        "substituted value".into(),
        vec!["substituted".into()],
    )
    .unwrap();
    let review = edit_review(
        MemoryMutationKind::Set,
        ExpectedMemoryEntryState::Absent,
        Some(substituted),
        b"set-candidate-cross-wire",
    );
    assert_rejected_edit_preview(
        None,
        review,
        format!(
            "/memory set {} \"{}\"\n{}\n{}\n",
            profile().profile_id(),
            requested.display_key(),
            requested.value(),
            requested.purpose_tags().join(", "),
        )
        .into_bytes(),
    );
}

#[test]
fn fallback_set_id_selector_rejects_a_same_operation_profile_cross_wire() {
    let requested = MemoryEntryDraft::new(
        "Requested Key".into(),
        "requested value".into(),
        vec!["requested".into()],
    )
    .unwrap();
    let mut review = edit_review(
        MemoryMutationKind::Set,
        ExpectedMemoryEntryState::Absent,
        Some(requested.clone()),
        b"set-profile-cross-wire",
    );
    review.profile = different_profile().reference();
    assert_rejected_edit_preview(
        None,
        review,
        format!(
            "/memory set {} \"{}\"\n{}\n{}\n",
            profile().profile_id(),
            requested.display_key(),
            requested.value(),
            requested.purpose_tags().join(", "),
        )
        .into_bytes(),
    );
}

#[test]
fn fallback_delete_rejects_a_same_operation_normalized_key_cross_wire() {
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"delete-key-cross-wire",
    );
    assert_rejected_edit_preview(
        None,
        review,
        format!(
            "/memory delete {} \"Different Key\"\n",
            profile().profile_id()
        )
        .into_bytes(),
    );
}

#[test]
fn fallback_delete_id_selector_rejects_a_same_operation_profile_cross_wire() {
    let mut review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"delete-profile-cross-wire",
    );
    review.profile = different_profile().reference();
    assert_rejected_edit_preview(
        None,
        review,
        format!(
            "/memory delete {} \"Earnings Thesis\"\n",
            profile().profile_id()
        )
        .into_bytes(),
    );
}

#[test]
fn fallback_delete_rejects_a_diff_without_the_state_transition() {
    let mut review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"delete-missing-state",
    );
    review.diff.remove(0);
    assert_rejected_edit_preview(
        None,
        review,
        format!(
            "/memory delete {} \"Earnings Thesis\"\n",
            profile().profile_id()
        )
        .into_bytes(),
    );
}

#[test]
fn fallback_delete_rejects_a_diff_that_retains_deleted_content() {
    let mut review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"delete-retains-value",
    );
    review.diff[1].after = MemoryFieldValue::Text("retained value".into());
    assert_rejected_edit_preview(
        None,
        review,
        format!(
            "/memory delete {} \"Earnings Thesis\"\n",
            profile().profile_id()
        )
        .into_bytes(),
    );
}

#[test]
fn fallback_absent_set_rejects_an_incomplete_missing_to_candidate_diff() {
    let requested = MemoryEntryDraft::new(
        "Requested Key".into(),
        "requested value".into(),
        vec!["requested".into()],
    )
    .unwrap();
    let mut review = edit_review(
        MemoryMutationKind::Set,
        ExpectedMemoryEntryState::Absent,
        Some(requested.clone()),
        b"set-incomplete-diff",
    );
    review.diff.pop();
    assert_rejected_edit_preview(
        None,
        review,
        format!(
            "/memory set {} \"{}\"\n{}\n{}\n",
            profile().profile_id(),
            requested.display_key(),
            requested.value(),
            requested.purpose_tags().join(", "),
        )
        .into_bytes(),
    );
}

#[test]
fn fallback_rejects_noncanonical_edit_diff_before_confirmation() {
    let current = entry(&profile());
    let mut review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(current.reference()),
        None,
        b"duplicate-diff",
    );
    review.diff.insert(1, review.diff[0].clone());
    let (runtime, state) = workflow_runtime(
        Some(current.clone()),
        None,
        Some(Ok(MemoryEditPreview::Review(review))),
        None,
    );
    let mut output = Vec::new();
    let error = FallbackRunner::new(runtime.client(), false)
        .run(
            Cursor::new(
                format!(
                    "/memory delete {} \"{}\"\n",
                    profile().profile_id(),
                    current.display_key(),
                )
                .into_bytes(),
            ),
            &mut output,
        )
        .unwrap_err();

    assert!(matches!(error, UiError::Panicked));
    assert!(mutation_commands(&state).is_empty());
    assert_eq!(state.lock().unwrap().preview_count, 1);
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    assert!(
        !String::from_utf8(output)
            .unwrap()
            .contains("Memory delete review")
    );
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn fallback_confirmation_is_untrimmed_bounded_action_specific_and_retains_review() {
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"confirmation",
    );
    let exact = format!("delete {}", review.review_digest);
    let (runtime, state) = workflow_runtime(
        None,
        None,
        Some(Ok(MemoryEditPreview::Review(review))),
        None,
    );
    let input = format!(
        "/memory delete {} \"Earnings Thesis\"\nyes\n {exact}\n{exact} \n{}\n/help\n{exact}\n",
        profile().profile_id(),
        "x".repeat(81),
    );
    let mut output = Vec::new();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(input.into_bytes()), &mut output)
        .unwrap();

    assert_eq!(mutation_commands(&state).len(), 1);
    assert_eq!(
        state
            .lock()
            .unwrap()
            .commands
            .iter()
            .filter(|command| matches!(command, ApplicationCommand::ShowHelp))
            .count(),
        0
    );
    assert_eq!(state.lock().unwrap().cancel_count, 0);
    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.matches("Confirmation did not match").count(), 5);
    runtime.finish_and_join(reason).unwrap();
}

#[test]
fn fallback_approve_and_reject_submit_only_the_requested_bound_action() {
    for action in [
        MemoryResolutionAction::Approve,
        MemoryResolutionAction::Reject,
    ] {
        let review = resolution_review(
            action,
            match action {
                MemoryResolutionAction::Approve => b"approve",
                MemoryResolutionAction::Reject => b"reject",
            },
        );
        let verb = match action {
            MemoryResolutionAction::Approve => "approve",
            MemoryResolutionAction::Reject => "reject",
        };
        let opposite = match action {
            MemoryResolutionAction::Approve => "reject",
            MemoryResolutionAction::Reject => "approve",
        };
        let proposal_id = review.proposal.reference().proposal_id();
        let exact = format!("{verb} {}", review.review_digest);
        let wrong_action = format!("{opposite} {}", review.review_digest);
        let view = proposal_view(review.proposal.clone());
        let (runtime, state) = workflow_runtime(None, Some(view), None, Some(Ok(review.clone())));
        let input = format!("/memory {verb} {proposal_id}\n{wrong_action}\n{exact}\n");
        let mut output = Vec::new();
        let reason = FallbackRunner::new(runtime.client(), false)
            .run(Cursor::new(input.into_bytes()), &mut output)
            .unwrap();

        let commands = mutation_commands(&state);
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            (&action, &commands[0]),
            (
                MemoryResolutionAction::Approve,
                ApplicationCommand::ApproveMemoryProposal { .. }
            ) | (
                MemoryResolutionAction::Reject,
                ApplicationCommand::RejectMemoryProposal { .. }
            )
        ));
        assert_eq!(state.lock().unwrap().cancel_count, 0);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(&format!("Memory proposal {verb} review")));
        assert!(output.contains(MEMORY_PLAINTEXT_WARNING));
        assert_eq!(output.matches("Confirmation did not match").count(), 1);
        runtime.finish_and_join(reason).unwrap();
    }
}

#[test]
fn fallback_rejects_a_cross_wired_resolution_preview_without_submitting() {
    let review = resolution_review(MemoryResolutionAction::Reject, b"cross-wired");
    let proposal_id = review.proposal.reference().proposal_id();
    let view = proposal_view(review.proposal.clone());
    let (runtime, state) = workflow_runtime(None, Some(view), None, Some(Ok(review.clone())));
    let input = format!(
        "/memory approve {proposal_id}\nreject {}\n",
        review.review_digest
    );
    let mut output = Vec::new();
    let error = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(input.into_bytes()), &mut output)
        .unwrap_err();

    assert!(mutation_commands(&state).is_empty());
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    assert!(matches!(
        error,
        ai_stock_forum::ui::command::UiError::Panicked
    ));
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn fallback_resolution_review_renders_full_bound_provenance_and_sensitive_detail_safely() {
    let mut review = resolution_review(MemoryResolutionAction::Approve, b"full-resolution");
    review.proposer_is_historical = true;
    review.proposer_identity.display_name = "\u{1b}[31mHistorical Proposer".into();
    let text = render_memory_resolution_review(&review);
    let proposal = &review.proposal;

    assert!(text.contains(&proposal.reference().proposal_id().to_string()));
    assert!(text.contains(proposal.reference().content_digest().as_str()));
    assert!(text.contains(&review.approval_id.to_string()));
    assert!(text.contains("Pending"));
    assert!(text.contains(&proposal.namespace_id().to_string()));
    assert!(text.contains(proposal.display_key()));
    assert!(text.contains(proposal.normalized_key().as_str()));
    assert!(text.contains("Operation: Set"));
    assert!(text.contains("private proposed value\\nsecond line"));
    assert!(text.contains("private rationale\\nsecond line"));
    assert!(
        text.contains(
            &review
                .proposer_identity
                .profile
                .profile_version_id()
                .to_string()
        )
    );
    assert!(text.contains(review.proposer_identity.profile.content_digest().as_str()));
    assert!(
        text.contains(
            &review
                .namespace_owner_identity
                .profile
                .profile_version_id()
                .to_string()
        )
    );
    assert!(text.contains("historical profile version"));
    assert!(text.contains("\\u{1b}[31mHistorical Proposer"));
    assert!(!text.contains('\u{1b}'));
    assert!(text.contains(MEMORY_PLAINTEXT_WARNING));
    assert!(text.len() < 25_000);
}

fn render_memory_resolution_review(review: &MemoryProposalResolutionReview) -> String {
    let mut bytes = Vec::new();
    TextRenderer::render_memory_resolution_review(review, &mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn memory_edit_renderer_omits_noncanonical_or_oversized_public_diffs() {
    const SENTINEL: &str = "ADVERSARIAL_DIFF_MUST_NOT_RENDER";
    let base = MemoryFieldDiff {
        field: MemoryField::State,
        before: MemoryFieldValue::Text(SENTINEL.into()),
        after: MemoryFieldValue::Missing,
    };
    let cases = [
        vec![base.clone(); 128],
        vec![base.clone(), base.clone()],
        vec![
            MemoryFieldDiff {
                field: MemoryField::PurposeTags,
                before: MemoryFieldValue::Text(SENTINEL.into()),
                after: MemoryFieldValue::Missing,
            },
            base,
        ],
    ];

    for diff in cases {
        let mut review = edit_review(
            MemoryMutationKind::Delete,
            ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
            None,
            b"adversarial-render",
        );
        review.diff = diff;
        let mut bytes = Vec::new();
        TextRenderer::render_memory_edit_review(&review, &mut bytes).unwrap();
        let text = String::from_utf8(bytes).unwrap();

        assert!(text.contains("Review diff omitted:"));
        assert!(!text.contains(SENTINEL));
        assert!(text.len() < 25_000);
    }
}

#[test]
fn every_memory_action_confirmation_repeats_the_plaintext_warning_and_exact_phrase() {
    for action in ["set", "delete", "approve", "reject"] {
        let expected = format!("{action} {}", sha256(action.as_bytes()));
        let mut bytes = Vec::new();
        TextRenderer::render_memory_confirmation(&expected, &mut bytes).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(
            text,
            format!("{MEMORY_PLAINTEXT_WARNING}\nType exactly: {expected}\n")
        );
    }
}

#[test]
fn fallback_set_seed_read_precedes_and_is_the_only_activity_before_draft_review() {
    let (runtime, state) = workflow_runtime(Some(entry(&profile())), None, None, None);
    let mut output = Vec::new();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(
            Cursor::new(
                format!(
                    "/memory set {} \"Earnings Thesis\"\n",
                    profile().profile_id()
                )
                .into_bytes(),
            ),
            &mut output,
        )
        .unwrap();

    let state = state.lock().unwrap();
    assert_eq!(state.commands.len(), 1);
    assert!(matches!(
        state.commands[0],
        ApplicationCommand::ShowMemoryEntry { .. }
    ));
    assert_eq!(state.cancel_count, 0);
    drop(state);
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Memory editor [Value]")
    );
    runtime.finish_and_join(reason).unwrap();
}

#[test]
fn fallback_invalid_tombstone_seed_is_content_free_and_does_not_install_a_workflow() {
    let tombstone = entry(&profile())
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(204)),
            Actor::Human,
            1_700_000_000_011,
            None,
            EventId::from_uuid(Uuid::from_u128(205)),
        )
        .unwrap();
    let (runtime, state) = workflow_runtime(Some(tombstone.clone()), None, None, None);
    let mut output = Vec::new();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(
            Cursor::new(
                format!(
                    "/memory set {} \"Requested Key\"\n:cancel\n/help\n",
                    profile().profile_id()
                )
                .into_bytes(),
            ),
            &mut output,
        )
        .unwrap();

    assert_eq!(reason, ShutdownReason::InputClosed);
    let state = state.lock().unwrap();
    assert_eq!(state.preview_count, 0);
    assert_eq!(state.cancel_count, 0);
    assert!(state.commands.iter().all(|command| !matches!(
        command,
        ApplicationCommand::SetMemoryEntry { .. }
            | ApplicationCommand::DeleteMemoryEntry { .. }
            | ApplicationCommand::ApproveMemoryProposal { .. }
            | ApplicationCommand::RejectMemoryProposal { .. }
    )));
    assert_eq!(
        state
            .commands
            .iter()
            .filter(|command| matches!(command, ApplicationCommand::ShowMemoryEntry { .. }))
            .count(),
        1
    );
    assert_eq!(
        state
            .commands
            .iter()
            .filter(|command| matches!(command, ApplicationCommand::ShowHelp))
            .count(),
        1
    );
    drop(state);
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Memory editor input was rejected [invalid_memory_editor_seed]."));
    assert!(output.contains("Available commands"));
    assert!(!output.contains(tombstone.display_key()));
    assert!(!output.contains(&tombstone.reference().entry_version_id().to_string()));
    assert!(!output.contains(tombstone.reference().content_digest().as_str()));
    runtime.finish_and_join(reason).unwrap();
}

fn delete_script(review: &MemoryEditReview, confirmations: usize) -> Vec<u8> {
    let exact = format!("delete {}", review.review_digest);
    let key = match &review.expected {
        ExpectedMemoryEntryState::Present(entry) | ExpectedMemoryEntryState::Deleted(entry) => {
            entry.normalized_key().as_str()
        }
        ExpectedMemoryEntryState::Absent => "thesis",
    };
    format!(
        "/memory delete {} \"{key}\"\n{}",
        profile().profile_id(),
        format!("{exact}\n").repeat(confirmations),
    )
    .into_bytes()
}

#[test]
fn recoverable_memory_confirmation_failures_retain_one_review_and_exact_retry() {
    for persistence in [
        PersistenceError::Contention,
        PersistenceError::Capacity,
        PersistenceError::QueryFailed,
    ] {
        let review = edit_review(
            MemoryMutationKind::Delete,
            ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
            None,
            format!("recoverable-{persistence:?}").as_bytes(),
        );
        let exact = format!("delete {}", review.review_digest);
        let (runtime, state) = workflow_runtime(
            None,
            None,
            Some(Ok(MemoryEditPreview::Review(review.clone()))),
            None,
        );
        state.lock().unwrap().mutation_results.extend([
            Err(AppError::Persistence(persistence)),
            Ok(workflow_outcome(CommandView::Help(HelpView))),
        ]);
        let mut output = Vec::new();
        let reason = FallbackRunner::new(runtime.client(), false)
            .run(Cursor::new(delete_script(&review, 2)), &mut output)
            .unwrap();

        let state = state.lock().unwrap();
        let commands = state
            .commands
            .iter()
            .filter(|command| matches!(command, ApplicationCommand::DeleteMemoryEntry { .. }))
            .collect::<Vec<_>>();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0], commands[1]);
        assert_eq!(state.preview_count, 1);
        assert_eq!(state.cancel_count, 0);
        drop(state);
        let output = String::from_utf8(output).unwrap();
        assert!(output.matches(&exact).count() >= 2);
        assert!(output.matches(MEMORY_PLAINTEXT_WARNING).count() >= 2);
        runtime.finish_and_join(reason).unwrap();
    }
}

struct QueueSaturatingWriter {
    output: Vec<u8>,
    client: RuntimeClient,
    entered: Receiver<()>,
    release: Sender<()>,
    pending: Vec<PendingOutcome>,
    saturated: bool,
    released: bool,
}

impl QueueSaturatingWriter {
    fn text(&self) -> String {
        String::from_utf8(self.output.clone()).unwrap()
    }
}

impl Write for QueueSaturatingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.extend_from_slice(bytes);
        let text = String::from_utf8_lossy(&self.output);
        if !self.saturated && text.contains("Type exactly: delete ") {
            let first = self
                .client
                .try_submit(ApplicationCommand::ShowStatus)
                .unwrap();
            self.entered.recv().unwrap();
            let second = self
                .client
                .try_submit(ApplicationCommand::ShowHelp)
                .unwrap();
            self.pending.extend([first, second]);
            self.saturated = true;
        }
        if self.saturated && !self.released && text.contains("Command queue is busy; try again.") {
            self.release.send(()).unwrap();
            for pending in self.pending.drain(..) {
                pending.recv().unwrap();
            }
            self.released = true;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn enqueue_backpressure_retains_the_registered_review_for_exact_retry() {
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"enqueue-backpressure",
    );
    let state = Arc::new(Mutex::new(WorkflowExecutorState::default()));
    let (entered_tx, entered_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    let runtime = ApplicationRuntime::spawn(
        WorkflowExecutor {
            state: state.clone(),
            entry: None,
            proposal: None,
            edit_preview: Some(Ok(MemoryEditPreview::Review(review.clone()))),
            resolution_preview: None,
            execution_gate: Some((entered_tx, release_rx)),
            panic_on_mutation: false,
        },
        1,
    )
    .unwrap();
    let mut writer = QueueSaturatingWriter {
        output: Vec::new(),
        client: runtime.client(),
        entered: entered_rx,
        release: release_tx,
        pending: Vec::new(),
        saturated: false,
        released: false,
    };
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(delete_script(&review, 2)), &mut writer)
        .unwrap();

    let state = state.lock().unwrap();
    assert_eq!(
        mutation_commands(&Arc::new(Mutex::new(WorkflowExecutorState {
            commands: state.commands.clone(),
            ..WorkflowExecutorState::default()
        })))
        .len(),
        1
    );
    assert_eq!(state.preview_count, 1);
    assert_eq!(state.cancel_count, 0);
    drop(state);
    assert!(writer.text().contains("Command queue is busy; try again."));
    assert!(writer.text().contains(MEMORY_PLAINTEXT_WARNING));
    runtime.finish_and_join(reason).unwrap();
}

#[test]
fn terminal_memory_confirmation_failures_cancel_once_and_cannot_retry() {
    let failures = [
        AppError::Domain(DomainError::MemoryExpectedStateMismatch),
        AppError::Domain(DomainError::MemoryReviewUnavailable),
        AppError::CommandConflict,
        AppError::CapabilityDenied {
            capability: Capability::MemoryMutate,
            decision: PolicyDecision::Denied,
        },
        AppError::Persistence(PersistenceError::MemoryRowMismatch),
    ];
    for failure in failures {
        let review = edit_review(
            MemoryMutationKind::Delete,
            ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
            None,
            format!("terminal-{}", failure.code()).as_bytes(),
        );
        let (runtime, state) = workflow_runtime(
            None,
            None,
            Some(Ok(MemoryEditPreview::Review(review.clone()))),
            None,
        );
        state
            .lock()
            .unwrap()
            .mutation_results
            .push_back(Err(failure));
        let mut output = Vec::new();
        let reason = FallbackRunner::new(runtime.client(), false)
            .run(Cursor::new(delete_script(&review, 2)), &mut output)
            .unwrap();

        let state = state.lock().unwrap();
        assert_eq!(
            state
                .commands
                .iter()
                .filter(|command| matches!(command, ApplicationCommand::DeleteMemoryEntry { .. }))
                .count(),
            1
        );
        assert_eq!(state.preview_count, 1);
        assert_eq!(state.cancel_count, 1);
        drop(state);
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("Start a fresh /memory command")
        );
        runtime.finish_and_join(reason).unwrap();
    }
}

#[test]
fn non_application_worker_failure_clears_confirmation_and_requires_a_fresh_review() {
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"worker-failure",
    );
    let state = Arc::new(Mutex::new(WorkflowExecutorState::default()));
    let runtime = ApplicationRuntime::spawn(
        WorkflowExecutor {
            state: state.clone(),
            entry: None,
            proposal: None,
            edit_preview: Some(Ok(MemoryEditPreview::Review(review.clone()))),
            resolution_preview: None,
            execution_gate: None,
            panic_on_mutation: true,
        },
        4,
    )
    .unwrap();
    let mut output = Vec::new();
    let error = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(delete_script(&review, 1)), &mut output)
        .unwrap_err();

    assert!(matches!(
        error,
        UiError::Runtime(RuntimeError::WorkerPanicked)
    ));
    assert_eq!(mutation_commands(&state).len(), 1);
    assert_eq!(state.lock().unwrap().preview_count, 1);
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("Application worker stopped unexpectedly."));
    assert!(output.contains("Start a fresh /memory command"));
    assert!(matches!(
        runtime.finish_and_join(ShutdownReason::ApplicationError),
        Err(RuntimeError::WorkerPanicked)
    ));
}

#[test]
fn cancellation_failure_does_not_mask_the_primary_terminal_memory_error() {
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"cancel-precedence",
    );
    let (runtime, state) = workflow_runtime(
        None,
        None,
        Some(Ok(MemoryEditPreview::Review(review.clone()))),
        None,
    );
    {
        let mut state = state.lock().unwrap();
        state.mutation_results.push_back(Err(AppError::Domain(
            DomainError::MemoryExpectedStateMismatch,
        )));
        state
            .cancel_results
            .push_back(Err(AppError::Persistence(PersistenceError::QueryFailed)));
    }
    let mut output = Vec::new();
    let error = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(delete_script(&review, 1)), &mut output)
        .unwrap_err();

    assert!(matches!(
        error,
        UiError::Runtime(RuntimeError::Application(AppError::Domain(
            DomainError::MemoryExpectedStateMismatch
        )))
    ));
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    assert!(
        String::from_utf8(output)
            .unwrap()
            .contains("Agent profile operation could not be completed.")
    );
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

struct OneLineThenError {
    line: Vec<u8>,
    position: usize,
}

impl Read for OneLineThenError {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl BufRead for OneLineThenError {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.position == self.line.len() {
            Err(io::Error::other("private input failure"))
        } else {
            Ok(&self.line[self.position..])
        }
    }

    fn consume(&mut self, amount: usize) {
        self.position = self.position.saturating_add(amount).min(self.line.len());
    }
}

struct FailOnTextWriter {
    output: Vec<u8>,
    needle: &'static str,
    panic: bool,
}

impl Write for FailOnTextWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut combined = self.output.clone();
        combined.extend_from_slice(bytes);
        if String::from_utf8_lossy(&combined).contains(self.needle) {
            if self.panic {
                panic!("private writer panic");
            }
            return Err(io::Error::other("private writer failure"));
        }
        self.output.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn delete_review_runtime(
    digest_seed: &[u8],
) -> (
    ApplicationRuntime,
    Arc<Mutex<WorkflowExecutorState>>,
    MemoryEditReview,
) {
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        digest_seed,
    );
    let (runtime, state) = workflow_runtime(
        None,
        None,
        Some(Ok(MemoryEditPreview::Review(review.clone()))),
        None,
    );
    (runtime, state, review)
}

#[test]
fn registered_memory_review_is_cancelled_once_on_cancel_quit_eof_and_input_error() {
    for suffix in [":cancel\n", "/quit\n", ""] {
        let (runtime, state, review) = delete_review_runtime(suffix.as_bytes());
        let mut input = delete_script(&review, 0);
        input.extend_from_slice(suffix.as_bytes());
        let mut output = Vec::new();
        let reason = FallbackRunner::new(runtime.client(), false)
            .run(Cursor::new(input), &mut output)
            .unwrap();

        assert_eq!(state.lock().unwrap().preview_count, 1);
        assert_eq!(state.lock().unwrap().cancel_count, 1);
        assert!(mutation_commands(&state).is_empty());
        let expected_reason = if suffix == "/quit\n" {
            ShutdownReason::UserQuit
        } else {
            ShutdownReason::InputClosed
        };
        assert_eq!(reason, expected_reason);
        runtime.finish_and_join(reason).unwrap();
    }

    let (runtime, state, review) = delete_review_runtime(b"input-error");
    state
        .lock()
        .unwrap()
        .cancel_results
        .push_back(Err(AppError::Persistence(PersistenceError::QueryFailed)));
    let reader = OneLineThenError {
        line: delete_script(&review, 0),
        position: 0,
    };
    let error = FallbackRunner::new(runtime.client(), false)
        .run(reader, Vec::new())
        .unwrap_err();
    assert!(matches!(error, UiError::Read));
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn review_render_error_cancels_once_and_success_output_error_never_cancels() {
    let (runtime, state, review) = delete_review_runtime(b"review-write");
    let mut writer = FailOnTextWriter {
        output: Vec::new(),
        needle: "Memory delete review",
        panic: false,
    };
    let error = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(delete_script(&review, 0)), &mut writer)
        .unwrap_err();
    assert!(matches!(error, UiError::Write));
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();

    let (runtime, state, review) = delete_review_runtime(b"commit-write");
    let mut writer = FailOnTextWriter {
        output: Vec::new(),
        needle: "Available commands",
        panic: false,
    };
    let error = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(delete_script(&review, 1)), &mut writer)
        .unwrap_err();
    assert!(matches!(error, UiError::Write));
    assert_eq!(mutation_commands(&state).len(), 1);
    assert_eq!(state.lock().unwrap().cancel_count, 0);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn invariant_failure_after_workflow_take_still_cancels_the_registered_review_once() {
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Absent,
        None,
        b"invalid-delete-expected",
    );
    let (runtime, state) = workflow_runtime(
        None,
        None,
        Some(Ok(MemoryEditPreview::Review(review.clone()))),
        None,
    );
    let error = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(delete_script(&review, 1)), Vec::new())
        .unwrap_err();

    assert!(matches!(error, UiError::Panicked));
    assert!(mutation_commands(&state).is_empty());
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn caught_review_writer_panic_cancels_the_registered_review_once() {
    let (runtime, state, review) = delete_review_runtime(b"review-panic");
    let runner = FallbackRunner::new(runtime.client(), false);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut writer = FailOnTextWriter {
            output: Vec::new(),
            needle: "Memory delete review",
            panic: true,
        };
        let _ = runner.run(Cursor::new(delete_script(&review, 0)), &mut writer);
    }));

    assert!(result.is_err());
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

struct BackpressureConfirmationPanicWriter(QueueSaturatingWriter);

impl Write for BackpressureConfirmationPanicWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.0.write(bytes)?;
        if self.0.released && self.0.text().matches("Type exactly: delete ").count() >= 2 {
            panic!("private retained-confirmation writer panic");
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct BackpressureConfirmationErrorWriter(QueueSaturatingWriter);

impl Write for BackpressureConfirmationErrorWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.0.write(bytes)?;
        if self.0.released && self.0.text().matches("Type exactly: delete ").count() >= 2 {
            return Err(io::Error::other("private retained-confirmation failure"));
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn backpressure_runtime(
    review: &MemoryEditReview,
) -> (
    ApplicationRuntime,
    Arc<Mutex<WorkflowExecutorState>>,
    Receiver<()>,
    Sender<()>,
) {
    let state = Arc::new(Mutex::new(WorkflowExecutorState::default()));
    let (entered_tx, entered_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    let runtime = ApplicationRuntime::spawn(
        WorkflowExecutor {
            state: state.clone(),
            entry: None,
            proposal: None,
            edit_preview: Some(Ok(MemoryEditPreview::Review(review.clone()))),
            resolution_preview: None,
            execution_gate: Some((entered_tx, release_rx)),
            panic_on_mutation: false,
        },
        1,
    )
    .unwrap();
    (runtime, state, entered_rx, release_tx)
}

fn queue_saturating_writer(
    runtime: &ApplicationRuntime,
    entered: Receiver<()>,
    release: Sender<()>,
) -> QueueSaturatingWriter {
    QueueSaturatingWriter {
        output: Vec::new(),
        client: runtime.client(),
        entered,
        release,
        pending: Vec::new(),
        saturated: false,
        released: false,
    }
}

#[test]
fn retained_confirmation_output_error_after_backpressure_cancels_once() {
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"backpressure-write",
    );
    let (runtime, state, entered, release) = backpressure_runtime(&review);
    let mut writer =
        BackpressureConfirmationErrorWriter(queue_saturating_writer(&runtime, entered, release));
    let error = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(delete_script(&review, 1)), &mut writer)
        .unwrap_err();

    assert!(matches!(error, UiError::Write));
    assert_eq!(state.lock().unwrap().preview_count, 1);
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn caught_retained_confirmation_panic_after_backpressure_cancels_once() {
    let review = edit_review(
        MemoryMutationKind::Delete,
        ExpectedMemoryEntryState::Present(entry(&profile()).reference()),
        None,
        b"backpressure-panic",
    );
    let (runtime, state, entered, release) = backpressure_runtime(&review);
    let runner = FallbackRunner::new(runtime.client(), false);
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut writer = BackpressureConfirmationPanicWriter(queue_saturating_writer(
            &runtime, entered, release,
        ));
        let _ = runner.run(Cursor::new(delete_script(&review, 1)), &mut writer);
    }));

    assert!(result.is_err());
    assert_eq!(state.lock().unwrap().preview_count, 1);
    assert_eq!(state.lock().unwrap().cancel_count, 1);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn parser_maps_all_documented_memory_read_routes_exactly() {
    let proposal_id = MemoryProposalId::from_uuid(Uuid::from_u128(41));
    let summary_id = EpisodicSummaryId::from_uuid(Uuid::from_u128(42));

    assert_eq!(
        command(b"/memory list analyst"),
        ApplicationCommand::ListMemoryEntries {
            selector: AgentProfileSelector::Name("analyst".into()),
        }
    );
    assert_eq!(
        command(b"/memory get \"Research Agent\" \"Risk Appetite\""),
        ApplicationCommand::ShowMemoryEntry {
            selector: AgentProfileSelector::Name("Research Agent".into()),
            display_key: "Risk Appetite".into(),
        }
    );
    assert_eq!(
        command(b"/memory history analyst thesis"),
        ApplicationCommand::ShowMemoryEntryHistory {
            selector: AgentProfileSelector::Name("analyst".into()),
            display_key: "thesis".into(),
        }
    );
    assert_eq!(
        command(b"/memory history analyst thesis 7"),
        ApplicationCommand::ShowMemoryEntryVersion {
            selector: AgentProfileSelector::Name("analyst".into()),
            display_key: "thesis".into(),
            version: ObjectVersion::new(7).unwrap(),
        }
    );
    assert_eq!(
        command(b"/memory proposals analyst"),
        ApplicationCommand::ListMemoryProposals {
            selector: AgentProfileSelector::Name("analyst".into()),
            filter: MemoryProposalFilter::Pending,
        }
    );
    assert_eq!(
        command(b"/memory proposals analyst pending"),
        ApplicationCommand::ListMemoryProposals {
            selector: AgentProfileSelector::Name("analyst".into()),
            filter: MemoryProposalFilter::Pending,
        }
    );
    assert_eq!(
        command(b"/memory proposals analyst all"),
        ApplicationCommand::ListMemoryProposals {
            selector: AgentProfileSelector::Name("analyst".into()),
            filter: MemoryProposalFilter::All,
        }
    );
    assert_eq!(
        command(format!("/memory proposal {proposal_id}").as_bytes()),
        ApplicationCommand::ShowMemoryProposal { proposal_id }
    );
    assert_eq!(
        command(b"/memory episodes analyst"),
        ApplicationCommand::ListEpisodicSummaries {
            selector: AgentProfileSelector::Name("analyst".into()),
        }
    );
    assert_eq!(
        command(format!("/memory episode {summary_id}").as_bytes()),
        ApplicationCommand::ShowEpisodicSummary { summary_id }
    );
}

#[test]
fn parser_maps_memory_writes_only_to_typed_workflow_intents() {
    let proposal_id = MemoryProposalId::from_uuid(Uuid::from_u128(51));

    assert_eq!(
        parse_line(b"/memory set \"Research Agent\" \"Risk Appetite\""),
        ParsedLine::MemoryWorkflow(MemoryWorkflowCommand::Set {
            agent: AgentProfileSelector::Name("Research Agent".into()),
            key: "Risk Appetite".into(),
        })
    );
    assert_eq!(
        parse_line(b"/memory delete analyst thesis"),
        ParsedLine::MemoryWorkflow(MemoryWorkflowCommand::Delete {
            agent: AgentProfileSelector::Name("analyst".into()),
            key: "thesis".into(),
        })
    );
    assert_eq!(
        parse_line(format!("/memory approve {proposal_id}").as_bytes()),
        ParsedLine::MemoryWorkflow(MemoryWorkflowCommand::Approve { proposal_id })
    );
    assert_eq!(
        parse_line(format!("/memory reject {proposal_id}").as_bytes()),
        ParsedLine::MemoryWorkflow(MemoryWorkflowCommand::Reject { proposal_id })
    );
}

#[test]
fn parser_rejects_every_unsupported_or_ambiguous_memory_shape() {
    for input in [
        b"/memory".as_slice(),
        b"/memory list",
        b"/memory list analyst extra",
        b"/memory get analyst",
        b"/memory get analyst password",
        b"/memory get analyst thesis extra",
        b"/memory history analyst",
        b"/memory history analyst \"api key\"",
        b"/memory history analyst thesis 0",
        b"/memory history analyst thesis nope",
        b"/memory history analyst thesis 18446744073709551616",
        b"/memory history analyst thesis 1 extra",
        b"/memory set analyst",
        b"/memory set analyst password",
        b"/memory set analyst thesis extra",
        b"/memory delete analyst",
        b"/memory delete analyst \"api key\"",
        b"/memory proposals",
        b"/memory proposals analyst accepted",
        b"/memory proposals analyst all extra",
        b"/memory proposal not-a-uuid",
        b"/memory approve not-a-uuid",
        b"/memory reject not-a-uuid",
        b"/memory episodes",
        b"/memory episodes analyst extra",
        b"/memory episode not-a-uuid",
        b"/memory propose analyst thesis",
        b"/memory summary create analyst",
        b"/memory snapshot analyst",
        b"/memory get \"unterminated",
        b"/memory unknown hunter2",
    ] {
        malformed(input);
    }
}

#[test]
fn bare_memory_remains_unknown_and_memory_rejections_retain_no_arguments() {
    let ApplicationCommand::RejectInput(bare) = command(b"memory list analyst") else {
        panic!("bare memory must be rejected");
    };
    assert_eq!(bare.category, InputRejectionCategory::Unknown);

    let ApplicationCommand::RejectInput(rejection) = command(b"/memory unknown hunter2") else {
        panic!("unsupported memory syntax must be rejected");
    };
    assert_eq!(rejection.category, InputRejectionCategory::Malformed);
    assert_eq!(rejection.safe_token.as_deref(), Some("/memory"));
    let encoded = serde_json::to_string(&rejection).unwrap();
    assert!(!encoded.contains("hunter2"));
    assert!(!encoded.contains("raw_input"));
}

fn profile() -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(101)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(102)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(103)),
        1_700_000_000_000,
        AgentProfileDraft::new(
            "Memory Agent".into(),
            "Fallback renderer fixture.".into(),
            AgentRole::Custom,
            "research".into(),
            vec!["analysis".into()],
            "Careful.".into(),
            "Use evidence.".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn different_profile() -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(201)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(202)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(203)),
        1_700_000_000_000,
        AgentProfileDraft::new(
            "Different Memory Agent".into(),
            "Adversarial fallback fixture.".into(),
            AgentRole::Custom,
            "research".into(),
            vec!["analysis".into()],
            "Careful.".into(),
            "Use evidence.".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn current_profile(historical: &AgentProfileVersion) -> AgentProfileVersion {
    AgentProfileVersion::next_version(
        historical,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(120)),
        1_700_000_000_001,
        AgentProfileDraft::new(
            "Memory Agent Current".into(),
            "Current fallback renderer fixture.".into(),
            AgentRole::Custom,
            "research".into(),
            vec!["analysis".into()],
            "Careful.".into(),
            "Use current evidence.".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
    )
    .unwrap()
}

fn entry(profile: &AgentProfileVersion) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(104)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(105)),
        MemoryEntryDraft::new(
            "Earnings Thesis".into(),
            "private thesis\nsecond line".into(),
            vec!["Catalyst".into()],
        )
        .unwrap(),
        Actor::Human,
        1_700_000_000_010,
        None,
        EventId::from_uuid(Uuid::from_u128(106)),
    )
    .unwrap()
}

fn proposal(profile: &AgentProfileVersion) -> MemoryProposal {
    MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(107)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                "Earnings Thesis".into(),
                "private proposed value\nsecond line".into(),
                vec!["Catalyst".into()],
            )
            .unwrap(),
        },
        "Earnings Thesis".into(),
        ExpectedMemoryEntryState::Absent,
        "private rationale\nsecond line".into(),
        1_700_000_000_020,
        EventId::from_uuid(Uuid::from_u128(108)),
        ApprovalId::from_uuid(Uuid::from_u128(109)),
    )
    .unwrap()
}

fn episodic_summary(profile: &AgentProfileVersion) -> EpisodicSummary {
    EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::from_u128(110)),
        profile,
        "Quarterly synthesis".into(),
        "private summary body\nverify each source".into(),
        vec!["Catalyst".into()],
        vec![
            EpisodicSourceRef::new(
                17,
                EventId::from_uuid(Uuid::from_u128(111)),
                "memory_entry_set".into(),
                sha256(b"episodic source"),
            )
            .unwrap(),
        ],
        1_700_000_000_030,
        18,
        EventId::from_uuid(Uuid::from_u128(112)),
    )
    .unwrap()
}

fn render(view: CommandView) -> String {
    let mut bytes = Vec::new();
    TextRenderer::render_view(&view, &mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn tui_model() -> TuiModel {
    TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(201)),
            session_id: SessionId::from_uuid(Uuid::from_u128(202)),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: vec![],
            agent_profiles: AgentProfilesView {
                profiles: vec![],
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

fn assert_exact_memory_usage(text: &str) {
    for route in [
        "/memory list <agent>",
        "/memory get <agent> <key>",
        "/memory history <agent> <key> [positive-version]",
        "/memory set <agent> <key>",
        "/memory delete <agent> <key>",
        "/memory proposals <agent> [pending|all]",
        "/memory proposal <proposal-id>",
        "/memory approve <proposal-id>",
        "/memory reject <proposal-id>",
        "/memory episodes <agent>",
        "/memory episode <summary-id>",
    ] {
        assert_exact_line(text, &format!("  {route}"));
    }
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("  /memory "))
            .count(),
        11
    );
    for unsupported in [
        "  /memory propose ",
        "  /memory summary ",
        "  /memory snapshot ",
    ] {
        assert!(!text.contains(unsupported));
    }
    assert!(!text.lines().any(|line| line == "  /memory"));
}

fn assert_exact_line(text: &str, expected: &str) {
    assert!(
        text.lines().any(|line| line == expected),
        "missing exact line `{expected}` in:\n{text}"
    );
}

#[test]
fn memory_list_renderers_cap_rows_escape_terminal_text_and_omit_detail_prose() {
    let profile = profile();
    let entry = entry(&profile);
    let proposal = proposal(&profile);
    let summary = episodic_summary(&profile);

    let mut entries = (0..100)
        .map(|index| MemoryEntrySummary {
            entry: entry.reference(),
            display_key: if index == 0 {
                "\u{1b}[31mHostile Key".into()
            } else {
                format!("Visible Key {index}")
            },
            purpose_tags: vec!["tag\nline".into()],
            value_bytes: 26,
            created_at_ms: entry.created_at_ms(),
        })
        .collect::<Vec<_>>();
    entries.push(MemoryEntrySummary {
        entry: entry.reference(),
        display_key: "ENTRY_SENTINEL_MUST_NOT_RENDER".into(),
        purpose_tags: vec![],
        value_bytes: 26,
        created_at_ms: entry.created_at_ms(),
    });
    let entries_text = render(CommandView::MemoryEntries(MemoryEntriesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        entries,
        total_count: 101,
        returned_count: 101,
        omitted_count: 0,
    }));
    assert!(entries_text.contains("returned 100 of 101 (1 omitted)"));
    assert!(entries_text.contains("\\u{1b}[31mHostile Key"));
    assert!(entries_text.contains("tag\\nline"));
    assert!(entries_text.contains("1 entries omitted"));
    assert!(!entries_text.contains("ENTRY_SENTINEL_MUST_NOT_RENDER"));
    assert!(!entries_text.contains("private thesis"));
    assert!(!entries_text.contains('\u{1b}'));

    let mut history = (0..100)
        .map(|index| MemoryEntryHistorySummary {
            entry: entry.reference(),
            display_key: format!("History Key {index}"),
            created_at_ms: entry.created_at_ms(),
            accepted_proposal: None,
        })
        .collect::<Vec<_>>();
    history.push(MemoryEntryHistorySummary {
        entry: entry.reference(),
        display_key: "HISTORY_SENTINEL_MUST_NOT_RENDER".into(),
        created_at_ms: entry.created_at_ms(),
        accepted_proposal: None,
    });
    let history_text = render(CommandView::MemoryEntryHistory(MemoryEntryHistoryView {
        profile: profile.reference(),
        current: entry.reference(),
        versions: history,
        total_count: 101,
        returned_count: 101,
        omitted_count: 0,
    }));
    assert!(history_text.contains("returned 100 of 101 (1 omitted)"));
    assert!(history_text.contains("1 versions omitted"));
    assert!(!history_text.contains("HISTORY_SENTINEL_MUST_NOT_RENDER"));
    assert!(!history_text.contains("private thesis"));

    let mut proposals = (0..100)
        .map(|index| MemoryProposalSummary {
            proposal: proposal.reference(),
            proposer: profile.reference(),
            operation: MemoryProposalOperationKind::Set,
            display_key: format!("Proposal Key {index}"),
            status: MemoryProposalStatus::Pending,
            created_at_ms: proposal.created_at_ms(),
        })
        .collect::<Vec<_>>();
    proposals.push(MemoryProposalSummary {
        proposal: proposal.reference(),
        proposer: profile.reference(),
        operation: MemoryProposalOperationKind::Set,
        display_key: "PROPOSAL_SENTINEL_MUST_NOT_RENDER".into(),
        status: MemoryProposalStatus::Pending,
        created_at_ms: proposal.created_at_ms(),
    });
    let proposals_text = render(CommandView::MemoryProposals(MemoryProposalsView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        filter: MemoryProposalFilter::Pending,
        proposals,
        total_count: 101,
        returned_count: 101,
        omitted_count: 0,
    }));
    assert!(proposals_text.contains("returned 100 of 101 (1 omitted)"));
    assert!(proposals_text.contains("1 proposals omitted"));
    assert!(!proposals_text.contains("PROPOSAL_SENTINEL_MUST_NOT_RENDER"));
    assert!(!proposals_text.contains("private rationale"));
    assert!(!proposals_text.contains("private proposed value"));

    let mut summaries = (0..100)
        .map(|index| EpisodicSummaryListItem {
            summary: summary.reference(),
            label: format!("Summary {index}"),
            purpose_tags: vec!["Catalyst".into()],
            source_count: 1,
            created_at_ms: summary.created_at_ms(),
        })
        .collect::<Vec<_>>();
    summaries.push(EpisodicSummaryListItem {
        summary: summary.reference(),
        label: "SUMMARY_SENTINEL_MUST_NOT_RENDER".into(),
        purpose_tags: vec![],
        source_count: 1,
        created_at_ms: summary.created_at_ms(),
    });
    let summaries_text = render(CommandView::EpisodicSummaries(EpisodicSummariesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        summaries,
        total_count: 101,
        returned_count: 101,
        omitted_count: 0,
    }));
    assert!(summaries_text.contains("returned 100 of 101 (1 omitted)"));
    assert!(summaries_text.contains("1 summaries omitted"));
    assert_eq!(
        summaries_text.matches("Summary — verify sources").count(),
        100,
        "each visible episodic list item must carry the verification qualification"
    );
    assert!(!summaries_text.contains("SUMMARY_SENTINEL_MUST_NOT_RENDER"));
    assert!(!summaries_text.contains("private summary body"));
    assert!(!summaries_text.contains("memory_entry_set"));
}

#[test]
fn memory_entry_details_render_deliberate_escaped_plaintext() {
    let historical_profile = profile();
    let entry = entry(&historical_profile);

    for view in [
        CommandView::MemoryEntry(MemoryEntryView {
            profile: historical_profile.reference(),
            entry: entry.clone(),
        }),
        CommandView::MemoryEntryVersion(MemoryEntryVersionView {
            profile: historical_profile.reference(),
            entry: entry.clone(),
        }),
    ] {
        let text = render(view);
        assert!(text.contains("Earnings Thesis"));
        assert!(text.contains("private thesis\\nsecond line"));
        assert!(text.contains("Catalyst"));
        assert!(text.contains(entry.reference().content_digest().as_str()));
        assert!(!text.contains("private thesis\nsecond line"));
    }
}

#[test]
fn proposal_detail_renders_canonical_key_and_complete_historical_profile_provenance() {
    let historical_profile = profile();
    let current_profile = current_profile(&historical_profile);
    let proposal = proposal(&historical_profile);
    let proposal_text = render(CommandView::MemoryProposal(MemoryProposalView {
        proposal: proposal.clone(),
        status: MemoryProposalStatus::Pending,
        resolution: None,
        current_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: true,
        proposer_identity: MemoryProfileIdentityView {
            profile: historical_profile.reference(),
            display_name: "\u{1b}[31mHistorical Proposer".into(),
        },
        namespace_owner_identity: MemoryProfileIdentityView {
            profile: current_profile.reference(),
            display_name: "Memory Owner".into(),
        },
    }));
    assert_exact_line(&proposal_text, "Normalized key: earnings thesis");
    assert_exact_line(
        &proposal_text,
        &format!(
            "Proposer profile: {}@{}",
            historical_profile.profile_id(),
            historical_profile.version().get()
        ),
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Proposer profile version ID: {}",
            historical_profile.profile_version_id()
        ),
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Proposer profile digest: {}",
            historical_profile.content_digest()
        ),
    );
    assert_exact_line(
        &proposal_text,
        "Proposer warning: historical profile version",
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Namespace owner profile: {}@{}",
            current_profile.profile_id(),
            current_profile.version().get()
        ),
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Namespace owner profile version ID: {}",
            current_profile.profile_version_id()
        ),
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Namespace owner profile digest: {}",
            current_profile.content_digest()
        ),
    );
    assert_ne!(
        historical_profile.profile_version_id(),
        current_profile.profile_version_id()
    );
    assert_ne!(
        historical_profile.content_digest(),
        current_profile.content_digest()
    );
    assert!(proposal_text.contains("private proposed value\\nsecond line"));
    assert!(proposal_text.contains("private rationale\\nsecond line"));
    assert!(proposal_text.contains("historical"));
    assert!(proposal_text.contains("\\u{1b}[31mHistorical Proposer"));
    assert!(!proposal_text.contains('\u{1b}'));
}

#[test]
fn episodic_detail_renders_pinned_profile_creation_and_source_provenance() {
    let historical_profile = profile();
    let summary = episodic_summary(&historical_profile);
    let summary_text = render(CommandView::EpisodicSummary(EpisodicSummaryView {
        summary: summary.clone(),
        qualification: EpisodicQualification::SummaryVerifySources,
    }));
    assert!(summary_text.contains("Summary — verify sources"));
    assert!(summary_text.contains("Quarterly synthesis"));
    assert!(summary_text.contains("private summary body\\nverify each source"));
    assert_exact_line(
        &summary_text,
        &format!(
            "Profile: {}@{}",
            historical_profile.profile_id(),
            historical_profile.version().get()
        ),
    );
    assert_exact_line(
        &summary_text,
        &format!(
            "Profile version ID: {}",
            historical_profile.profile_version_id()
        ),
    );
    assert_exact_line(
        &summary_text,
        &format!("Profile digest: {}", historical_profile.content_digest()),
    );
    assert_exact_line(
        &summary_text,
        &format!(
            "Creation event sequence: {}",
            summary.creation_event_sequence()
        ),
    );
    assert_exact_line(
        &summary_text,
        &format!("Creation event ID: {}", summary.creation_event_id()),
    );
    assert_exact_line(
        &summary_text,
        &format!("Source set digest: {}", summary.source_set_digest()),
    );
    assert_exact_line(
        &summary_text,
        &format!(
            "  sequence {} event {} type {} digest {}",
            summary.sources()[0].sequence(),
            summary.sources()[0].event_id(),
            summary.sources()[0].event_type(),
            summary.sources()[0].event_digest()
        ),
    );
    assert!(!summary_text.contains("private summary body\nverify each source"));
}

#[test]
fn memory_mutation_renderers_show_identifiers_versions_statuses_and_digests() {
    let profile = profile();
    let entry = entry(&profile);
    let proposal = proposal(&profile);
    let resolution = MemoryProposalResolution::new(
        proposal.reference(),
        MemoryProposalStatus::Accepted,
        proposal.approval_id(),
        Actor::Human,
        1_700_000_000_040,
        EventId::from_uuid(Uuid::from_u128(113)),
    )
    .unwrap();

    let mutation = render(CommandView::MemoryEntryMutation(MemoryEntryMutationView {
        entry: entry.reference(),
        expired_proposals: vec![proposal.reference()],
    }));
    assert!(mutation.contains(&entry.reference().entry_version_id().to_string()));
    assert!(mutation.contains(entry.reference().content_digest().as_str()));
    assert!(mutation.contains(proposal.reference().content_digest().as_str()));

    let created = render(CommandView::MemoryProposalCreated(
        MemoryProposalCreatedView {
            proposal: proposal.reference(),
            approval_id: proposal.approval_id(),
            status: MemoryProposalStatus::Pending,
        },
    ));
    assert!(created.contains(&proposal.reference().proposal_id().to_string()));
    assert!(created.contains(proposal.reference().content_digest().as_str()));
    assert!(created.contains("Pending"));

    let resolved = render(CommandView::MemoryProposalResolution(
        MemoryProposalResolutionView {
            resolution,
            entry: Some(entry.reference()),
            expired_proposals: vec![proposal.reference()],
        },
    ));
    assert!(resolved.contains(proposal.reference().content_digest().as_str()));
    assert!(resolved.contains(entry.reference().content_digest().as_str()));
    assert!(resolved.contains("Accepted"));
}

#[test]
fn help_lists_only_supported_memory_routes_and_no_internal_route() {
    let text = render(CommandView::Help(HelpView));

    for route in [
        "/memory list <agent>",
        "/memory get <agent> <key>",
        "/memory history <agent> <key> [version]",
        "/memory set <agent> <key>",
        "/memory delete <agent> <key>",
        "/memory proposals <agent> [pending|all]",
        "/memory proposal <proposal-id>",
        "/memory approve <proposal-id>",
        "/memory reject <proposal-id>",
        "/memory episodes <agent>",
        "/memory episode <summary-id>",
    ] {
        assert!(text.contains(route), "missing {route}");
    }
    assert!(!text.contains("/memory propose"));
    assert!(!text.contains("/memory snapshot"));
    assert!(!text.contains("/memory summary"));
    assert!(text.contains("internal producers are unavailable"));
}

#[test]
fn malformed_memory_rejection_renders_only_static_actionable_usage() {
    let ApplicationCommand::RejectInput(rejection) =
        command(b"/memory unknown \x1b[31mcredential=top-secret-password")
    else {
        panic!("expected malformed memory rejection");
    };
    let input_digest = rejection.input_digest.to_string();

    let text = render(CommandView::InputRejected(InputRejectedView { rejection }));

    assert!(text.starts_with("Input rejected: malformed command.\nUsage:\n"));
    assert_exact_memory_usage(&text);
    assert!(!text.contains("credential"));
    assert!(!text.contains("top-secret-password"));
    assert!(!text.contains(&input_digest));
    assert!(!text.contains('\x1b'));
}

#[test]
fn tui_malformed_memory_is_consumed_without_history_and_its_usage_is_visible() {
    let mut model = tui_model();
    model.set_focus(Focus::Command);
    model
        .command
        .ingest("/memory unknown credential=top-secret-password");

    let effect = handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );

    let ControllerEffect::Submit(ApplicationCommand::RejectInput(rejection)) = effect else {
        panic!("malformed memory must submit its typed rejection");
    };
    assert_eq!(model.command.text(), "");
    assert_eq!(model.command.cursor_byte(), 0);
    assert_eq!(model.command.history_len(), 0);
    assert!(model.command_in_flight);
    let text = render(CommandView::InputRejected(InputRejectedView { rejection }));
    assert_exact_memory_usage(&text);
    assert!(!text.contains("top-secret-password"));
}

#[test]
fn tui_direct_memory_read_is_consumed_submitted_and_remembered() {
    let mut model = tui_model();
    model.set_focus(Focus::Command);
    model.command.remember("/status".into());
    model.command.ingest("/memory list analyst");

    let effect = handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );

    assert_eq!(
        effect,
        ControllerEffect::Submit(ApplicationCommand::ListMemoryEntries {
            selector: AgentProfileSelector::Name("analyst".into()),
        })
    );
    assert_eq!(model.command.text(), "");
    assert_eq!(model.command.cursor_byte(), 0);
    assert_eq!(model.command.history_len(), 2);
    assert_eq!(model.command.history_back(), Some("/memory list analyst"));
    assert!(model.command_in_flight);
}

#[test]
fn tui_memory_workflow_only_shows_guidance_without_navigation_or_history_mutation() {
    let mut model = tui_model();
    model.active_view = View::Help;
    model.set_focus(Focus::Command);
    model.workspace_body_width = 80;
    model.workspace_body_height = 1;
    model.agents.pane = AgentsPane::History;
    model.agents.selected_profile = 7;
    model.agents.selected_history_version = 9;
    model.agents.list_scroll = 5;
    model.agents.detail_scroll = 6;
    model.agents.history_scroll = 8;
    model.workspace_scroll = 11;
    model.command.remember("/help".into());
    model.command.ingest("/memory delete analyst thesis");
    model.command.move_left();
    let mut expected = model.clone();
    expected.message = Some(UiMessage {
        severity: Severity::Info,
        text: "Open Agents → Memory to edit or resolve memory.".into(),
    });

    let effect = handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );

    assert_eq!(effect, ControllerEffect::Redraw);
    assert_eq!(model, expected);
}
