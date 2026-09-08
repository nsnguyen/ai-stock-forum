use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, PendingEvent},
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CorrelationId, EventId,
        MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId, MemoryProposalId,
    },
    memory::{
        ExpectedMemoryEntryState, MemoryEntryDraft, MemoryEntryVersion, MemoryProposal,
        MemoryProposalFilter, MemoryProposalOperation, MemoryProposalResolution,
        MemoryProposalStatus, MemoryPurposeScope, MemoryRetrievalBudget, MemoryRetrievalRequest,
        MemoryRetrievalScope, NormalizedMemoryKey,
    },
    persistence::{
        Database, EventRepository, MemoryRepository, PersistenceError, insert_expected_version,
    },
    policy::{ApprovalAction, ApprovalRecord, ApprovalStatus},
};
use uuid::Uuid;

fn database() -> Database {
    let paths = AppPaths::for_test(tempfile::tempdir().unwrap().keep());
    Database::open(&paths).unwrap()
}

#[test]
fn every_authenticated_entry_column_tamper_fails_closed_without_exposing_content() {
    for mutation in [
        "UPDATE memory_entry_versions SET memory_namespace_id='00000000-0000-0000-0000-000000000099'",
        "UPDATE memory_entry_versions SET entry_id='00000000-0000-0000-0000-000000000099'",
        "UPDATE memory_entry_versions SET entry_version_id='00000000-0000-0000-0000-000000000099'",
        "UPDATE memory_entry_versions SET version=2",
        "UPDATE memory_entry_versions SET predecessor_version_id='00000000-0000-0000-0000-000000000099'",
        "UPDATE memory_entry_versions SET content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE memory_entry_versions SET display_key='Changed'",
        "UPDATE memory_entry_versions SET normalized_key='changed'",
        "UPDATE memory_entry_versions SET state='deleted'",
        "UPDATE memory_entry_versions SET value_text='Changed private value'",
        "UPDATE memory_entry_versions SET value_bytes=value_bytes+1",
        "UPDATE memory_entry_versions SET purpose_tags_json=CAST('[\"other\"]' AS BLOB)",
        "UPDATE memory_entry_versions SET created_by_kind='system'",
        "UPDATE memory_entry_versions SET created_by_id='00000000-0000-0000-0000-000000000099'",
        "UPDATE memory_entry_versions SET accepted_proposal_id='00000000-0000-0000-0000-000000000099'",
        "UPDATE memory_entry_versions SET accepted_proposal_version=1",
        "UPDATE memory_entry_versions SET accepted_proposal_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE memory_entry_versions SET plaintext_validation_version=2",
        "UPDATE memory_entry_versions SET creation_event_sequence=creation_event_sequence+1",
        "UPDATE memory_entry_versions SET creation_event_id='00000000-0000-0000-0000-000000000099'",
        "UPDATE memory_entry_versions SET record_json=CAST('{}' AS BLOB)",
        "UPDATE memory_entry_versions SET created_at_ms=created_at_ms+1",
        "UPDATE memory_entry_versions SET record_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
    ] {
        let mut database = database();
        let event_id = EventId::from_uuid(Uuid::from_u128(100));
        let tx = database.immediate_transaction().unwrap();
        EventRepository::append(
            &tx,
            PendingEvent {
                event_id,
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 1,
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(101)),
                causation_id: None,
                object: None,
                event: ApplicationEvent::HelpViewed,
            },
        )
        .unwrap();
        let entry = MemoryEntryVersion::create_present(
            MemoryNamespaceId::from_uuid(Uuid::from_u128(1)),
            MemoryEntryId::from_uuid(Uuid::from_u128(2)),
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(3)),
            MemoryEntryDraft::new("Thesis".into(), "Private value".into(), vec![]).unwrap(),
            Actor::Human,
            2,
            None,
            event_id,
        )
        .unwrap();
        MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
        MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
        tx.commit().unwrap();
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER memory_entry_versions_no_update;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        let result = MemoryRepository::load_current_entry(
            &tx,
            entry.reference().namespace_id(),
            entry.reference().normalized_key(),
        );
        tx.rollback().unwrap();
        let error = result.expect_err(mutation);
        assert_eq!(error, PersistenceError::MemoryRowMismatch, "{mutation}");
        assert_eq!(error.code(), "memory_row_mismatch");
        assert!(!error.to_string().contains("Private value"));
    }
}

