mod support;

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AppError, ApplicationCommand, ApplicationEvent, AuthorizationDecision, CommandEnvelope,
        CommandOutcome, CommandView, EVENT_SCHEMA_VERSION, MemoryEntryMutationView,
        MemoryProposalCreatedView, MemoryProposalResolutionView, PendingEvent, ShutdownDisposition,
        ShutdownReason,
    },
    domain::{
        Actor, ApprovalId, CausationId, CommandId, CorrelationId, EventId, MemoryEntryId,
        MemoryEntryVersionId, MemoryProposalId, ObjectRef, canonical_json_bytes, sha256,
    },
    memory::{
        ExpectedMemoryEntryState, MemoryEntryDraft, MemoryEntryState, MemoryEntryVersion,
        MemoryProposal, MemoryProposalFilter, MemoryProposalOperation, MemoryProposalResolution,
        MemoryProposalStatus, MemoryPurposeScope, MemoryRetrievalBudget, MemoryRetrievalRequest,
        MemoryRetrievalScope,
    },
    persistence::{
        CommandReceiptRecord, CommandReceiptRepository, Database, EventRepository,
        MemoryRepository, PersistenceError, ProjectionRepository,
    },
    policy::{ApprovalAction, ApprovalRecord, ApprovalStatus, Capability},
    recovery::reduce,
};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

fn envelope(id: u128, actor: Actor, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 20_000)),
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
                format!("Receipt Reader {id}"),
                "Receipt contract profile.".into(),
                AgentRole::Custom,
                "research".into(),
                vec![],
                "Careful.".into(),
                "Keep exact receipts.".into(),
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
        .find(|profile| profile.display_name() == format!("Receipt Reader {id}"))
        .unwrap()
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

fn seed_entry(app: &support::TestApp, entry: &MemoryEntryVersion) {
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let mut projection = ProjectionRepository::load_in(&tx).unwrap();
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
    ProjectionRepository::store(&tx, &projection).unwrap();
    tx.commit().unwrap();
}

#[derive(Serialize)]
struct ReceiptRequest<'a> {
    correlation_id: CorrelationId,
    actor: &'a Actor,
    command: &'a ApplicationCommand,
}

fn insert_success_receipt(
    app: &support::TestApp,
    command: &CommandEnvelope,
    event: ai_stock_forum::app::EventEnvelope,
    view: CommandView,
    capability: Capability,
) -> CommandOutcome {
    let outcome = CommandOutcome {
        command_id: command.command_id,
        correlation_id: command.correlation_id,
        committed_events: vec![event.clone()],
        view,
        shutdown: ShutdownDisposition::Continue,
    };
    let request_json = String::from_utf8(
        canonical_json_bytes(&ReceiptRequest {
            correlation_id: command.correlation_id,
            actor: &command.actor,
            command: &command.command,
        })
        .unwrap(),
    )
    .unwrap();
    let outcome_json = String::from_utf8(
        canonical_json_bytes(&serde_json::json!({
            "type": "success",
            "data": { "outcome": outcome.clone() },
        }))
        .unwrap(),
    )
    .unwrap();
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    CommandReceiptRepository::insert(
        &tx,
        &CommandReceiptRecord {
            command_id: command.command_id,
            command_fingerprint: sha256(request_json.as_bytes()),
            request_json,
            capability: match capability {
                Capability::MemoryRead => "memory_read",
                Capability::MemoryMutate => "memory_mutate",
                Capability::MemoryPropose => "memory_propose",
                Capability::MemoryResolve => "memory_resolve",
                _ => panic!("memory capability required"),
            }
            .into(),
            policy_decision: "granted".into(),
            outcome_json,
            event_ids: vec![event.event_id],
        },
    )
    .unwrap();
    tx.commit().unwrap();
    outcome
}

fn receipt_outcome_json(database: &Database, command_id: CommandId) -> String {
    database
        .connection()
        .query_row(
            "SELECT outcome_json FROM command_receipts WHERE command_id = ?1",
            [command_id.to_string()],
            |row| row.get(0),
        )
        .unwrap()
}

fn proposal_summary_rows(value: &mut serde_json::Value) -> &mut Vec<serde_json::Value> {
    value
        .pointer_mut("/data/outcome/view/data/proposals")
        .and_then(serde_json::Value::as_array_mut)
        .expect("stored proposal-list rows")
}

