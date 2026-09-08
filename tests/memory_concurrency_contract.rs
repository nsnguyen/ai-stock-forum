mod support;

use std::{
    sync::{Arc, Barrier},
    thread,
};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, AppError, ApplicationCommand, AuthorizationDecision, CommandEnvelope,
        CommandTransactionHook, CommandView, MemoryEditPreview,
    },
    domain::{Actor, CommandId, CorrelationId},
    memory::{
        ExpectedMemoryEntryState, MemoryEditReview, MemoryEntryDraft, MemoryProposalOperation,
        MemoryProposalRef, MemoryResolutionAction,
    },
    persistence::PersistenceError,
};
use uuid::Uuid;

fn envelope(id: u128, actor: Actor, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 100_000)),
        actor,
        command,
    }
}

fn create_profile(app: &mut support::TestApp, id: u128) -> AgentProfileVersion {
    app.execute(envelope(
        id,
        Actor::Human,
        ApplicationCommand::CreateAgentProfile {
            draft: AgentProfileDraft::new(
                format!("Memory race {id}"),
                "Concurrent memory fixture.".into(),
                AgentRole::Custom,
                "research".into(),
                vec![],
                "Careful.".into(),
                "Use evidence.".into(),
                AgentBindings::default(),
                vec![],
                vec![],
            )
            .unwrap(),
            template_provenance: None,
        },
    ))
    .unwrap();
    app.projection()
        .agent_profiles
        .active_profiles()
        .into_iter()
        .find(|profile| profile.display_name() == format!("Memory race {id}"))
        .unwrap()
}

fn candidate(value: &str) -> MemoryEntryDraft {
    MemoryEntryDraft::new("Thesis".into(), value.into(), vec!["race".into()]).unwrap()
}

fn set_command(review: MemoryEditReview) -> ApplicationCommand {
    ApplicationCommand::SetMemoryEntry {
        profile: review.profile,
        expected: review.expected,
        candidate: review.candidate.unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest,
    }
}

fn propose(
    app: &mut support::TestApp,
    profile: &AgentProfileVersion,
    id: u128,
) -> MemoryProposalRef {
    let outcome = app
        .execute(envelope(
            id,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: candidate("Proposal race value."),
                },
                rationale: "Concurrent resolution evidence.".into(),
            },
        ))
        .unwrap();
    match outcome.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("expected proposal view, got {other:?}"),
    }
}

struct DeterministicMemoryRaceHook {
    first: CommandId,
    second: CommandId,
    first_preflight: Barrier,
    allow_first: Barrier,
    first_reserved: Barrier,
    finish_first: Barrier,
    second_preflight: Barrier,
    allow_second: Barrier,
}

impl DeterministicMemoryRaceHook {
    fn new(first: CommandId, second: CommandId) -> Self {
        Self {
            first,
            second,
            first_preflight: Barrier::new(2),
            allow_first: Barrier::new(2),
            first_reserved: Barrier::new(2),
            finish_first: Barrier::new(2),
            second_preflight: Barrier::new(2),
            allow_second: Barrier::new(2),
        }
    }
}

impl CommandTransactionHook for DeterministicMemoryRaceHook {
    fn before_memory_review_operation(&self, command_id: CommandId) {
        if command_id == self.first {
            self.first_preflight.wait();
            self.allow_first.wait();
        } else if command_id == self.second {
            self.second_preflight.wait();
            self.allow_second.wait();
        }
    }

    fn after_memory_review_reservation(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
        command_id: CommandId,
    ) -> Result<(), PersistenceError> {
        if command_id == self.first {
            self.first_reserved.wait();
            self.finish_first.wait();
        }
        Ok(())
    }

