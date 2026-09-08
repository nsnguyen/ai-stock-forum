mod support;

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, ApplicationCommand, ApplicationEvent, AuthorizationDecision,
        CommandEnvelope, CommandView, EVENT_SCHEMA_VERSION, MemoryEntriesView, PendingEvent,
    },
    domain::{
        Actor, ApprovalId, CommandId, CorrelationId, Digest, EpisodicSummaryId, EventId,
        MemoryEntryId, MemoryEntryVersionId, MemoryProposalId, ObjectRef, ObjectVersion,
        canonical_json_bytes,
    },
    memory::{
        EpisodicQualification, EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState,
        MemoryEntryDraft, MemoryEntryState, MemoryEntryVersion, MemoryProposal,
        MemoryProposalFilter, MemoryProposalOperation, MemoryProposalResolution,
        MemoryProposalStatus, MemoryPurposeScope, MemoryRetrievalBudget, MemoryRetrievalRequest,
        MemoryRetrievalScope,
    },
    persistence::{EventRepository, MemoryRepository, ProjectionRepository},
    policy::{ApprovalAction, ApprovalRecord, ApprovalStatus},
    recovery::reduce,
};
use std::sync::Arc;
use uuid::Uuid;

fn envelope(id: u128, actor: Actor, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 10_000)),
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
                format!("Memory Reader {id}"),
                "Read contract profile.".into(),
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
        .find(|profile| profile.display_name() == format!("Memory Reader {id}"))
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

fn seed_entries(app: &support::TestApp, entries: &[MemoryEntryVersion]) {
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

fn present_entry(
    profile: &AgentProfileVersion,
    id: u128,
    display_key: String,
    value: String,
    tags: Vec<String>,
) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(id)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(id + 10_000)),
        MemoryEntryDraft::new(display_key, value, tags).unwrap(),
        Actor::Human,
        i64::try_from(id).unwrap(),
        None,
        EventId::from_uuid(Uuid::from_u128(id + 20_000)),
    )
    .unwrap()
}

fn pending_proposal(
    profile: &AgentProfileVersion,
    id: u128,
    key: &str,
    created_at_ms: i64,
) -> (MemoryProposal, ApprovalRecord) {
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(id)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                key.into(),
                format!("candidate plaintext {id}"),
                vec!["Catalyst".into()],
            )
            .unwrap(),
        },
        key.into(),
        ExpectedMemoryEntryState::Absent,
        format!("private rationale {id}"),
        created_at_ms,
        EventId::from_uuid(Uuid::from_u128(id + 10_000)),
        ApprovalId::from_uuid(Uuid::from_u128(id + 20_000)),
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