fn rewrite_first_proposal_summary(
    database: &Database,
    command_id: CommandId,
    rewrite: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) {
    database
        .connection()
        .execute_batch("DROP TRIGGER IF EXISTS command_receipts_no_update;")
        .unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(&receipt_outcome_json(database, command_id)).unwrap();
    rewrite(
        proposal_summary_rows(&mut value)[0]
            .as_object_mut()
            .expect("proposal summary object"),
    );
    let canonical = String::from_utf8(canonical_json_bytes(&value).unwrap()).unwrap();
    database
        .connection()
        .execute(
            "UPDATE command_receipts SET outcome_json = ?1 WHERE command_id = ?2",
            rusqlite::params![canonical, command_id.to_string()],
        )
        .unwrap();
}

fn pending_proposal(
    profile: &AgentProfileVersion,
    id: u128,
    event_id: EventId,
) -> (MemoryProposal, ApprovalRecord) {
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(id)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                "Receipt Proposal".into(),
                "proposal candidate plaintext".into(),
                vec![],
            )
            .unwrap(),
        },
        "Receipt Proposal".into(),
        ExpectedMemoryEntryState::Absent,
        "proposal rationale plaintext".into(),
        500,
        event_id,
        ApprovalId::from_uuid(Uuid::from_u128(id + 1)),
    )
    .unwrap();
    let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
        .approval_id(proposal.approval_id())
        .object(proposal.object_ref().unwrap())
        .actor(Actor::Agent(profile.profile_id()))
        .created_at_millis(proposal.created_at_ms())
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

fn commit_entry_command_event(
    app: &support::TestApp,
    command: &CommandEnvelope,
    entry: &MemoryEntryVersion,
) -> ai_stock_forum::app::EventEnvelope {
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
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let mut projection = ProjectionRepository::load_in(&tx).unwrap();
    let committed = EventRepository::append(
        &tx,
        PendingEvent {
            event_id: entry.creation_event_id(),
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: command.actor.clone(),
            occurred_at_ms: entry.created_at_ms(),
            correlation_id: command.correlation_id,
            causation_id: Some(CausationId::from_uuid(command.command_id.as_uuid())),
            object: Some(entry_object(entry)),
            event,
        },
    )
    .unwrap();
    reduce(&mut projection, &committed).unwrap();
    MemoryRepository::insert_entry_version(&tx, committed.sequence, entry).unwrap();
    MemoryRepository::replace_current_entry(&tx, entry).unwrap();
    ProjectionRepository::store(&tx, &projection).unwrap();
    tx.commit().unwrap();
    committed
}

#[test]
fn exact_memory_read_replay_returns_before_policy_dependencies_or_writes() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let profile = create_profile(&mut app, 200);
    let command = envelope(
        201,
        Actor::Human,
        ApplicationCommand::ListMemoryEntries {
            selector: profile.profile_id().into(),
        },
    );
    let original = app.execute(command.clone()).unwrap();
    let policy_calls = policy.calls();
    let clock_calls = app.clock.calls();
    let id_calls = app.ids.calls();
    let event_count = app.count_rows("event_stream");
    let receipt_count = app.count_rows("command_receipts");

    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));
    assert_eq!(app.execute(command.clone()).unwrap(), original);
    assert_eq!(policy.calls(), policy_calls);
    assert_eq!(app.clock.calls(), clock_calls);
    assert_eq!(app.ids.calls(), id_calls);
    assert_eq!(app.count_rows("event_stream"), event_count);
    assert_eq!(app.count_rows("command_receipts"), receipt_count);

    let mut changed_actor = command;
    changed_actor.actor = Actor::System;
    assert_eq!(app.execute(changed_actor), Err(AppError::CommandConflict));
    assert_eq!(policy.calls(), policy_calls);
}

