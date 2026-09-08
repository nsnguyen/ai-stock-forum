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
        MemoryProposalFilter, MemoryProposalOperation,
    },
    persistence::{
        Database, EventRepository, MemoryRepository, PersistenceError, insert_expected_version,
    },
    policy::{ApprovalAction, ApprovalRecord},
};
use uuid::Uuid;

fn database() -> Database {
    let paths = AppPaths::for_test(tempfile::tempdir().unwrap().keep());
    Database::open(&paths).unwrap()
}

#[test]
fn every_authenticated_entry_column_tamper_fails_closed_without_exposing_content() {
    for mutation in [
        "UPDATE memory_entry_versions SET content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE memory_entry_versions SET display_key='Changed'",
        "UPDATE memory_entry_versions SET record_json=CAST('{}' AS BLOB)",
        "UPDATE memory_entry_versions SET created_at_ms=created_at_ms+1",
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
            .execute_batch("PRAGMA foreign_keys=OFF; DROP TRIGGER memory_entry_versions_no_update;")
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        let error = MemoryRepository::load_current_entry(
            &tx,
            entry.reference().namespace_id(),
            entry.reference().normalized_key(),
        )
        .unwrap_err();
        tx.rollback().unwrap();
        assert_eq!(error, PersistenceError::MemoryRowMismatch, "{mutation}");
        assert_eq!(error.code(), "memory_row_mismatch");
        assert!(!error.to_string().contains("Private value"));
    }
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