fn seed_proposal(
    app: &support::TestApp,
    proposal: &MemoryProposal,
    approval: &ApprovalRecord,
    terminal_status: Option<MemoryProposalStatus>,
) {
    let mut database = app.open_database();
    let tx = database.immediate_transaction().unwrap();
    let mut projection = ProjectionRepository::load_in(&tx).unwrap();
    let created = EventRepository::append(
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
    reduce(&mut projection, &created).unwrap();
    MemoryRepository::insert_proposal_with_approval(&tx, created.sequence, proposal, approval)
        .unwrap();
    if let Some(status) = terminal_status {
        let resolution_event_id = EventId::from_uuid(Uuid::from_u128(
            proposal.creation_event_id().as_uuid().as_u128() + 1,
        ));
        let resolution = MemoryProposalResolution::new(
            proposal.reference(),
            status,
            proposal.approval_id(),
            Actor::Human,
            proposal.created_at_ms() + 1,
            resolution_event_id,
        )
        .unwrap();
        let terminal_approval = approval
            .resolve(
                match status {
                    MemoryProposalStatus::Accepted => ApprovalStatus::Accepted,
                    MemoryProposalStatus::Rejected => ApprovalStatus::Rejected,
                    MemoryProposalStatus::Expired => ApprovalStatus::Expired,
                    MemoryProposalStatus::Pending => panic!("terminal status required"),
                },
                Actor::Human,
                proposal.created_at_ms() + 1,
            )
            .unwrap();
        let resolved = EventRepository::append(
            &tx,
            PendingEvent {
                event_id: resolution_event_id,
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: proposal.created_at_ms() + 1,
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(
                    resolution_event_id.as_uuid().as_u128() + 1_000_000,
                )),
                causation_id: None,
                object: Some(proposal.object_ref().unwrap()),
                event: ApplicationEvent::MemoryProposalRejected {
                    resolution: resolution.clone(),
                },
            },
        )
        .unwrap();
        reduce(&mut projection, &resolved).unwrap();
        MemoryRepository::resolve_proposal(&tx, resolved.sequence, &resolution, &terminal_approval)
            .unwrap();
    }
    ProjectionRepository::store(&tx, &projection).unwrap();
    tx.commit().unwrap();
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

fn seed_summary(
    app: &support::TestApp,
    profile: &AgentProfileVersion,
    id: u128,
    label: &str,
    body: &str,
    created_at_ms: i64,
) -> EpisodicSummary {
    let mut database = app.open_database();
    let (source_sequence, source_id, source_type, source_digest) = database
        .connection()
        .query_row(
            "SELECT sequence,event_id,event_type,event_digest FROM event_stream ORDER BY sequence LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .unwrap();
    let source = EpisodicSourceRef::new(
        u64::try_from(source_sequence).unwrap(),
        source_id.parse().unwrap(),
        source_type,
        Digest::parse(&source_digest).unwrap(),
    )
    .unwrap();
    let tx = database.immediate_transaction().unwrap();
    let mut projection = ProjectionRepository::load_in(&tx).unwrap();
    let creation_sequence = projection.last_sequence + 1;
    let creation_event_id = EventId::from_uuid(Uuid::from_u128(id + 10_000));
    let summary = EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::from_u128(id)),
        profile,
        label.into(),
        body.into(),
        vec!["Catalyst".into()],
        vec![source],
        created_at_ms,
        creation_sequence,
        creation_event_id,
    )
    .unwrap();
    let reference = summary.reference();
    let committed = EventRepository::append(
        &tx,
        PendingEvent {
            event_id: creation_event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::System,
            occurred_at_ms: created_at_ms,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 20_000)),
            causation_id: None,
            object: Some(
                ObjectRef::new(
                    "episodic_summary",
                    reference.summary_id().to_string(),
                    reference.version(),
                    reference.content_digest().clone(),
                )
                .unwrap(),
            ),
            event: ApplicationEvent::EpisodicSummaryRecorded {
                summary: summary.clone(),
            },
        },
    )
    .unwrap();
    assert_eq!(committed.sequence, creation_sequence);
    reduce(&mut projection, &committed).unwrap();
    ProjectionRepository::store(&tx, &projection).unwrap();
    tx.commit().unwrap();

    let profile_ref = reference.profile();
    database
        .connection()
        .execute(
            "INSERT INTO episodic_summaries (
                summary_id,version,memory_namespace_id,profile_id,profile_version_id,
                profile_version,profile_content_digest,label,body,purpose_tags_json,
                source_count,plaintext_validation_version,created_at_ms,creation_event_sequence,
                creation_event_id,source_set_digest,content_digest,record_digest,record_json
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
            rusqlite::params![
                reference.summary_id().to_string(),
                i64::try_from(reference.version().get()).unwrap(),
                reference.namespace_id().to_string(),
                profile_ref.profile_id().to_string(),
                profile_ref.profile_version_id().to_string(),
                i64::try_from(profile_ref.version().get()).unwrap(),
                profile_ref.content_digest().as_str(),
                summary.label(),
                summary.body(),
                canonical_json_bytes(&summary.purpose_tags().to_vec()).unwrap(),
                i64::try_from(summary.sources().len()).unwrap(),
                i64::from(summary.plaintext_validation_version()),
                summary.created_at_ms(),
                i64::try_from(summary.creation_event_sequence()).unwrap(),
                summary.creation_event_id().to_string(),
                summary.source_set_digest().as_str(),
                summary.content_digest().as_str(),
                summary.record_digest().as_str(),
                canonical_json_bytes(&summary).unwrap(),
            ],
        )
        .unwrap();
    for (ordinal, source) in summary.sources().iter().enumerate() {
        database
            .connection()
            .execute(
                "INSERT INTO episodic_summary_sources (
                    summary_id,source_ordinal,event_sequence,event_id,event_type,event_digest
                 ) VALUES (?1,?2,?3,?4,?5,?6)",
                rusqlite::params![
                    reference.summary_id().to_string(),
                    i64::try_from(ordinal).unwrap(),
                    i64::try_from(source.sequence()).unwrap(),
                    source.event_id().to_string(),
                    source.event_type(),
                    source.event_digest().as_str(),
                ],
            )
            .unwrap();
    }
    summary
}