#[test]
fn predecessor_substitution_with_an_existing_version_fails_closed() {
    let mut database = database();
    let event_id = EventId::from_uuid(Uuid::from_u128(150));
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(151)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    let namespace = MemoryNamespaceId::from_uuid(Uuid::from_u128(152));
    let first = MemoryEntryVersion::create_present(
        namespace,
        MemoryEntryId::from_uuid(Uuid::from_u128(153)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(154)),
        MemoryEntryDraft::new("Chain".into(), "one".into(), vec![]).unwrap(),
        Actor::Human,
        2,
        None,
        event_id,
    )
    .unwrap();
    let second = first
        .next_present(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(155)),
            MemoryEntryDraft::new("Chain".into(), "two".into(), vec![]).unwrap(),
            Actor::Human,
            3,
            None,
            event_id,
        )
        .unwrap();
    let unrelated = MemoryEntryVersion::create_present(
        namespace,
        MemoryEntryId::from_uuid(Uuid::from_u128(156)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(157)),
        MemoryEntryDraft::new("Other".into(), "other".into(), vec![]).unwrap(),
        Actor::Human,
        4,
        None,
        event_id,
    )
    .unwrap();
    for entry in [&first, &second, &unrelated] {
        MemoryRepository::insert_entry_version(&tx, 1, entry).unwrap();
    }
    MemoryRepository::replace_current_entry(&tx, &first).unwrap();
    MemoryRepository::replace_current_entry(&tx, &second).unwrap();
    tx.commit().unwrap();

    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF; DROP TRIGGER memory_entry_versions_no_update;")
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE memory_entry_versions SET predecessor_version_id=?1 WHERE entry_version_id=?2",
            [
                unrelated.reference().entry_version_id().to_string(),
                second.reference().entry_version_id().to_string(),
            ],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::load_entry_history(
            &tx,
            namespace,
            second.reference().normalized_key(),
            100,
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.rollback().unwrap();
}

#[test]
fn current_pointer_and_terminal_proposal_status_tampering_fail_closed() {
    let mut database = database();
    let event_id = EventId::from_uuid(Uuid::from_u128(200));
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(201)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    let entry = MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(2)),
        MemoryEntryId::from_uuid(Uuid::from_u128(3)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(4)),
        MemoryEntryDraft::new("Pointer".into(), "value".into(), vec![]).unwrap(),
        Actor::Human,
        2,
        None,
        event_id,
    )
    .unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
    MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
    tx.commit().unwrap();
    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    database
        .connection()
        .execute("UPDATE current_memory_entries SET version=2", [])
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::load_current_entry(
            &tx,
            entry.reference().namespace_id(),
            entry.reference().normalized_key(),
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    assert_eq!(
        MemoryRepository::list_current_entries(&tx, entry.reference().namespace_id(), 100)
            .unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.rollback().unwrap();
}

#[test]
fn current_pointer_identity_tampering_is_not_silently_omitted_from_current_lists() {
    let mut database = database();
    let event_id = EventId::from_uuid(Uuid::from_u128(300));
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(301)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    let entry = MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(3)),
        MemoryEntryId::from_uuid(Uuid::from_u128(4)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(5)),
        MemoryEntryDraft::new("Identity".into(), "value".into(), vec![]).unwrap(),
        Actor::Human,
        2,
        None,
        event_id,
    )
    .unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
    MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
    tx.commit().unwrap();

    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_entries SET entry_id=?1",
            [MemoryEntryId::from_uuid(Uuid::from_u128(999)).to_string()],
        )
        .unwrap();

    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::load_current_entry(
            &tx,
            entry.reference().namespace_id(),
            entry.reference().normalized_key(),
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    assert_eq!(
        MemoryRepository::list_current_entries(&tx, entry.reference().namespace_id(), 100)
            .unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.rollback().unwrap();
}

#[test]
fn current_pointer_namespace_drift_fails_current_list_and_count() {
    let mut database = database();
    let event_id = EventId::from_uuid(Uuid::from_u128(350));
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(351)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    let entry = MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(352)),
        MemoryEntryId::from_uuid(Uuid::from_u128(353)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(354)),
        MemoryEntryDraft::new("Namespace".into(), "value".into(), vec![]).unwrap(),
        Actor::Human,
        2,
        None,
        event_id,
    )
    .unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
    MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
    tx.commit().unwrap();
    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_entries SET memory_namespace_id=?1",
            [MemoryNamespaceId::from_uuid(Uuid::from_u128(355)).to_string()],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::load_current_entry(
            &tx,
            entry.reference().namespace_id(),
            entry.reference().normalized_key(),
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    assert_eq!(
        MemoryRepository::list_current_entries(&tx, entry.reference().namespace_id(), 100)
            .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    assert_eq!(
        MemoryRepository::count_active_entries(&tx, entry.reference().namespace_id()).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    tx.rollback().unwrap();
}

#[test]
fn current_pointer_key_drift_fails_detail_instead_of_returning_none() {
    let mut database = database();
    let event_id = EventId::from_uuid(Uuid::from_u128(360));
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(361)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    let entry = MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(362)),
        MemoryEntryId::from_uuid(Uuid::from_u128(363)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(364)),
        MemoryEntryDraft::new("Pointer key".into(), "value".into(), vec![]).unwrap(),
        Actor::Human,
        2,
        None,
        event_id,
    )
    .unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
    MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
    tx.commit().unwrap();
    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_entries SET normalized_key='changed'",
            [],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::load_current_entry(
            &tx,
            entry.reference().namespace_id(),
            entry.reference().normalized_key(),
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.rollback().unwrap();
}

#[test]
fn memory_approval_read_rejects_a_structurally_valid_actor_substitution() {
    let mut database = database();
    let event_id = EventId::from_uuid(Uuid::from_u128(400));
    let profile = AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(401)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(402)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(403)),
        1,
        AgentProfileDraft::new(
            "Reviewer".into(),
            "description".into(),
            AgentRole::Custom,
            "kind".into(),
            vec![],
            "persona".into(),
            "instructions".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(404)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, &profile).unwrap();
    tx.commit().unwrap();
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(405)),
        &profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new("Key".into(), "value".into(), vec![]).unwrap(),
        },
        "Key".into(),
        ExpectedMemoryEntryState::Absent,
        "reason".into(),
        2,
        event_id,
        ApprovalId::from_uuid(Uuid::from_u128(406)),
    )
    .unwrap();
    let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
        .approval_id(proposal.approval_id())
        .object(proposal.object_ref().unwrap())
        .actor(Actor::Agent(profile.profile_id()))
        .created_at_millis(2)
        .build()
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_proposal_with_approval(&tx, 1, &proposal, &approval).unwrap();
    tx.commit().unwrap();
    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_proposal_status SET status='accepted', resolution_event_id=?1 WHERE proposal_id=?2",
            [event_id.to_string(), proposal.reference().proposal_id().to_string()],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::list_proposals(
            &tx,
            proposal.namespace_id(),
            MemoryProposalFilter::Pending,
            100,
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    tx.rollback().unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_proposal_status SET status='pending', resolution_event_id=NULL WHERE proposal_id=?1",
            [proposal.reference().proposal_id().to_string()],
        )
        .unwrap();
    database.connection().execute_batch(
        "PRAGMA foreign_keys=OFF; DROP TRIGGER approval_records_identity_guard; DROP TRIGGER approval_records_transition_guard;",
    ).unwrap();
    database
        .connection()
        .execute(
            "UPDATE approval_records SET actor_kind='human', actor_id=NULL WHERE approval_id=?1",
            [approval.approval_id().to_string()],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::load_memory_approval(&tx, approval.approval_id()).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    tx.rollback().unwrap();
}

