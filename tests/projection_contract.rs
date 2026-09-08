use ai_stock_forum::{
    app::{
        ApplicationEvent, AuditLimit, EVENT_SCHEMA_VERSION, EventEnvelope, InputRejection,
        InputRejectionCategory, PendingEvent, SafeToken, ShutdownReason,
    },
    config::AppPaths,
    domain::{
        Actor, ConfigurationVersionId, CorrelationId, EventId, InstallationId, ObjectVersion,
        SessionId, SetupDraftId, sha256,
    },
    persistence::{
        Database, EventRepository, PersistenceError, ProjectionRepository, RecoveryError,
    },
    recovery::{InstallationProjection, SessionEndProjection, SessionProjection},
    recovery::{ProjectionState, ReducerEffect, reduce},
    setup::{
        CapabilityReadiness, CapabilityReadinessStatus, InstallationConfigurationVersion,
        SetupDraft, SetupDraftState, SetupPath, SetupStatus, SetupStepOutcome, SetupStepStatus,
    },
};
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};
use tempfile::TempDir;
use uuid::Uuid;

fn database() -> (TempDir, Database) {
    let temporary_directory = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    (temporary_directory, database)
}

fn pending(event_id: u128, event: ApplicationEvent) -> PendingEvent {
    PendingEvent {
        event_id: EventId::from_uuid(Uuid::from_u128(event_id)),
        event_schema_version: EVENT_SCHEMA_VERSION,
        actor: Actor::System,
        occurred_at_ms: 1_700_000_000_000 + i64::try_from(event_id).unwrap(),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(100 + event_id)),
        causation_id: None,
        object: None,
        event,
    }
}

fn append(database: &mut Database, event_id: u128, event: ApplicationEvent) -> EventEnvelope {
    let transaction = database.immediate_transaction().unwrap();
    let envelope = EventRepository::append(&transaction, pending(event_id, event)).unwrap();
    transaction.commit().unwrap();
    envelope
}

fn installation_id() -> InstallationId {
    InstallationId::from_uuid(Uuid::from_u128(10))
}

fn session_id() -> SessionId {
    SessionId::from_uuid(Uuid::from_u128(20))
}

fn second_session_id() -> SessionId {
    SessionId::from_uuid(Uuid::from_u128(21))
}

fn installation_session_events(database: &mut Database) -> Vec<EventEnvelope> {
    vec![
        append(
            database,
            1,
            ApplicationEvent::InstallationInitialized {
                installation_id: installation_id(),
            },
        ),
        append(
            database,
            2,
            ApplicationEvent::ProcessSessionStarted {
                session_id: session_id(),
            },
        ),
        append(
            database,
            3,
            ApplicationEvent::ProcessSessionEnded {
                session_id: session_id(),
                reason: ShutdownReason::UserQuit,
            },
        ),
    ]
}

fn reduce_all(events: &[EventEnvelope]) -> ProjectionState {
    let mut state = ProjectionState::default();
    for event in events {
        reduce(&mut state, event).unwrap();
    }
    state
}

fn all_event_variants(database: &mut Database) -> Vec<EventEnvelope> {
    vec![
        append(
            database,
            1,
            ApplicationEvent::InstallationInitialized {
                installation_id: installation_id(),
            },
        ),
        append(
            database,
            2,
            ApplicationEvent::ProcessSessionStarted {
                session_id: session_id(),
            },
        ),
        append(database, 3, ApplicationEvent::HelpViewed),
        append(database, 4, ApplicationEvent::StatusViewed),
        append(database, 5, ApplicationEvent::SetupStatusViewed),
        append(
            database,
            6,
            ApplicationEvent::AuditTailViewed {
                limit: AuditLimit::new(1).unwrap(),
            },
        ),
        append(
            database,
            7,
            ApplicationEvent::CommandRejected {
                rejection: InputRejection::from_input(
                    InputRejectionCategory::Unknown,
                    Some(SafeToken::new("/unknown").unwrap()),
                    b"/unknown secret",
                ),
            },
        ),
        append(database, 8, ApplicationEvent::ShutdownRequested),
        append(
            database,
            9,
            ApplicationEvent::ProcessSessionEnded {
                session_id: session_id(),
                reason: ShutdownReason::InputClosed,
            },
        ),
        append(
            database,
            10,
            ApplicationEvent::ProcessSessionStarted {
                session_id: second_session_id(),
            },
        ),
        append(
            database,
            11,
            ApplicationEvent::PreviousSessionInterrupted {
                session_id: second_session_id(),
            },
        ),
        append(
            database,
            12,
            ApplicationEvent::ProjectionRebuilt {
                through_sequence: 11,
            },
        ),
    ]
}

#[test]
fn direct_reduction_rebuilds_installation_and_session_tombstone() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);

    let state = reduce_all(&events);

    assert_eq!(
        state.installation.as_ref().unwrap().installation_id,
        installation_id()
    );
    let session = state.sessions.get(&session_id()).unwrap();
    assert_eq!(session.started_event_id, events[1].event_id);
    assert_eq!(
        session.ended.as_ref().unwrap().reason,
        ShutdownReason::UserQuit
    );
    assert_eq!(state.setup_status, SetupStatus::NotStarted);
    assert_eq!(state.last_sequence, 3);
    assert_eq!(
        state.last_event_digest.as_ref(),
        Some(&events[2].event_digest)
    );
}

#[test]
fn reducer_rejects_out_of_order_duplicate_illegal_and_future_events() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let mut state = ProjectionState::default();

    assert_eq!(
        reduce(&mut state, &events[1]),
        Err(RecoveryError::EventSequenceGap)
    );
    reduce(&mut state, &events[0]).unwrap();
    reduce(&mut state, &events[1]).unwrap();
    assert_eq!(
        reduce(&mut state, &events[1]),
        Err(RecoveryError::EventSequenceGap)
    );

    let mut illegal = events[2].clone();
    illegal.event = ApplicationEvent::ProcessSessionEnded {
        session_id: SessionId::from_uuid(Uuid::from_u128(21)),
        reason: ShutdownReason::UserQuit,
    };
    assert_eq!(
        reduce(&mut state, &illegal),
        Err(RecoveryError::InvalidEventRecord)
    );

    let mut future = events[2].clone();
    future.event_schema_version = EVENT_SCHEMA_VERSION + 1;
    assert_eq!(
        reduce(&mut state, &future),
        Err(RecoveryError::UnsupportedEventSchema)
    );
}

