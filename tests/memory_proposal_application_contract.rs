mod support;

use std::sync::{Arc, Mutex};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, AppError, ApplicationCommand, ApplicationEvent,
        AuthorizationDecision, CommandEnvelope, CommandTransactionHook, CommandView,
        EVENT_SCHEMA_VERSION, MemoryEditPreview, PendingEvent,
    },
    audit::AuditEntry,
    domain::{
        Actor, ApprovalId, CommandId, CorrelationId, DomainError, EventId, MemoryEntryId,
        MemoryEntryVersionId, MemoryProposalId, ObjectRef,
    },
    memory::{
        ExpectedMemoryEntryState, MemoryEntryDraft, MemoryEntryState, MemoryEntryVersion,
        MemoryPlaintextAcknowledgement, MemoryProposalOperation, MemoryResolutionAction,
    },
    persistence::{EventRepository, MemoryRepository, PersistenceError, ProjectionRepository},
    policy::{ApprovalAction, ApprovalStatus, Capability, PolicyDecision},
    recovery::reduce,
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
                format!("Proposal Agent {id}"),
                "Memory proposal contract profile.".into(),
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
        .find(|profile| profile.display_name() == format!("Proposal Agent {id}"))
        .unwrap()
}

fn candidate(value: &str) -> MemoryEntryDraft {
    MemoryEntryDraft::new(
        "Earnings thesis".into(),
        value.into(),
        vec!["earnings".into()],
    )
    .unwrap()
}

fn direct_set(
    app: &mut support::TestApp,
    profile: &AgentProfileVersion,
    id: u128,
    draft: MemoryEntryDraft,
) -> ai_stock_forum::memory::MemoryEntryRef {
    let review = match app
        .preview_memory_set(AgentProfileSelector::from(profile.profile_id()), draft)
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let outcome = app
        .execute(envelope(
            id,
            Actor::Human,
            ApplicationCommand::SetMemoryEntry {
                profile: review.profile,
                expected: review.expected,
                candidate: review.candidate.unwrap(),
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap();
    match outcome.view {
        CommandView::MemoryEntryMutation(view) => view.entry,
        other => panic!("unexpected view {other:?}"),
    }
}

fn propose_set(
    app: &mut support::TestApp,
    profile: &AgentProfileVersion,
    id: u128,
    expected: ExpectedMemoryEntryState,
    value: &str,
) -> ai_stock_forum::memory::MemoryProposalRef {
    propose_set_draft(app, profile, id, expected, candidate(value))
}

fn propose_set_draft(
    app: &mut support::TestApp,
    profile: &AgentProfileVersion,
    id: u128,
    expected: ExpectedMemoryEntryState,
    candidate: MemoryEntryDraft,
) -> ai_stock_forum::memory::MemoryProposalRef {
    let outcome = app
        .execute(envelope(
            id,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected,
                operation: MemoryProposalOperation::Set { candidate },
                rationale: format!("Evidence for proposal {id}."),
            },
        ))
        .unwrap();
    match outcome.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("unexpected view {other:?}"),
    }
}

fn propose_delete(
    app: &mut support::TestApp,
    profile: &AgentProfileVersion,
    id: u128,
    expected: ai_stock_forum::memory::MemoryEntryRef,
) -> ai_stock_forum::memory::MemoryProposalRef {
    let outcome = app
        .execute(envelope(
            id,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Present(expected),
                operation: MemoryProposalOperation::Delete,
                rationale: format!("Evidence for deletion proposal {id}."),
            },
        ))
        .unwrap();
    match outcome.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("unexpected view {other:?}"),
    }
}

