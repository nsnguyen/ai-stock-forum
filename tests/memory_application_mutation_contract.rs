mod support;

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, ApplicationCommand, ApplicationEvent, AuthorizationDecision,
        CommandEnvelope, CommandOutcome, CommandTransactionHook, CommandView, EVENT_SCHEMA_VERSION,
        MemoryEditPreview, PendingEvent, ShutdownReason,
    },
    domain::{
        Actor, ApprovalId, CommandId, CorrelationId, EventId, MemoryEntryId, MemoryEntryVersionId,
        MemoryProposalId, ObjectRef,
    },
    memory::{
        ExpectedMemoryEntryState, MemoryEditReview, MemoryEntryDraft, MemoryEntryState,
        MemoryEntryVersion, MemoryMutationKind, MemoryNoChange, MemoryPlaintextAcknowledgement,
        MemoryProposal, MemoryProposalOperation, MemoryProposalStatus,
    },
    persistence::{EventRepository, MemoryRepository, PersistenceError, ProjectionRepository},
    policy::{ApprovalAction, ApprovalRecord, ApprovalStatus},
    recovery::reduce,
    runtime::RuntimeError,
};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 100_000)),
        actor: Actor::Human,
        command,
    }
}

fn create_profile(app: &mut support::TestApp, id: u128) -> AgentProfileVersion {
    app.execute(envelope(
        id,
        ApplicationCommand::CreateAgentProfile {
            draft: AgentProfileDraft::new(
                format!("Memory Editor {id}"),
                "Direct mutation contract profile.".into(),
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
        .find(|profile| profile.display_name() == format!("Memory Editor {id}"))
        .unwrap()
}

fn candidate() -> MemoryEntryDraft {
    MemoryEntryDraft::new(
        "Earnings thesis".into(),
        "Margins should expand next year.".into(),
        vec!["earnings".into()],
    )
    .unwrap()
}

fn execute_review(
    app: &mut support::TestApp,
    id: u128,
    review: MemoryEditReview,
) -> CommandOutcome {
    let command = match review.operation {
        MemoryMutationKind::Set => ApplicationCommand::SetMemoryEntry {
            profile: review.profile,
            expected: review.expected,
            candidate: review.candidate.unwrap(),
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
        MemoryMutationKind::Delete => ApplicationCommand::DeleteMemoryEntry {
            profile: review.profile,
            expected: match review.expected {
                ExpectedMemoryEntryState::Present(reference) => reference,
                other => panic!("delete review had unexpected state {other:?}"),
            },
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
    };
    app.execute(envelope(id, command)).unwrap()
}

fn committed_entry(outcome: &CommandOutcome) -> &MemoryEntryVersion {
    match &outcome.committed_events[0].event {
        ApplicationEvent::MemoryEntrySet { entry, .. }
        | ApplicationEvent::MemoryEntryDeleted { entry, .. } => entry,
        other => panic!("unexpected event {other:?}"),
    }
}

fn entry_object(entry: &MemoryEntryVersion) -> ObjectRef {
    let reference = entry.reference();
    ObjectRef::new(
        "memory_entry_version",
        reference.entry_version_id().to_string(),
        reference.version(),
        reference.content_digest().clone(),
    )
    .unwrap()
}

fn seed_entry_versions(app: &support::TestApp, entries: &[MemoryEntryVersion]) {
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let mut projection = ProjectionRepository::load_in(&tx).unwrap();
    for entry in entries {
        let event = match entry.reference().state() {
            MemoryEntryState::Present => ApplicationEvent::MemoryEntrySet {
                entry: entry.clone(),
                expired_proposals: vec![],
            },
            MemoryEntryState::Deleted => ApplicationEvent::MemoryEntryDeleted {
                entry: entry.clone(),
                expired_proposals: vec![],
            },
        };
        let committed = EventRepository::append(
            &tx,
            PendingEvent {
                event_id: entry.creation_event_id(),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: entry.created_at_ms(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(
                    entry.creation_event_id().as_uuid().as_u128() + 1_000_000,
                )),
                causation_id: None,
                object: Some(entry_object(entry)),
                event,
            },
        )
        .unwrap();
        reduce(&mut projection, &committed).unwrap();
        MemoryRepository::insert_entry_version(&tx, committed.sequence, entry).unwrap();
        MemoryRepository::replace_current_entry(&tx, entry).unwrap();
    }
    ProjectionRepository::store(&tx, &projection).unwrap();
    tx.commit().unwrap();
}

fn pending_proposal(
    profile: &AgentProfileVersion,
    id: u128,
    display_key: &str,
    expected: ExpectedMemoryEntryState,
    created_at_ms: i64,
) -> (MemoryProposal, ApprovalRecord) {
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(id)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                display_key.into(),
                format!("proposal value {id}"),
                vec!["proposal-tag".into()],
            )
            .unwrap(),
        },
        display_key.into(),
        expected,
        format!("proposal rationale {id}"),
        created_at_ms,
        EventId::from_uuid(Uuid::from_u128(id + 100_000)),
        ApprovalId::from_uuid(Uuid::from_u128(id + 200_000)),
    )
    .unwrap();
    let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
        .approval_id(proposal.approval_id())
        .object(proposal.object_ref().unwrap())
        .actor(Actor::Agent(profile.profile_id()))
        .created_at_millis(created_at_ms)
        .build()
        .unwrap();
    (proposal, approval)
}

fn seed_pending_proposals(app: &support::TestApp, proposals: &[(MemoryProposal, ApprovalRecord)]) {
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let mut projection = ProjectionRepository::load_in(&tx).unwrap();
    for (proposal, approval) in proposals {
        let committed = EventRepository::append(
            &tx,
            PendingEvent {
                event_id: proposal.creation_event_id(),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Agent(proposal.proposer().profile_id()),
                occurred_at_ms: proposal.created_at_ms(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(
                    proposal.creation_event_id().as_uuid().as_u128() + 1_000_000,
                )),
                causation_id: None,
                object: Some(proposal.object_ref().unwrap()),
                event: ApplicationEvent::MemoryProposalCreated {
                    proposal: proposal.clone(),
                    approval: approval.clone(),
                },
            },
        )
        .unwrap();
        reduce(&mut projection, &committed).unwrap();
        MemoryRepository::insert_proposal_with_approval(
            &tx,
            committed.sequence,
            proposal,
            approval,
        )
        .unwrap();
    }
    ProjectionRepository::store(&tx, &projection).unwrap();
    tx.commit().unwrap();
}

struct FailNextReceiptHook {
    next_error: Mutex<Option<PersistenceError>>,
}

impl FailNextReceiptHook {
    fn new() -> Self {
        Self {
            next_error: Mutex::new(None),
        }
    }

    fn fail_next(&self, error: PersistenceError) {
        *self.next_error.lock().unwrap() = Some(error);
    }
}

impl CommandTransactionHook for FailNextReceiptHook {
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
        self.next_error.lock().unwrap().take().map_or(Ok(()), Err)
    }
}

#[test]
fn passive_preview_returns_typed_no_change_or_an_exact_review_without_durable_writes() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 70_000);
    let selector = AgentProfileSelector::from(profile.profile_id());
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let before_events = app.count_rows("event_stream");
    let before_receipts = app.count_rows("command_receipts");

    let no_change = app
        .preview_memory_delete(selector.clone(), "Never created".into())
        .unwrap();
    assert_eq!(
        no_change,
        MemoryEditPreview::NoChange(MemoryNoChange::AlreadyAbsent)
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(app.count_rows("event_stream"), before_events);
    assert_eq!(app.count_rows("command_receipts"), before_receipts);
    assert_eq!(app.count_rows("memory_entry_versions"), 0);
    assert_eq!(app.count_rows("approval_records"), 0);

    let draft = candidate();
    let review = match app.preview_memory_set(selector, draft.clone()).unwrap() {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    assert_eq!(review.profile, profile.reference());
    assert_eq!(review.namespace_id, profile.memory_namespace_id());
    assert_eq!(review.expected, ExpectedMemoryEntryState::Absent);
    assert_eq!(review.operation, MemoryMutationKind::Set);
    assert_eq!(review.candidate.as_ref(), Some(&draft));
    assert_eq!(
        review.plaintext_acknowledgement,
        MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1
    );
    assert_eq!(app.ids.calls(), before_ids + 1);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(app.count_rows("event_stream"), before_events);
    assert_eq!(app.count_rows("command_receipts"), before_receipts);
    app.cancel_memory_review().unwrap();
}

#[test]
fn confirmed_absent_set_commits_one_exact_version_event_and_receipt_without_an_approval() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 71_000);
    let draft = candidate();
    let review = match app
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            draft.clone(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let command_id = CommandId::from_uuid(Uuid::from_u128(71_500));
    let outcome = app
        .execute(CommandEnvelope {
            command_id,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(71_501)),
            actor: Actor::Human,
            command: ApplicationCommand::SetMemoryEntry {
                profile: review.profile,
                expected: review.expected,
                candidate: review.candidate.unwrap(),
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        })
        .unwrap();

    let (entry_ref, expired) = match &outcome.view {
        CommandView::MemoryEntryMutation(view) => (&view.entry, &view.expired_proposals),
        other => panic!("unexpected view {other:?}"),
    };
    assert!(expired.is_empty());
    assert_eq!(entry_ref.version().get(), 1);
    assert_eq!(entry_ref.namespace_id(), profile.memory_namespace_id());
    assert_eq!(outcome.committed_events.len(), 1);
    let committed = &outcome.committed_events[0];
    let entry = match &committed.event {
        ApplicationEvent::MemoryEntrySet {
            entry,
            expired_proposals,
        } => {
            assert!(expired_proposals.is_empty());
            entry
        }
        other => panic!("unexpected event {other:?}"),
    };
    assert_eq!(entry.reference(), *entry_ref);
    assert_eq!(
        entry.reference().entry_id().as_uuid(),
        Uuid::from_u128(10_000 + u128::try_from(before_ids).unwrap())
    );
    assert_eq!(
        entry.reference().entry_version_id().as_uuid(),
        Uuid::from_u128(10_001 + u128::try_from(before_ids).unwrap())
    );
    assert_eq!(
        committed.event_id.as_uuid(),
        Uuid::from_u128(10_002 + u128::try_from(before_ids).unwrap())
    );
    assert_eq!(entry.creation_event_id(), committed.event_id);
    assert_eq!(entry.created_at_ms(), committed.occurred_at_ms);
    assert_eq!(
        committed.occurred_at_ms,
        1_700_000_000_000 + i64::try_from(before_clock).unwrap()
    );
    assert_eq!(app.ids.calls(), before_ids + 3);
    assert_eq!(app.clock.calls(), before_clock + 1);
    assert_eq!(app.count_rows("memory_entry_versions"), 1);
    assert_eq!(app.count_rows("current_memory_entries"), 1);
    assert_eq!(app.count_rows("approval_records"), 0);
    assert_eq!(app.event_ref_ordinals(command_id), vec![0]);

    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let history = MemoryRepository::load_entry_history(
        &tx,
        profile.memory_namespace_id(),
        &draft.normalized_key(),
        100,
    )
    .unwrap();
    assert_eq!(history.versions, vec![entry.clone()]);
    tx.commit().unwrap();
}

#[test]
fn runtime_preview_cancel_and_worker_execution_share_one_memory_review_registry() {
    let fixture = support::runtime();
    let client = fixture.client();
    let created = client
        .submit(ApplicationCommand::CreateAgentProfile {
            draft: AgentProfileDraft::new(
                "Runtime Memory Editor".into(),
                "Runtime review contract profile.".into(),
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
        })
        .unwrap();
    let profile_id = match created.view {
        CommandView::AgentProfileCreated(view) => view.profile_id,
        other => panic!("unexpected view {other:?}"),
    };
    let review = match client
        .preview_memory_set(AgentProfileSelector::from(profile_id), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let created = client
        .submit(ApplicationCommand::SetMemoryEntry {
            profile: review.profile,
            expected: review.expected,
            candidate: review.candidate.unwrap(),
            review_token: review.review_token,
            review_digest: review.review_digest,
        })
        .unwrap();
    let current = match created.view {
        CommandView::MemoryEntryMutation(view) => view.entry,
        other => panic!("unexpected view {other:?}"),
    };

    let delete = match client
        .preview_memory_delete(
            AgentProfileSelector::from(profile_id),
            "Earnings thesis".into(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    assert_eq!(delete.expected, ExpectedMemoryEntryState::Present(current));
    client.cancel_memory_review().unwrap();
    let error = client
        .submit(ApplicationCommand::DeleteMemoryEntry {
            profile: delete.profile,
            expected: match delete.expected {
                ExpectedMemoryEntryState::Present(reference) => reference,
                other => panic!("unexpected expected state {other:?}"),
            },
            review_token: delete.review_token,
            review_digest: delete.review_digest,
        })
        .unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::Application(ref error) if error.code() == "memory_review_unavailable"
    ));
    fixture.finish_and_join(ai_stock_forum::app::ShutdownReason::UserQuit);
}

#[test]
fn overwrite_delete_and_recreate_keep_one_logical_history_and_noops_are_side_effect_free() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 73_000);
    let selector = AgentProfileSelector::from(profile.profile_id());

    let first_review = match app
        .preview_memory_set(selector.clone(), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let first_outcome = execute_review(&mut app, 73_100, first_review);
    let first = committed_entry(&first_outcome).clone();

    let replacement = MemoryEntryDraft::new(
        "Earnings Thesis".into(),
        "Margins expanded sooner than expected.".into(),
        vec!["earnings".into(), "updated".into()],
    )
    .unwrap();
    let overwrite_review = match app
        .preview_memory_set(selector.clone(), replacement.clone())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    assert_eq!(
        overwrite_review.expected,
        ExpectedMemoryEntryState::Present(first.reference())
    );

    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let before_events = app.count_rows("event_stream");
    let before_receipts = app.count_rows("command_receipts");
    assert_eq!(
        app.preview_memory_set(selector.clone(), candidate())
            .unwrap(),
        MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent)
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(app.count_rows("event_stream"), before_events);
    assert_eq!(app.count_rows("command_receipts"), before_receipts);
    let overwrite_outcome = execute_review(&mut app, 73_200, overwrite_review);
    let overwrite = committed_entry(&overwrite_outcome).clone();
    assert_eq!(
        overwrite.reference().entry_id(),
        first.reference().entry_id()
    );
    assert_eq!(overwrite.reference().version().get(), 2);
    assert_eq!(
        overwrite.predecessor_version_id(),
        Some(first.reference().entry_version_id())
    );

    let delete_review = match app
        .preview_memory_delete(selector.clone(), replacement.display_key().into())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    assert_eq!(delete_review.operation, MemoryMutationKind::Delete);
    assert!(delete_review.candidate.is_none());
    let delete_outcome = execute_review(&mut app, 73_300, delete_review);
    let deleted = committed_entry(&delete_outcome).clone();
    assert_eq!(deleted.reference().state(), MemoryEntryState::Deleted);
    assert_eq!(deleted.reference().entry_id(), first.reference().entry_id());
    assert_eq!(deleted.reference().version().get(), 3);

    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let before_events = app.count_rows("event_stream");
    assert_eq!(
        app.preview_memory_delete(selector.clone(), replacement.display_key().into())
            .unwrap(),
        MemoryEditPreview::NoChange(MemoryNoChange::AlreadyAbsent)
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(app.count_rows("event_stream"), before_events);

    let recreate = MemoryEntryDraft::new(
        "Earnings thesis".into(),
        "The thesis is active again.".into(),
        vec!["earnings".into()],
    )
    .unwrap();
    let recreate_review = match app.preview_memory_set(selector, recreate.clone()).unwrap() {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    assert_eq!(
        recreate_review.expected,
        ExpectedMemoryEntryState::Deleted(deleted.reference())
    );
    let recreate_outcome = execute_review(&mut app, 73_400, recreate_review);
    let recreated = committed_entry(&recreate_outcome);
    assert_eq!(recreated.reference().state(), MemoryEntryState::Present);
    assert_eq!(
        recreated.reference().entry_id(),
        first.reference().entry_id()
    );
    assert_eq!(recreated.reference().version().get(), 4);
    assert_eq!(app.count_rows("memory_entry_versions"), 4);
    assert_eq!(app.count_rows("current_memory_entries"), 1);
    assert_eq!(app.count_rows("approval_records"), 0);

    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let history = MemoryRepository::load_entry_history(
        &tx,
        profile.memory_namespace_id(),
        &recreate.normalized_key(),
        100,
    )
    .unwrap();
    assert_eq!(history.total_count, 4);
    assert_eq!(
        history
            .versions
            .iter()
            .map(|entry| entry.reference().version().get())
            .collect::<Vec<_>>(),
        vec![4, 3, 2, 1]
    );
    tx.commit().unwrap();
}

#[test]
fn stale_confirmation_never_rebases_into_a_noop_and_invalidates_the_review() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 74_000);
    let selector = AgentProfileSelector::from(profile.profile_id());
    let initial = match app
        .preview_memory_set(selector.clone(), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => execute_review(&mut app, 74_100, review),
        other => panic!("expected review, got {other:?}"),
    };
    let initial = committed_entry(&initial).clone();
    let replacement = MemoryEntryDraft::new(
        "Earnings thesis".into(),
        "The state changed after preview.".into(),
        vec!["earnings".into()],
    )
    .unwrap();
    let review = match app
        .preview_memory_set(selector, replacement.clone())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let successor = initial
        .next_present(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(740_001)),
            replacement,
            Actor::Human,
            1_800_000_000_000,
            None,
            EventId::from_uuid(Uuid::from_u128(740_002)),
        )
        .unwrap();
    seed_entry_versions(&app, &[successor]);
    let command = ApplicationCommand::SetMemoryEntry {
        profile: review.profile,
        expected: review.expected,
        candidate: review.candidate.unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest,
    };
    let before_versions = app.count_rows("memory_entry_versions");

    let first_error = app.execute(envelope(74_200, command.clone())).unwrap_err();
    assert_eq!(first_error.code(), "memory_expected_state_mismatch");
    assert_eq!(app.count_rows("memory_entry_versions"), before_versions);
    assert_eq!(app.count_rows("command_receipts"), 2);
    let retry_error = app.execute(envelope(74_201, command)).unwrap_err();
    assert_eq!(retry_error.code(), "memory_review_unavailable");
    assert_eq!(app.count_rows("memory_entry_versions"), before_versions);
}

#[test]
fn changed_candidate_actor_and_action_each_invalidate_the_one_use_review() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 75_000);
    let selector = AgentProfileSelector::from(profile.profile_id());

    let review = match app
        .preview_memory_set(selector.clone(), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let original = ApplicationCommand::SetMemoryEntry {
        profile: review.profile.clone(),
        expected: review.expected.clone(),
        candidate: review.candidate.clone().unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest.clone(),
    };
    let changed = match &original {
        ApplicationCommand::SetMemoryEntry {
            profile,
            expected,
            review_token,
            review_digest,
            ..
        } => ApplicationCommand::SetMemoryEntry {
            profile: profile.clone(),
            expected: expected.clone(),
            candidate: MemoryEntryDraft::new(
                "Earnings thesis".into(),
                "A changed candidate cannot borrow the token.".into(),
                vec![],
            )
            .unwrap(),
            review_token: *review_token,
            review_digest: review_digest.clone(),
        },
        _ => unreachable!(),
    };
    assert_eq!(
        app.execute(envelope(75_100, changed)).unwrap_err().code(),
        "memory_review_unavailable"
    );
    assert_eq!(
        app.execute(envelope(75_101, original)).unwrap_err().code(),
        "memory_review_unavailable"
    );

    let review = match app
        .preview_memory_set(selector.clone(), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let human_command = ApplicationCommand::SetMemoryEntry {
        profile: review.profile,
        expected: review.expected,
        candidate: review.candidate.unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest,
    };
    let mut system_envelope = envelope(75_200, human_command.clone());
    system_envelope.actor = Actor::System;
    assert_eq!(
        app.execute(system_envelope).unwrap_err().code(),
        "capability_denied"
    );
    assert_eq!(
        app.execute(envelope(75_201, human_command))
            .unwrap_err()
            .code(),
        "memory_review_unavailable"
    );

    let create_review = match app
        .preview_memory_set(selector.clone(), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let created = execute_review(&mut app, 75_300, create_review);
    let current = committed_entry(&created).reference();
    let delete = match app
        .preview_memory_delete(selector, "Earnings thesis".into())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let wrong_action = ApplicationCommand::SetMemoryEntry {
        profile: delete.profile.clone(),
        expected: ExpectedMemoryEntryState::Present(current),
        candidate: MemoryEntryDraft::new("Earnings thesis".into(), "Wrong action.".into(), vec![])
            .unwrap(),
        review_token: delete.review_token,
        review_digest: delete.review_digest.clone(),
    };
    assert_eq!(
        app.execute(envelope(75_400, wrong_action))
            .unwrap_err()
            .code(),
        "memory_review_unavailable"
    );
    let exact_delete = ApplicationCommand::DeleteMemoryEntry {
        profile: delete.profile,
        expected: match delete.expected {
            ExpectedMemoryEntryState::Present(reference) => reference,
            other => panic!("unexpected expected state {other:?}"),
        },
        review_token: delete.review_token,
        review_digest: delete.review_digest,
    };
    assert_eq!(
        app.execute(envelope(75_401, exact_delete))
            .unwrap_err()
            .code(),
        "memory_review_unavailable"
    );
}

#[test]
fn recoverable_query_failure_releases_exact_review_and_receipt_replay_needs_no_token() {
    let policy = Arc::new(support::RecordingPolicy::new(
        AuthorizationDecision::Granted,
    ));
    let hook = Arc::new(FailNextReceiptHook::new());
    let mut app = support::app_with_policy_and_hook(policy.clone(), hook.clone());
    let profile = create_profile(&mut app, 76_000);
    let review = match app
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            candidate(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let request = CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(76_100)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(76_101)),
        actor: Actor::Human,
        command: ApplicationCommand::SetMemoryEntry {
            profile: review.profile,
            expected: review.expected,
            candidate: review.candidate.unwrap(),
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
    };
    let before_events = app.count_rows("event_stream");
    hook.fail_next(PersistenceError::QueryFailed);
    let error = app.execute(request.clone()).unwrap_err();
    assert_eq!(
        error,
        ai_stock_forum::app::AppError::Persistence(PersistenceError::QueryFailed)
    );
    assert_eq!(app.count_rows("event_stream"), before_events);
    assert_eq!(app.count_rows("memory_entry_versions"), 0);
    assert_eq!(app.count_rows("command_receipts"), 1);

    let first = app.execute(request.clone()).unwrap();
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let before_rows = (
        app.count_rows("event_stream"),
        app.count_rows("memory_entry_versions"),
        app.count_rows("command_receipts"),
    );
    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));
    let policy_calls = policy.calls();
    let replay = app.execute(request.clone()).unwrap();
    assert_eq!(replay, first);
    assert_eq!(policy.calls(), policy_calls);
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(
        (
            app.count_rows("event_stream"),
            app.count_rows("memory_entry_versions"),
            app.count_rows("command_receipts"),
        ),
        before_rows
    );

    let mut changed_request = request.clone();
    if let ApplicationCommand::SetMemoryEntry { candidate, .. } = &mut changed_request.command {
        *candidate = MemoryEntryDraft::new(
            "Earnings thesis".into(),
            "A genuinely changed replay request.".into(),
            vec!["earnings".into()],
        )
        .unwrap();
    } else {
        panic!("expected set command");
    }
    assert_eq!(
        app.execute(changed_request).unwrap_err().code(),
        "command_conflict"
    );

    let mut changed_actor = request;
    changed_actor.actor = Actor::System;
    assert_eq!(
        app.execute(changed_actor).unwrap_err().code(),
        "command_conflict"
    );
}