#[test]
fn projections_replay_deterministically_and_store_transactionally_idempotently() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let direct = reduce_all(&events);
    assert_eq!(
        reduce_all(&events).digest().unwrap(),
        direct.digest().unwrap()
    );

    let transaction = database.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &direct).unwrap();
    transaction.commit().unwrap();
    let first = ProjectionRepository::load(database.connection()).unwrap();

    let transaction = database.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &direct).unwrap();
    transaction.commit().unwrap();
    let repeated = ProjectionRepository::load(database.connection()).unwrap();
    assert_eq!(repeated.digest().unwrap(), first.digest().unwrap());
    assert_eq!(repeated.digest().unwrap(), direct.digest().unwrap());

    let (installation, created_event, created_at): (String, String, i64) = database
        .connection()
        .query_row(
            "SELECT installation_id, created_event_id, created_at_ms FROM installation_projection",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(installation, installation_id().to_string());
    assert_eq!(created_event, events[0].event_id.to_string());
    assert_eq!(created_at, events[0].occurred_at_ms);

    let (ended_event, ended_at, reason): (Option<String>, Option<i64>, Option<String>) = database
        .connection()
        .query_row(
            "SELECT ended_event_id, ended_at_ms, end_reason FROM process_session_projection WHERE session_id = ?1",
            [session_id().to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(ended_event, Some(events[2].event_id.to_string()));
    assert_eq!(ended_at, Some(events[2].occurred_at_ms));
    assert_eq!(reason.as_deref(), Some("user_quit"));
    let count: i64 = database
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM process_session_projection",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn rebuild_writes_the_same_projection_without_changing_event_authority() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let event_digest_before: String = database
        .connection()
        .query_row(
            "SELECT event_digest FROM event_stream WHERE sequence = 3",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let rebuilt = ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();

    assert_eq!(
        rebuilt.digest().unwrap(),
        reduce_all(&events).digest().unwrap()
    );
    assert_eq!(
        database
            .connection()
            .query_row(
                "SELECT event_digest FROM event_stream WHERE sequence = 3",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        event_digest_before
    );
}

#[test]
fn setup_and_projection_deserialization_reject_invalid_states() {
    assert!(
        serde_json::from_value::<SetupDraft>(json!({
            "draft_id": "00000000-0000-0000-0000-000000000030",
            "schema_version": 0,
            "state": "drafting",
            "path": SetupPath::QuickStart,
            "current_review_digest": null,
            "payload": {},
            "created_at_ms": 1,
            "updated_at_ms": 1
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ProjectionState>(json!({
            "installation": null,
            "sessions": {},
            "setup_status": "not_started",
            "last_sequence": 0,
            "last_event_digest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "previous_session_interrupted": false
        }))
        .is_err()
    );
}

mod memory_reduction {
    use ai_stock_forum::{
        agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
        app::{
            ApplicationEvent, EVENT_SCHEMA_VERSION, EventEnvelope, MemoryProposalStatusRef,
            PendingEvent,
        },
        domain::{
            Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CorrelationId,
            EpisodicSummaryId, EventId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId,
            MemoryProposalId, ObjectRef, canonical_json_bytes, sha256,
        },
        memory::{
            EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState, MemoryEntryDraft,
            MemoryEntryVersion, MemoryProposal, MemoryProposalFilter, MemoryProposalOperation,
            MemoryProposalResolution, MemoryProposalStatus, MemoryPurposeScope,
            MemoryRetrievalBudget, MemoryRetrievalRequest, MemoryRetrievalScope, select_snapshot,
        },
        persistence::{EventRepository, ProjectionRepository, RecoveryError},
        policy::{ApprovalAction, ApprovalRecord, ApprovalStatus},
        recovery::{ProjectionState, reduce},
    };
    use uuid::Uuid;

    fn uuid(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn profile(seed: u128) -> AgentProfileVersion {
        AgentProfileVersion::create(
            AgentProfileId::from_uuid(uuid(seed)),
            AgentProfileVersionId::from_uuid(uuid(seed + 1)),
            MemoryNamespaceId::from_uuid(uuid(seed + 2)),
            10,
            AgentProfileDraft::new(
                format!("Memory Agent {seed}"),
                "Fixture profile.".into(),
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

    fn event_id(value: u128) -> EventId {
        EventId::from_uuid(uuid(value))
    }

    fn envelope(
        state: &ProjectionState,
        id: u128,
        actor: Actor,
        occurred_at_ms: i64,
        object: Option<ObjectRef>,
        event: ApplicationEvent,
    ) -> EventEnvelope {
        let sequence = state.last_sequence + 1;
        EventEnvelope {
            sequence,
            event_id: event_id(id),
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor,
            occurred_at_ms,
            correlation_id: CorrelationId::from_uuid(uuid(10_000 + id)),
            causation_id: None,
            object,
            event,
            previous_event_digest: state.last_event_digest.clone(),
            event_digest: sha256(&sequence.to_be_bytes()),
        }
    }

    fn seed_profile(state: &mut ProjectionState, profile: &AgentProfileVersion, id: u128) {
        let event = envelope(
            state,
            id,
            Actor::Human,
            profile.created_at_ms(),
            None,
            ApplicationEvent::AgentProfileCreated {
                profile: profile.clone(),
            },
        );
        reduce(state, &event).unwrap();
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

    fn summary_object(summary: &EpisodicSummary) -> ObjectRef {
        let reference = summary.reference();
        ObjectRef::new(
            "episodic_summary",
            reference.summary_id().to_string(),
            reference.version(),
            reference.content_digest().clone(),
        )
        .unwrap()
    }

    fn proposal(
        profile: &AgentProfileVersion,
        id: u128,
        key: &str,
        expected: ExpectedMemoryEntryState,
    ) -> MemoryProposal {
        MemoryProposal::new(
            MemoryProposalId::from_uuid(uuid(id)),
            profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Set {
                candidate: MemoryEntryDraft::new(
                    key.into(),
                    format!("private candidate {id}"),
                    vec![],
                )
                .unwrap(),
            },
            key.into(),
            expected,
            format!("private rationale {id}"),
            i64::try_from(id).unwrap(),
            event_id(id),
            ApprovalId::from_uuid(uuid(1_000 + id)),
        )
        .unwrap()
    }

    fn approval(proposal: &MemoryProposal) -> ApprovalRecord {
        ApprovalRecord::builder(ApprovalAction::MemoryMutation)
            .approval_id(proposal.approval_id())
            .object(proposal.object_ref().unwrap())
            .actor(Actor::Agent(proposal.proposer().profile_id()))
            .created_at_millis(proposal.created_at_ms())
            .build()
            .unwrap()
    }

    fn create_proposal(state: &mut ProjectionState, proposal: &MemoryProposal) {
        let event = envelope(
            state,
            proposal.creation_event_id().as_uuid().as_u128(),
            Actor::Agent(proposal.proposer().profile_id()),
            proposal.created_at_ms(),
            Some(proposal.object_ref().unwrap()),
            ApplicationEvent::MemoryProposalCreated {
                proposal: proposal.clone(),
                approval: approval(proposal),
            },
        );
        reduce(state, &event).unwrap();
    }

    fn resolution(
        proposal: &MemoryProposal,
        status: MemoryProposalStatus,
        event: u128,
        time: i64,
    ) -> MemoryProposalResolution {
        MemoryProposalResolution::new(
            proposal.reference(),
            status,
            proposal.approval_id(),
            Actor::Human,
            time,
            event_id(event),
        )
        .unwrap()
    }

    fn direct_entry(
        profile: &AgentProfileVersion,
        entry_id: u128,
        version_id: u128,
        event: u128,
        time: i64,
        key: &str,
    ) -> MemoryEntryVersion {
        MemoryEntryVersion::create_present(
            profile.memory_namespace_id(),
            MemoryEntryId::from_uuid(uuid(entry_id)),
            MemoryEntryVersionId::from_uuid(uuid(version_id)),
            MemoryEntryDraft::new(
                key.into(),
                "private current value".into(),
                vec!["tag".into()],
            )
            .unwrap(),
            Actor::Human,
            time,
            None,
            event_id(event),
        )
        .unwrap()
    }

    fn assert_invalid_unchanged(state: &ProjectionState, event: EventEnvelope) {
        let mut candidate = state.clone();
        assert_eq!(
            reduce(&mut candidate, &event),
            Err(RecoveryError::InvalidEventRecord)
        );
        assert_eq!(&candidate, state);
    }

    #[test]
    fn direct_entry_mutations_reduce_and_memory_reads_are_no_ops() {
        let profile = profile(1);
        let mut state = ProjectionState::default();
        seed_profile(&mut state, &profile, 19);
        let entry = direct_entry(&profile, 20, 21, 22, 22, "Earnings Thesis");
        let set = envelope(
            &state,
            22,
            Actor::Human,
            22,
            Some(entry_object(&entry)),
            ApplicationEvent::MemoryEntrySet {
                entry: entry.clone(),
                expired_proposals: vec![],
            },
        );
        reduce(&mut state, &set).unwrap();
        assert_eq!(
            state.memory.current_entry(
                profile.memory_namespace_id(),
                entry.reference().normalized_key()
            ),
            Some(&entry.reference())
        );

        let deleted = entry
            .next_deleted(
                MemoryEntryVersionId::from_uuid(uuid(23)),
                Actor::Human,
                24,
                None,
                event_id(24),
            )
            .unwrap();
        let delete = envelope(
            &state,
            24,
            Actor::Human,
            24,
            Some(entry_object(&deleted)),
            ApplicationEvent::MemoryEntryDeleted {
                entry: deleted.clone(),
                expired_proposals: vec![],
            },
        );
        reduce(&mut state, &delete).unwrap();
        assert_eq!(
            state.memory.current_entry(
                profile.memory_namespace_id(),
                deleted.reference().normalized_key()
            ),
            Some(&deleted.reference())
        );

        let before = state.memory.clone();
        let read = envelope(
            &state,
            25,
            Actor::Human,
            25,
            None,
            ApplicationEvent::MemoryEntryVersionShown {
                profile: profile.reference(),
                entry: deleted.reference(),
            },
        );
        reduce(&mut state, &read).unwrap();
        assert_eq!(state.memory, before);
    }

    #[test]
    fn proposal_create_accept_and_reject_validate_transitions() {
        let profile = profile(100);
        let accepted_proposal = proposal(
            &profile,
            120,
            "Accepted Key",
            ExpectedMemoryEntryState::Absent,
        );
        let mut accepted_state = ProjectionState::default();
        seed_profile(&mut accepted_state, &profile, 119);
        create_proposal(&mut accepted_state, &accepted_proposal);
        let accepted_resolution =
            resolution(&accepted_proposal, MemoryProposalStatus::Accepted, 121, 121);
        let accepted_entry = MemoryEntryVersion::create_present(
            profile.memory_namespace_id(),
            MemoryEntryId::from_uuid(uuid(122)),
            MemoryEntryVersionId::from_uuid(uuid(123)),
            MemoryEntryDraft::new("Accepted Key".into(), "private accepted".into(), vec![])
                .unwrap(),
            Actor::Human,
            121,
            Some(accepted_proposal.reference()),
            event_id(121),
        )
        .unwrap();
        let accepted = envelope(
            &accepted_state,
            121,
            Actor::Human,
            121,
            Some(accepted_proposal.object_ref().unwrap()),
            ApplicationEvent::MemoryProposalAccepted {
                resolution: accepted_resolution,
                entry: accepted_entry.clone(),
                expired_proposals: vec![],
            },
        );
        reduce(&mut accepted_state, &accepted).unwrap();
        assert_eq!(
            accepted_state.memory.current_entry(
                profile.memory_namespace_id(),
                accepted_entry.reference().normalized_key()
            ),
            Some(&accepted_entry.reference())
        );
        assert!(
            serde_json::to_string(&accepted_state.memory)
                .unwrap()
                .contains("Accepted")
        );

        let rejected_proposal = proposal(
            &profile,
            130,
            "Rejected Key",
            ExpectedMemoryEntryState::Absent,
        );
        let mut rejected_state = ProjectionState::default();
        seed_profile(&mut rejected_state, &profile, 129);
        create_proposal(&mut rejected_state, &rejected_proposal);
        let rejected = envelope(
            &rejected_state,
            131,
            Actor::Human,
            131,
            Some(rejected_proposal.object_ref().unwrap()),
            ApplicationEvent::MemoryProposalRejected {
                resolution: resolution(
                    &rejected_proposal,
                    MemoryProposalStatus::Rejected,
                    131,
                    131,
                ),
            },
        );
        reduce(&mut rejected_state, &rejected).unwrap();
        assert!(
            serde_json::to_string(&rejected_state.memory)
                .unwrap()
                .contains("Rejected")
        );
    }

    #[test]
    fn proposal_approval_and_terminal_resolution_links_are_exhaustive() {
        let profile = profile(150);
        let proposal = proposal(
            &profile,
            170,
            "Linked Proposal",
            ExpectedMemoryEntryState::Absent,
        );
        let mut base = ProjectionState::default();
        seed_profile(&mut base, &profile, 169);
        let build = |approval| {
            envelope(
                &base,
                170,
                Actor::Agent(profile.profile_id()),
                170,
                Some(proposal.object_ref().unwrap()),
                ApplicationEvent::MemoryProposalCreated {
                    proposal: proposal.clone(),
                    approval,
                },
            )
        };
        let wrong_id = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
            .approval_id(ApprovalId::from_uuid(uuid(9_170)))
            .object(proposal.object_ref().unwrap())
            .actor(Actor::Agent(profile.profile_id()))
            .created_at_millis(170)
            .build()
            .unwrap();
        let wrong_action = ApprovalRecord::builder(ApprovalAction::DiscussionRun)
            .approval_id(proposal.approval_id())
            .object(proposal.object_ref().unwrap())
            .actor(Actor::Agent(profile.profile_id()))
            .created_at_millis(170)
            .build()
            .unwrap();
        let wrong_object = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
            .approval_id(proposal.approval_id())
            .object(
                ObjectRef::new(
                    "memory_proposal",
                    MemoryProposalId::from_uuid(uuid(9_171)).to_string(),
                    proposal.reference().version(),
                    proposal.reference().content_digest().clone(),
                )
                .unwrap(),
            )
            .actor(Actor::Agent(profile.profile_id()))
            .created_at_millis(170)
            .build()
            .unwrap();
        let wrong_actor = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
            .approval_id(proposal.approval_id())
            .object(proposal.object_ref().unwrap())
            .actor(Actor::Human)
            .created_at_millis(170)
            .build()
            .unwrap();
        let wrong_time = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
            .approval_id(proposal.approval_id())
            .object(proposal.object_ref().unwrap())
            .actor(Actor::Agent(profile.profile_id()))
            .created_at_millis(171)
            .build()
            .unwrap();
        let expiring = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
            .approval_id(proposal.approval_id())
            .object(proposal.object_ref().unwrap())
            .actor(Actor::Agent(profile.profile_id()))
            .created_at_millis(170)
            .expires_at_millis(171)
            .build()
            .unwrap();
        let terminal = approval(&proposal)
            .resolve(ApprovalStatus::Accepted, Actor::Human, 171)
            .unwrap();
        for forged in [
            wrong_id,
            wrong_action,
            wrong_object,
            wrong_actor,
            wrong_time,
            expiring,
            terminal,
        ] {
            assert_invalid_unchanged(&base, build(forged));
        }

        let valid_create = build(approval(&proposal));
        let mut pending = base;
        reduce(&mut pending, &valid_create).unwrap();
        let accepted = resolution(&proposal, MemoryProposalStatus::Accepted, 171, 171);
        let unassociated_entry = MemoryEntryVersion::create_present(
            profile.memory_namespace_id(),
            MemoryEntryId::from_uuid(uuid(172)),
            MemoryEntryVersionId::from_uuid(uuid(173)),
            MemoryEntryDraft::new("Linked Proposal".into(), "private value".into(), vec![])
                .unwrap(),
            Actor::Human,
            171,
            None,
            event_id(171),
        )
        .unwrap();
        assert_invalid_unchanged(
            &pending,
            envelope(
                &pending,
                171,
                Actor::Human,
                171,
                Some(proposal.object_ref().unwrap()),
                ApplicationEvent::MemoryProposalAccepted {
                    resolution: accepted.clone(),
                    entry: unassociated_entry,
                    expired_proposals: vec![],
                },
            ),
        );
        let wrong_approval = MemoryProposalResolution::new(
            proposal.reference(),
            MemoryProposalStatus::Accepted,
            ApprovalId::from_uuid(uuid(9_172)),
            Actor::Human,
            171,
            event_id(171),
        )
        .unwrap();
        let associated_entry = MemoryEntryVersion::create_present(
            profile.memory_namespace_id(),
            MemoryEntryId::from_uuid(uuid(174)),
            MemoryEntryVersionId::from_uuid(uuid(175)),
            MemoryEntryDraft::new("Linked Proposal".into(), "private value".into(), vec![])
                .unwrap(),
            Actor::Human,
            171,
            Some(proposal.reference()),
            event_id(171),
        )
        .unwrap();
        assert_invalid_unchanged(
            &pending,
            envelope(
                &pending,
                171,
                Actor::Human,
                171,
                Some(proposal.object_ref().unwrap()),
                ApplicationEvent::MemoryProposalAccepted {
                    resolution: wrong_approval,
                    entry: associated_entry.clone(),
                    expired_proposals: vec![],
                },
            ),
        );
        let valid = envelope(
            &pending,
            171,
            Actor::Human,
            171,
            Some(proposal.object_ref().unwrap()),
            ApplicationEvent::MemoryProposalAccepted {
                resolution: accepted,
                entry: associated_entry,
                expired_proposals: vec![],
            },
        );
        reduce(&mut pending, &valid).unwrap();
        let retry = envelope(
            &pending,
            176,
            Actor::Human,
            176,
            Some(proposal.object_ref().unwrap()),
            ApplicationEvent::MemoryProposalRejected {
                resolution: resolution(&proposal, MemoryProposalStatus::Rejected, 176, 176),
            },
        );
        assert_invalid_unchanged(&pending, retry);
    }

    #[test]
    fn mutation_envelopes_require_exact_primary_object_actor_time_and_embedded_links() {
        let profile = profile(200);
        let entry = direct_entry(&profile, 220, 221, 222, 222, "Exact Entry");
        let valid_set = |state: &ProjectionState| {
            envelope(
                state,
                222,
                Actor::Human,
                222,
                Some(entry_object(&entry)),
                ApplicationEvent::MemoryEntrySet {
                    entry: entry.clone(),
                    expired_proposals: vec![],
                },
            )
        };
        let base = ProjectionState::default();
        let mut no_object = valid_set(&base);
        no_object.object = None;
        assert_invalid_unchanged(&base, no_object);
        let mut wrong_actor = valid_set(&base);
        wrong_actor.actor = Actor::System;
        assert_invalid_unchanged(&base, wrong_actor);
        let mut wrong_time = valid_set(&base);
        wrong_time.occurred_at_ms += 1;
        assert_invalid_unchanged(&base, wrong_time);

        let proposed = proposal(
            &profile,
            230,
            "Exact Proposal",
            ExpectedMemoryEntryState::Absent,
        );
        let valid_create = envelope(
            &base,
            230,
            Actor::Agent(profile.profile_id()),
            230,
            Some(proposed.object_ref().unwrap()),
            ApplicationEvent::MemoryProposalCreated {
                proposal: proposed.clone(),
                approval: ApprovalRecord::builder(ApprovalAction::DiscussionRun)
                    .approval_id(proposed.approval_id())
                    .object(proposed.object_ref().unwrap())
                    .actor(Actor::Agent(profile.profile_id()))
                    .created_at_millis(230)
                    .build()
                    .unwrap(),
            },
        );
        assert_invalid_unchanged(&base, valid_create);

        let mut summary_state = base.clone();
        seed_profile(&mut summary_state, &profile, 239);
        let source_event = envelope(
            &summary_state,
            240,
            Actor::Human,
            240,
            None,
            ApplicationEvent::HelpViewed,
        );
        reduce(&mut summary_state, &source_event).unwrap();
        let summary_sequence = summary_state.last_sequence + 1;
        let summary = EpisodicSummary::new(
            EpisodicSummaryId::from_uuid(uuid(241)),
            &profile,
            "private label".into(),
            "private body".into(),
            vec![],
            vec![
                EpisodicSourceRef::new(
                    summary_state.last_sequence,
                    event_id(240),
                    "help_viewed".into(),
                    sha256(b"source"),
                )
                .unwrap(),
            ],
            241,
            summary_sequence,
            event_id(241),
        )
        .unwrap();
        let summary_event = envelope(
            &summary_state,
            241,
            Actor::System,
            241,
            Some(summary_object(&summary)),
            ApplicationEvent::EpisodicSummaryRecorded { summary },
        );
        let mut wrong_summary_actor = summary_event.clone();
        wrong_summary_actor.actor = Actor::Human;
        assert_invalid_unchanged(&summary_state, wrong_summary_actor);
        let mut missing_summary_object = summary_event.clone();
        missing_summary_object.object = None;
        assert_invalid_unchanged(&summary_state, missing_summary_object);
        let mut wrong_summary_time = summary_event.clone();
        wrong_summary_time.occurred_at_ms += 1;
        assert_invalid_unchanged(&summary_state, wrong_summary_time);
        let memory_before = summary_state.memory.clone();
        reduce(&mut summary_state, &summary_event).unwrap();
        assert_eq!(summary_state.memory, memory_before);
    }

    #[test]
    fn sibling_expirations_are_exact_sorted_unique_and_same_key() {
        let profile = profile(300);
        let first = proposal(
            &profile,
            320,
            "Shared Key",
            ExpectedMemoryEntryState::Absent,
        );
        let second = proposal(
            &profile,
            321,
            "Shared Key",
            ExpectedMemoryEntryState::Absent,
        );
        let unrelated = proposal(&profile, 322, "Other Key", ExpectedMemoryEntryState::Absent);
        let mut state = ProjectionState::default();
        seed_profile(&mut state, &profile, 319);
        create_proposal(&mut state, &first);
        create_proposal(&mut state, &second);
        create_proposal(&mut state, &unrelated);
        let entry = direct_entry(&profile, 330, 331, 332, 332, "Shared Key");
        let first_expired = resolution(&first, MemoryProposalStatus::Expired, 332, 332);
        let second_expired = resolution(&second, MemoryProposalStatus::Expired, 332, 332);
        let make = |expired_proposals| {
            envelope(
                &state,
                332,
                Actor::Human,
                332,
                Some(entry_object(&entry)),
                ApplicationEvent::MemoryEntrySet {
                    entry: entry.clone(),
                    expired_proposals,
                },
            )
        };
        assert_invalid_unchanged(&state, make(vec![first_expired.clone()]));
        assert_invalid_unchanged(
            &state,
            make(vec![second_expired.clone(), first_expired.clone()]),
        );
        assert_invalid_unchanged(
            &state,
            make(vec![first_expired.clone(), first_expired.clone()]),
        );
        assert_invalid_unchanged(
            &state,
            make(vec![
                first_expired.clone(),
                second_expired.clone(),
                resolution(&unrelated, MemoryProposalStatus::Expired, 332, 332),
            ]),
        );
        let valid = make(vec![first_expired, second_expired]);
        reduce(&mut state, &valid).unwrap();
    }

    #[test]
    fn read_event_cross_field_invariants_fail_closed_without_mutating_state() {
        let owner = profile(400);
        let other_profile = profile(500);
        let entry = direct_entry(&owner, 420, 421, 422, 422, "Read Entry");
        let deleted = entry
            .next_deleted(
                MemoryEntryVersionId::from_uuid(uuid(423)),
                Actor::Human,
                423,
                None,
                event_id(423),
            )
            .unwrap();
        let mut state = ProjectionState::default();
        seed_profile(&mut state, &owner, 410);
        seed_profile(&mut state, &other_profile, 411);
        let set = envelope(
            &state,
            422,
            Actor::Human,
            422,
            Some(entry_object(&entry)),
            ApplicationEvent::MemoryEntrySet {
                entry: entry.clone(),
                expired_proposals: vec![],
            },
        );
        reduce(&mut state, &set).unwrap();
        let delete = envelope(
            &state,
            423,
            Actor::Human,
            423,
            Some(entry_object(&deleted)),
            ApplicationEvent::MemoryEntryDeleted {
                entry: deleted.clone(),
                expired_proposals: vec![],
            },
        );
        reduce(&mut state, &delete).unwrap();

        let mismatched_namespace = envelope(
            &state,
            430,
            Actor::Human,
            430,
            None,
            ApplicationEvent::MemoryEntriesListed {
                profile: owner.reference(),
                namespace_id: other_profile.memory_namespace_id(),
                entries: vec![],
                total_count: 0,
                returned_count: 0,
                omitted_count: 0,
            },
        );
        assert_invalid_unchanged(&state, mismatched_namespace);

        let deleted_current = envelope(
            &state,
            431,
            Actor::Human,
            431,
            None,
            ApplicationEvent::MemoryEntryShown {
                profile: owner.reference(),
                entry: deleted.reference(),
            },
        );
        assert_invalid_unchanged(&state, deleted_current);

        let wrong_profile_history = envelope(
            &state,
            432,
            Actor::Human,
            432,
            None,
            ApplicationEvent::MemoryEntryHistoryShown {
                profile: other_profile.reference(),
                current: deleted.reference(),
                versions: vec![deleted.reference(), entry.reference()],
                total_count: 2,
                returned_count: 2,
                omitted_count: 0,
            },
        );
        assert_invalid_unchanged(&state, wrong_profile_history);

        let proposed = proposal(
            &owner,
            440,
            "Read Proposal",
            ExpectedMemoryEntryState::Absent,
        );
        let wrong_filter = envelope(
            &state,
            440,
            Actor::Human,
            440,
            None,
            ApplicationEvent::MemoryProposalsListed {
                profile: owner.reference(),
                filter: MemoryProposalFilter::Pending,
                proposals: vec![MemoryProposalStatusRef {
                    proposal: proposed.reference(),
                    status: MemoryProposalStatus::Rejected,
                }],
                total_count: 1,
                returned_count: 1,
                omitted_count: 0,
            },
        );
        assert_invalid_unchanged(&state, wrong_filter);

        let mut bad_object = envelope(
            &state,
            441,
            Actor::Human,
            441,
            None,
            ApplicationEvent::MemoryEntryVersionShown {
                profile: owner.reference(),
                entry: entry.reference(),
            },
        );
        bad_object.object = Some(entry_object(&entry));
        assert_invalid_unchanged(&state, bad_object);
    }

    #[test]
    fn empty_memory_preserves_legacy_bytes_and_nonempty_memory_is_digest_material() {
        let empty = ProjectionState::default();
        let legacy = br#"{"installation":null,"last_event_digest":null,"last_sequence":0,"sessions":{},"setup_status":"not_started"}"#;
        assert_eq!(canonical_json_bytes(&empty).unwrap(), legacy);
        assert_eq!(empty.digest().unwrap(), sha256(legacy));
        assert_eq!(
            serde_json::from_slice::<ProjectionState>(legacy).unwrap(),
            empty
        );

        let profile = profile(600);
        let entry = direct_entry(&profile, 620, 621, 622, 622, "Digest Entry");
        let event = envelope(
            &empty,
            622,
            Actor::Human,
            622,
            Some(entry_object(&entry)),
            ApplicationEvent::MemoryEntrySet {
                entry,
                expired_proposals: vec![],
            },
        );
        let mut nonempty = empty.clone();
        reduce(&mut nonempty, &event).unwrap();
        let bytes = canonical_json_bytes(&nonempty).unwrap();
        assert!(std::str::from_utf8(&bytes).unwrap().contains("\"memory\""));
        assert_ne!(nonempty.digest().unwrap(), empty.digest().unwrap());
    }

    #[test]
    fn every_memory_read_event_validates_and_leaves_memory_unchanged() {
        let profile = profile(800);
        let mut state = ProjectionState::default();
        seed_profile(&mut state, &profile, 809);
        let entry = direct_entry(&profile, 820, 821, 822, 822, "Readable Entry");
        let set = envelope(
            &state,
            822,
            Actor::Human,
            822,
            Some(entry_object(&entry)),
            ApplicationEvent::MemoryEntrySet {
                entry: entry.clone(),
                expired_proposals: vec![],
            },
        );
        reduce(&mut state, &set).unwrap();
        let proposal = proposal(
            &profile,
            830,
            "Proposed Entry",
            ExpectedMemoryEntryState::Absent,
        );
        create_proposal(&mut state, &proposal);
        let summary = EpisodicSummary::new(
            EpisodicSummaryId::from_uuid(uuid(840)),
            &profile,
            "private label".into(),
            "private summary".into(),
            vec![],
            vec![
                EpisodicSourceRef::new(
                    1,
                    event_id(809),
                    "agent_profile_created".into(),
                    sha256(b"source"),
                )
                .unwrap(),
            ],
            840,
            2,
            event_id(840),
        )
        .unwrap();
        let request = MemoryRetrievalRequest::new(
            MemoryRetrievalScope::new(&profile, MemoryPurposeScope::General).unwrap(),
            MemoryRetrievalBudget::default(),
        )
        .unwrap();
        let snapshot = select_snapshot(&request, std::iter::empty(), std::iter::empty()).unwrap();
        let entry_ref = entry.reference();
        let proposal_ref = proposal.reference();
        let reads = vec![
            ApplicationEvent::MemoryEntriesListed {
                profile: profile.reference(),
                namespace_id: profile.memory_namespace_id(),
                entries: vec![entry_ref.clone()],
                total_count: 1,
                returned_count: 1,
                omitted_count: 0,
            },
            ApplicationEvent::MemoryEntryShown {
                profile: profile.reference(),
                entry: entry_ref.clone(),
            },
            ApplicationEvent::MemoryEntryHistoryShown {
                profile: profile.reference(),
                current: entry_ref.clone(),
                versions: vec![entry_ref.clone()],
                total_count: 1,
                returned_count: 1,
                omitted_count: 0,
            },
            ApplicationEvent::MemoryEntryVersionShown {
                profile: profile.reference(),
                entry: entry_ref,
            },
            ApplicationEvent::MemoryProposalsListed {
                profile: profile.reference(),
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
                status: MemoryProposalStatus::Pending,
                resolution: None,
            },
            ApplicationEvent::EpisodicSummariesListed {
                profile: profile.reference(),
                summaries: vec![summary.reference()],
                total_count: 1,
                returned_count: 1,
                omitted_count: 0,
            },
            ApplicationEvent::EpisodicSummaryShown {
                summary: summary.reference(),
            },
            ApplicationEvent::MemorySnapshotBuilt {
                metadata: snapshot.metadata(),
            },
        ];
        assert_eq!(reads.len(), 9);
        for (index, read) in reads.into_iter().enumerate() {
            let before = state.memory.clone();
            let event = envelope(
                &state,
                850 + index as u128,
                Actor::Human,
                850 + index as i64,
                None,
                read,
            );
            reduce(&mut state, &event).unwrap();
            assert_eq!(state.memory, before, "read event {index}");
        }
    }

    #[test]
    fn projection_repository_load_and_rebuild_preserve_reduced_memory() {
        let (_temporary_directory, mut database) = super::database();
        let profile = profile(700);
        let entry = direct_entry(&profile, 720, 721, 722, 722, "Persisted Entry");
        let event = ApplicationEvent::MemoryEntrySet {
            entry: entry.clone(),
            expired_proposals: vec![],
        };
        let transaction = database.immediate_transaction().unwrap();
        let committed = EventRepository::append(
            &transaction,
            PendingEvent {
                event_id: event_id(722),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 722,
                correlation_id: CorrelationId::from_uuid(uuid(10_722)),
                causation_id: None,
                object: Some(entry_object(&entry)),
                event,
            },
        )
        .unwrap();
        transaction.commit().unwrap();

        let mut expected = ProjectionState::default();
        reduce(&mut expected, &committed).unwrap();
        let transaction = database.immediate_transaction().unwrap();
        ProjectionRepository::store(&transaction, &expected).unwrap();
        transaction.commit().unwrap();
        let loaded = ProjectionRepository::load(database.connection()).unwrap();
        assert_eq!(loaded.memory, expected.memory);

        let rebuilt =
            ProjectionRepository::rebuild(database.connection_mut(), &[committed]).unwrap();
        assert_eq!(rebuilt.memory, expected.memory);
        assert_eq!(
            ProjectionRepository::load(database.connection())
                .unwrap()
                .memory,
            expected.memory
        );
    }
}

#[test]
fn reducer_accepts_every_valid_event_variant_without_startup_local_history() {
    let (_temporary_directory, mut database) = database();
    let events = all_event_variants(&mut database);

    let direct = reduce_all(&events);
    let rebuilt = ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();

    assert_eq!(direct, rebuilt);
    assert_eq!(direct.last_sequence, 12);
    assert_eq!(
        direct
            .sessions
            .get(&second_session_id())
            .unwrap()
            .ended
            .as_ref()
            .unwrap()
            .reason,
        ShutdownReason::Interrupted
    );
    assert_eq!(
        ProjectionRepository::load(database.connection()).unwrap(),
        direct
    );
}

#[test]
fn reducer_rejects_a_second_open_session_and_leaves_state_unchanged() {
    let (_temporary_directory, mut database) = database();
    let initialized = append(
        &mut database,
        1,
        ApplicationEvent::InstallationInitialized {
            installation_id: installation_id(),
        },
    );
    let started = append(
        &mut database,
        2,
        ApplicationEvent::ProcessSessionStarted {
            session_id: session_id(),
        },
    );
    let second_start = append(
        &mut database,
        3,
        ApplicationEvent::ProcessSessionStarted {
            session_id: second_session_id(),
        },
    );
    let mut state = ProjectionState::default();
    reduce(&mut state, &initialized).unwrap();
    reduce(&mut state, &started).unwrap();
    let before = state.clone();

    assert_eq!(
        reduce(&mut state, &second_start),
        Err(RecoveryError::InvalidEventRecord)
    );
    assert_eq!(state, before);
}

#[test]
fn reducer_reports_sequence_overflow_with_a_typed_error() {
    let (_temporary_directory, mut database) = database();
    let mut event = append(&mut database, 1, ApplicationEvent::HelpViewed);
    let marker = sha256(b"overflow-marker");
    event.sequence = u64::MAX;
    event.previous_event_digest = Some(marker.clone());
    let mut state = ProjectionState {
        last_sequence: u64::MAX,
        last_event_digest: Some(marker),
        ..ProjectionState::default()
    };

    let error = reduce(&mut state, &event).unwrap_err();
    assert_eq!(error.code(), "event_sequence_overflow");
}

#[test]
fn store_rejects_stale_fabricated_and_immutable_projection_overwrites() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let direct = reduce_all(&events);
    let transaction = database.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &direct).unwrap();
    transaction.commit().unwrap();

    let stale = reduce_all(&events[..2]);
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(
        ProjectionRepository::store(&transaction, &stale),
        Err(PersistenceError::ProjectionStateConflict)
    );
    transaction.rollback().unwrap();

    let mut changed_start = direct.clone();
    changed_start
        .sessions
        .get_mut(&session_id())
        .unwrap()
        .started_at_ms += 1;
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(
        ProjectionRepository::store(&transaction, &changed_start),
        Err(PersistenceError::ProjectionStateConflict)
    );
    transaction.rollback().unwrap();

    let mut removed_tombstone = direct.clone();
    removed_tombstone
        .sessions
        .get_mut(&session_id())
        .unwrap()
        .ended = None;
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(
        ProjectionRepository::store(&transaction, &removed_tombstone),
        Err(PersistenceError::ProjectionStateConflict)
    );
    transaction.rollback().unwrap();

    let mut rewritten_installation = direct.clone();
    rewritten_installation
        .installation
        .as_mut()
        .unwrap()
        .created_at_ms += 1;
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(
        ProjectionRepository::store(&transaction, &rewritten_installation),
        Err(PersistenceError::ProjectionStateConflict)
    );
    transaction.rollback().unwrap();

    let mut modified_tombstone = direct.clone();
    modified_tombstone
        .sessions
        .get_mut(&session_id())
        .unwrap()
        .ended
        .as_mut()
        .unwrap()
        .ended_at_ms += 1;
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(
        ProjectionRepository::store(&transaction, &modified_tombstone),
        Err(PersistenceError::ProjectionStateConflict)
    );
    transaction.rollback().unwrap();

    let next_event = append(&mut database, 4, ApplicationEvent::HelpViewed);
    let mut newer = direct.clone();
    reduce(&mut newer, &next_event).unwrap();
    let transaction = database.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &newer).unwrap();
    transaction.commit().unwrap();
    assert_eq!(
        ProjectionRepository::load(database.connection()).unwrap(),
        newer
    );
}

#[test]
fn load_rejects_partial_projection_rows_instead_of_treating_them_as_empty() {
    let (_temporary_directory, mut database) = database();
    let initialized = append(
        &mut database,
        1,
        ApplicationEvent::InstallationInitialized {
            installation_id: installation_id(),
        },
    );
    database
        .connection()
        .execute(
            "INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (1, ?1, ?2, ?3)",
            (installation_id().to_string(), initialized.event_id.to_string(), initialized.occurred_at_ms),
        )
        .unwrap();

    assert_eq!(
        ProjectionRepository::load(database.connection()),
        Err(RecoveryError::InvalidEventRecord)
    );
}

#[test]
fn setup_and_nested_projection_models_reject_unknown_deserialization_fields() {
    let digest = sha256(b"setup-model");
    let draft_id = SetupDraftId::from_uuid(Uuid::from_u128(30));
    let configuration_id = ConfigurationVersionId::from_uuid(Uuid::from_u128(31));
    let event_id = EventId::from_uuid(Uuid::from_u128(32));
    let mut draft = serde_json::to_value(
        SetupDraft::new(
            draft_id,
            1,
            SetupDraftState::Drafting,
            SetupPath::QuickStart,
            None,
            json!({}),
            1,
            1,
        )
        .unwrap(),
    )
    .unwrap();
    draft
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<SetupDraft>(draft).is_err());

    let mut configuration = serde_json::to_value(InstallationConfigurationVersion::new(
        configuration_id,
        ObjectVersion::new(1).unwrap(),
        draft_id,
        digest.clone(),
        digest.clone(),
        json!({}),
        event_id,
        1,
    ))
    .unwrap();
    configuration
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<InstallationConfigurationVersion>(configuration).is_err());

    let mut outcome = serde_json::to_value(
        SetupStepOutcome::new(
            draft_id,
            "connectivity".into(),
            1,
            SetupStepStatus::Passed,
            None,
            1,
        )
        .unwrap(),
    )
    .unwrap();
    outcome
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<SetupStepOutcome>(outcome).is_err());

    let mut readiness = serde_json::to_value(
        CapabilityReadiness::new(
            configuration_id,
            "help_read".into(),
            CapabilityReadinessStatus::Ready,
            None,
            1,
            digest,
        )
        .unwrap(),
    )
    .unwrap();
    readiness
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<CapabilityReadiness>(readiness).is_err());

    let mut installation = serde_json::to_value(InstallationProjection {
        installation_id: installation_id(),
        created_event_id: event_id,
        created_at_ms: 1,
    })
    .unwrap();
    installation
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<InstallationProjection>(installation).is_err());

    let mut end = serde_json::to_value(SessionEndProjection {
        ended_event_id: event_id,
        ended_at_ms: 2,
        reason: ShutdownReason::UserQuit,
    })
    .unwrap();
    end.as_object_mut()
        .unwrap()
        .insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<SessionEndProjection>(end).is_err());

    let mut session = serde_json::to_value(SessionProjection {
        session_id: session_id(),
        started_event_id: event_id,
        started_at_ms: 1,
        ended: None,
    })
    .unwrap();
    session
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), json!(true));
    assert!(serde_json::from_value::<SessionProjection>(session).is_err());
}

#[test]
fn event_append_and_projection_store_rollback_atomically() {
    let (_temporary_directory, mut database) = database();
    let transaction = database.immediate_transaction().unwrap();
    let event = EventRepository::append(
        &transaction,
        pending(
            1,
            ApplicationEvent::InstallationInitialized {
                installation_id: installation_id(),
            },
        ),
    )
    .unwrap();
    let mut state = ProjectionState::default();
    reduce(&mut state, &event).unwrap();
    ProjectionRepository::store(&transaction, &state).unwrap();
    transaction.rollback().unwrap();

    let event_count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM event_stream", [], |row| row.get(0))
        .unwrap();
    let projection_count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM projection_metadata", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!((event_count, projection_count), (0, 0));
}

#[test]
fn rebuild_acquires_an_immediate_snapshot_before_reading_the_stream() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut first = Database::open(&paths).unwrap();
    let mut second = Database::open(&paths).unwrap();
    second.connection().busy_timeout(Duration::ZERO).unwrap();
    let transaction = first.immediate_transaction().unwrap();

    assert_eq!(
        ProjectionRepository::rebuild(second.connection_mut(), &[]),
        Err(RecoveryError::QueryFailed)
    );
    transaction.rollback().unwrap();
}