fn accept(
    app: &mut support::TestApp,
    id: u128,
    proposal: ai_stock_forum::memory::MemoryProposalRef,
) -> ai_stock_forum::app::MemoryProposalResolutionView {
    let review = app
        .preview_memory_proposal_approval(proposal.clone())
        .unwrap();
    let outcome = app
        .execute(envelope(
            id,
            Actor::Human,
            ApplicationCommand::ApproveMemoryProposal {
                proposal,
                approval_id: review.approval_id,
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap();
    match outcome.view {
        CommandView::MemoryProposalResolution(view) => view,
        other => panic!("unexpected view {other:?}"),
    }
}

fn seed_pending_capacity(app: &support::TestApp, profile: &AgentProfileVersion, count: u128) {
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let mut projection = ProjectionRepository::load_in(&tx).unwrap();
    for index in 0..count {
        let proposal = ai_stock_forum::memory::MemoryProposal::new(
            MemoryProposalId::from_uuid(Uuid::from_u128(2_000_000 + index)),
            profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Set {
                candidate: candidate(&format!("Seeded pending candidate {index}.")),
            },
            "Earnings thesis".into(),
            ExpectedMemoryEntryState::Absent,
            format!("Seeded capacity rationale {index}."),
            1_600_000_000_000 + i64::try_from(index).unwrap(),
            EventId::from_uuid(Uuid::from_u128(2_100_000 + index)),
            ApprovalId::from_uuid(Uuid::from_u128(2_200_000 + index)),
        )
        .unwrap();
        let approval =
            ai_stock_forum::policy::ApprovalRecord::builder(ApprovalAction::MemoryMutation)
                .approval_id(proposal.approval_id())
                .object(proposal.object_ref().unwrap())
                .actor(Actor::Agent(profile.profile_id()))
                .created_at_millis(proposal.created_at_ms())
                .build()
                .unwrap();
        let committed = EventRepository::append(
            &tx,
            PendingEvent {
                event_id: proposal.creation_event_id(),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Agent(profile.profile_id()),
                occurred_at_ms: proposal.created_at_ms(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(2_300_000 + index)),
                causation_id: None,
                object: Some(proposal.object_ref().unwrap()),
                event: ApplicationEvent::MemoryProposalCreated {
                    proposal: proposal.clone(),
                    approval: approval.clone(),
                },
            },
        )
        .unwrap();
        MemoryRepository::insert_proposal_with_approval(
            &tx,
            committed.sequence,
            &proposal,
            &approval,
        )
        .unwrap();
        reduce(&mut projection, &committed).unwrap();
    }
    ProjectionRepository::store(&tx, &projection).unwrap();
    tx.commit().unwrap();
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
        let reference = entry.reference();
        let committed = EventRepository::append(
            &tx,
            PendingEvent {
                event_id: entry.creation_event_id(),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: entry.created_at_ms(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(
                    entry.creation_event_id().as_uuid().as_u128() + 500_000,
                )),
                causation_id: None,
                object: Some(
                    ObjectRef::new(
                        "memory_entry_version",
                        reference.entry_version_id().to_string(),
                        reference.version(),
                        reference.content_digest().clone(),
                    )
                    .unwrap(),
                ),
                event,
            },
        )
        .unwrap();
        MemoryRepository::insert_entry_version(&tx, committed.sequence, entry).unwrap();
        MemoryRepository::replace_current_entry(&tx, entry).unwrap();
        reduce(&mut projection, &committed).unwrap();
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
fn agent_proposal_creation_commits_one_pending_proposal_approval_event_and_receipt() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 110_000);
    let before_entries = app.count_rows("memory_entry_versions");
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let outcome = app
        .execute(envelope(
            110_100,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: candidate("Margins should expand next year."),
                },
                rationale: "The latest filing supports a durable margin thesis.".into(),
            },
        ))
        .unwrap();

    let view = match &outcome.view {
        CommandView::MemoryProposalCreated(view) => view,
        other => panic!("unexpected view {other:?}"),
    };
    assert_eq!(
        view.status,
        ai_stock_forum::memory::MemoryProposalStatus::Pending
    );
    assert_eq!(outcome.committed_events.len(), 1);
    match &outcome.committed_events[0].event {
        ApplicationEvent::MemoryProposalCreated { proposal, approval } => {
            assert_eq!(proposal.proposer(), &profile.reference());
            assert_eq!(proposal.approval_id(), approval.approval_id());
            assert_eq!(
                proposal.creation_event_id(),
                outcome.committed_events[0].event_id
            );
            assert_eq!(
                proposal.created_at_ms(),
                outcome.committed_events[0].occurred_at_ms
            );
            assert_eq!(approval.action(), ApprovalAction::MemoryMutation);
            assert_eq!(approval.object(), &proposal.object_ref().unwrap());
            assert_eq!(approval.actor(), &Actor::Agent(profile.profile_id()));
            assert_eq!(approval.status(), ApprovalStatus::Pending);
            assert_eq!(approval.created_at_millis(), proposal.created_at_ms());
            assert_eq!(approval.expires_at_millis(), None);
            assert_eq!(approval.resolution(), None);
            assert_eq!(
                proposal.reference().proposal_id().as_uuid(),
                Uuid::from_u128(10_000 + u128::try_from(before_ids).unwrap())
            );
            assert_eq!(
                proposal.approval_id().as_uuid(),
                Uuid::from_u128(10_001 + u128::try_from(before_ids).unwrap())
            );
            assert_eq!(
                proposal.creation_event_id().as_uuid(),
                Uuid::from_u128(10_002 + u128::try_from(before_ids).unwrap())
            );
            let mut database = app.open_database();
            let tx = database.immediate_transaction().unwrap();
            let (stored, status, resolution) =
                MemoryRepository::load_proposal(&tx, proposal.reference().proposal_id())
                    .unwrap()
                    .unwrap();
            let stored_approval =
                MemoryRepository::load_memory_approval(&tx, approval.approval_id())
                    .unwrap()
                    .unwrap();
            assert_eq!(stored, *proposal);
            assert_eq!(
                status,
                ai_stock_forum::memory::MemoryProposalStatus::Pending
            );
            assert_eq!(resolution, None);
            assert_eq!(stored_approval, *approval);
        }
        other => panic!("unexpected event {other:?}"),
    }
    let audit = AuditEntry::from_event(&outcome.committed_events[0]);
    assert_eq!(audit.kind, "memory_proposal_created");
    assert!(audit.summary.contains("proposal="));
    for plaintext in [
        "Earnings thesis",
        "Margins should expand next year.",
        "earnings",
        "The latest filing supports a durable margin thesis.",
    ] {
        assert!(!audit.summary.contains(plaintext));
    }
    assert_eq!(app.count_rows("memory_proposals"), 1);
    assert_eq!(app.count_rows("approval_records"), 1);
    assert_eq!(app.count_rows("memory_proposal_resolutions"), 0);
    assert_eq!(app.count_rows("memory_entry_versions"), before_entries);
    assert_eq!(app.count_rows("command_receipts"), 2);
    assert_eq!(app.ids.calls(), before_ids + 3);
    assert_eq!(app.clock.calls(), before_clock + 1);
}

