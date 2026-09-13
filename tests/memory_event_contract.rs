use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, MemoryProposalStatusRef},
    audit::AuditEntry,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CorrelationId, EpisodicSummaryId,
        EventId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId, MemoryProposalId,
    },
    memory::{
        EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState, MemoryEntryDraft,
        MemoryEntryVersion, MemoryProposal, MemoryProposalFilter, MemoryProposalOperation,
        MemoryProposalResolution, MemoryProposalStatus, MemoryPurposeScope, MemoryRetrievalBudget,
        MemoryRetrievalRequest, MemoryRetrievalScope, select_snapshot,
    },
    policy::{ApprovalAction, ApprovalRecord},
};
use uuid::Uuid;

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn profile() -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(uuid(1)),
        AgentProfileVersionId::from_uuid(uuid(2)),
        MemoryNamespaceId::from_uuid(uuid(3)),
        10,
        AgentProfileDraft::new(
            "Memory Agent".into(),
            "Memory contract fixture.".into(),
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
        None,
    )
    .unwrap()
}

struct Records {
    profile: AgentProfileVersion,
    entry: MemoryEntryVersion,
    deleted: MemoryEntryVersion,
    proposal: MemoryProposal,
    approval: ApprovalRecord,
    accepted: MemoryProposalResolution,
    rejected: MemoryProposalResolution,
    expired: MemoryProposalResolution,
    summary: EpisodicSummary,
}

fn records() -> Records {
    let profile = profile();
    let proposal_event = EventId::from_uuid(uuid(10));
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(uuid(11)),
        &profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                "Earnings Thesis".into(),
                "private proposed value".into(),
                vec!["Catalyst".into()],
            )
            .unwrap(),
        },
        "Earnings Thesis".into(),
        ExpectedMemoryEntryState::Absent,
        "private proposal rationale".into(),
        20,
        proposal_event,
        ApprovalId::from_uuid(uuid(12)),
    )
    .unwrap();
    let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
        .approval_id(proposal.approval_id())
        .object(proposal.object_ref().unwrap())
        .actor(Actor::Agent(profile.profile_id()))
        .created_at_millis(20)
        .build()
        .unwrap();
    let entry = MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(uuid(13)),
        MemoryEntryVersionId::from_uuid(uuid(14)),
        MemoryEntryDraft::new(
            "Earnings Thesis".into(),
            "private investment thesis".into(),
            vec!["Catalyst".into()],
        )
        .unwrap(),
        Actor::Human,
        30,
        Some(proposal.reference()),
        EventId::from_uuid(uuid(15)),
    )
    .unwrap();
    let deleted = entry
        .next_deleted(
            MemoryEntryVersionId::from_uuid(uuid(16)),
            Actor::Human,
            31,
            None,
            EventId::from_uuid(uuid(17)),
        )
        .unwrap();
    let resolution = |status, id| {
        MemoryProposalResolution::new(
            proposal.reference(),
            status,
            proposal.approval_id(),
            Actor::Human,
            30,
            EventId::from_uuid(uuid(id)),
        )
        .unwrap()
    };
    let summary_event = EventId::from_uuid(uuid(30));
    let summary = EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(uuid(31)),
        &profile,
        "private episodic label".into(),
        "private episodic body".into(),
        vec!["Catalyst".into()],
        vec![
            EpisodicSourceRef::new(
                1,
                EventId::from_uuid(uuid(29)),
                "help_viewed".into(),
                ai_stock_forum::domain::sha256(b"source"),
            )
            .unwrap(),
        ],
        40,
        2,
        summary_event,
    )
    .unwrap();
    let accepted = resolution(MemoryProposalStatus::Accepted, 21);
    let rejected = resolution(MemoryProposalStatus::Rejected, 22);
    let expired = resolution(MemoryProposalStatus::Expired, 23);
    Records {
        profile,
        entry,
        deleted,
        proposal,
        approval,
        accepted,
        rejected,
        expired,
        summary,
    }
}

fn snapshot_metadata(
    profile: &AgentProfileVersion,
) -> ai_stock_forum::memory::MemorySnapshotMetadata {
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(profile, MemoryPurposeScope::General).unwrap(),
        MemoryRetrievalBudget::default(),
    )
    .unwrap();
    select_snapshot(&request, std::iter::empty(), std::iter::empty())
        .unwrap()
        .metadata()
}