#[test]
fn projection_state_deserialization_rejects_zero_marker_installations_and_multiple_open_sessions() {
    let event_id = EventId::from_uuid(Uuid::from_u128(40));
    let zero_marker_installation = ProjectionState {
        installation: Some(InstallationProjection {
            installation_id: installation_id(),
            created_event_id: event_id,
            created_at_ms: 1,
        }),
        ..ProjectionState::default()
    };
    assert!(
        serde_json::from_value::<ProjectionState>(
            serde_json::to_value(zero_marker_installation).unwrap()
        )
        .is_err()
    );

    let mut sessions = BTreeMap::new();
    for id in [session_id(), second_session_id()] {
        sessions.insert(
            id,
            SessionProjection {
                session_id: id,
                started_event_id: event_id,
                started_at_ms: 1,
                ended: None,
            },
        );
    }
    let multiple_open_sessions = ProjectionState {
        installation: Some(InstallationProjection {
            installation_id: installation_id(),
            created_event_id: event_id,
            created_at_ms: 1,
        }),
        sessions,
        agent_profiles: Default::default(),
        skills: Default::default(),
        memory: Default::default(),
        setup_status: SetupStatus::NotStarted,
        last_sequence: 1,
        last_event_digest: Some(sha256(b"marker")),
    };
    assert!(
        serde_json::from_value::<ProjectionState>(
            serde_json::to_value(multiple_open_sessions).unwrap()
        )
        .is_err()
    );
}