#[test]
fn human_memory_list_read_commits_one_metadata_event_and_bounded_view() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy));
    let profile = create_profile(&mut app, 100);
    let event_count = app.count_rows("event_stream");
    let receipt_count = app.count_rows("command_receipts");

    let outcome = app
        .execute(envelope(
            101,
            Actor::Human,
            ApplicationCommand::ListMemoryEntries {
                selector: profile.profile_id().into(),
            },
        ))
        .unwrap();

    assert_eq!(outcome.committed_events.len(), 1);
    assert_eq!(
        outcome.committed_events[0].event.kind(),
        "memory_entries_listed"
    );
    assert_eq!(app.count_rows("event_stream"), event_count + 1);
    assert_eq!(app.count_rows("command_receipts"), receipt_count + 1);
    assert_eq!(
        outcome.view,
        CommandView::MemoryEntries(MemoryEntriesView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            entries: vec![],
            total_count: 0,
            returned_count: 0,
            omitted_count: 0,
        })
    );
}

#[test]
fn explicit_memory_policy_denial_wins_without_an_event() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let profile = create_profile(&mut app, 150);
    let events = app.count_rows("event_stream");
    let receipts = app.count_rows("command_receipts");
    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));

    assert_eq!(
        app.execute(envelope(
            151,
            Actor::Human,
            ApplicationCommand::ListMemoryEntries {
                selector: profile.profile_id().into(),
            },
        )),
        Err(ai_stock_forum::app::AppError::CapabilityDenied {
            capability: ai_stock_forum::policy::Capability::MemoryRead,
            decision: ai_stock_forum::policy::PolicyDecision::Denied,
        })
    );
    assert_eq!(app.count_rows("event_stream"), events);
    assert_eq!(app.count_rows("command_receipts"), receipts + 1);
}