#[test]
fn passive_approval_preview_returns_an_exact_action_bound_review_without_writes() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 111_000);
    let created = app
        .execute(envelope(
            111_100,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: candidate("A proposal awaiting human review."),
                },
                rationale: "Evidence is strong enough to request review.".into(),
            },
        ))
        .unwrap();
    let proposal_ref = match created.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("unexpected view {other:?}"),
    };
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let before_events = app.count_rows("event_stream");

    let review = app
        .preview_memory_proposal_approval(proposal_ref.clone())
        .unwrap();
    assert_eq!(review.action, MemoryResolutionAction::Approve);
    assert_eq!(review.proposal.reference(), proposal_ref);
    assert_eq!(review.approval_id, review.proposal.approval_id());
    assert_eq!(review.expected_approval_status, ApprovalStatus::Pending);
    assert_eq!(review.expected_entry, ExpectedMemoryEntryState::Absent);
    assert!(!review.proposer_is_historical);
    assert_eq!(review.proposer_identity.profile, profile.reference());
    assert_eq!(review.namespace_owner_identity.profile, profile.reference());
    assert_eq!(
        review.plaintext_acknowledgement,
        MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1
    );
    assert_eq!(app.ids.calls(), before_ids + 1);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(app.count_rows("event_stream"), before_events);
    app.cancel_memory_review().unwrap();
}

#[test]
fn reviewed_human_approval_accepts_the_proposal_and_applies_its_entry_atomically() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 112_000);
    let created = app
        .execute(envelope(
            112_100,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: candidate("A reviewed proposal becomes current memory."),
                },
                rationale: "The evidence is durable and relevant.".into(),
            },
        ))
        .unwrap();
    let proposal_ref = match created.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("unexpected view {other:?}"),
    };
    let review = app
        .preview_memory_proposal_approval(proposal_ref.clone())
        .unwrap();
    let selected_approval_id = review.approval_id;

    let outcome = app
        .execute(envelope(
            112_200,
            Actor::Human,
            ApplicationCommand::ApproveMemoryProposal {
                proposal: proposal_ref.clone(),
                approval_id: review.approval_id,
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap();

    let view = match &outcome.view {
        CommandView::MemoryProposalResolution(view) => view,
        other => panic!("unexpected view {other:?}"),
    };
    assert_eq!(
        view.resolution.status(),
        ai_stock_forum::memory::MemoryProposalStatus::Accepted
    );
    assert_eq!(view.resolution.proposal(), &proposal_ref);
    assert_eq!(
        view.entry.as_ref().unwrap().state(),
        ai_stock_forum::memory::MemoryEntryState::Present
    );
    assert!(view.expired_proposals.is_empty());
    match &outcome.committed_events[0].event {
        ApplicationEvent::MemoryProposalAccepted {
            resolution,
            entry,
            expired_proposals,
        } => {
            assert_eq!(resolution, &view.resolution);
            assert_eq!(entry.reference(), *view.entry.as_ref().unwrap());
            assert_eq!(entry.accepted_proposal(), Some(&proposal_ref));
            assert!(expired_proposals.is_empty());
        }
        other => panic!("unexpected event {other:?}"),
    }
    let audit = AuditEntry::from_event(&outcome.committed_events[0]);
    assert_eq!(audit.kind, "memory_proposal_accepted");
    assert!(audit.summary.contains("proposal="));
    assert!(audit.summary.contains("entry_version="));
    assert!(
        !audit
            .summary
            .contains("A reviewed proposal becomes current memory.")
    );
    assert_eq!(app.count_rows("memory_entry_versions"), 1);
    assert_eq!(app.count_rows("memory_proposal_resolutions"), 1);
    assert_eq!(app.count_rows("current_memory_proposal_status"), 1);
    assert_eq!(app.count_rows("approval_records"), 1);
    assert_eq!(app.count_rows("command_receipts"), 3);
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let approval = MemoryRepository::load_memory_approval(&tx, selected_approval_id)
        .unwrap()
        .unwrap();
    assert_eq!(approval.status(), ApprovalStatus::Accepted);
    let approval_resolution = approval.resolution().unwrap();
    assert_eq!(approval_resolution.actor(), &Actor::Human);
    assert_eq!(
        approval_resolution.resolved_at_millis(),
        outcome.committed_events[0].occurred_at_ms
    );
}

#[test]
fn reviewed_human_rejection_resolves_only_the_proposal_without_an_entry_mutation() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 113_000);
    let created = app
        .execute(envelope(
            113_100,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: candidate("A proposal the reviewer will reject."),
                },
                rationale: "This is intentionally insufficient evidence.".into(),
            },
        ))
        .unwrap();
    let proposal_ref = match created.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("unexpected view {other:?}"),
    };
    let approve = app
        .preview_memory_proposal_approval(proposal_ref.clone())
        .unwrap();
    app.cancel_memory_review().unwrap();
    let review = app
        .preview_memory_proposal_rejection(proposal_ref.clone())
        .unwrap();
    let selected_approval_id = review.approval_id;
    assert_eq!(review.action, MemoryResolutionAction::Reject);
    assert_ne!(review.review_digest, approve.review_digest);

    let outcome = app
        .execute(envelope(
            113_200,
            Actor::Human,
            ApplicationCommand::RejectMemoryProposal {
                proposal: proposal_ref.clone(),
                approval_id: review.approval_id,
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap();

    let view = match &outcome.view {
        CommandView::MemoryProposalResolution(view) => view,
        other => panic!("unexpected view {other:?}"),
    };
    assert_eq!(
        view.resolution.status(),
        ai_stock_forum::memory::MemoryProposalStatus::Rejected
    );
    assert_eq!(view.resolution.proposal(), &proposal_ref);
    assert!(view.entry.is_none());
    assert!(view.expired_proposals.is_empty());
    assert!(matches!(
        &outcome.committed_events[0].event,
        ApplicationEvent::MemoryProposalRejected { resolution }
            if resolution == &view.resolution
    ));
    assert_eq!(app.count_rows("memory_entry_versions"), 0);
    assert_eq!(app.count_rows("memory_proposal_resolutions"), 1);
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let approval = MemoryRepository::load_memory_approval(&tx, selected_approval_id)
        .unwrap()
        .unwrap();
    assert_eq!(approval.status(), ApprovalStatus::Rejected);
    assert_eq!(approval.resolution().unwrap().actor(), &Actor::Human);
}

#[test]
fn only_the_exact_agent_proposer_may_create_a_proposal_and_checks_precede_allocation() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 114_000);
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    for (offset, actor) in [
        Actor::Human,
        Actor::System,
        Actor::Agent(ai_stock_forum::domain::AgentProfileId::from_uuid(
            Uuid::from_u128(114_999),
        )),
    ]
    .into_iter()
    .enumerate()
    {
        let result = app.execute(envelope(
            114_100 + u128::try_from(offset).unwrap(),
            actor,
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: candidate("Unauthorized proposal."),
                },
                rationale: "The actor must be the exact proposer.".into(),
            },
        ));
        assert_eq!(
            result,
            Err(AppError::CapabilityDenied {
                capability: Capability::MemoryPropose,
                decision: PolicyDecision::Denied,
            })
        );
    }
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(app.count_rows("memory_proposals"), 0);
    assert_eq!(app.count_rows("approval_records"), 0);
}