#[test]
fn proposal_context_failure_leaves_no_orphan_approval_when_caller_commits() {
    let mut database = database();
    let event_id = EventId::from_uuid(Uuid::from_u128(500));
    let profile = AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(501)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(502)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(503)),
        1,
        AgentProfileDraft::new(
            "Missing profile".into(),
            "description".into(),
            AgentRole::Custom,
            "kind".into(),
            vec![],
            "persona".into(),
            "instructions".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(504)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(505)),
        &profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new("Key".into(), "value".into(), vec![]).unwrap(),
        },
        "Key".into(),
        ExpectedMemoryEntryState::Absent,
        "reason".into(),
        2,
        event_id,
        ApprovalId::from_uuid(Uuid::from_u128(506)),
    )
    .unwrap();
    let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
        .approval_id(proposal.approval_id())
        .object(proposal.object_ref().unwrap())
        .actor(Actor::Agent(profile.profile_id()))
        .created_at_millis(2)
        .build()
        .unwrap();
    assert_eq!(
        MemoryRepository::insert_proposal_with_approval(&tx, 1, &proposal, &approval).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    tx.commit().unwrap();
    assert_eq!(
        database
            .connection()
            .query_row("SELECT COUNT(*) FROM approval_records", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
    );
}

fn install_profile(database: &mut Database, profile: &AgentProfileVersion) {
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, profile).unwrap();
    tx.commit().unwrap();
}

fn proposal_fixture(
    profile: &AgentProfileVersion,
    event_id: EventId,
    proposal_number: u128,
    approval_number: u128,
    created_at_ms: i64,
) -> (MemoryProposal, ApprovalRecord) {
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(proposal_number)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                format!("Key {proposal_number}"),
                "candidate".into(),
                vec![],
            )
            .unwrap(),
        },
        format!("Key {proposal_number}"),
        ExpectedMemoryEntryState::Absent,
        "reason".into(),
        created_at_ms,
        event_id,
        ApprovalId::from_uuid(Uuid::from_u128(approval_number)),
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

fn pending_proposal_database() -> (
    Database,
    AgentProfileVersion,
    MemoryProposal,
    ApprovalRecord,
    EventId,
) {
    let mut database = database();
    let event_id = EventId::from_uuid(Uuid::from_u128(1_000));
    let profile = AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(1_001)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(1_002)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(1_003)),
        1,
        AgentProfileDraft::new(
            "Integrity profile".into(),
            "description".into(),
            AgentRole::Custom,
            "kind".into(),
            vec![],
            "persona".into(),
            "instructions".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(1_004)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    install_profile(&mut database, &profile);
    let (proposal, approval) = proposal_fixture(&profile, event_id, 1_005, 1_006, 10);
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_proposal_with_approval(&tx, 1, &proposal, &approval).unwrap();
    tx.commit().unwrap();
    (database, profile, proposal, approval, event_id)
}