#[test]
fn snapshot_replay_uses_historical_refs_and_accounting_after_current_memory_changes() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let profile = create_profile(&mut app, 300);
    let first = MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(5_000_000)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(5_000_001)),
        MemoryEntryDraft::new(
            "Replay Key".into(),
            "historical snapshot plaintext".into(),
            vec![],
        )
        .unwrap(),
        Actor::Human,
        100,
        None,
        EventId::from_uuid(Uuid::from_u128(5_000_002)),
    )
    .unwrap();
    seed_entry(&app, &first);
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(&profile, MemoryPurposeScope::General).unwrap(),
        MemoryRetrievalBudget::new(1, 0, 32_768, 0).unwrap(),
    )
    .unwrap();
    let command = envelope(
        301,
        Actor::Human,
        ApplicationCommand::BuildMemorySnapshot {
            request: request.clone(),
        },
    );
    let original = app.execute(command.clone()).unwrap();
    let CommandView::MemorySnapshot(original_view) = &original.view else {
        panic!("expected snapshot view");
    };
    assert_eq!(
        original_view.snapshot.entries()[0].entry(),
        &first.reference()
    );

    let successor = first
        .next_present(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(5_000_003)),
            MemoryEntryDraft::new(
                "Replay Key".into(),
                "new live snapshot plaintext".into(),
                vec![],
            )
            .unwrap(),
            Actor::Human,
            200,
            None,
            EventId::from_uuid(Uuid::from_u128(5_000_004)),
        )
        .unwrap();
    seed_entry(&app, &successor);
    let event_count = app.count_rows("event_stream");
    let receipt_count = app.count_rows("command_receipts");
    let policy_calls = policy.calls();
    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));

    assert_eq!(app.execute(command.clone()).unwrap(), original);
    assert_eq!(policy.calls(), policy_calls);
    assert_eq!(app.count_rows("event_stream"), event_count);
    assert_eq!(app.count_rows("command_receipts"), receipt_count);

    let changed_budget = envelope(
        301,
        Actor::Human,
        ApplicationCommand::BuildMemorySnapshot {
            request: MemoryRetrievalRequest::new(
                request.scope().clone(),
                MemoryRetrievalBudget::new(0, 0, 0, 0).unwrap(),
            )
            .unwrap(),
        },
    );
    assert_eq!(app.execute(changed_budget), Err(AppError::CommandConflict));
    assert_eq!(policy.calls(), policy_calls);
}

#[test]
fn proposal_created_receipt_replays_pending_after_live_terminal_resolution() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let profile = create_profile(&mut app, 400);
    let command_id = CommandId::from_uuid(Uuid::from_u128(401));
    let correlation_id = CorrelationId::from_uuid(Uuid::from_u128(40_401));
    let creation_event_id = EventId::from_uuid(Uuid::from_u128(6_000_000));
    let (proposal, approval) = pending_proposal(&profile, 6_000_100, creation_event_id);
    let command = CommandEnvelope {
        command_id,
        correlation_id,
        actor: Actor::Agent(profile.profile_id()),
        command: ApplicationCommand::ProposeMemoryMutation {
            proposer: profile.reference(),
            expected: ExpectedMemoryEntryState::Absent,
            operation: proposal.operation().clone(),
            rationale: proposal.rationale().into(),
        },
    };
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let mut projection = ProjectionRepository::load_in(&tx).unwrap();
    let created = EventRepository::append(
        &tx,
        PendingEvent {
            event_id: creation_event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: command.actor.clone(),
            occurred_at_ms: proposal.created_at_ms(),
            correlation_id,
            causation_id: Some(CausationId::from_uuid(command_id.as_uuid())),
            object: Some(proposal.object_ref().unwrap()),
            event: ApplicationEvent::MemoryProposalCreated {
                proposal: proposal.clone(),
                approval: approval.clone(),
            },
        },
    )
    .unwrap();
    reduce(&mut projection, &created).unwrap();
    MemoryRepository::insert_proposal_with_approval(&tx, created.sequence, &proposal, &approval)
        .unwrap();
    ProjectionRepository::store(&tx, &projection).unwrap();
    tx.commit().unwrap();
    let expected = insert_success_receipt(
        &app,
        &command,
        created,
        CommandView::MemoryProposalCreated(MemoryProposalCreatedView {
            proposal: proposal.reference(),
            approval_id: proposal.approval_id(),
            status: MemoryProposalStatus::Pending,
        }),
        Capability::MemoryPropose,
    );
    let list_command = envelope(
        402,
        Actor::Human,
        ApplicationCommand::ListMemoryProposals {
            selector: profile.profile_id().into(),
            filter: MemoryProposalFilter::Pending,
        },
    );
    let listed_pending = app.execute(list_command.clone()).unwrap();
    let show_command = envelope(
        403,
        Actor::Human,
        ApplicationCommand::ShowMemoryProposal {
            proposal_id: proposal.reference().proposal_id(),
        },
    );
    let shown_pending = app.execute(show_command.clone()).unwrap();

    let resolution_event_id = EventId::from_uuid(Uuid::from_u128(6_000_001));
    let resolution = MemoryProposalResolution::new(
        proposal.reference(),
        MemoryProposalStatus::Rejected,
        proposal.approval_id(),
        Actor::Human,
        600,
        resolution_event_id,
    )
    .unwrap();
    let terminal_approval = approval
        .resolve(ApprovalStatus::Rejected, Actor::Human, 600)
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    let mut projection = ProjectionRepository::load_in(&tx).unwrap();
    let rejected = EventRepository::append(
        &tx,
        PendingEvent {
            event_id: resolution_event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 600,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(40_402)),
            causation_id: None,
            object: Some(proposal.object_ref().unwrap()),
            event: ApplicationEvent::MemoryProposalRejected {
                resolution: resolution.clone(),
            },
        },
    )
    .unwrap();
    reduce(&mut projection, &rejected).unwrap();
    MemoryRepository::resolve_proposal(&tx, rejected.sequence, &resolution, &terminal_approval)
        .unwrap();
    ProjectionRepository::store(&tx, &projection).unwrap();
    tx.commit().unwrap();

    let policy_calls = policy.calls();
    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));
    assert_eq!(app.execute(command).unwrap(), expected);
    assert_eq!(app.execute(list_command).unwrap(), listed_pending);
    assert_eq!(app.execute(show_command).unwrap(), shown_pending);
    assert_eq!(policy.calls(), policy_calls);
}