    fn before_outcome_materialization(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn before_receipt_write(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }
}

fn run_threads(
    hook: &DeterministicMemoryRaceHook,
    mut first_service: ai_stock_forum::app::IndependentApplicationService,
    first: CommandEnvelope,
    mut second_service: ai_stock_forum::app::IndependentApplicationService,
    second: CommandEnvelope,
) -> (
    Result<ai_stock_forum::app::CommandOutcome, AppError>,
    Result<ai_stock_forum::app::CommandOutcome, AppError>,
) {
    let first_thread = thread::spawn(move || first_service.execute(first));
    hook.first_preflight.wait();
    let second_thread = thread::spawn(move || second_service.execute(second));
    hook.second_preflight.wait();

    hook.allow_first.wait();
    hook.first_reserved.wait();
    hook.allow_second.wait();
    hook.finish_first.wait();

    (first_thread.join().unwrap(), second_thread.join().unwrap())
}

#[test]
fn independent_edit_edit_race_has_one_winner_and_exact_retry_semantics() {
    let first_id = CommandId::from_uuid(Uuid::from_u128(1_000_001));
    let second_id = CommandId::from_uuid(Uuid::from_u128(1_000_002));
    let hook = Arc::new(DeterministicMemoryRaceHook::new(first_id, second_id));
    let mut app = support::app_with_policy_and_hook(
        Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        )),
        hook.clone(),
    );
    let profile = create_profile(&mut app, 1_000_000);
    let first_service = app.independent_profile_instance().unwrap();
    let second_service = app.independent_profile_instance().unwrap();
    let first_review = match first_service
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            candidate("First wins."),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let second_review = match second_service
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            candidate("Second loses."),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let first = envelope(1_000_001, Actor::Human, set_command(first_review));
    let second = envelope(1_000_002, Actor::Human, set_command(second_review));
    let before_events = app.count_rows("event_stream");
    let before_receipts = app.count_rows("command_receipts");

    let (winner, loser) = run_threads(
        hook.as_ref(),
        first_service,
        first.clone(),
        second_service,
        second.clone(),
    );
    let winner = winner.unwrap();
    assert_eq!(loser.unwrap_err().code(), "memory_expected_state_mismatch");
    assert_eq!(app.execute(first.clone()).unwrap(), winner);
    let mut changed = first;
    if let ApplicationCommand::SetMemoryEntry { candidate, .. } = &mut changed.command {
        *candidate = self::candidate("Changed replay payload.");
    }
    assert_eq!(app.execute(changed).unwrap_err(), AppError::CommandConflict);
    assert_eq!(app.count_rows("event_stream"), before_events + 1);
    assert_eq!(app.count_rows("memory_entry_versions"), 1);
    assert_eq!(app.count_rows("current_memory_entries"), 1);
    assert_eq!(app.count_rows("command_receipts"), before_receipts + 1);
    assert!(app.event_ref_rows(second.command_id).is_empty());
}

fn resolution_command(
    action: MemoryResolutionAction,
    proposal: MemoryProposalRef,
    review: ai_stock_forum::app::MemoryProposalResolutionReview,
) -> ApplicationCommand {
    match action {
        MemoryResolutionAction::Approve => ApplicationCommand::ApproveMemoryProposal {
            proposal,
            approval_id: review.approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
        MemoryResolutionAction::Reject => ApplicationCommand::RejectMemoryProposal {
            proposal,
            approval_id: review.approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
    }
}

fn resolution_race(second_action: MemoryResolutionAction, seed: u128) {
    let first_id = CommandId::from_uuid(Uuid::from_u128(seed + 2));
    let second_id = CommandId::from_uuid(Uuid::from_u128(seed + 3));
    let hook = Arc::new(DeterministicMemoryRaceHook::new(first_id, second_id));
    let mut app = support::app_with_policy_and_hook(
        Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        )),
        hook.clone(),
    );
    let profile = create_profile(&mut app, seed);
    let proposal = propose(&mut app, &profile, seed + 1);
    let first_service = app.independent_profile_instance().unwrap();
    let second_service = app.independent_profile_instance().unwrap();
    let first_review = first_service
        .preview_memory_proposal_approval(proposal.clone())
        .unwrap();
    let second_review = match second_action {
        MemoryResolutionAction::Approve => second_service
            .preview_memory_proposal_approval(proposal.clone())
            .unwrap(),
        MemoryResolutionAction::Reject => second_service
            .preview_memory_proposal_rejection(proposal.clone())
            .unwrap(),
    };
    let first = envelope(
        seed + 2,
        Actor::Human,
        resolution_command(
            MemoryResolutionAction::Approve,
            proposal.clone(),
            first_review,
        ),
    );
    let second = envelope(
        seed + 3,
        Actor::Human,
        resolution_command(second_action, proposal, second_review),
    );
    let before_events = app.count_rows("event_stream");
    let before_receipts = app.count_rows("command_receipts");

    let (winner, loser) = run_threads(
        hook.as_ref(),
        first_service,
        first.clone(),
        second_service,
        second.clone(),
    );
    let winner = winner.unwrap();
    assert_eq!(
        loser.unwrap_err().code(),
        "memory_proposal_review_unavailable"
    );
    assert_eq!(app.execute(first.clone()).unwrap(), winner);
    let mut changed = second.clone();
    changed.command_id = first.command_id;
    changed.correlation_id = first.correlation_id;
    assert_eq!(app.execute(changed).unwrap_err(), AppError::CommandConflict);
    assert_eq!(app.count_rows("event_stream"), before_events + 1);
    assert_eq!(app.count_rows("memory_proposal_resolutions"), 1);
    assert_eq!(app.count_rows("memory_entry_versions"), 1);
    assert_eq!(app.count_rows("command_receipts"), before_receipts + 1);
    assert!(app.event_ref_rows(second.command_id).is_empty());
}

#[test]
fn independent_approve_approve_race_has_one_terminal_winner() {
    resolution_race(MemoryResolutionAction::Approve, 1_100_000);
}

#[test]
fn independent_approve_reject_race_has_one_terminal_winner() {
    resolution_race(MemoryResolutionAction::Reject, 1_200_000);
}
