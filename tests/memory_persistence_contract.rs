use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, PendingEvent},
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CorrelationId, EventId,
        MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId, MemoryProposalId,
    },
    memory::{
        MemoryEntryDraft, MemoryEntryVersion, MemoryProposal, MemoryProposalOperation,
        MemoryProposalResolution, MemoryProposalStatus, MemoryPurposeScope, MemoryRetrievalRequest,
        MemoryRetrievalScope, NormalizedMemoryKey,
    },
    persistence::{
        Database, EventRepository, MemoryRepository, PersistenceError, insert_expected_version,
    },
    policy::{ApprovalAction, ApprovalRecord, ApprovalStatus},
};
use uuid::Uuid;

fn database() -> Database {
    let dir = tempfile::tempdir().unwrap().keep();
    Database::open(&AppPaths::for_test(&dir)).unwrap()
}

fn append_help(database: &mut Database, number: u128) -> EventId {
    let event_id = EventId::from_uuid(Uuid::from_u128(number));
    let tx = database.immediate_transaction().unwrap();
    EventRepository::append(
        &tx,
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1_726_000_000_000 + number as i64,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(number + 1_000)),
            causation_id: None,
            object: None,
            event: ApplicationEvent::HelpViewed,
        },
    )
    .unwrap();
    tx.commit().unwrap();
    event_id
}

fn entry(event_id: EventId) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(1)),
        MemoryEntryId::from_uuid(Uuid::from_u128(2)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(3)),
        MemoryEntryDraft::new(
            "Thesis".into(),
            "Margins are improving.".into(),
            vec!["earnings".into()],
        )
        .unwrap(),
        Actor::Human,
        1_726_000_000_100,
        None,
        event_id,
    )
    .unwrap()
}

fn indexed_entry(event_id: EventId, index: u128) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(1)),
        MemoryEntryId::from_uuid(Uuid::from_u128(1_000 + index)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(2_000 + index)),
        MemoryEntryDraft::new(format!("Key {index:03}"), "v".into(), vec![]).unwrap(),
        Actor::Human,
        1_000 + index as i64,
        None,
        event_id,
    )
    .unwrap()
}

fn profile() -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(50)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(51)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(1)),
        1,
        AgentProfileDraft::new(
            "Memory Analyst".into(),
            "Stores reviewed memory.".into(),
            AgentRole::Custom,
            "research".into(),
            vec!["memory".into()],
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

fn pending_proposal(
    profile: &AgentProfileVersion,
    event_id: EventId,
    suffix: u128,
) -> (MemoryProposal, ApprovalRecord) {
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(60 + suffix)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                format!("Key {suffix}"),
                "Candidate value".into(),
                vec![],
            )
            .unwrap(),
        },
        format!("Key {suffix}"),
        ai_stock_forum::memory::ExpectedMemoryEntryState::Absent,
        "Reviewed rationale".into(),
        100 + suffix as i64,
        event_id,
        ApprovalId::from_uuid(Uuid::from_u128(70 + suffix)),
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

#[test]
fn immutable_entry_round_trip_is_idempotent_and_current_listing_is_sql_ordered() {
    let mut database = database();
    let entry = entry(append_help(&mut database, 10));
    let namespace = entry.reference().namespace_id();
    let key = entry.reference().normalized_key().clone();
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
    MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
    assert_eq!(
        MemoryRepository::load_current_entry(&tx, namespace, &key).unwrap(),
        Some(entry.clone())
    );
    let page = MemoryRepository::list_current_entries(&tx, namespace, 100).unwrap();
    assert_eq!(
        (
            page.total_count,
            page.returned_count,
            page.records[0].display_key.as_str()
        ),
        (1, 1, "Thesis")
    );
    tx.commit().unwrap();
}

#[test]
fn tombstone_is_current_detail_but_not_current_list_and_history_is_newest_first() {
    let mut database = database();
    let first = entry(append_help(&mut database, 20));
    let deleted = first
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(4)),
            Actor::Human,
            1_726_000_000_101,
            None,
            append_help(&mut database, 21),
        )
        .unwrap();
    let namespace = first.reference().namespace_id();
    let key = NormalizedMemoryKey::new("Thesis").unwrap();
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &first).unwrap();
    MemoryRepository::replace_current_entry(&tx, &first).unwrap();
    MemoryRepository::insert_entry_version(&tx, 2, &deleted).unwrap();
    MemoryRepository::replace_current_entry(&tx, &deleted).unwrap();
    assert_eq!(
        MemoryRepository::load_current_entry(&tx, namespace, &key).unwrap(),
        Some(deleted.clone())
    );
    assert!(
        MemoryRepository::list_current_entries(&tx, namespace, 100)
            .unwrap()
            .records
            .is_empty()
    );
    assert_eq!(
        MemoryRepository::load_entry_history(&tx, namespace, &key, 100)
            .unwrap()
            .versions,
        vec![deleted, first]
    );
    tx.commit().unwrap();
}