#[test]
fn proposal_list_receipt_replays_exact_namespace_after_resolution_and_service_reopen() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let fixture = support::persistent_fixture();
    let mut first = fixture.service_with_policy(Arc::new(policy.clone()));
    let created_profile = first
        .execute(envelope(
            450,
            Actor::Human,
            ApplicationCommand::CreateAgentProfile {
                draft: AgentProfileDraft::new(
                    "Receipt Reopen Owner".into(),
                    "Receipt contract profile.".into(),
                    AgentRole::Custom,
                    "research".into(),
                    vec![],
                    "Careful.".into(),
                    "Keep exact receipts.".into(),
                    AgentBindings::default(),
                    vec![],
                    vec![],
                )
                .unwrap(),
                template_provenance: None,
            },
        ))
        .unwrap();
    let profile_id = match created_profile.view {
        CommandView::AgentProfileCreated(view) => view.profile_id,
        other => panic!("expected created profile, got {other:?}"),
    };
    let profile = fixture.active_profile(profile_id);
    let proposed = first
        .execute(envelope(
            451,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: MemoryEntryDraft::new(
                        "Receipt Namespace Proposal".into(),
                        "proposal candidate plaintext".into(),
                        vec![],
                    )
                    .unwrap(),
                },
                rationale: "receipt namespace rationale".into(),
            },
        ))
        .unwrap();
    let proposal = match proposed.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("expected proposal created, got {other:?}"),
    };
    let list_command = envelope(
        452,
        Actor::Human,
        ApplicationCommand::ListMemoryProposals {
            selector: profile.profile_id().into(),
            filter: MemoryProposalFilter::Pending,
        },
    );
    let historical = first.execute(list_command.clone()).unwrap();
    let CommandView::MemoryProposals(list) = &historical.view else {
        panic!("expected proposal list");
    };
    assert_eq!(list.proposals.len(), 1);
    assert_eq!(
        serde_json::to_value(&list.proposals[0]).unwrap()["namespace_id"],
        serde_json::Value::String(profile.memory_namespace_id().to_string())
    );
    let database = fixture.open_database();
    let mut stored: serde_json::Value =
        serde_json::from_str(&receipt_outcome_json(&database, list_command.command_id)).unwrap();
    assert_eq!(
        proposal_summary_rows(&mut stored)[0]["namespace_id"],
        serde_json::Value::String(profile.memory_namespace_id().to_string())
    );
    drop(database);

    let review = first
        .preview_memory_proposal_rejection(proposal.clone())
        .unwrap();
    let review_proposal = review.proposal.reference();
    first
        .execute(envelope(
            453,
            Actor::Human,
            ApplicationCommand::RejectMemoryProposal {
                proposal: review_proposal,
                approval_id: review.approval_id,
                expected_approval_status: review.expected_approval_status,
                expected_entry: review.expected_entry,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap();
    first.finish(ShutdownReason::UserQuit).unwrap();
    drop(first);

    let mut reopened = fixture.service_with_policy(Arc::new(policy.clone()));
    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));
    let side_effects = fixture.side_effect_calls();
    let policy_calls = policy.calls();
    let event_count = fixture.count_rows("event_stream");
    let receipt_count = fixture.count_rows("command_receipts");

    assert_eq!(reopened.execute(list_command).unwrap(), historical);
    assert_eq!(fixture.side_effect_calls(), side_effects);
    assert_eq!(policy.calls(), policy_calls);
    assert_eq!(fixture.count_rows("event_stream"), event_count);
    assert_eq!(fixture.count_rows("command_receipts"), receipt_count);
}