#[test]
fn independent_services_do_not_share_reviews_and_shutdown_invalidates_the_shared_review() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 77_000);
    let selector = AgentProfileSelector::from(profile.profile_id());
    let review = match app
        .preview_memory_set(selector.clone(), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let command = ApplicationCommand::SetMemoryEntry {
        profile: review.profile,
        expected: review.expected,
        candidate: review.candidate.unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest,
    };
    let mut independent = app.independent_profile_instance().unwrap();
    assert_eq!(
        independent
            .execute(envelope(77_100, command.clone()))
            .unwrap_err()
            .code(),
        "memory_review_unavailable"
    );
    app.execute(envelope(77_101, command)).unwrap();

    let review = match app
        .preview_memory_set(
            selector,
            MemoryEntryDraft::new(
                "Another key".into(),
                "This pending review is cancelled by shutdown.".into(),
                vec![],
            )
            .unwrap(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let command = ApplicationCommand::SetMemoryEntry {
        profile: review.profile,
        expected: review.expected,
        candidate: review.candidate.unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest,
    };
    app.execute(envelope(77_200, ApplicationCommand::RequestShutdown))
        .unwrap();
    assert_eq!(
        app.execute(envelope(77_201, command)).unwrap_err().code(),
        "memory_review_unavailable"
    );
    app.finish(ShutdownReason::UserQuit).unwrap();
}

#[test]
fn active_entry_capacity_blocks_only_absent_or_tombstone_creation_and_releases_for_retry() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 78_000);
    let namespace_id = profile.memory_namespace_id();
    let mut entries = Vec::with_capacity(1_026);
    for index in 0_u128..1_024 {
        let id = 1_000_000 + index;
        entries.push(
            MemoryEntryVersion::create_present(
                namespace_id,
                MemoryEntryId::from_uuid(Uuid::from_u128(id)),
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(id + 10_000)),
                MemoryEntryDraft::new(
                    format!("Capacity key {index:04}"),
                    format!("capacity value {index}"),
                    vec![],
                )
                .unwrap(),
                Actor::Human,
                1_600_000_000_000 + i64::try_from(index).unwrap(),
                None,
                EventId::from_uuid(Uuid::from_u128(id + 20_000)),
            )
            .unwrap(),
        );
    }
    let tombstone_present = MemoryEntryVersion::create_present(
        namespace_id,
        MemoryEntryId::from_uuid(Uuid::from_u128(1_100_000)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_110_000)),
        MemoryEntryDraft::new(
            "Capacity tombstone".into(),
            "old tombstoned value".into(),
            vec![],
        )
        .unwrap(),
        Actor::Human,
        1_600_000_010_000,
        None,
        EventId::from_uuid(Uuid::from_u128(1_120_000)),
    )
    .unwrap();
    let tombstone = tombstone_present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_110_001)),
            Actor::Human,
            1_600_000_010_001,
            None,
            EventId::from_uuid(Uuid::from_u128(1_120_001)),
        )
        .unwrap();
    entries.push(tombstone_present);
    entries.push(tombstone.clone());
    seed_entry_versions(&app, &entries);

    let selector = AgentProfileSelector::from(profile.profile_id());
    let overflow = MemoryEntryDraft::new(
        "Overflow key".into(),
        "This cannot consume slot 1025.".into(),
        vec![],
    )
    .unwrap();
    let overflow_review = match app
        .preview_memory_set(selector.clone(), overflow.clone())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let overflow_command = ApplicationCommand::SetMemoryEntry {
        profile: overflow_review.profile,
        expected: overflow_review.expected,
        candidate: overflow_review.candidate.unwrap(),
        review_token: overflow_review.review_token,
        review_digest: overflow_review.review_digest,
    };
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let before_versions = app.count_rows("memory_entry_versions");
    assert_eq!(
        app.execute(envelope(78_100, overflow_command.clone()))
            .unwrap_err(),
        ai_stock_forum::app::AppError::Persistence(PersistenceError::Capacity)
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(app.count_rows("memory_entry_versions"), before_versions);

    let freed = entries[0]
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_130_000)),
            Actor::Human,
            1_600_000_020_000,
            None,
            EventId::from_uuid(Uuid::from_u128(1_140_000)),
        )
        .unwrap();
    seed_entry_versions(&app, &[freed]);
    let before_retry_ids = app.ids.calls();
    let created = app.execute(envelope(78_100, overflow_command)).unwrap();
    let created = committed_entry(&created);
    assert_eq!(
        created.reference().entry_id().as_uuid(),
        Uuid::from_u128(10_000 + u128::try_from(before_retry_ids).unwrap())
    );
    assert_eq!(app.ids.calls(), before_retry_ids + 3);
    assert_eq!(app.clock.calls(), before_clock + 1);

    let recreate = MemoryEntryDraft::new(
        "Capacity tombstone".into(),
        "Recreation also needs a free active slot.".into(),
        vec![],
    )
    .unwrap();
    let recreate = match app.preview_memory_set(selector.clone(), recreate).unwrap() {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    assert_eq!(
        recreate.expected,
        ExpectedMemoryEntryState::Deleted(tombstone.reference())
    );
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    assert_eq!(
        app.execute(envelope(
            78_200,
            ApplicationCommand::SetMemoryEntry {
                profile: recreate.profile,
                expected: recreate.expected,
                candidate: recreate.candidate.unwrap(),
                review_token: recreate.review_token,
                review_digest: recreate.review_digest,
            },
        ))
        .unwrap_err(),
        ai_stock_forum::app::AppError::Persistence(PersistenceError::Capacity)
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);

    let overwrite = MemoryEntryDraft::new(
        "Capacity key 0001".into(),
        "Overwrite is allowed at capacity.".into(),
        vec![],
    )
    .unwrap();
    let overwrite = match app.preview_memory_set(selector.clone(), overwrite).unwrap() {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    execute_review(&mut app, 78_300, overwrite);
    assert_eq!(app.ids.calls(), before_ids + 2);
    assert_eq!(app.clock.calls(), before_clock + 1);

    let delete = match app
        .preview_memory_delete(selector, "Capacity key 0002".into())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    execute_review(&mut app, 78_400, delete);
    assert_eq!(app.ids.calls(), before_ids + 2);
    assert_eq!(app.clock.calls(), before_clock + 1);
}

#[test]
fn direct_mutation_expires_only_newly_stale_same_key_siblings_in_proposal_id_order() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 79_000);
    let selector = AgentProfileSelector::from(profile.profile_id());
    let initial = match app
        .preview_memory_set(selector.clone(), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => execute_review(&mut app, 79_100, review),
        other => panic!("expected review, got {other:?}"),
    };
    let current = committed_entry(&initial).reference();
    let same_key = [
        pending_proposal(
            &profile,
            790_003,
            "Earnings thesis",
            ExpectedMemoryEntryState::Present(current.clone()),
            30,
        ),
        pending_proposal(
            &profile,
            790_001,
            "Earnings thesis",
            ExpectedMemoryEntryState::Present(current.clone()),
            90,
        ),
        pending_proposal(
            &profile,
            790_002,
            "Earnings thesis",
            ExpectedMemoryEntryState::Present(current),
            60,
        ),
    ];
    let other = pending_proposal(
        &profile,
        790_004,
        "Other key",
        ExpectedMemoryEntryState::Absent,
        10,
    );
    let mut proposals = same_key.to_vec();
    proposals.push(other.clone());
    seed_pending_proposals(&app, &proposals);

    let replacement = MemoryEntryDraft::new(
        "Earnings thesis".into(),
        "A direct edit supersedes same-key proposals.".into(),
        vec!["earnings".into()],
    )
    .unwrap();
    let review = match app.preview_memory_set(selector, replacement).unwrap() {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let outcome = execute_review(&mut app, 79_200, review);
    let (entry, expired) = match &outcome.committed_events[0].event {
        ApplicationEvent::MemoryEntrySet {
            entry,
            expired_proposals,
        } => (entry, expired_proposals),
        other => panic!("unexpected event {other:?}"),
    };
    assert_eq!(
        expired
            .iter()
            .map(|resolution| resolution.proposal().proposal_id())
            .collect::<Vec<_>>(),
        vec![
            MemoryProposalId::from_uuid(Uuid::from_u128(790_001)),
            MemoryProposalId::from_uuid(Uuid::from_u128(790_002)),
            MemoryProposalId::from_uuid(Uuid::from_u128(790_003)),
        ]
    );
    for resolution in expired {
        assert_eq!(resolution.status(), MemoryProposalStatus::Expired);
        assert_eq!(resolution.resolved_by(), &Actor::Human);
        assert_eq!(resolution.resolved_at_ms(), entry.created_at_ms());
        assert_eq!(resolution.resolution_event_id(), entry.creation_event_id());
    }
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    for (proposal, _) in &same_key {
        let (_, status, resolution) =
            MemoryRepository::load_proposal(&tx, proposal.reference().proposal_id())
                .unwrap()
                .unwrap();
        assert_eq!(status, MemoryProposalStatus::Expired);
        assert_eq!(
            resolution.unwrap().resolution_event_id(),
            entry.creation_event_id()
        );
        assert_eq!(
            MemoryRepository::load_memory_approval(&tx, proposal.approval_id())
                .unwrap()
                .unwrap()
                .status(),
            ApprovalStatus::Expired
        );
    }
    let (_, other_status, other_resolution) =
        MemoryRepository::load_proposal(&tx, other.0.reference().proposal_id())
            .unwrap()
            .unwrap();
    assert_eq!(other_status, MemoryProposalStatus::Pending);
    assert!(other_resolution.is_none());
    tx.commit().unwrap();
}