#[test]
fn newly_applied_interruption_returns_a_transient_effect_but_replay_does_not_store_it() {
    let (_temporary_directory, mut database) = database();
    let initialized = append(
        &mut database,
        1,
        ApplicationEvent::InstallationInitialized {
            installation_id: installation_id(),
        },
    );
    let started = append(
        &mut database,
        2,
        ApplicationEvent::ProcessSessionStarted {
            session_id: session_id(),
        },
    );
    let interrupted = append(
        &mut database,
        3,
        ApplicationEvent::PreviousSessionInterrupted {
            session_id: session_id(),
        },
    );
    let mut state = ProjectionState::default();
    reduce(&mut state, &initialized).unwrap();
    reduce(&mut state, &started).unwrap();

    assert_eq!(
        reduce(&mut state, &interrupted).unwrap(),
        ReducerEffect::PreviousSessionInterrupted {
            session_id: session_id()
        }
    );
    let replayed = reduce_all(&[initialized, started, interrupted]);
    assert_eq!(replayed, state);
    assert_eq!(
        serde_json::to_value(&replayed)
            .unwrap()
            .get("previous_session_interrupted"),
        None
    );
}

#[test]
fn projection_state_deserialization_independently_rejects_marker_digest_and_reachability_violations()
 {
    for value in [
        json!({"installation":null,"sessions":{},"setup_status":"not_started","last_sequence":0,"last_event_digest":sha256(b"x")} ),
        json!({"installation":null,"sessions":{},"setup_status":"not_started","last_sequence":1,"last_event_digest":null}),
        json!({"installation":null,"sessions":{},"setup_status":{"draft_saved":{"draft_id":"00000000-0000-0000-0000-000000000030"}},"last_sequence":1,"last_event_digest":sha256(b"x")}),
    ] {
        assert!(serde_json::from_value::<ProjectionState>(value).is_err());
    }
}