#[test]
fn proposal_list_receipt_missing_namespace_fails_before_dependencies_or_writes() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let profile = create_profile(&mut app, 460);
    let proposal = pending_proposal(
        &profile,
        6_100_100,
        EventId::from_uuid(Uuid::from_u128(6_100_000)),
    );
    seed_pending_proposals(&app, std::slice::from_ref(&proposal));
    let command = envelope(
        461,
        Actor::Human,
        ApplicationCommand::ListMemoryProposals {
            selector: profile.profile_id().into(),
            filter: MemoryProposalFilter::Pending,
        },
    );
    app.execute(command.clone()).unwrap();
    let database = app.open_database();
    rewrite_first_proposal_summary(&database, command.command_id, |summary| {
        assert!(
            summary.remove("namespace_id").is_some(),
            "namespace provenance must be required in stored proposal rows"
        );
    });
    drop(database);
    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));
    let database_before = app.raw_database_snapshot();
    let policy_calls = policy.calls();
    let clock_calls = app.clock.calls();
    let id_calls = app.ids.calls();

    assert_eq!(
        app.execute(command),
        Err(AppError::Persistence(PersistenceError::InvalidEventRecord))
    );
    assert_eq!(app.raw_database_snapshot(), database_before);
    assert_eq!(policy.calls(), policy_calls);
    assert_eq!(app.clock.calls(), clock_calls);
    assert_eq!(app.ids.calls(), id_calls);
}

#[test]
fn proposal_list_receipt_foreign_valid_namespace_fails_before_dependencies_or_writes() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let owner = create_profile(&mut app, 470);
    let other = create_profile(&mut app, 471);
    let proposal = pending_proposal(
        &owner,
        6_200_100,
        EventId::from_uuid(Uuid::from_u128(6_200_000)),
    );
    seed_pending_proposals(&app, std::slice::from_ref(&proposal));
    let command = envelope(
        472,
        Actor::Human,
        ApplicationCommand::ListMemoryProposals {
            selector: owner.profile_id().into(),
            filter: MemoryProposalFilter::Pending,
        },
    );
    app.execute(command.clone()).unwrap();
    let database = app.open_database();
    rewrite_first_proposal_summary(&database, command.command_id, |summary| {
        summary.insert(
            "namespace_id".into(),
            serde_json::Value::String(other.memory_namespace_id().to_string()),
        );
    });
    drop(database);
    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));
    let database_before = app.raw_database_snapshot();
    let policy_calls = policy.calls();
    let clock_calls = app.clock.calls();
    let id_calls = app.ids.calls();

    assert_eq!(
        app.execute(command),
        Err(AppError::Persistence(PersistenceError::InvalidEventRecord))
    );
    assert_eq!(app.raw_database_snapshot(), database_before);
    assert_eq!(policy.calls(), policy_calls);
    assert_eq!(app.clock.calls(), clock_calls);
    assert_eq!(app.ids.calls(), id_calls);
}