#[test]
fn entry_reads_are_bounded_ordered_namespace_isolated_and_tombstone_aware() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy));
    let owner = create_profile(&mut app, 200);
    let other = create_profile(&mut app, 201);

    let removed = present_entry(
        &owner,
        1_000_000,
        "Removed Key".into(),
        "historical plaintext".into(),
        vec!["Archive".into()],
    );
    let tombstone = removed
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_020_001)),
            Actor::Human,
            1_000_001,
            None,
            EventId::from_uuid(Uuid::from_u128(1_030_001)),
        )
        .unwrap();
    let mut entries = vec![removed.clone(), tombstone.clone()];
    entries.extend((0..101_u128).map(|index| {
        present_entry(
            &owner,
            1_100_000 + index,
            format!("Key {index:03}"),
            format!("private value {index:03}"),
            if index == 0 {
                vec!["Catalyst".into()]
            } else {
                vec![]
            },
        )
    }));
    let other_entry = present_entry(
        &other,
        1_200_000,
        "Other Key".into(),
        "other profile secret".into(),
        vec![],
    );
    entries.push(other_entry.clone());
    seed_entries(&app, &entries);

    let list = app
        .execute(envelope(
            202,
            Actor::Human,
            ApplicationCommand::ListMemoryEntries {
                selector: AgentProfileSelector::from_input(owner.display_name()).unwrap(),
            },
        ))
        .unwrap();
    let CommandView::MemoryEntries(list) = list.view else {
        panic!("expected memory entries view");
    };
    assert_eq!(list.profile, owner.reference());
    assert_eq!(list.namespace_id, owner.memory_namespace_id());
    assert_eq!(
        (list.total_count, list.returned_count, list.omitted_count),
        (101, 100, 1)
    );
    assert_eq!(list.entries.first().unwrap().display_key, "Key 000");
    assert_eq!(list.entries.last().unwrap().display_key, "Key 099");
    let list_json = serde_json::to_string(&list).unwrap();
    assert!(!list_json.contains("private value"));
    assert!(!list_json.contains("other profile secret"));

    let isolated = app
        .execute(envelope(
            203,
            Actor::Human,
            ApplicationCommand::ListMemoryEntries {
                selector: other.profile_id().into(),
            },
        ))
        .unwrap();
    let CommandView::MemoryEntries(isolated) = isolated.view else {
        panic!("expected memory entries view");
    };
    assert_eq!(isolated.entries.len(), 1);
    assert_eq!(isolated.entries[0].entry, other_entry.reference());

    let not_found = app
        .execute(envelope(
            204,
            Actor::Human,
            ApplicationCommand::ShowMemoryEntry {
                selector: owner.profile_id().into(),
                display_key: "Removed Key".into(),
            },
        ))
        .unwrap_err();
    assert_eq!(not_found.code(), "memory_entry_not_found");
    assert!(!not_found.to_string().contains("Removed Key"));

    let history = app
        .execute(envelope(
            205,
            Actor::Human,
            ApplicationCommand::ShowMemoryEntryHistory {
                selector: owner.profile_id().into(),
                display_key: "Removed Key".into(),
            },
        ))
        .unwrap();
    let CommandView::MemoryEntryHistory(history) = history.view else {
        panic!("expected memory entry history view");
    };
    assert_eq!(history.current, tombstone.reference());
    assert_eq!(
        (
            history.total_count,
            history.returned_count,
            history.omitted_count
        ),
        (2, 2, 0)
    );
    assert_eq!(history.versions[0].entry, tombstone.reference());
    assert_eq!(history.versions[1].entry, removed.reference());
    assert!(
        !serde_json::to_string(&history)
            .unwrap()
            .contains("historical plaintext")
    );

    let historical = app
        .execute(envelope(
            206,
            Actor::Human,
            ApplicationCommand::ShowMemoryEntryVersion {
                selector: owner.profile_id().into(),
                display_key: "Removed Key".into(),
                version: ObjectVersion::new(1).unwrap(),
            },
        ))
        .unwrap();
    let CommandView::MemoryEntryVersion(historical) = historical.view else {
        panic!("expected memory entry version view");
    };
    assert_eq!(historical.entry, removed);
    assert_eq!(historical.entry.value(), Some("historical plaintext"));

    let current = app
        .execute(envelope(
            207,
            Actor::Human,
            ApplicationCommand::ShowMemoryEntry {
                selector: owner.profile_id().into(),
                display_key: "Key 000".into(),
            },
        ))
        .unwrap();
    let CommandView::MemoryEntry(current) = current.view else {
        panic!("expected memory entry view");
    };
    assert_eq!(current.entry.value(), Some("private value 000"));
}