fn resolved_proposal_database() -> (
    Database,
    AgentProfileVersion,
    MemoryProposal,
    ApprovalRecord,
    MemoryProposalResolution,
) {
    let (mut database, profile, proposal, approval, _) = pending_proposal_database();
    let resolution_event_id = EventId::from_uuid(Uuid::from_u128(1_007));
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id: resolution_event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 20,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(1_008)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    let resolution = MemoryProposalResolution::new(
        proposal.reference(),
        MemoryProposalStatus::Accepted,
        proposal.approval_id(),
        Actor::Human,
        20,
        resolution_event_id,
    )
    .unwrap();
    let resolved_approval = approval
        .resolve(ApprovalStatus::Accepted, Actor::Human, 20)
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::resolve_proposal(&tx, 2, &resolution, &resolved_approval).unwrap();
    tx.commit().unwrap();
    (database, profile, proposal, approval, resolution)
}

fn append_help_event(database: &mut Database, number: u128, occurred_at_ms: i64) -> EventId {
    let event_id = EventId::from_uuid(Uuid::from_u128(number));
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(number + 1)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    event_id
}

fn event_sequence(database: &Database, event_id: EventId) -> u64 {
    database
        .connection()
        .query_row(
            "SELECT sequence FROM event_stream WHERE event_id=?1",
            [event_id.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as u64
}

fn accepted_proposal_entry_database(
    deleted: bool,
) -> (
    Database,
    AgentProfileVersion,
    MemoryProposal,
    MemoryEntryVersion,
) {
    let (mut database, profile, accepted_proposal, _, _) = resolved_proposal_database();
    let present_event = append_help_event(&mut database, 1_020, 30);
    let present = MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(1_021)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_022)),
        MemoryEntryDraft::new(
            accepted_proposal.display_key().into(),
            "accepted value".into(),
            vec![],
        )
        .unwrap(),
        Actor::Human,
        30,
        Some(accepted_proposal.reference()),
        present_event,
    )
    .unwrap();
    let present_event_sequence = event_sequence(&database, present_event);
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_entry_version(&tx, present_event_sequence, &present).unwrap();
    MemoryRepository::replace_current_entry(&tx, &present).unwrap();
    tx.commit().unwrap();
    if !deleted {
        return (database, profile, accepted_proposal, present);
    }

    let deleted_event = append_help_event(&mut database, 1_023, 40);
    let deleted = present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_024)),
            Actor::Human,
            40,
            Some(accepted_proposal.reference()),
            deleted_event,
        )
        .unwrap();
    let deleted_event_sequence = event_sequence(&database, deleted_event);
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_entry_version(&tx, deleted_event_sequence, &deleted).unwrap();
    MemoryRepository::replace_current_entry(&tx, &deleted).unwrap();
    tx.commit().unwrap();
    (database, profile, accepted_proposal, deleted)
}

fn insert_pending_proposal_for_expected_entry(
    database: &mut Database,
    profile: &AgentProfileVersion,
    entry: &MemoryEntryVersion,
    deleted: bool,
) -> MemoryProposal {
    let event_id = append_help_event(database, 1_030, 50);
    let expected = if deleted {
        ExpectedMemoryEntryState::Deleted(entry.reference())
    } else {
        ExpectedMemoryEntryState::Present(entry.reference())
    };
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(1_031)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                entry.display_key().into(),
                "next candidate".into(),
                vec![],
            )
            .unwrap(),
        },
        entry.display_key().into(),
        expected,
        "expected-entry dependency test".into(),
        50,
        event_id,
        ApprovalId::from_uuid(Uuid::from_u128(1_032)),
    )
    .unwrap();
    let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
        .approval_id(proposal.approval_id())
        .object(proposal.object_ref().unwrap())
        .actor(Actor::Agent(profile.profile_id()))
        .created_at_millis(50)
        .build()
        .unwrap();
    let creation_event_sequence = event_sequence(database, event_id);
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_proposal_with_approval(
        &tx,
        creation_event_sequence,
        &proposal,
        &approval,
    )
    .unwrap();
    tx.commit().unwrap();
    proposal
}

fn assert_all_entry_batch_paths_fail(
    database: &mut Database,
    profile: &AgentProfileVersion,
    entry: &MemoryEntryVersion,
) {
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(profile, MemoryPurposeScope::General).unwrap(),
        MemoryRetrievalBudget::default(),
    )
    .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::load_current_entry(
            &tx,
            profile.memory_namespace_id(),
            entry.reference().normalized_key(),
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
        "direct entry detail",
    );
    assert_eq!(
        MemoryRepository::list_current_entries(&tx, profile.memory_namespace_id(), 100)
            .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
        "current entry list",
    );
    assert_eq!(
        MemoryRepository::count_active_entries(&tx, profile.memory_namespace_id()).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
        "active entry count",
    );
    assert_eq!(
        MemoryRepository::load_entry_history(
            &tx,
            profile.memory_namespace_id(),
            entry.reference().normalized_key(),
            100,
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
        "entry history",
    );
    assert_eq!(
        MemoryRepository::build_snapshot(&tx, &request).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
        "snapshot",
    );
    tx.rollback().unwrap();
}