#[test]
fn direct_set_and_delete_receipts_replay_exact_immutable_versions() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let profile = create_profile(&mut app, 500);
    let candidate = MemoryEntryDraft::new(
        "Direct Replay".into(),
        "set receipt plaintext".into(),
        vec![],
    )
    .unwrap();
    let set_command = envelope(
        501,
        Actor::Human,
        ApplicationCommand::SetMemoryEntry {
            profile: profile.reference(),
            expected: ExpectedMemoryEntryState::Absent,
            candidate: candidate.clone(),
            review_token: ai_stock_forum::domain::MemoryReviewToken::from_uuid(Uuid::from_u128(
                7_000_000,
            )),
            review_digest: sha256(b"set replay review"),
        },
    );
    let set_entry = MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(7_000_001)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(7_000_002)),
        candidate,
        Actor::Human,
        700,
        None,
        EventId::from_uuid(Uuid::from_u128(7_000_003)),
    )
    .unwrap();
    let set_event = commit_entry_command_event(&app, &set_command, &set_entry);
    let set_expected = insert_success_receipt(
        &app,
        &set_command,
        set_event,
        CommandView::MemoryEntryMutation(MemoryEntryMutationView {
            entry: set_entry.reference(),
            expired_proposals: vec![],
        }),
        Capability::MemoryMutate,
    );

    let delete_command = envelope(
        502,
        Actor::Human,
        ApplicationCommand::DeleteMemoryEntry {
            profile: profile.reference(),
            expected: set_entry.reference(),
            review_token: ai_stock_forum::domain::MemoryReviewToken::from_uuid(Uuid::from_u128(
                7_000_004,
            )),
            review_digest: sha256(b"delete replay review"),
        },
    );
    let deleted = set_entry
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(7_000_005)),
            Actor::Human,
            701,
            None,
            EventId::from_uuid(Uuid::from_u128(7_000_006)),
        )
        .unwrap();
    let delete_event = commit_entry_command_event(&app, &delete_command, &deleted);
    let delete_expected = insert_success_receipt(
        &app,
        &delete_command,
        delete_event,
        CommandView::MemoryEntryMutation(MemoryEntryMutationView {
            entry: deleted.reference(),
            expired_proposals: vec![],
        }),
        Capability::MemoryMutate,
    );

    let policy_calls = policy.calls();
    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));
    assert_eq!(app.execute(set_command).unwrap(), set_expected);
    assert_eq!(app.execute(delete_command).unwrap(), delete_expected);
    assert_eq!(policy.calls(), policy_calls);
}