#[test]
fn no_effect_proposal_is_rejected_before_ids_time_or_durable_work() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 115_000);
    let draft = candidate("Already authoritative memory.");
    let current = direct_set(&mut app, &profile, 115_050, draft.clone());
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let before_events = app.count_rows("event_stream");
    let before_receipts = app.count_rows("command_receipts");

    assert_eq!(
        app.execute(envelope(
            115_100,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Present(current),
                operation: MemoryProposalOperation::Set { candidate: draft },
                rationale: "This proposal has no effect.".into(),
            },
        )),
        Err(AppError::Domain(DomainError::InvalidMemoryProposal))
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(app.count_rows("event_stream"), before_events);
    assert_eq!(app.count_rows("command_receipts"), before_receipts);
    assert_eq!(app.count_rows("memory_proposals"), 0);
}

#[test]
fn preview_pins_historical_proposer_identity_while_using_the_current_namespace_owner() {
    let mut app = support::app();
    let original = create_profile(&mut app, 116_000);
    let mut successor = original.to_draft();
    successor.display_name = "Renamed Proposal Agent".into();
    successor.description = "Activated after proposal creation.".into();
    let profile_review = app
        .preview_agent_profile_edit(
            original.profile_id(),
            original.profile_version_id(),
            successor.clone(),
        )
        .unwrap();
    app.execute(envelope(
        116_200,
        Actor::Human,
        ApplicationCommand::ActivateAgentProfileVersion {
            profile_id: original.profile_id(),
            expected_active_version_id: original.profile_version_id(),
            candidate: successor,
            review_token: profile_review.review_token,
            review_digest: profile_review.review_digest,
        },
    ))
    .unwrap();
    let current = app
        .projection()
        .agent_profiles
        .active_profile(original.profile_id())
        .unwrap()
        .clone();
    let proposal = propose_set(
        &mut app,
        &original,
        116_100,
        ExpectedMemoryEntryState::Absent,
        "Pinned to the original proposer version.",
    );

    let review = app.preview_memory_proposal_approval(proposal).unwrap();
    assert!(review.proposer_is_historical);
    assert_eq!(review.proposer_identity.profile, original.reference());
    assert_eq!(
        review.proposer_identity.display_name,
        original.display_name()
    );
    assert_eq!(review.namespace_owner_identity.profile, current.reference());
    assert_eq!(
        review.namespace_owner_identity.display_name,
        current.display_name()
    );
}