#[test]
fn contention_releases_but_terminal_transaction_failure_invalidates_the_review() {
    for (index, injected, retry_succeeds) in [
        (0_u128, PersistenceError::Contention, true),
        (1_u128, PersistenceError::InvalidEventRecord, false),
    ] {
        let policy = Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        ));
        let hook = Arc::new(FailNextReceiptHook::new());
        let mut app = support::app_with_policy_and_hook(policy, hook.clone());
        let profile = create_profile(&mut app, 80_000 + index * 1_000);
        let review = match app
            .preview_memory_set(
                AgentProfileSelector::from(profile.profile_id()),
                MemoryEntryDraft::new(
                    format!("Failure disposition {index}"),
                    "Synthetic retry value.".into(),
                    vec![],
                )
                .unwrap(),
            )
            .unwrap()
        {
            MemoryEditPreview::Review(review) => review,
            other => panic!("expected review, got {other:?}"),
        };
        let request = ApplicationCommand::SetMemoryEntry {
            profile: review.profile,
            expected: review.expected,
            candidate: review.candidate.unwrap(),
            review_token: review.review_token,
            review_digest: review.review_digest,
        };
        hook.fail_next(injected);
        assert_eq!(
            app.execute(envelope(80_100 + index * 1_000, request.clone()))
                .unwrap_err(),
            ai_stock_forum::app::AppError::Persistence(injected)
        );
        assert_eq!(app.count_rows("memory_entry_versions"), 0);
        let retry = app.execute(envelope(80_101 + index * 1_000, request));
        if retry_succeeds {
            assert!(retry.is_ok());
            assert_eq!(app.count_rows("memory_entry_versions"), 1);
        } else {
            assert_eq!(retry.unwrap_err().code(), "memory_review_unavailable");
            assert_eq!(app.count_rows("memory_entry_versions"), 0);
        }
    }
}