#[test]
fn interruption_rejects_wrong_target_and_already_ended_session_without_mutation() {
    let (_temporary_directory, mut fixture) = database();
    let initialized = append(
        &mut fixture,
        1,
        ApplicationEvent::InstallationInitialized {
            installation_id: installation_id(),
        },
    );
    let started = append(
        &mut fixture,
        2,
        ApplicationEvent::ProcessSessionStarted {
            session_id: session_id(),
        },
    );
    let mut state = reduce_all(&[initialized, started]);
    let before = state.clone();
    let wrong = append(
        &mut fixture,
        4,
        ApplicationEvent::PreviousSessionInterrupted {
            session_id: second_session_id(),
        },
    );
    assert_eq!(
        reduce(&mut state, &wrong),
        Err(RecoveryError::InvalidEventRecord)
    );
    assert_eq!(state, before);

    let (_other_temporary_directory, mut other_database) = database();
    let initialized = append(
        &mut other_database,
        1,
        ApplicationEvent::InstallationInitialized {
            installation_id: installation_id(),
        },
    );
    let started_a = append(
        &mut other_database,
        2,
        ApplicationEvent::ProcessSessionStarted {
            session_id: session_id(),
        },
    );
    let ended_a = append(
        &mut other_database,
        3,
        ApplicationEvent::ProcessSessionEnded {
            session_id: session_id(),
            reason: ShutdownReason::UserQuit,
        },
    );
    let started_b = append(
        &mut other_database,
        4,
        ApplicationEvent::ProcessSessionStarted {
            session_id: second_session_id(),
        },
    );
    let mut ended_state = reduce_all(&[initialized, started_a, ended_a, started_b]);
    let ended_before = ended_state.clone();
    let ended = append(
        &mut other_database,
        5,
        ApplicationEvent::PreviousSessionInterrupted {
            session_id: session_id(),
        },
    );
    assert_eq!(
        reduce(&mut ended_state, &ended),
        Err(RecoveryError::InvalidEventRecord)
    );
    assert_eq!(ended_state, ended_before);
}