#[test]
fn accepted_proposal_expires_newly_stale_same_key_siblings_in_proposal_id_order() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 117_000);
    let first = propose_set(
        &mut app,
        &profile,
        117_100,
        ExpectedMemoryEntryState::Absent,
        "First competing proposal.",
    );
    let selected = propose_set(
        &mut app,
        &profile,
        117_200,
        ExpectedMemoryEntryState::Absent,
        "Selected proposal.",
    );
    let third = propose_set(
        &mut app,
        &profile,
        117_300,
        ExpectedMemoryEntryState::Absent,
        "Third competing proposal.",
    );
    let review = app
        .preview_memory_proposal_approval(selected.clone())
        .unwrap();
    let outcome = app
        .execute(envelope(
            117_400,
            Actor::Human,
            ApplicationCommand::ApproveMemoryProposal {
                proposal: selected,
                approval_id: review.approval_id,
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap();
    let view = match outcome.view {
        CommandView::MemoryProposalResolution(view) => view,
        other => panic!("unexpected view {other:?}"),
    };
    assert_eq!(view.expired_proposals, vec![first.clone(), third.clone()]);
    let event = &outcome.committed_events[0];
    let ApplicationEvent::MemoryProposalAccepted {
        resolution,
        expired_proposals,
        ..
    } = &event.event
    else {
        panic!("unexpected event {:?}", event.event);
    };
    assert_eq!(
        resolution.status(),
        ai_stock_forum::memory::MemoryProposalStatus::Accepted
    );
    assert_eq!(
        expired_proposals
            .iter()
            .map(|resolution| resolution.proposal().clone())
            .collect::<Vec<_>>(),
        vec![first.clone(), third.clone()]
    );
    assert!(expired_proposals.iter().all(|resolution| {
        resolution.status() == ai_stock_forum::memory::MemoryProposalStatus::Expired
            && resolution.resolution_event_id() == event.event_id
            && resolution.resolved_at_ms() == event.occurred_at_ms
    }));
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    for (proposal, expected) in [
        (first, ai_stock_forum::memory::MemoryProposalStatus::Expired),
        (
            view.resolution.proposal().clone(),
            ai_stock_forum::memory::MemoryProposalStatus::Accepted,
        ),
        (third, ai_stock_forum::memory::MemoryProposalStatus::Expired),
    ] {
        let (stored, status, resolution) =
            MemoryRepository::load_proposal(&tx, proposal.proposal_id())
                .unwrap()
                .unwrap();
        assert_eq!(status, expected);
        assert_eq!(resolution.unwrap().resolution_event_id(), event.event_id);
        let approval = MemoryRepository::load_memory_approval(&tx, stored.approval_id())
            .unwrap()
            .unwrap();
        assert_eq!(
            approval.status(),
            match expected {
                ai_stock_forum::memory::MemoryProposalStatus::Accepted => ApprovalStatus::Accepted,
                ai_stock_forum::memory::MemoryProposalStatus::Expired => ApprovalStatus::Expired,
                other => panic!("unexpected terminal state {other:?}"),
            }
        );
        assert_eq!(approval.resolution().unwrap().actor(), &Actor::Human);
    }
}

#[test]
fn exact_acceptance_receipt_replay_succeeds_after_the_one_use_review_is_consumed() {
    let policy = Arc::new(support::RecordingPolicy::new(
        AuthorizationDecision::Granted,
    ));
    let mut app = support::app_with_policy(policy.clone());
    let profile = create_profile(&mut app, 118_000);
    let proposal = propose_set(
        &mut app,
        &profile,
        118_100,
        ExpectedMemoryEntryState::Absent,
        "Replay preserves the accepted outcome.",
    );
    let review = app
        .preview_memory_proposal_approval(proposal.clone())
        .unwrap();
    let command = envelope(
        118_200,
        Actor::Human,
        ApplicationCommand::ApproveMemoryProposal {
            proposal,
            approval_id: review.approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
    );
    let first = app.execute(command.clone()).unwrap();
    let ids = app.ids.calls();
    let clock = app.clock.calls();
    let events = app.count_rows("event_stream");
    policy.set_decision(AuthorizationDecision::Denied(PolicyDecision::Denied));
    let policy_calls = policy.calls();
    let second = app.execute(command).unwrap();
    assert_eq!(second, first);
    assert_eq!(app.ids.calls(), ids);
    assert_eq!(app.clock.calls(), clock);
    assert_eq!(app.count_rows("event_stream"), events);
    assert_eq!(policy.calls(), policy_calls);
}

#[test]
fn pending_proposal_capacity_is_checked_before_ids_time_or_partial_durable_work() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 119_000);
    seed_pending_capacity(&app, &profile, 256);
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    let before_events = app.count_rows("event_stream");
    let before_approvals = app.count_rows("approval_records");

    assert_eq!(
        app.execute(envelope(
            119_999,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: candidate("Proposal 257 is over capacity."),
                },
                rationale: "Capacity must be authoritative.".into(),
            },
        )),
        Err(AppError::Persistence(
            ai_stock_forum::persistence::PersistenceError::Capacity
        ))
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(app.count_rows("event_stream"), before_events);
    assert_eq!(app.count_rows("memory_proposals"), 256);
    assert_eq!(app.count_rows("approval_records"), before_approvals);
}