fn events() -> Vec<ApplicationEvent> {
    let records = records();
    let entry_ref = records.entry.reference();
    let summary_ref = records.summary.reference();
    let proposal_ref = records.proposal.reference();
    vec![
        ApplicationEvent::MemoryEntrySet {
            entry: records.entry.clone(),
            expired_proposals: vec![records.expired.clone()],
        },
        ApplicationEvent::MemoryEntryDeleted {
            entry: records.deleted,
            expired_proposals: vec![],
        },
        ApplicationEvent::MemoryProposalCreated {
            proposal: records.proposal,
            approval: records.approval,
        },
        ApplicationEvent::MemoryProposalAccepted {
            resolution: records.accepted,
            entry: records.entry,
            expired_proposals: vec![records.expired],
        },
        ApplicationEvent::MemoryProposalRejected {
            resolution: records.rejected.clone(),
        },
        ApplicationEvent::EpisodicSummaryRecorded {
            summary: records.summary,
        },
        ApplicationEvent::MemoryEntriesListed {
            profile: records.profile.reference(),
            namespace_id: records.profile.memory_namespace_id(),
            entries: vec![entry_ref.clone()],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        },
        ApplicationEvent::MemoryEntryShown {
            profile: records.profile.reference(),
            entry: entry_ref.clone(),
        },
        ApplicationEvent::MemoryEntryHistoryShown {
            profile: records.profile.reference(),
            current: entry_ref.clone(),
            versions: vec![entry_ref.clone()],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        },
        ApplicationEvent::MemoryEntryVersionShown {
            profile: records.profile.reference(),
            entry: entry_ref,
        },
        ApplicationEvent::MemoryProposalsListed {
            profile: records.profile.reference(),
            filter: MemoryProposalFilter::Pending,
            proposals: vec![MemoryProposalStatusRef {
                proposal: proposal_ref.clone(),
                status: MemoryProposalStatus::Pending,
            }],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        },
        ApplicationEvent::MemoryProposalShown {
            proposal: proposal_ref,
            status: MemoryProposalStatus::Rejected,
            resolution: Some(records.rejected),
        },
        ApplicationEvent::EpisodicSummariesListed {
            profile: records.profile.reference(),
            summaries: vec![summary_ref.clone()],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        },
        ApplicationEvent::EpisodicSummaryShown {
            summary: summary_ref,
        },
        ApplicationEvent::MemorySnapshotBuilt {
            metadata: snapshot_metadata(&records.profile),
        },
    ]
}

#[test]
fn all_memory_event_kinds_are_stable_and_schema_stays_v1() {
    assert_eq!(EVENT_SCHEMA_VERSION, 1);
    assert_eq!(
        events()
            .iter()
            .map(ApplicationEvent::kind)
            .collect::<Vec<_>>(),
        [
            "memory_entry_set",
            "memory_entry_deleted",
            "memory_proposal_created",
            "memory_proposal_accepted",
            "memory_proposal_rejected",
            "episodic_summary_recorded",
            "memory_entries_listed",
            "memory_entry_shown",
            "memory_entry_history_shown",
            "memory_entry_version_shown",
            "memory_proposals_listed",
            "memory_proposal_shown",
            "episodic_summaries_listed",
            "episodic_summary_shown",
            "memory_snapshot_built",
        ]
    );
}

#[test]
fn event_payloads_are_strict_and_legacy_unit_bytes_are_unchanged() {
    assert_eq!(
        serde_json::to_string(&ApplicationEvent::HelpViewed).unwrap(),
        r#"{"type":"help_viewed"}"#
    );
    for event in events() {
        let encoded = serde_json::to_value(&event).unwrap();
        assert_eq!(
            serde_json::from_value::<ApplicationEvent>(encoded.clone()).unwrap(),
            event
        );
        let mut forged = encoded;
        forged["data"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ApplicationEvent>(forged).is_err());
    }
}

#[test]
fn read_events_expose_only_references_counts_and_digests() {
    for event in events().into_iter().skip(6) {
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("private investment thesis"));
        assert!(!json.contains("private proposed value"));
        assert!(!json.contains("private proposal rationale"));
        assert!(!json.contains("private episodic label"));
        assert!(!json.contains("private episodic body"));
        assert!(!json.contains("help_viewed"));
    }
}

#[test]
fn mutation_events_retain_complete_immutable_records_but_audit_is_prose_free() {
    for (sequence, event) in events().into_iter().take(6).enumerate() {
        let payload = serde_json::to_string(&event).unwrap();
        let envelope = ai_stock_forum::app::EventEnvelope {
            sequence: sequence as u64 + 1,
            event_id: EventId::from_uuid(uuid(100 + sequence as u128)),
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 100,
            correlation_id: CorrelationId::from_uuid(uuid(200 + sequence as u128)),
            causation_id: None,
            object: None,
            event,
            previous_event_digest: None,
            event_digest: ai_stock_forum::domain::sha256(b"fixture"),
        };
        let audit = AuditEntry::from_event(&envelope);
        for prose in [
            "private investment thesis",
            "private proposed value",
            "private proposal rationale",
            "private episodic label",
            "private episodic body",
        ] {
            assert!(!audit.summary.contains(prose));
        }
        if matches!(
            envelope.event,
            ApplicationEvent::MemoryEntrySet { .. }
                | ApplicationEvent::MemoryProposalCreated { .. }
                | ApplicationEvent::MemoryProposalAccepted { .. }
                | ApplicationEvent::EpisodicSummaryRecorded { .. }
        ) {
            assert!(payload.contains("private"));
        }
    }
}

#[test]
fn proposal_status_reference_is_strict_and_contains_no_prose() {
    let records = records();
    let status = MemoryProposalStatusRef {
        proposal: records.proposal.reference(),
        status: MemoryProposalStatus::Pending,
    };
    let mut json = serde_json::to_value(&status).unwrap();
    assert!(!json.to_string().contains("private"));
    json["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<MemoryProposalStatusRef>(json).is_err());
}