#[test]
fn entry_history_has_an_exact_one_hundred_of_one_hundred_one_newest_prefix() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy));
    let owner = create_profile(&mut app, 250);
    let mut current = present_entry(
        &owner,
        1_500_000,
        "Long History".into(),
        "version 001".into(),
        vec![],
    );
    let mut versions = vec![current.clone()];
    for version in 2..=101_u128 {
        current = current
            .next_present(
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_510_000 + version)),
                MemoryEntryDraft::new(
                    "Long History".into(),
                    format!("version {version:03}"),
                    vec![],
                )
                .unwrap(),
                Actor::Human,
                i64::try_from(version).unwrap(),
                None,
                EventId::from_uuid(Uuid::from_u128(1_520_000 + version)),
            )
            .unwrap();
        versions.push(current.clone());
    }
    seed_entries(&app, &versions);

    let outcome = app
        .execute(envelope(
            251,
            Actor::Human,
            ApplicationCommand::ShowMemoryEntryHistory {
                selector: owner.profile_id().into(),
                display_key: "Long History".into(),
            },
        ))
        .unwrap();
    let CommandView::MemoryEntryHistory(history) = outcome.view else {
        panic!("expected memory entry history view");
    };
    assert_eq!(
        (
            history.total_count,
            history.returned_count,
            history.omitted_count
        ),
        (101, 100, 1)
    );
    assert_eq!(history.current.version().get(), 101);
    assert_eq!(history.versions.first().unwrap().entry.version().get(), 101);
    assert_eq!(history.versions.last().unwrap().entry.version().get(), 2);
    assert!(
        history
            .versions
            .windows(2)
            .all(|pair| { pair[0].entry.version().get() == pair[1].entry.version().get() + 1 })
    );
}

#[test]
fn proposal_reads_filter_order_redact_lists_and_expose_deliberate_detail() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy));
    let owner = create_profile(&mut app, 300);
    let (pending, pending_approval) = pending_proposal(&owner, 2_000_000, "Pending Key", 100);
    let (rejected, rejected_approval) = pending_proposal(&owner, 2_000_100, "Rejected Key", 200);
    seed_proposal(&app, &pending, &pending_approval, None);
    seed_proposal(
        &app,
        &rejected,
        &rejected_approval,
        Some(MemoryProposalStatus::Rejected),
    );

    let pending_list = app
        .execute(envelope(
            301,
            Actor::Human,
            ApplicationCommand::ListMemoryProposals {
                selector: owner.profile_id().into(),
                filter: MemoryProposalFilter::Pending,
            },
        ))
        .unwrap();
    let CommandView::MemoryProposals(pending_list) = pending_list.view else {
        panic!("expected memory proposals view");
    };
    assert_eq!(pending_list.proposals.len(), 1);
    assert_eq!(pending_list.proposals[0].proposal, pending.reference());
    assert_eq!(
        pending_list.proposals[0].status,
        MemoryProposalStatus::Pending
    );

    let all = app
        .execute(envelope(
            302,
            Actor::Human,
            ApplicationCommand::ListMemoryProposals {
                selector: owner.profile_id().into(),
                filter: MemoryProposalFilter::All,
            },
        ))
        .unwrap();
    let CommandView::MemoryProposals(all) = all.view else {
        panic!("expected memory proposals view");
    };
    assert_eq!(
        (all.total_count, all.returned_count, all.omitted_count),
        (2, 2, 0)
    );
    assert_eq!(all.proposals[0].proposal, rejected.reference());
    assert_eq!(all.proposals[1].proposal, pending.reference());
    let list_json = serde_json::to_string(&all).unwrap();
    assert!(!list_json.contains("candidate plaintext"));
    assert!(!list_json.contains("private rationale"));

    let shown = app
        .execute(envelope(
            303,
            Actor::Human,
            ApplicationCommand::ShowMemoryProposal {
                proposal_id: pending.reference().proposal_id(),
            },
        ))
        .unwrap();
    let CommandView::MemoryProposal(shown) = shown.view else {
        panic!("expected memory proposal view");
    };
    assert_eq!(shown.proposal, pending);
    assert_eq!(shown.status, MemoryProposalStatus::Pending);
    assert_eq!(shown.resolution, None);
    assert_eq!(shown.current_entry, ExpectedMemoryEntryState::Absent);
    assert!(!shown.proposer_is_historical);
    assert_eq!(shown.proposer_identity.profile, owner.reference());
    assert_eq!(shown.namespace_owner_identity.profile, owner.reference());
    let detail_json = serde_json::to_string(&shown).unwrap();
    assert!(detail_json.contains("candidate plaintext"));
    assert!(detail_json.contains("private rationale"));

    let mut successor = owner.to_draft();
    successor.description = "Activated after proposal creation.".into();
    let preview = app
        .preview_agent_profile_edit(
            owner.profile_id(),
            owner.profile_version_id(),
            successor.clone(),
        )
        .unwrap();
    app.execute(envelope(
        305,
        Actor::Human,
        ApplicationCommand::ActivateAgentProfileVersion {
            profile_id: owner.profile_id(),
            expected_active_version_id: owner.profile_version_id(),
            candidate: successor,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    ))
    .unwrap();
    let current_owner = app
        .projection()
        .agent_profiles
        .active_profile(owner.profile_id())
        .unwrap()
        .clone();
    let historical = app
        .execute(envelope(
            306,
            Actor::Human,
            ApplicationCommand::ShowMemoryProposal {
                proposal_id: pending.reference().proposal_id(),
            },
        ))
        .unwrap();
    let CommandView::MemoryProposal(historical) = historical.view else {
        panic!("expected memory proposal view");
    };
    assert!(historical.proposer_is_historical);
    assert_eq!(historical.proposer_identity.profile, owner.reference());
    assert_eq!(
        historical.namespace_owner_identity.profile,
        current_owner.reference()
    );

    let missing_id = MemoryProposalId::from_uuid(Uuid::from_u128(9_999_999));
    let not_found = app
        .execute(envelope(
            307,
            Actor::Human,
            ApplicationCommand::ShowMemoryProposal {
                proposal_id: missing_id,
            },
        ))
        .unwrap_err();
    assert_eq!(not_found.code(), "memory_proposal_not_found");
    assert!(!not_found.to_string().contains(&missing_id.to_string()));
}