fn assert_all_proposal_batch_paths_fail(
    database: &mut Database,
    profile: &AgentProfileVersion,
    proposal: &MemoryProposal,
) {
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::load_proposal(&tx, proposal.reference().proposal_id()).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
        "direct proposal detail",
    );
    for filter in [MemoryProposalFilter::All, MemoryProposalFilter::Pending] {
        assert_eq!(
            MemoryRepository::list_proposals(&tx, profile.memory_namespace_id(), filter, 100)
                .unwrap_err(),
            PersistenceError::MemoryRowMismatch,
            "proposal list {filter:?}",
        );
    }
    assert_eq!(
        MemoryRepository::count_pending_proposals(&tx, profile.memory_namespace_id()).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
        "pending count",
    );
    assert_eq!(
        MemoryRepository::load_pending_proposals_for_key(
            &tx,
            profile.memory_namespace_id(),
            proposal.normalized_key(),
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
        "pending proposals for key",
    );
    tx.rollback().unwrap();
}

#[test]
fn accepted_proposal_dependency_tampering_fails_every_entry_batch_path() {
    for (case, mutation) in [
        (
            "terminal approval actor",
            "UPDATE approval_records SET resolution_actor_kind='system', resolution_actor_id=NULL",
        ),
        (
            "terminal resolution coherence",
            "UPDATE memory_proposal_resolutions SET resolved_at_ms=resolved_at_ms+1",
        ),
        (
            "missing status row",
            "DELETE FROM current_memory_proposal_status",
        ),
    ] {
        let (mut database, profile, _, entry) = accepted_proposal_entry_database(false);
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER approval_records_transition_guard;
                 DROP TRIGGER memory_proposal_resolutions_no_update;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        assert_all_entry_batch_paths_fail(&mut database, &profile, &entry);
        eprintln!("verified accepted proposal dependency tamper: {case}");
    }
}

#[test]
fn unaccepted_current_successor_still_authenticates_accepted_predecessor_graph() {
    for (case, mutation) in [
        (
            "terminal approval actor",
            "UPDATE approval_records SET resolution_actor_kind='system', resolution_actor_id=NULL",
        ),
        (
            "terminal resolution coherence",
            "UPDATE memory_proposal_resolutions SET resolved_at_ms=resolved_at_ms+1",
        ),
        (
            "missing status row",
            "DELETE FROM current_memory_proposal_status",
        ),
    ] {
        let (mut database, profile, _, predecessor) = accepted_proposal_entry_database(false);
        let successor_event = append_help_event(&mut database, 1_025, 40);
        let successor = predecessor
            .next_present(
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_026)),
                MemoryEntryDraft::new(
                    predecessor.display_key().into(),
                    "direct successor".into(),
                    vec![],
                )
                .unwrap(),
                Actor::Human,
                40,
                None,
                successor_event,
            )
            .unwrap();
        let successor_sequence = event_sequence(&database, successor_event);
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::insert_entry_version(&tx, successor_sequence, &successor).unwrap();
        MemoryRepository::replace_current_entry(&tx, &successor).unwrap();
        tx.commit().unwrap();

        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER approval_records_transition_guard;
                 DROP TRIGGER memory_proposal_resolutions_no_update;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let request = MemoryRetrievalRequest::new(
            MemoryRetrievalScope::new(&profile, MemoryPurposeScope::General).unwrap(),
            MemoryRetrievalBudget::default(),
        )
        .unwrap();
        let tx = database.immediate_transaction().unwrap();
        assert_eq!(
            MemoryRepository::list_current_entries(&tx, profile.memory_namespace_id(), 100)
                .unwrap_err(),
            PersistenceError::MemoryRowMismatch,
            "current list: {case}",
        );
        assert_eq!(
            MemoryRepository::count_active_entries(&tx, profile.memory_namespace_id()).unwrap_err(),
            PersistenceError::MemoryRowMismatch,
            "active count: {case}",
        );
        assert_eq!(
            MemoryRepository::build_snapshot(&tx, &request).unwrap_err(),
            PersistenceError::MemoryRowMismatch,
            "snapshot: {case}",
        );
        assert_eq!(
            MemoryRepository::load_entry_history(
                &tx,
                profile.memory_namespace_id(),
                successor.reference().normalized_key(),
                1,
            )
            .unwrap_err(),
            PersistenceError::MemoryRowMismatch,
            "history limit one: {case}",
        );
        tx.rollback().unwrap();
    }
}

#[test]
fn expected_entry_dependency_tampering_fails_every_proposal_batch_path() {
    for (case, deleted, mutation) in [
        (
            "present creation event relationship",
            false,
            "UPDATE memory_entry_versions SET creation_event_sequence=creation_event_sequence+100",
        ),
        (
            "deleted accepted proposal dependency",
            true,
            "UPDATE memory_proposals SET memory_namespace_id='00000000-0000-0000-0000-000000009998';
             UPDATE current_memory_proposal_status SET memory_namespace_id='00000000-0000-0000-0000-000000009998'
              WHERE proposal_id='00000000-0000-0000-0000-0000000003ed'",
        ),
    ] {
        let (mut database, profile, _, entry) = accepted_proposal_entry_database(deleted);
        let pending =
            insert_pending_proposal_for_expected_entry(&mut database, &profile, &entry, deleted);
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER memory_entry_versions_no_update;
                 DROP TRIGGER memory_proposals_no_update;",
            )
            .unwrap();
        database.connection().execute_batch(mutation).unwrap();
        assert_all_proposal_batch_paths_fail(&mut database, &profile, &pending);
        eprintln!("verified expected entry dependency tamper: {case}");
    }
}