#[test]
fn profile_substitution_and_policy_denial_invalidate_the_bound_review() {
    let policy = Arc::new(support::RecordingPolicy::new(
        AuthorizationDecision::Granted,
    ));
    let mut app = support::app_with_policy(policy.clone());
    let first = create_profile(&mut app, 82_000);
    let second = create_profile(&mut app, 82_100);
    let review = match app
        .preview_memory_set(AgentProfileSelector::from(first.profile_id()), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let exact = ApplicationCommand::SetMemoryEntry {
        profile: review.profile,
        expected: review.expected,
        candidate: review.candidate.unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest,
    };
    let substituted = match &exact {
        ApplicationCommand::SetMemoryEntry {
            expected,
            candidate,
            review_token,
            review_digest,
            ..
        } => ApplicationCommand::SetMemoryEntry {
            profile: second.reference(),
            expected: expected.clone(),
            candidate: candidate.clone(),
            review_token: *review_token,
            review_digest: review_digest.clone(),
        },
        _ => unreachable!(),
    };
    assert_eq!(
        app.execute(envelope(82_200, substituted))
            .unwrap_err()
            .code(),
        "memory_review_unavailable"
    );
    assert_eq!(
        app.execute(envelope(82_201, exact)).unwrap_err().code(),
        "memory_review_unavailable"
    );

    let review = match app
        .preview_memory_set(AgentProfileSelector::from(first.profile_id()), candidate())
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let exact = ApplicationCommand::SetMemoryEntry {
        profile: review.profile,
        expected: review.expected,
        candidate: review.candidate.unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest,
    };
    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));
    assert_eq!(
        app.execute(envelope(82_300, exact.clone()))
            .unwrap_err()
            .code(),
        "capability_denied"
    );
    policy.set_decision(AuthorizationDecision::Granted);
    assert_eq!(
        app.execute(envelope(82_301, exact)).unwrap_err().code(),
        "memory_review_unavailable"
    );
}