#[test]
fn accepted_and_rejected_proposal_receipts_replay_exact_terminal_outcomes() {
    for (case, status) in [
        MemoryProposalStatus::Accepted,
        MemoryProposalStatus::Rejected,
    ]
    .into_iter()
    .enumerate()
    {
        let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
        let mut app = support::app_with_policy(Arc::new(policy.clone()));
        let profile = create_profile(&mut app, 600 + case as u128);
        let creation_event_id = EventId::from_uuid(Uuid::from_u128(8_000_000 + case as u128 * 100));
        let (proposal, approval) =
            pending_proposal(&profile, 8_000_010 + case as u128 * 100, creation_event_id);
        let mut database = app.open_database();
        let tx = database.immediate_transaction().unwrap();
        let mut projection = ProjectionRepository::load_in(&tx).unwrap();
        let created = EventRepository::append(
            &tx,
            PendingEvent {
                event_id: creation_event_id,
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Agent(profile.profile_id()),
                occurred_at_ms: proposal.created_at_ms(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(80_000 + case as u128)),
                causation_id: None,
                object: Some(proposal.object_ref().unwrap()),
                event: ApplicationEvent::MemoryProposalCreated {
                    proposal: proposal.clone(),
                    approval: approval.clone(),
                },
            },
        )
        .unwrap();
        reduce(&mut projection, &created).unwrap();
        MemoryRepository::insert_proposal_with_approval(
            &tx,
            created.sequence,
            &proposal,
            &approval,
        )
        .unwrap();
        ProjectionRepository::store(&tx, &projection).unwrap();
        tx.commit().unwrap();

        let command = envelope(
            610 + case as u128,
            Actor::Human,
            match status {
                MemoryProposalStatus::Accepted => ApplicationCommand::ApproveMemoryProposal {
                    proposal: proposal.reference(),
                    approval_id: proposal.approval_id(),
                    expected_approval_status: ApprovalStatus::Pending,
                    expected_entry: ExpectedMemoryEntryState::Absent,
                    review_token: ai_stock_forum::domain::MemoryReviewToken::from_uuid(
                        Uuid::from_u128(8_000_020 + case as u128),
                    ),
                    review_digest: sha256(b"approve replay review"),
                },
                MemoryProposalStatus::Rejected => ApplicationCommand::RejectMemoryProposal {
                    proposal: proposal.reference(),
                    approval_id: proposal.approval_id(),
                    expected_approval_status: ApprovalStatus::Pending,
                    expected_entry: ExpectedMemoryEntryState::Absent,
                    review_token: ai_stock_forum::domain::MemoryReviewToken::from_uuid(
                        Uuid::from_u128(8_000_020 + case as u128),
                    ),
                    review_digest: sha256(b"reject replay review"),
                },
                _ => unreachable!(),
            },
        );
        let resolution_event_id = EventId::from_uuid(Uuid::from_u128(8_000_030 + case as u128));
        let resolution = MemoryProposalResolution::new(
            proposal.reference(),
            status,
            proposal.approval_id(),
            Actor::Human,
            800,
            resolution_event_id,
        )
        .unwrap();
        let terminal_approval = approval
            .resolve(
                match status {
                    MemoryProposalStatus::Accepted => ApprovalStatus::Accepted,
                    MemoryProposalStatus::Rejected => ApprovalStatus::Rejected,
                    _ => unreachable!(),
                },
                Actor::Human,
                800,
            )
            .unwrap();
        let accepted_entry = (status == MemoryProposalStatus::Accepted).then(|| {
            let MemoryProposalOperation::Set { candidate } = proposal.operation() else {
                unreachable!()
            };
            MemoryEntryVersion::create_present(
                profile.memory_namespace_id(),
                MemoryEntryId::from_uuid(Uuid::from_u128(8_000_040 + case as u128)),
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(8_000_050 + case as u128)),
                candidate.clone(),
                Actor::Human,
                800,
                Some(proposal.reference()),
                resolution_event_id,
            )
            .unwrap()
        });
        let terminal_event = match &accepted_entry {
            Some(entry) => ApplicationEvent::MemoryProposalAccepted {
                resolution: resolution.clone(),
                entry: entry.clone(),
                expired_proposals: vec![],
            },
            None => ApplicationEvent::MemoryProposalRejected {
                resolution: resolution.clone(),
            },
        };
        let tx = database.immediate_transaction().unwrap();
        let mut projection = ProjectionRepository::load_in(&tx).unwrap();
        let committed = EventRepository::append(
            &tx,
            PendingEvent {
                event_id: resolution_event_id,
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 800,
                correlation_id: command.correlation_id,
                causation_id: Some(CausationId::from_uuid(command.command_id.as_uuid())),
                object: Some(proposal.object_ref().unwrap()),
                event: terminal_event,
            },
        )
        .unwrap();
        reduce(&mut projection, &committed).unwrap();
        MemoryRepository::resolve_proposal(
            &tx,
            committed.sequence,
            &resolution,
            &terminal_approval,
        )
        .unwrap();
        if let Some(entry) = &accepted_entry {
            MemoryRepository::insert_entry_version(&tx, committed.sequence, entry).unwrap();
            MemoryRepository::replace_current_entry(&tx, entry).unwrap();
        }
        ProjectionRepository::store(&tx, &projection).unwrap();
        tx.commit().unwrap();
        let expected = insert_success_receipt(
            &app,
            &command,
            committed,
            CommandView::MemoryProposalResolution(MemoryProposalResolutionView {
                resolution: resolution.clone(),
                entry: accepted_entry.as_ref().map(MemoryEntryVersion::reference),
                expired_proposals: vec![],
            }),
            Capability::MemoryResolve,
        );

        let policy_calls = policy.calls();
        policy.set_decision(AuthorizationDecision::Denied(
            ai_stock_forum::policy::PolicyDecision::Denied,
        ));
        assert_eq!(app.execute(command).unwrap(), expected);
        assert_eq!(policy.calls(), policy_calls);
    }
}