#[test]
fn every_proposal_immutable_column_tamper_fails_closed() {
    for mutation in [
        "UPDATE memory_proposals SET proposal_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposals SET version=2",
        "UPDATE memory_proposals SET proposer_profile_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposals SET proposer_profile_version_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposals SET proposer_profile_version=2",
        "UPDATE memory_proposals SET proposer_profile_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE memory_proposals SET memory_namespace_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposals SET operation='delete'",
        "UPDATE memory_proposals SET display_key='Changed'",
        "UPDATE memory_proposals SET normalized_key='changed'",
        "UPDATE memory_proposals SET expected_kind='present'",
        "UPDATE memory_proposals SET expected_entry_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposals SET expected_entry_version_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposals SET expected_entry_version=1",
        "UPDATE memory_proposals SET expected_entry_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE memory_proposals SET candidate_value='changed'",
        "UPDATE memory_proposals SET candidate_value_bytes=candidate_value_bytes+1",
        "UPDATE memory_proposals SET candidate_purpose_tags_json=CAST('[\"other\"]' AS BLOB)",
        "UPDATE memory_proposals SET rationale='changed'",
        "UPDATE memory_proposals SET plaintext_validation_version=2",
        "UPDATE memory_proposals SET created_at_ms=created_at_ms+1",
        "UPDATE memory_proposals SET creation_event_sequence=creation_event_sequence+1",
        "UPDATE memory_proposals SET creation_event_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposals SET approval_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposals SET content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE memory_proposals SET record_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE memory_proposals SET record_json=CAST('{}' AS BLOB)",
    ] {
        let (mut database, profile, _, _, _) = pending_proposal_database();
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER memory_proposals_no_update;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        let error = MemoryRepository::list_proposals(
            &tx,
            profile.memory_namespace_id(),
            MemoryProposalFilter::All,
            100,
        )
        .expect_err(mutation);
        tx.rollback().unwrap();
        assert_eq!(error, PersistenceError::MemoryRowMismatch, "{mutation}");
        assert!(!error.to_string().contains("candidate"), "{mutation}");
    }
}