#[test]
fn proposal_pending_and_all_lists_have_exact_independent_page_bounds_and_orders() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy));
    let owner = create_profile(&mut app, 350);
    let proposals = (0..101_u128)
        .map(|index| {
            pending_proposal(
                &owner,
                2_500_000 + index,
                &format!("Bounded Proposal {index:03}"),
                i64::try_from(index).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    seed_pending_proposals(&app, &proposals);

    for (id, filter, first, last) in [
        (351, MemoryProposalFilter::Pending, 0_usize, 99_usize),
        (352, MemoryProposalFilter::All, 100_usize, 1_usize),
    ] {
        let outcome = app
            .execute(envelope(
                id,
                Actor::Human,
                ApplicationCommand::ListMemoryProposals {
                    selector: owner.profile_id().into(),
                    filter,
                },
            ))
            .unwrap();
        let CommandView::MemoryProposals(view) = outcome.view else {
            panic!("expected memory proposals view");
        };
        assert_eq!(
            (view.total_count, view.returned_count, view.omitted_count),
            (101, 100, 1)
        );
        assert_eq!(view.proposals[0].proposal, proposals[first].0.reference());
        assert_eq!(view.proposals[99].proposal, proposals[last].0.reference());
    }
}

#[test]
fn episodic_reads_order_and_redact_lists_but_label_deliberate_plaintext_detail() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy));
    let owner = create_profile(&mut app, 400);
    let older = seed_summary(
        &app,
        &owner,
        3_000_000,
        "Older label",
        "older private episodic body",
        100,
    );
    let newer = seed_summary(
        &app,
        &owner,
        3_000_100,
        "Newer label",
        "newer private episodic body",
        200,
    );

    let listed = app
        .execute(envelope(
            401,
            Actor::Human,
            ApplicationCommand::ListEpisodicSummaries {
                selector: owner.profile_id().into(),
            },
        ))
        .unwrap();
    let CommandView::EpisodicSummaries(listed) = listed.view else {
        panic!("expected episodic summaries view");
    };
    assert_eq!(
        (
            listed.total_count,
            listed.returned_count,
            listed.omitted_count
        ),
        (2, 2, 0)
    );
    assert_eq!(listed.summaries[0].summary, newer.reference());
    assert_eq!(listed.summaries[1].summary, older.reference());
    let list_json = serde_json::to_string(&listed).unwrap();
    assert!(list_json.contains("Newer label"));
    assert!(!list_json.contains("private episodic body"));

    let shown = app
        .execute(envelope(
            402,
            Actor::Human,
            ApplicationCommand::ShowEpisodicSummary {
                summary_id: older.reference().summary_id(),
            },
        ))
        .unwrap();
    let CommandView::EpisodicSummary(shown) = shown.view else {
        panic!("expected episodic summary view");
    };
    assert_eq!(shown.summary, older);
    assert_eq!(
        shown.qualification,
        EpisodicQualification::SummaryVerifySources
    );
    assert!(
        serde_json::to_string(&shown)
            .unwrap()
            .contains("older private episodic body")
    );

    let missing_id = EpisodicSummaryId::from_uuid(Uuid::from_u128(9_999_998));
    let not_found = app
        .execute(envelope(
            403,
            Actor::Human,
            ApplicationCommand::ShowEpisodicSummary {
                summary_id: missing_id,
            },
        ))
        .unwrap_err();
    assert_eq!(not_found.code(), "episodic_summary_not_found");
    assert!(!not_found.to_string().contains(&missing_id.to_string()));

    let database = app.open_database();
    database
        .connection()
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             DROP TRIGGER episodic_summary_sources_no_update;",
        )
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE episodic_summary_sources SET event_digest=?1 WHERE summary_id=?2",
            rusqlite::params![
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                older.reference().summary_id().to_string(),
            ],
        )
        .unwrap();
    drop(database);
    let mismatch = app
        .execute(envelope(
            404,
            Actor::Human,
            ApplicationCommand::ShowEpisodicSummary {
                summary_id: older.reference().summary_id(),
            },
        ))
        .unwrap_err();
    assert_eq!(mismatch.code(), "memory_row_mismatch");
    assert!(!mismatch.to_string().contains("older private episodic body"));
}