#[test]
fn projection_lower_bound_rejects_each_underrepresented_installation_session_and_tombstone_shape() {
    let event_id = EventId::from_uuid(Uuid::from_u128(50));
    let installation = InstallationProjection {
        installation_id: installation_id(),
        created_event_id: event_id,
        created_at_ms: 1,
    };
    let started = SessionProjection {
        session_id: session_id(),
        started_event_id: event_id,
        started_at_ms: 1,
        ended: None,
    };
    let ended = SessionProjection {
        ended: Some(SessionEndProjection {
            ended_event_id: event_id,
            ended_at_ms: 2,
            reason: ShutdownReason::UserQuit,
        }),
        ..started.clone()
    };
    for (sessions, sequence) in [
        (BTreeMap::from([(session_id(), started)]), 1),
        (BTreeMap::from([(session_id(), ended)]), 2),
    ] {
        let state = ProjectionState {
            installation: Some(installation.clone()),
            sessions,
            agent_profiles: Default::default(),
            skills: Default::default(),
            memory: Default::default(),
            setup_status: SetupStatus::NotStarted,
            last_sequence: sequence,
            last_event_digest: if sequence == 0 {
                None
            } else {
                Some(sha256(b"marker"))
            },
        };
        assert_eq!(
            serde_json::from_value::<ProjectionState>(serde_json::to_value(state).unwrap())
                .unwrap_err()
                .to_string(),
            "event record is invalid"
        );
    }
}