#[test]
fn resolution_binding_rejects_action_proposal_and_approval_substitution_and_invalidates_token() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 120_000);
    let first = propose_set(
        &mut app,
        &profile,
        120_100,
        ExpectedMemoryEntryState::Absent,
        "First exact proposal.",
    );
    let second = propose_set(
        &mut app,
        &profile,
        120_200,
        ExpectedMemoryEntryState::Absent,
        "Second exact proposal.",
    );

    for substitution in 0..3 {
        let review = app.preview_memory_proposal_approval(first.clone()).unwrap();
        let exact = ApplicationCommand::ApproveMemoryProposal {
            proposal: first.clone(),
            approval_id: review.approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry.clone(),
            review_token: review.review_token,
            review_digest: review.review_digest.clone(),
        };
        let substituted = match substitution {
            0 => ApplicationCommand::RejectMemoryProposal {
                proposal: first.clone(),
                approval_id: review.approval_id,
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
            1 => ApplicationCommand::ApproveMemoryProposal {
                proposal: second.clone(),
                approval_id: review.approval_id,
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
            2 => ApplicationCommand::ApproveMemoryProposal {
                proposal: first.clone(),
                approval_id: ai_stock_forum::domain::ApprovalId::from_uuid(Uuid::from_u128(
                    120_999,
                )),
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
            _ => unreachable!(),
        };
        assert_eq!(
            app.execute(envelope(120_300 + substitution, Actor::Human, substituted)),
            Err(AppError::Domain(
                DomainError::MemoryProposalReviewUnavailable
            ))
        );
        assert_eq!(
            app.execute(envelope(120_400 + substitution, Actor::Human, exact)),
            Err(AppError::Domain(
                DomainError::MemoryProposalReviewUnavailable
            ))
        );
    }
}

#[test]
fn canceling_a_local_resolution_review_never_cancels_or_time_expires_the_proposal() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 121_000);
    let proposal = propose_set(
        &mut app,
        &profile,
        121_100,
        ExpectedMemoryEntryState::Absent,
        "Pending proposal survives local review cancellation.",
    );
    let review = app
        .preview_memory_proposal_rejection(proposal.clone())
        .unwrap();
    app.cancel_memory_review().unwrap();
    for offset in 0..3 {
        app.execute(envelope(
            121_200 + offset,
            Actor::Human,
            ApplicationCommand::ShowStatus,
        ))
        .unwrap();
    }
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let (_, status, resolution) = MemoryRepository::load_proposal(&tx, proposal.proposal_id())
        .unwrap()
        .unwrap();
    assert_eq!(
        status,
        ai_stock_forum::memory::MemoryProposalStatus::Pending
    );
    assert_eq!(resolution, None);
    drop(tx);
    drop(database);
    assert_eq!(
        app.execute(envelope(
            121_300,
            Actor::Human,
            ApplicationCommand::RejectMemoryProposal {
                proposal: proposal.clone(),
                approval_id: review.approval_id,
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        )),
        Err(AppError::Domain(
            DomainError::MemoryProposalReviewUnavailable
        ))
    );
    assert!(app.preview_memory_proposal_approval(proposal).is_ok());
}

#[test]
fn non_human_resolution_is_denied_before_policy_or_allocation_and_invalidates_the_review() {
    let policy = Arc::new(support::RecordingPolicy::new(
        AuthorizationDecision::Granted,
    ));
    let mut app = support::app_with_policy(policy.clone());
    let profile = create_profile(&mut app, 122_000);
    let proposal = propose_set(
        &mut app,
        &profile,
        122_100,
        ExpectedMemoryEntryState::Absent,
        "Only a Human can resolve this proposal.",
    );
    let review = app
        .preview_memory_proposal_approval(proposal.clone())
        .unwrap();
    let command = ApplicationCommand::ApproveMemoryProposal {
        proposal,
        approval_id: review.approval_id,
        expected_approval_status: review.expected_approval_status,
        expected_entry: review.expected_entry,
        review_token: review.review_token,
        review_digest: review.review_digest,
    };
    let before_policy = policy.calls();
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    assert_eq!(
        app.execute(envelope(
            122_200,
            Actor::Agent(profile.profile_id()),
            command.clone(),
        )),
        Err(AppError::CapabilityDenied {
            capability: Capability::MemoryResolve,
            decision: PolicyDecision::Denied,
        })
    );
    assert_eq!(policy.calls(), before_policy);
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(
        app.execute(envelope(122_201, Actor::Human, command)),
        Err(AppError::Domain(
            DomainError::MemoryProposalReviewUnavailable
        ))
    );
}

#[test]
fn resolution_failures_release_only_retryable_errors_and_roll_back_every_write() {
    for (index, injected, retry_succeeds) in [
        (0_u128, PersistenceError::Contention, true),
        (1_u128, PersistenceError::InvalidEventRecord, false),
    ] {
        let policy = Arc::new(support::RecordingPolicy::new(
            AuthorizationDecision::Granted,
        ));
        let hook = Arc::new(FailNextReceiptHook::new());
        let mut app = support::app_with_policy_and_hook(policy, hook.clone());
        let profile = create_profile(&mut app, 123_000 + index * 1_000);
        let proposal = propose_set(
            &mut app,
            &profile,
            123_100 + index * 1_000,
            ExpectedMemoryEntryState::Absent,
            "Failure disposition proposal.",
        );
        let review = app
            .preview_memory_proposal_approval(proposal.clone())
            .unwrap();
        let command = ApplicationCommand::ApproveMemoryProposal {
            proposal: proposal.clone(),
            approval_id: review.approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        };
        hook.fail_next(injected);
        assert_eq!(
            app.execute(envelope(
                123_200 + index * 1_000,
                Actor::Human,
                command.clone(),
            )),
            Err(AppError::Persistence(injected))
        );
        assert_eq!(app.count_rows("memory_entry_versions"), 0);
        assert_eq!(app.count_rows("memory_proposal_resolutions"), 0);
        {
            let mut database = app.open_database();
            let tx = database.immediate_transaction().unwrap();
            let (_, status, resolution) =
                MemoryRepository::load_proposal(&tx, proposal.proposal_id())
                    .unwrap()
                    .unwrap();
            assert_eq!(
                status,
                ai_stock_forum::memory::MemoryProposalStatus::Pending
            );
            assert_eq!(resolution, None);
        }
        let retry = app.execute(envelope(123_201 + index * 1_000, Actor::Human, command));
        if retry_succeeds {
            assert!(retry.is_ok());
            assert_eq!(app.count_rows("memory_entry_versions"), 1);
        } else {
            assert_eq!(
                retry,
                Err(AppError::Domain(
                    DomainError::MemoryProposalReviewUnavailable
                ))
            );
            assert_eq!(app.count_rows("memory_entry_versions"), 0);
        }
    }
}

#[test]
fn accepted_create_overwrite_delete_and_recreate_reuse_the_logical_entry_and_exact_id_budget() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 125_000);

    let before = app.ids.calls();
    let proposal = propose_set(
        &mut app,
        &profile,
        125_100,
        ExpectedMemoryEntryState::Absent,
        "Accepted create.",
    );
    let created = accept(&mut app, 125_200, proposal);
    let created = created.entry.unwrap();
    assert_eq!(created.version().get(), 1);
    assert_eq!(app.ids.calls(), before + 7);

    let before = app.ids.calls();
    let proposal = propose_set(
        &mut app,
        &profile,
        125_300,
        ExpectedMemoryEntryState::Present(created.clone()),
        "Accepted overwrite.",
    );
    let overwritten = accept(&mut app, 125_400, proposal).entry.unwrap();
    assert_eq!(overwritten.entry_id(), created.entry_id());
    assert_eq!(overwritten.version().get(), 2);
    assert_eq!(app.ids.calls(), before + 6);

    let before = app.ids.calls();
    let proposal = propose_delete(&mut app, &profile, 125_500, overwritten.clone());
    let deleted = accept(&mut app, 125_600, proposal).entry.unwrap();
    assert_eq!(deleted.entry_id(), created.entry_id());
    assert_eq!(deleted.version().get(), 3);
    assert_eq!(
        deleted.state(),
        ai_stock_forum::memory::MemoryEntryState::Deleted
    );
    assert_eq!(app.ids.calls(), before + 6);

    let before = app.ids.calls();
    let proposal = propose_set(
        &mut app,
        &profile,
        125_700,
        ExpectedMemoryEntryState::Deleted(deleted),
        "Accepted recreation.",
    );
    let recreated = accept(&mut app, 125_800, proposal).entry.unwrap();
    assert_eq!(recreated.entry_id(), created.entry_id());
    assert_eq!(recreated.version().get(), 4);
    assert_eq!(
        recreated.state(),
        ai_stock_forum::memory::MemoryEntryState::Present
    );
    assert_eq!(app.ids.calls(), before + 6);
}