#[test]
fn episodic_summary_list_has_exact_one_hundred_of_one_hundred_one_newest_order() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy));
    let owner = create_profile(&mut app, 450);
    let summaries = (0..101_u128)
        .map(|index| {
            seed_summary(
                &app,
                &owner,
                3_500_000 + index,
                &format!("Bounded Summary {index:03}"),
                &format!("private bounded summary body {index:03}"),
                i64::try_from(index).unwrap(),
            )
        })
        .collect::<Vec<_>>();

    let outcome = app
        .execute(envelope(
            451,
            Actor::Human,
            ApplicationCommand::ListEpisodicSummaries {
                selector: owner.profile_id().into(),
            },
        ))
        .unwrap();
    let CommandView::EpisodicSummaries(view) = outcome.view else {
        panic!("expected episodic summaries view");
    };
    assert_eq!(
        (view.total_count, view.returned_count, view.omitted_count),
        (101, 100, 1)
    );
    assert_eq!(view.summaries[0].summary, summaries[100].reference());
    assert_eq!(view.summaries[99].summary, summaries[1].reference());
    assert!(
        !serde_json::to_string(&view)
            .unwrap()
            .contains("private bounded summary body")
    );
}

#[test]
fn snapshot_is_built_in_transaction_with_deterministic_content_and_metadata_only_event() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy));
    let owner = create_profile(&mut app, 500);
    let general = present_entry(
        &owner,
        4_000_000,
        "General Key".into(),
        "general snapshot plaintext".into(),
        vec![],
    );
    let tagged = present_entry(
        &owner,
        4_000_100,
        "Tagged Key".into(),
        "tagged snapshot plaintext".into(),
        vec!["Catalyst".into()],
    );
    seed_entries(&app, &[general, tagged.clone()]);
    let summary = seed_summary(
        &app,
        &owner,
        4_000_200,
        "Snapshot summary",
        "snapshot episodic plaintext",
        300,
    );
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &owner,
            MemoryPurposeScope::tagged(vec!["Catalyst".into()]).unwrap(),
        )
        .unwrap(),
        MemoryRetrievalBudget::new(1, 1, 32_768, 128).unwrap(),
    )
    .unwrap();

    let outcome = app
        .execute(envelope(
            501,
            Actor::Human,
            ApplicationCommand::BuildMemorySnapshot {
                request: request.clone(),
            },
        ))
        .unwrap();
    let CommandView::MemorySnapshot(view) = &outcome.view else {
        panic!("expected memory snapshot view");
    };
    assert_eq!(view.snapshot.scope(), request.scope());
    assert_eq!(view.snapshot.budget(), request.budget());
    assert_eq!(view.snapshot.entries().len(), 1);
    assert_eq!(view.snapshot.entries()[0].entry(), &tagged.reference());
    assert_eq!(
        view.snapshot.entries()[0].value(),
        "tagged snapshot plaintext"
    );
    assert_eq!(view.snapshot.summaries().len(), 1);
    assert_eq!(view.snapshot.summaries()[0].summary(), &summary.reference());
    assert_eq!(
        view.snapshot.summaries()[0].body(),
        "snapshot episodic plaintext"
    );
    assert_eq!(view.snapshot.accounting().eligible_entry_count(), 2);
    assert_eq!(view.snapshot.accounting().omitted_entry_count(), 1);
    let event_json = serde_json::to_string(&outcome.committed_events[0]).unwrap();
    assert!(event_json.contains("memory_snapshot_built"));
    assert!(!event_json.contains("snapshot plaintext"));
    assert!(!event_json.contains("snapshot episodic plaintext"));
}