#[test]
fn newer_store_rejects_a_digest_consistent_fabricated_persisted_prefix() {
    let (_temporary_directory, mut database) = database();
    let events = installation_session_events(&mut database);
    let fabricated = ProjectionState {
        installation: reduce_all(&events[..1]).installation,
        sessions: BTreeMap::new(),
        agent_profiles: Default::default(),
        skills: Default::default(),
        memory: Default::default(),
        setup_status: SetupStatus::NotStarted,
        last_sequence: 2,
        last_event_digest: Some(events[1].event_digest.clone()),
    };
    let digest = fabricated.digest().unwrap();
    database.connection().execute("INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (1, ?1, ?2, ?3)", (installation_id().to_string(), events[0].event_id.to_string(), events[0].occurred_at_ms)).unwrap();
    database.connection().execute("INSERT INTO projection_metadata (singleton, last_event_sequence, last_event_digest, projection_digest) VALUES (1, 2, ?1, ?2)", (events[1].event_digest.as_str(), digest.as_str())).unwrap();
    let newer_event = append(&mut database, 4, ApplicationEvent::HelpViewed);
    let mut newer = reduce_all(&events);
    reduce(&mut newer, &newer_event).unwrap();
    let transaction = database.immediate_transaction().unwrap();
    assert_eq!(
        ProjectionRepository::store(&transaction, &newer),
        Err(PersistenceError::ProjectionStateConflict)
    );
    transaction.rollback().unwrap();
}

#[test]
fn load_uses_one_snapshot_across_stream_and_projection_reads() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut first = Database::open(&paths).unwrap();
    let mut second = Database::open(&paths).unwrap();
    let events = installation_session_events(&mut first);
    let initial = reduce_all(&events);
    let transaction = first.immediate_transaction().unwrap();
    ProjectionRepository::store(&transaction, &initial).unwrap();
    transaction.commit().unwrap();

    let loaded = ProjectionRepository::load_with_before_projection_rows(first.connection(), || {
        let next = append(&mut second, 4, ApplicationEvent::HelpViewed);
        let mut newer = initial.clone();
        reduce(&mut newer, &next).unwrap();
        let transaction = second.immediate_transaction().unwrap();
        ProjectionRepository::store(&transaction, &newer).unwrap();
        transaction.commit().unwrap();
    })
    .unwrap();
    assert_eq!(loaded, initial);
}