#[test]
fn acceptance_capacity_applies_only_to_absent_or_tombstone_to_present_transitions() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 126_000);
    let namespace = profile.memory_namespace_id();
    let mut entries = (0_u128..1_024)
        .map(|index| {
            MemoryEntryVersion::create_present(
                namespace,
                MemoryEntryId::from_uuid(Uuid::from_u128(3_000_000 + index)),
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(3_100_000 + index)),
                MemoryEntryDraft::new(
                    format!("Capacity key {index:04}"),
                    format!("Capacity value {index}."),
                    vec![],
                )
                .unwrap(),
                Actor::Human,
                1_500_000_000_000 + i64::try_from(index).unwrap(),
                None,
                EventId::from_uuid(Uuid::from_u128(3_200_000 + index)),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let tombstone_present = MemoryEntryVersion::create_present(
        namespace,
        MemoryEntryId::from_uuid(Uuid::from_u128(3_300_000)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(3_300_001)),
        MemoryEntryDraft::new("Capacity tombstone".into(), "Former value.".into(), vec![]).unwrap(),
        Actor::Human,
        1_500_000_010_000,
        None,
        EventId::from_uuid(Uuid::from_u128(3_300_002)),
    )
    .unwrap();
    let tombstone = tombstone_present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(3_300_003)),
            Actor::Human,
            1_500_000_010_001,
            None,
            EventId::from_uuid(Uuid::from_u128(3_300_004)),
        )
        .unwrap();
    entries.push(tombstone_present);
    entries.push(tombstone.clone());
    seed_entry_versions(&app, &entries);

    let absent = propose_set_draft(
        &mut app,
        &profile,
        126_100,
        ExpectedMemoryEntryState::Absent,
        MemoryEntryDraft::new("Overflow key".into(), "Overflow value.".into(), vec![]).unwrap(),
    );
    let absent_review = app
        .preview_memory_proposal_approval(absent.clone())
        .unwrap();
    let absent_command = ApplicationCommand::ApproveMemoryProposal {
        proposal: absent,
        approval_id: absent_review.approval_id,
        expected_approval_status: absent_review.expected_approval_status,
        expected_entry: absent_review.expected_entry,
        review_token: absent_review.review_token,
        review_digest: absent_review.review_digest,
    };
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    assert_eq!(
        app.execute(envelope(126_200, Actor::Human, absent_command)),
        Err(AppError::Persistence(PersistenceError::Capacity))
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);

    let recreate = propose_set_draft(
        &mut app,
        &profile,
        126_300,
        ExpectedMemoryEntryState::Deleted(tombstone.reference()),
        MemoryEntryDraft::new(
            "Capacity tombstone".into(),
            "Recreated value.".into(),
            vec![],
        )
        .unwrap(),
    );
    let recreate_review = app
        .preview_memory_proposal_approval(recreate.clone())
        .unwrap();
    let recreate_command = ApplicationCommand::ApproveMemoryProposal {
        proposal: recreate.clone(),
        approval_id: recreate_review.approval_id,
        expected_approval_status: recreate_review.expected_approval_status,
        expected_entry: recreate_review.expected_entry,
        review_token: recreate_review.review_token,
        review_digest: recreate_review.review_digest,
    };
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    assert_eq!(
        app.execute(envelope(126_400, Actor::Human, recreate_command.clone(),)),
        Err(AppError::Persistence(PersistenceError::Capacity))
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);

    let overwrite = propose_set_draft(
        &mut app,
        &profile,
        126_500,
        ExpectedMemoryEntryState::Present(entries[1].reference()),
        MemoryEntryDraft::new(
            "Capacity key 0001".into(),
            "Overwrite at capacity.".into(),
            vec![],
        )
        .unwrap(),
    );
    assert!(accept(&mut app, 126_600, overwrite).entry.is_some());
    assert_eq!(
        MemoryRepository::count_active_entries(
            &app.open_database().immediate_transaction().unwrap(),
            namespace
        )
        .unwrap(),
        1_024
    );

    let deletion = propose_delete(&mut app, &profile, 126_700, entries[2].reference());
    let deleted = accept(&mut app, 126_800, deletion).entry.unwrap();
    assert_eq!(deleted.state(), MemoryEntryState::Deleted);

    let recreated = accept(&mut app, 126_900, recreate).entry.unwrap();
    assert_eq!(recreated.entry_id(), tombstone.reference().entry_id());
    assert_eq!(recreated.state(), MemoryEntryState::Present);
}