#[test]
fn proposal_approval_and_resolution_are_context_bound_and_second_resolution_fails() {
    let mut database = database();
    let profile = profile();
    let event_one = append_help(&mut database, 30);
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, &profile).unwrap();
    tx.commit().unwrap();
    let (proposal, approval) = pending_proposal(&profile, event_one, 1);
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_proposal_with_approval(&tx, 1, &proposal, &approval).unwrap();
    assert_eq!(
        MemoryRepository::count_pending_proposals(&tx, profile.memory_namespace_id()).unwrap(),
        1
    );
    tx.commit().unwrap();
    let event_two = append_help(&mut database, 31);
    let resolution = MemoryProposalResolution::new(
        proposal.reference(),
        MemoryProposalStatus::Accepted,
        proposal.approval_id(),
        Actor::Human,
        200,
        event_two,
    )
    .unwrap();
    let resolved_approval = approval
        .resolve(ApprovalStatus::Accepted, Actor::Human, 200)
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::resolve_proposal(&tx, 2, &resolution, &resolved_approval).unwrap();
    assert_eq!(
        MemoryRepository::load_proposal(&tx, proposal.reference().proposal_id())
            .unwrap()
            .unwrap()
            .1,
        MemoryProposalStatus::Accepted
    );
    MemoryRepository::resolve_proposal(&tx, 2, &resolution, &resolved_approval).unwrap();
    tx.commit().unwrap();
    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_proposal_status SET status='rejected'",
            [],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::load_proposal(&tx, proposal.reference().proposal_id()).unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.rollback().unwrap();
}

#[test]
fn replacing_an_already_current_exact_entry_is_idempotent() {
    let mut database = database();
    let entry = entry(append_help(&mut database, 41));
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
    MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
    MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
    tx.commit().unwrap();
}

#[test]
fn list_limit_counts_order_and_failed_same_id_write_are_transactional() {
    let mut database = database();
    let event = append_help(&mut database, 40);
    let tx = database.immediate_transaction().unwrap();
    for index in 0..101 {
        let entry = indexed_entry(event, index);
        MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
        MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
    }
    let namespace = MemoryNamespaceId::from_uuid(Uuid::from_u128(1));
    let page = MemoryRepository::list_current_entries(&tx, namespace, 500).unwrap();
    assert_eq!(
        (page.total_count, page.returned_count, page.omitted_count),
        (101, 100, 1)
    );
    assert_eq!(page.records.first().unwrap().display_key, "Key 000");
    assert_eq!(page.records.last().unwrap().display_key, "Key 099");
    let original = indexed_entry(event, 100);
    let collision = MemoryEntryVersion::create_present(
        namespace,
        original.reference().entry_id(),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(9_999)),
        MemoryEntryDraft::new("Different key".into(), "different".into(), vec![]).unwrap(),
        Actor::Human,
        2_000,
        None,
        event,
    )
    .unwrap();
    assert_eq!(
        MemoryRepository::insert_entry_version(&tx, 1, &collision).unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.rollback().unwrap();
    assert_eq!(
        database
            .connection()
            .query_row("SELECT COUNT(*) FROM memory_entry_versions", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}

#[test]
fn pending_proposal_capacity_is_exact_and_rejects_without_writing_the_overflow_approval() {
    let mut database = database();
    let profile = profile();
    let event = append_help(&mut database, 50);
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, &profile).unwrap();
    tx.commit().unwrap();
    let tx = database.immediate_transaction().unwrap();
    for suffix in 100..356 {
        let (proposal, approval) = pending_proposal(&profile, event, suffix);
        MemoryRepository::insert_proposal_with_approval(&tx, 1, &proposal, &approval).unwrap();
    }
    assert_eq!(
        MemoryRepository::count_pending_proposals(&tx, profile.memory_namespace_id()).unwrap(),
        256
    );
    let (overflow, approval) = pending_proposal(&profile, event, 356);
    assert_eq!(
        MemoryRepository::insert_proposal_with_approval(&tx, 1, &overflow, &approval).unwrap_err(),
        PersistenceError::Capacity
    );
    assert!(
        MemoryRepository::load_memory_approval(&tx, overflow.approval_id())
            .unwrap()
            .is_none()
    );
    tx.rollback().unwrap();
}

#[test]
fn snapshot_streams_tagged_matches_once_before_untagged_fallback() {
    let mut database = database();
    let profile = profile();
    let event = append_help(&mut database, 60);
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, &profile).unwrap();
    tx.commit().unwrap();
    let tagged = MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(8_001)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(8_002)),
        MemoryEntryDraft::new("Alpha".into(), "tagged".into(), vec!["earnings".into()]).unwrap(),
        Actor::Human,
        1,
        None,
        event,
    )
    .unwrap();
    let fallback = MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(8_003)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(8_004)),
        MemoryEntryDraft::new("Zebra".into(), "fallback".into(), vec![]).unwrap(),
        Actor::Human,
        2,
        None,
        event,
    )
    .unwrap();
    let scope = MemoryRetrievalScope::new(
        &profile,
        MemoryPurposeScope::tagged(vec!["earnings".into()]).unwrap(),
    )
    .unwrap();
    let request = MemoryRetrievalRequest::new(scope, Default::default()).unwrap();
    let tx = database.immediate_transaction().unwrap();
    for entry in [&tagged, &fallback] {
        MemoryRepository::insert_entry_version(&tx, 1, entry).unwrap();
        MemoryRepository::replace_current_entry(&tx, entry).unwrap();
    }
    let snapshot = MemoryRepository::build_snapshot(&tx, &request).unwrap();
    assert_eq!(
        snapshot
            .entries()
            .iter()
            .map(|item| item.display_key())
            .collect::<Vec<_>>(),
        vec!["Alpha", "Zebra"]
    );
    tx.commit().unwrap();
    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    database
        .connection()
        .execute("UPDATE current_memory_entries SET content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE entry_id=?1", [tagged.reference().entry_id().to_string()])
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::build_snapshot(&tx, &request).unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.rollback().unwrap();
}