#[test]
fn projection_namespace_tamper_is_rejected_by_every_proposal_page_path() {
    let (mut database, profile, proposal, approval, _) = pending_proposal_database();
    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_proposal_status SET memory_namespace_id=?1",
            [MemoryNamespaceId::from_uuid(Uuid::from_u128(9_998)).to_string()],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    for filter in [MemoryProposalFilter::Pending, MemoryProposalFilter::All] {
        assert_eq!(
            MemoryRepository::list_proposals(&tx, profile.memory_namespace_id(), filter, 100)
                .unwrap_err(),
            PersistenceError::MemoryRowMismatch,
        );
    }
    assert_eq!(
        MemoryRepository::count_pending_proposals(&tx, profile.memory_namespace_id()).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    assert_eq!(
        MemoryRepository::load_pending_proposals_for_key(
            &tx,
            profile.memory_namespace_id(),
            &NormalizedMemoryKey::new(proposal.display_key()).unwrap(),
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    assert_eq!(
        MemoryRepository::load_proposal(&tx, proposal.reference().proposal_id()).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    assert_eq!(
        MemoryRepository::load_memory_approval(&tx, approval.approval_id()).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    tx.rollback().unwrap();
}

#[test]
fn every_approval_column_and_actor_shape_tamper_fails_closed() {
    for mutation in [
        "UPDATE approval_records SET approval_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE approval_records SET action_kind='installation_change'",
        "UPDATE approval_records SET object_kind='other'",
        "UPDATE approval_records SET object_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE approval_records SET object_version=2",
        "UPDATE approval_records SET object_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE approval_records SET actor_kind='human'",
        "UPDATE approval_records SET actor_kind='system'",
        "UPDATE approval_records SET actor_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE approval_records SET actor_id=NULL",
        "UPDATE approval_records SET status='accepted'",
        "UPDATE approval_records SET created_at_ms=created_at_ms+1",
        "UPDATE approval_records SET expires_at_ms=created_at_ms+1",
        "UPDATE approval_records SET resolved_at_ms=created_at_ms+1",
        "UPDATE approval_records SET resolution_kind='accepted'",
        "UPDATE approval_records SET resolution_event_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE approval_records SET resolution_actor_kind='human'",
        "UPDATE approval_records SET resolution_actor_id='00000000-0000-0000-0000-000000009999'",
    ] {
        let (mut database, profile, _, _, _) = pending_proposal_database();
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER approval_records_identity_guard;
                 DROP TRIGGER approval_records_transition_guard;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        assert_eq!(
            MemoryRepository::list_proposals(
                &tx,
                profile.memory_namespace_id(),
                MemoryProposalFilter::All,
                100,
            )
            .expect_err(mutation),
            PersistenceError::MemoryRowMismatch,
            "{mutation}",
        );
        tx.rollback().unwrap();
    }

    for mutation in [
        "UPDATE approval_records SET resolution_actor_kind='human', resolution_actor_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE approval_records SET resolution_actor_kind='system', resolution_actor_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE approval_records SET resolution_actor_kind='agent', resolution_actor_id=NULL",
    ] {
        let (mut database, profile, _, _, _) = resolved_proposal_database();
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER approval_records_identity_guard;
                 DROP TRIGGER approval_records_transition_guard;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        assert_eq!(
            MemoryRepository::list_proposals(
                &tx,
                profile.memory_namespace_id(),
                MemoryProposalFilter::All,
                100,
            )
            .expect_err(mutation),
            PersistenceError::MemoryRowMismatch,
            "{mutation}",
        );
        tx.rollback().unwrap();
    }
}

#[test]
fn every_status_and_resolution_column_tamper_fails_closed() {
    for mutation in [
        "UPDATE current_memory_proposal_status SET proposal_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE current_memory_proposal_status SET proposal_version=2",
        "UPDATE current_memory_proposal_status SET proposal_content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE current_memory_proposal_status SET memory_namespace_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE current_memory_proposal_status SET normalized_key='changed'",
        "UPDATE current_memory_proposal_status SET status='pending'",
        "UPDATE current_memory_proposal_status SET resolution_event_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE current_memory_proposal_status SET created_at_ms=created_at_ms+1",
        "UPDATE memory_proposal_resolutions SET proposal_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposal_resolutions SET proposal_version=2",
        "UPDATE memory_proposal_resolutions SET proposal_content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE memory_proposal_resolutions SET status='rejected'",
        "UPDATE memory_proposal_resolutions SET approval_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposal_resolutions SET resolved_by_kind='system'",
        "UPDATE memory_proposal_resolutions SET resolved_by_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposal_resolutions SET resolved_at_ms=resolved_at_ms+1",
        "UPDATE memory_proposal_resolutions SET resolution_event_sequence=resolution_event_sequence+1",
        "UPDATE memory_proposal_resolutions SET resolution_event_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE memory_proposal_resolutions SET resolution_json=CAST('{}' AS BLOB)",
    ] {
        let (mut database, profile, _, _, _) = resolved_proposal_database();
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER memory_proposal_resolutions_no_update;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        assert_eq!(
            MemoryRepository::list_proposals(
                &tx,
                profile.memory_namespace_id(),
                MemoryProposalFilter::All,
                100,
            )
            .expect_err(mutation),
            PersistenceError::MemoryRowMismatch,
            "{mutation}",
        );
        tx.rollback().unwrap();
    }
}

#[test]
fn same_proposal_id_conflict_never_persists_the_fresh_approval_when_caller_commits() {
    let mut database = database();
    let event_id = EventId::from_uuid(Uuid::from_u128(600));
    let profile = AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(601)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(602)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(603)),
        1,
        AgentProfileDraft::new(
            "Conflict profile".into(),
            "description".into(),
            AgentRole::Custom,
            "kind".into(),
            vec![],
            "persona".into(),
            "instructions".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(604)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    install_profile(&mut database, &profile);

    let (first, first_approval) = proposal_fixture(&profile, event_id, 605, 606, 10);
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_proposal_with_approval(&tx, 1, &first, &first_approval).unwrap();
    tx.commit().unwrap();

    let (conflict, fresh_approval) = proposal_fixture(&profile, event_id, 605, 607, 11);
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::insert_proposal_with_approval(&tx, 1, &conflict, &fresh_approval)
            .unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    tx.commit().unwrap();

    assert_eq!(
        database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM approval_records WHERE approval_id=?1",
                [fresh_approval.approval_id().to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
    );
}

#[test]
fn proposal_insert_rolls_back_every_write_boundary_even_when_outer_transaction_commits() {
    for (case, trigger) in [
        (
            "approval",
            "CREATE TEMP TRIGGER fail_memory_write BEFORE INSERT ON approval_records BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        ),
        (
            "proposal",
            "CREATE TEMP TRIGGER fail_memory_write BEFORE INSERT ON memory_proposals BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        ),
        (
            "status",
            "CREATE TEMP TRIGGER fail_memory_write BEFORE INSERT ON current_memory_proposal_status BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        ),
    ] {
        let mut database = database();
        let event_id = EventId::from_uuid(Uuid::from_u128(700));
        let profile = AgentProfileVersion::create(
            AgentProfileId::from_uuid(Uuid::from_u128(701)),
            AgentProfileVersionId::from_uuid(Uuid::from_u128(702)),
            MemoryNamespaceId::from_uuid(Uuid::from_u128(703)),
            1,
            AgentProfileDraft::new(
                "Atomic insert".into(),
                "description".into(),
                AgentRole::Custom,
                "kind".into(),
                vec![],
                "persona".into(),
                "instructions".into(),
                AgentBindings::default(),
                vec![],
                vec![],
            )
            .unwrap(),
            None,
        )
        .unwrap();
        let tx = database.immediate_transaction().unwrap();
        EventRepository::append(
            &tx,
            PendingEvent {
                event_id,
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 1,
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(704)),
                causation_id: None,
                object: None,
                event: ApplicationEvent::HelpViewed,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        install_profile(&mut database, &profile);
        database.connection().execute_batch(trigger).unwrap();
        let (proposal, approval) = proposal_fixture(&profile, event_id, 705, 706, 10);

        let tx = database.immediate_transaction().unwrap();
        assert_eq!(
            MemoryRepository::insert_proposal_with_approval(&tx, 1, &proposal, &approval)
                .unwrap_err(),
            PersistenceError::QueryFailed,
            "{case}",
        );
        tx.commit().unwrap();

        for table in [
            "approval_records",
            "memory_proposals",
            "current_memory_proposal_status",
        ] {
            let count = database
                .connection()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{case}: {table}");
        }
    }
}

#[test]
fn proposal_resolution_rolls_back_every_write_boundary_even_when_outer_transaction_commits() {
    for (case_index, (case, trigger)) in [
        (
            "approval",
            "CREATE TEMP TRIGGER fail_resolution_write BEFORE UPDATE ON approval_records BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        ),
        (
            "resolution",
            "CREATE TEMP TRIGGER fail_resolution_write BEFORE INSERT ON memory_proposal_resolutions BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        ),
        (
            "status",
            "CREATE TEMP TRIGGER fail_resolution_write BEFORE UPDATE ON current_memory_proposal_status BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut database = database();
        let base = 800 + case_index as u128 * 20;
        let event_id = EventId::from_uuid(Uuid::from_u128(base));
        let resolution_event_id = EventId::from_uuid(Uuid::from_u128(base + 1));
        let profile = AgentProfileVersion::create(
            AgentProfileId::from_uuid(Uuid::from_u128(base + 2)),
            AgentProfileVersionId::from_uuid(Uuid::from_u128(base + 3)),
            MemoryNamespaceId::from_uuid(Uuid::from_u128(base + 4)),
            1,
            AgentProfileDraft::new(
                "Atomic resolution".into(),
                "description".into(),
                AgentRole::Custom,
                "kind".into(),
                vec![],
                "persona".into(),
                "instructions".into(),
                AgentBindings::default(),
                vec![],
                vec![],
            )
            .unwrap(),
            None,
        )
        .unwrap();
        let tx = database.immediate_transaction().unwrap();
        for (offset, id) in [event_id, resolution_event_id].into_iter().enumerate() {
            EventRepository::append(
                &tx,
                PendingEvent {
                    event_id: id,
                    event_schema_version: EVENT_SCHEMA_VERSION,
                    actor: Actor::Human,
                    occurred_at_ms: 1 + offset as i64,
                    correlation_id: CorrelationId::from_uuid(Uuid::from_u128(
                        base + 10 + offset as u128,
                    )),
                    causation_id: None,
                    object: None,
                    event: ApplicationEvent::HelpViewed,
                },
            )
            .unwrap();
        }
        tx.commit().unwrap();
        install_profile(&mut database, &profile);
        let (proposal, approval) =
            proposal_fixture(&profile, event_id, base + 5, base + 6, 10);
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::insert_proposal_with_approval(&tx, 1, &proposal, &approval).unwrap();
        tx.commit().unwrap();
        database.connection().execute_batch(trigger).unwrap();
        let resolution = ai_stock_forum::memory::MemoryProposalResolution::new(
            proposal.reference(),
            ai_stock_forum::memory::MemoryProposalStatus::Accepted,
            proposal.approval_id(),
            Actor::Human,
            20,
            resolution_event_id,
        )
        .unwrap();
        let resolved_approval = approval
            .resolve(ai_stock_forum::policy::ApprovalStatus::Accepted, Actor::Human, 20)
            .unwrap();

        let tx = database.immediate_transaction().unwrap();
        assert_eq!(
            MemoryRepository::resolve_proposal(&tx, 2, &resolution, &resolved_approval)
                .unwrap_err(),
            PersistenceError::QueryFailed,
            "{case}",
        );
        tx.commit().unwrap();

        assert_eq!(
            database
                .connection()
                .query_row(
                    "SELECT status FROM approval_records WHERE approval_id=?1",
                    [approval.approval_id().to_string()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "pending",
            "{case}: approval",
        );
        assert_eq!(
            database
                .connection()
                .query_row("SELECT COUNT(*) FROM memory_proposal_resolutions", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0,
            "{case}: resolution",
        );
        assert_eq!(
            database
                .connection()
                .query_row(
                    "SELECT status FROM current_memory_proposal_status WHERE proposal_id=?1",
                    [proposal.reference().proposal_id().to_string()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "pending",
            "{case}: status",
        );
    }
}