#[test]
fn runtime_preview_and_resolution_execution_share_the_resolution_review_registry() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 127_000);
    let proposal = propose_set(
        &mut app,
        &profile,
        127_100,
        ExpectedMemoryEntryState::Absent,
        "Runtime reviewed proposal.",
    );
    let fixture = app.into_runtime();
    let client = fixture.client();
    let review = client
        .preview_memory_proposal_approval(proposal.clone())
        .unwrap();
    let outcome = client
        .submit(ApplicationCommand::ApproveMemoryProposal {
            proposal,
            approval_id: review.approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        })
        .unwrap();
    assert!(matches!(
        outcome.view,
        CommandView::MemoryProposalResolution(ref view)
            if view.resolution.status()
                == ai_stock_forum::memory::MemoryProposalStatus::Accepted
    ));
    fixture.finish_and_join(ai_stock_forum::app::ShutdownReason::UserQuit);
}

#[test]
fn concurrent_current_state_change_makes_the_bound_resolution_stale_and_one_use() {
    let mut app = support::app();
    let profile = create_profile(&mut app, 128_000);
    let proposal = propose_set(
        &mut app,
        &profile,
        128_100,
        ExpectedMemoryEntryState::Absent,
        "This proposal becomes stale.",
    );
    let review = app
        .preview_memory_proposal_approval(proposal.clone())
        .unwrap();
    let command = ApplicationCommand::ApproveMemoryProposal {
        proposal,
        approval_id: review.approval_id,
        expected_approval_status: review.expected_approval_status,
        expected_entry: review.expected_entry,
        review_token: review.review_token,
        review_digest: review.review_digest,
    };

    let mut concurrent = app.independent_profile_instance().unwrap();
    let direct = match concurrent
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            candidate("A direct edit wins the race."),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    concurrent
        .execute(envelope(
            128_200,
            Actor::Human,
            ApplicationCommand::SetMemoryEntry {
                profile: direct.profile,
                expected: direct.expected,
                candidate: direct.candidate.unwrap(),
                review_token: direct.review_token,
                review_digest: direct.review_digest,
            },
        ))
        .unwrap();
    let before_ids = app.ids.calls();
    let before_clock = app.clock.calls();
    assert_eq!(
        app.execute(envelope(128_300, Actor::Human, command.clone())),
        Err(AppError::Domain(
            DomainError::MemoryProposalReviewUnavailable
        ))
    );
    assert_eq!(app.ids.calls(), before_ids);
    assert_eq!(app.clock.calls(), before_clock);
    assert_eq!(
        app.execute(envelope(128_301, Actor::Human, command)),
        Err(AppError::Domain(
            DomainError::MemoryProposalReviewUnavailable
        ))
    );
}

#[test]
fn proposal_creation_failure_rolls_back_event_approval_proposal_and_receipt_together() {
    let policy = Arc::new(support::RecordingPolicy::new(
        AuthorizationDecision::Granted,
    ));
    let hook = Arc::new(FailNextReceiptHook::new());
    let mut app = support::app_with_policy_and_hook(policy, hook.clone());
    let profile = create_profile(&mut app, 129_000);
    let before_events = app.count_rows("event_stream");
    let before_receipts = app.count_rows("command_receipts");
    hook.fail_next(PersistenceError::InvalidEventRecord);
    assert_eq!(
        app.execute(envelope(
            129_100,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: candidate("Rolled back proposal."),
                },
                rationale: "Injected failure must remain atomic.".into(),
            },
        )),
        Err(AppError::Persistence(PersistenceError::InvalidEventRecord))
    );
    assert_eq!(app.count_rows("event_stream"), before_events);
    assert_eq!(app.count_rows("command_receipts"), before_receipts);
    assert_eq!(app.count_rows("memory_proposals"), 0);
    assert_eq!(app.count_rows("approval_records"), 0);
}

#[test]
fn proposal_creation_receipt_replay_remains_the_original_pending_view_after_live_acceptance() {
    let policy = Arc::new(support::RecordingPolicy::new(
        AuthorizationDecision::Granted,
    ));
    let mut app = support::app_with_policy(policy.clone());
    let profile = create_profile(&mut app, 130_000);
    let creation = envelope(
        130_100,
        Actor::Agent(profile.profile_id()),
        ApplicationCommand::ProposeMemoryMutation {
            proposer: profile.reference(),
            expected: ExpectedMemoryEntryState::Absent,
            operation: MemoryProposalOperation::Set {
                candidate: candidate("Historically pending proposal view."),
            },
            rationale: "Replay is tied to the creation event.".into(),
        },
    );
    let created = app.execute(creation.clone()).unwrap();
    let proposal = match &created.view {
        CommandView::MemoryProposalCreated(view) => view.proposal.clone(),
        other => panic!("unexpected view {other:?}"),
    };
    accept(&mut app, 130_200, proposal);
    policy.set_decision(AuthorizationDecision::Denied(PolicyDecision::Denied));
    let calls = policy.calls();
    let ids = app.ids.calls();
    let clock = app.clock.calls();

    let replay = app.execute(creation).unwrap();
    assert_eq!(replay, created);
    assert!(matches!(
        replay.view,
        CommandView::MemoryProposalCreated(ref view)
            if view.status == ai_stock_forum::memory::MemoryProposalStatus::Pending
    ));
    assert_eq!(policy.calls(), calls);
    assert_eq!(app.ids.calls(), ids);
    assert_eq!(app.clock.calls(), clock);
}