#[test]
fn all_eight_deliberate_read_pairings_commit_once_and_replay_exactly() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy));
    let owner = create_profile(&mut app, 550);
    let entry = present_entry(
        &owner,
        4_500_000,
        "Replay Reads".into(),
        "read pairing plaintext".into(),
        vec![],
    );
    seed_entries(&app, std::slice::from_ref(&entry));
    let (proposal, approval) = pending_proposal(&owner, 4_500_100, "Replay Proposal", 400);
    seed_proposal(&app, &proposal, &approval, None);
    let summary = seed_summary(
        &app,
        &owner,
        4_500_200,
        "Replay Summary",
        "read replay episodic plaintext",
        500,
    );
    let commands = vec![
        ApplicationCommand::ListMemoryEntries {
            selector: owner.profile_id().into(),
        },
        ApplicationCommand::ShowMemoryEntry {
            selector: owner.profile_id().into(),
            display_key: "Replay Reads".into(),
        },
        ApplicationCommand::ShowMemoryEntryHistory {
            selector: owner.profile_id().into(),
            display_key: "Replay Reads".into(),
        },
        ApplicationCommand::ShowMemoryEntryVersion {
            selector: owner.profile_id().into(),
            display_key: "Replay Reads".into(),
            version: ObjectVersion::new(1).unwrap(),
        },
        ApplicationCommand::ListMemoryProposals {
            selector: owner.profile_id().into(),
            filter: MemoryProposalFilter::Pending,
        },
        ApplicationCommand::ShowMemoryProposal {
            proposal_id: proposal.reference().proposal_id(),
        },
        ApplicationCommand::ListEpisodicSummaries {
            selector: owner.profile_id().into(),
        },
        ApplicationCommand::ShowEpisodicSummary {
            summary_id: summary.reference().summary_id(),
        },
    ];
    for (index, command) in commands.into_iter().enumerate() {
        let command = envelope(551 + index as u128, Actor::Human, command);
        let events_before = app.count_rows("event_stream");
        let receipts_before = app.count_rows("command_receipts");
        let original = app.execute(command.clone()).unwrap();
        assert_eq!(original.committed_events.len(), 1);
        assert_eq!(app.count_rows("event_stream"), events_before + 1);
        assert_eq!(app.count_rows("command_receipts"), receipts_before + 1);
        assert_eq!(app.execute(command).unwrap(), original);
        assert_eq!(app.count_rows("event_stream"), events_before + 1);
        assert_eq!(app.count_rows("command_receipts"), receipts_before + 1);
    }
}
