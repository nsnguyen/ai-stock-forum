use std::{
    ffi::c_void,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, PendingEvent},
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CorrelationId, Digest,
        EpisodicSummaryId, EventId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId,
        MemoryProposalId, canonical_json_bytes,
    },
    memory::{
        EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState, MemoryEntryDraft,
        MemoryEntryVersion, MemoryProposal, MemoryProposalFilter, MemoryProposalOperation,
        MemoryProposalResolution, MemoryProposalStatus, MemoryPurposeScope, MemoryRetrievalRequest,
        MemoryRetrievalScope, NormalizedMemoryKey,
    },
    persistence::{
        Database, EventRepository, MemoryRepository, PersistenceError, insert_expected_version,
    },
    policy::{ApprovalAction, ApprovalRecord, ApprovalStatus},
};
use uuid::Uuid;

static TRACE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static SQLITE_STATEMENTS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn statement_trace(
    event: u32,
    _context: *mut c_void,
    _statement: *mut c_void,
    _sql: *mut c_void,
) -> i32 {
    if event == rusqlite::ffi::SQLITE_TRACE_STMT {
        SQLITE_STATEMENTS.fetch_add(1, Ordering::SeqCst);
    }
    rusqlite::ffi::SQLITE_OK
}

fn measured_statements<T>(
    database: &mut Database,
    operation: impl FnOnce(&mut Database) -> T,
) -> (T, usize) {
    let _guard = TRACE_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    SQLITE_STATEMENTS.store(0, Ordering::SeqCst);
    // SAFETY: the callback is a process-static function and is removed before
    // the borrowed connection leaves this synchronized test helper.
    unsafe {
        rusqlite::ffi::sqlite3_trace_v2(
            database.connection().handle(),
            rusqlite::ffi::SQLITE_TRACE_STMT,
            Some(statement_trace),
            std::ptr::null_mut(),
        );
    }
    let value = operation(database);
    // SAFETY: passing a zero mask and no callback unregisters the tracer from
    // this live connection.
    unsafe {
        rusqlite::ffi::sqlite3_trace_v2(
            database.connection().handle(),
            0,
            None,
            std::ptr::null_mut(),
        );
    }
    (value, SQLITE_STATEMENTS.load(Ordering::SeqCst))
}

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
fn normalized_key_collision_between_distinct_entries_is_rejected() {
    let mut database = database();
    let event = append_help(&mut database, 39);
    let namespace = MemoryNamespaceId::from_uuid(Uuid::from_u128(1));
    let first = MemoryEntryVersion::create_present(
        namespace,
        MemoryEntryId::from_uuid(Uuid::from_u128(3_901)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(3_902)),
        MemoryEntryDraft::new("Case Key".into(), "first".into(), vec![]).unwrap(),
        Actor::Human,
        1,
        None,
        event,
    )
    .unwrap();
    let conflicting = MemoryEntryVersion::create_present(
        namespace,
        MemoryEntryId::from_uuid(Uuid::from_u128(3_903)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(3_904)),
        MemoryEntryDraft::new("case key".into(), "second".into(), vec![]).unwrap(),
        Actor::Human,
        2,
        None,
        event,
    )
    .unwrap();
    assert_eq!(
        first.reference().normalized_key(),
        conflicting.reference().normalized_key()
    );

    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &first).unwrap();
    assert_eq!(
        MemoryRepository::insert_entry_version(&tx, 1, &conflicting).unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.commit().unwrap();
    assert_eq!(
        database
            .connection()
            .query_row("SELECT COUNT(*) FROM memory_entry_versions", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
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
fn proposal_pending_and_all_pages_preserve_filter_order_limit_and_terminal_retry() {
    let (mut database, profile) = database_with_pending_proposals(3);
    let creation_event = EventId::from_uuid(Uuid::from_u128(40_003));
    let (middle, middle_approval) = pending_proposal(&profile, creation_event, 50_001);
    let resolution_event = append_help(&mut database, 59_000);
    let resolution_sequence = database
        .connection()
        .query_row(
            "SELECT sequence FROM event_stream WHERE event_id=?1",
            [resolution_event.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let accepted = MemoryProposalResolution::new(
        middle.reference(),
        MemoryProposalStatus::Accepted,
        middle.approval_id(),
        Actor::Human,
        60_000,
        resolution_event,
    )
    .unwrap();
    let accepted_approval = middle_approval
        .clone()
        .resolve(ApprovalStatus::Accepted, Actor::Human, 60_000)
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::resolve_proposal(
        &tx,
        u64::try_from(resolution_sequence).unwrap(),
        &accepted,
        &accepted_approval,
    )
    .unwrap();
    let pending = MemoryRepository::list_proposals(
        &tx,
        profile.memory_namespace_id(),
        ai_stock_forum::memory::MemoryProposalFilter::Pending,
        100,
    )
    .unwrap();
    assert_eq!(
        (
            pending.total_count,
            pending.returned_count,
            pending.omitted_count
        ),
        (2, 2, 0)
    );
    assert_eq!(
        pending
            .records
            .iter()
            .map(|record| record.proposal.proposal_id())
            .collect::<Vec<_>>(),
        vec![
            MemoryProposalId::from_uuid(Uuid::from_u128(50_060)),
            MemoryProposalId::from_uuid(Uuid::from_u128(50_062)),
        ]
    );
    assert_eq!(
        MemoryRepository::count_pending_proposals(&tx, profile.memory_namespace_id()).unwrap(),
        2
    );
    assert!(
        MemoryRepository::load_pending_proposals_for_key(
            &tx,
            profile.memory_namespace_id(),
            middle.normalized_key(),
        )
        .unwrap()
        .is_empty()
    );
    let all = MemoryRepository::list_proposals(
        &tx,
        profile.memory_namespace_id(),
        ai_stock_forum::memory::MemoryProposalFilter::All,
        1,
    )
    .unwrap();
    assert_eq!(
        (all.total_count, all.returned_count, all.omitted_count),
        (3, 1, 2)
    );
    assert_eq!(
        all.records[0].proposal.proposal_id(),
        MemoryProposalId::from_uuid(Uuid::from_u128(50_062))
    );
    tx.commit().unwrap();

    let different_event = append_help(&mut database, 59_001);
    let different_sequence = database
        .connection()
        .query_row(
            "SELECT sequence FROM event_stream WHERE event_id=?1",
            [different_event.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let rejected = MemoryProposalResolution::new(
        middle.reference(),
        MemoryProposalStatus::Rejected,
        middle.approval_id(),
        Actor::Human,
        60_001,
        different_event,
    )
    .unwrap();
    let rejected_approval = middle_approval
        .resolve(ApprovalStatus::Rejected, Actor::Human, 60_001)
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::resolve_proposal(
            &tx,
            u64::try_from(different_sequence).unwrap(),
            &rejected,
            &rejected_approval,
        )
        .unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    assert_eq!(
        MemoryRepository::load_proposal(&tx, middle.reference().proposal_id())
            .unwrap()
            .unwrap()
            .1,
        MemoryProposalStatus::Accepted
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
    database
        .connection()
        .execute(
            "UPDATE current_memory_entries SET content_digest=?1 WHERE entry_version_id=?2",
            [
                tagged.reference().content_digest().as_str().to_owned(),
                tagged.reference().entry_version_id().to_string(),
            ],
        )
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_entries SET entry_id=?1 WHERE entry_version_id=?2",
            [
                MemoryEntryId::from_uuid(Uuid::from_u128(8_999)).to_string(),
                tagged.reference().entry_version_id().to_string(),
            ],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::build_snapshot(&tx, &request).unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.rollback().unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_entries SET entry_id=?1 WHERE entry_version_id=?2",
            [
                tagged.reference().entry_id().to_string(),
                tagged.reference().entry_version_id().to_string(),
            ],
        )
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_entries SET memory_namespace_id=?1 WHERE entry_version_id=?2",
            [
                MemoryNamespaceId::from_uuid(Uuid::from_u128(8_998)).to_string(),
                tagged.reference().entry_version_id().to_string(),
            ],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::build_snapshot(&tx, &request).unwrap_err(),
        PersistenceError::MemoryRowMismatch
    );
    tx.rollback().unwrap();

    database
        .connection()
        .execute(
            "UPDATE current_memory_entries SET memory_namespace_id=?1 WHERE entry_version_id=?2",
            [
                tagged.reference().namespace_id().to_string(),
                tagged.reference().entry_version_id().to_string(),
            ],
        )
        .unwrap();
    database
        .connection()
        .execute_batch("DROP TRIGGER memory_entry_versions_no_update;")
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE memory_entry_versions SET purpose_tags_json=CAST('[\"other\"]' AS BLOB) WHERE entry_version_id=?1",
            [tagged.reference().entry_version_id().to_string()],
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    assert_eq!(
        MemoryRepository::build_snapshot(&tx, &request).unwrap_err(),
        PersistenceError::MemoryRowMismatch,
    );
    tx.rollback().unwrap();
}

fn database_with_current_entries(count: u128) -> Database {
    let mut database = database();
    let event = append_help(&mut database, 10_000 + count);
    let tx = database.immediate_transaction().unwrap();
    for index in 0..count {
        let entry = indexed_entry(event, 20_000 + index);
        MemoryRepository::insert_entry_version(&tx, 1, &entry).unwrap();
        MemoryRepository::replace_current_entry(&tx, &entry).unwrap();
    }
    tx.commit().unwrap();
    database
}

fn database_with_entry_history(count: u128) -> (Database, MemoryEntryVersion) {
    let mut database = database();
    let event = append_help(&mut database, 30_000 + count);
    let mut current = MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(1)),
        MemoryEntryId::from_uuid(Uuid::from_u128(31_000)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(32_000)),
        MemoryEntryDraft::new("History".into(), "v0".into(), vec![]).unwrap(),
        Actor::Human,
        0,
        None,
        event,
    )
    .unwrap();
    let tx = database.immediate_transaction().unwrap();
    MemoryRepository::insert_entry_version(&tx, 1, &current).unwrap();
    for index in 1..count {
        current = current
            .next_present(
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(32_000 + index)),
                MemoryEntryDraft::new("History".into(), format!("v{index}"), vec![]).unwrap(),
                Actor::Human,
                index as i64,
                None,
                event,
            )
            .unwrap();
        MemoryRepository::insert_entry_version(&tx, 1, &current).unwrap();
    }
    tx.commit().unwrap();
    (database, current)
}

fn database_with_pending_proposals(count: u128) -> (Database, AgentProfileVersion) {
    let mut database = database();
    let profile = profile();
    let event = append_help(&mut database, 40_000 + count);
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, &profile).unwrap();
    tx.commit().unwrap();
    let tx = database.immediate_transaction().unwrap();
    for index in 0..count {
        let (proposal, approval) = pending_proposal(&profile, event, 50_000 + index);
        MemoryRepository::insert_proposal_with_approval(&tx, 1, &proposal, &approval).unwrap();
    }
    tx.commit().unwrap();
    (database, profile)
}

fn event_source(database: &Database, event_id: EventId) -> EpisodicSourceRef {
    let (sequence, event_type, event_digest) = database
        .connection()
        .query_row(
            "SELECT sequence,event_type,event_digest FROM event_stream WHERE event_id=?1",
            [event_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();
    EpisodicSourceRef::new(
        u64::try_from(sequence).unwrap(),
        event_id,
        event_type,
        Digest::parse(&event_digest).unwrap(),
    )
    .unwrap()
}

fn insert_summary_fixture(database: &Database, summary: &EpisodicSummary) {
    let reference = summary.reference();
    let profile = reference.profile();
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
                profile.profile_id().to_string(),
                profile.profile_version_id().to_string(),
                i64::try_from(profile.version().get()).unwrap(),
                profile.content_digest().as_str(),
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
                canonical_json_bytes(summary).unwrap(),
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
}

fn database_with_summaries(count: u128) -> (Database, AgentProfileVersion, EpisodicSummaryId) {
    let mut database = database();
    let profile = profile();
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, &profile).unwrap();
    tx.commit().unwrap();
    let source_event = append_help(&mut database, 60_000 + count);
    let source = event_source(&database, source_event);
    let mut last_id = EpisodicSummaryId::from_uuid(Uuid::from_u128(0));
    for index in 0..count {
        let creation_event = append_help(&mut database, 61_000 + count * 200 + index);
        let creation_sequence = database
            .connection()
            .query_row(
                "SELECT sequence FROM event_stream WHERE event_id=?1",
                [creation_event.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        last_id = EpisodicSummaryId::from_uuid(Uuid::from_u128(62_000 + index));
        let summary = EpisodicSummary::new(
            last_id,
            &profile,
            format!("Summary {index:03}"),
            "body".into(),
            vec![],
            vec![source.clone()],
            index as i64,
            u64::try_from(creation_sequence).unwrap(),
            creation_event,
        )
        .unwrap();
        insert_summary_fixture(&database, &summary);
    }
    (database, profile, last_id)
}

fn database_with_summary_sources(count: u128) -> (Database, EpisodicSummaryId) {
    let mut database = database();
    let profile = profile();
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, &profile).unwrap();
    tx.commit().unwrap();
    let mut sources = Vec::new();
    for index in 0..count {
        let event = append_help(&mut database, 80_000 + count * 200 + index);
        sources.push(event_source(&database, event));
    }
    let creation_event = append_help(&mut database, 90_000 + count);
    let creation_sequence = database
        .connection()
        .query_row(
            "SELECT sequence FROM event_stream WHERE event_id=?1",
            [creation_event.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let summary_id = EpisodicSummaryId::from_uuid(Uuid::from_u128(91_000 + count));
    let summary = EpisodicSummary::new(
        summary_id,
        &profile,
        "Sources".into(),
        "body".into(),
        vec![],
        sources,
        1,
        u64::try_from(creation_sequence).unwrap(),
        creation_event,
    )
    .unwrap();
    insert_summary_fixture(&database, &summary);
    (database, summary_id)
}

fn database_with_accepted_dependency_chain(
    count: u64,
) -> (Database, AgentProfileVersion, MemoryEntryVersion) {
    let mut database = database();
    let profile = profile();
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, &profile).unwrap();
    tx.commit().unwrap();

    let mut accepted_proposals = Vec::new();
    for index in 0..count {
        let base = 200_000 + u128::from(index) * 20;
        let proposal_event = append_help(&mut database, base);
        let proposal = MemoryProposal::new(
            MemoryProposalId::from_uuid(Uuid::from_u128(base + 2)),
            &profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Set {
                candidate: MemoryEntryDraft::new(
                    "Dependency key".into(),
                    format!("value {index}"),
                    vec![],
                )
                .unwrap(),
            },
            "Dependency key".into(),
            ExpectedMemoryEntryState::Absent,
            "bounded dependency graph".into(),
            100 + index as i64 * 10,
            proposal_event,
            ApprovalId::from_uuid(Uuid::from_u128(base + 3)),
        )
        .unwrap();
        let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
            .approval_id(proposal.approval_id())
            .object(proposal.object_ref().unwrap())
            .actor(Actor::Agent(profile.profile_id()))
            .created_at_millis(proposal.created_at_ms())
            .build()
            .unwrap();
        let proposal_sequence = event_sequence(&database, proposal_event);
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::insert_proposal_with_approval(
            &tx,
            proposal_sequence,
            &proposal,
            &approval,
        )
        .unwrap();
        tx.commit().unwrap();

        let resolution_event = append_help(&mut database, base + 4);
        let resolution = MemoryProposalResolution::new(
            proposal.reference(),
            MemoryProposalStatus::Accepted,
            proposal.approval_id(),
            Actor::Human,
            101 + index as i64 * 10,
            resolution_event,
        )
        .unwrap();
        let resolved_approval = approval
            .resolve(
                ApprovalStatus::Accepted,
                Actor::Human,
                101 + index as i64 * 10,
            )
            .unwrap();
        let resolution_sequence = event_sequence(&database, resolution_event);
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::resolve_proposal(
            &tx,
            resolution_sequence,
            &resolution,
            &resolved_approval,
        )
        .unwrap();
        tx.commit().unwrap();

        accepted_proposals.push(proposal);
    }

    let mut prior: Option<MemoryEntryVersion> = None;
    let mut entries = Vec::new();
    for (index, proposal) in accepted_proposals.iter().enumerate() {
        let index = index as u64;
        let base = 200_000 + u128::from(index) * 20;
        let entry_event = append_help(&mut database, base + 6);
        let next = match prior.as_ref() {
            None => MemoryEntryVersion::create_present(
                profile.memory_namespace_id(),
                MemoryEntryId::from_uuid(Uuid::from_u128(400_000)),
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(base + 7)),
                MemoryEntryDraft::new("Dependency key".into(), format!("value {index}"), vec![])
                    .unwrap(),
                Actor::Human,
                102 + index as i64 * 10,
                Some(proposal.reference()),
                entry_event,
            ),
            Some(entry) => entry.next_present(
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(base + 7)),
                MemoryEntryDraft::new("Dependency key".into(), format!("value {index}"), vec![])
                    .unwrap(),
                Actor::Human,
                102 + index as i64 * 10,
                Some(proposal.reference()),
                entry_event,
            ),
        }
        .unwrap();
        let entry_sequence = event_sequence(&database, entry_event);
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::insert_entry_version(&tx, entry_sequence, &next).unwrap();
        MemoryRepository::replace_current_entry(&tx, &next).unwrap();
        tx.commit().unwrap();
        prior = Some(next.clone());
        entries.push(next);
    }

    for (index, entry) in entries.iter().enumerate() {
        let index = index as u64;
        let base = 200_000 + u128::from(index) * 20;
        let event_id = append_help(&mut database, base + 8);
        let proposal = MemoryProposal::new(
            MemoryProposalId::from_uuid(Uuid::from_u128(base + 9)),
            &profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Set {
                candidate: MemoryEntryDraft::new(
                    "Dependency key".into(),
                    format!("pending {index}"),
                    vec![],
                )
                .unwrap(),
            },
            "Dependency key".into(),
            ExpectedMemoryEntryState::Present(entry.reference()),
            "bounded expected-entry graph".into(),
            103 + index as i64 * 10,
            event_id,
            ApprovalId::from_uuid(Uuid::from_u128(base + 10)),
        )
        .unwrap();
        let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
            .approval_id(proposal.approval_id())
            .object(proposal.object_ref().unwrap())
            .actor(Actor::Agent(profile.profile_id()))
            .created_at_millis(proposal.created_at_ms())
            .build()
            .unwrap();
        let sequence = event_sequence(&database, event_id);
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::insert_proposal_with_approval(&tx, sequence, &proposal, &approval)
            .unwrap();
        tx.commit().unwrap();
    }
    (database, profile, prior.unwrap())
}

#[test]
fn bounded_repository_pages_execute_constant_select_statement_counts() {
    let mut one_current = database_with_current_entries(1);
    let mut max_current = database_with_current_entries(100);
    let (_, one_current_count) = measured_statements(&mut one_current, |database| {
        let tx = database.immediate_transaction().unwrap();
        let page = MemoryRepository::list_current_entries(
            &tx,
            MemoryNamespaceId::from_uuid(Uuid::from_u128(1)),
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
        assert_eq!(page.returned_count, 1);
    });
    let (_, max_current_count) = measured_statements(&mut max_current, |database| {
        let tx = database.immediate_transaction().unwrap();
        let page = MemoryRepository::list_current_entries(
            &tx,
            MemoryNamespaceId::from_uuid(Uuid::from_u128(1)),
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
        assert_eq!(page.returned_count, 100);
    });
    assert_eq!((one_current_count, max_current_count), (12, 12));

    let (mut one_history, one_entry) = database_with_entry_history(1);
    let (mut max_history, max_entry) = database_with_entry_history(100);
    let (_, one_history_count) = measured_statements(&mut one_history, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::load_entry_history(
            &tx,
            one_entry.reference().namespace_id(),
            one_entry.reference().normalized_key(),
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    let (_, max_history_count) = measured_statements(&mut max_history, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::load_entry_history(
            &tx,
            max_entry.reference().namespace_id(),
            max_entry.reference().normalized_key(),
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((one_history_count, max_history_count), (13, 13));

    let (mut one_proposal, one_profile) = database_with_pending_proposals(1);
    let (mut max_proposal, max_profile) = database_with_pending_proposals(100);
    let (_, one_proposal_count) = measured_statements(&mut one_proposal, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::list_proposals(
            &tx,
            one_profile.memory_namespace_id(),
            ai_stock_forum::memory::MemoryProposalFilter::Pending,
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    let (_, max_proposal_count) = measured_statements(&mut max_proposal, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::list_proposals(
            &tx,
            max_profile.memory_namespace_id(),
            ai_stock_forum::memory::MemoryProposalFilter::Pending,
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((one_proposal_count, max_proposal_count), (9, 9));

    let (mut one_summary, one_summary_profile, _) = database_with_summaries(1);
    let (mut max_summary, max_summary_profile, _) = database_with_summaries(100);
    let (_, one_summary_count) = measured_statements(&mut one_summary, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::list_episodic_summaries(
            &tx,
            one_summary_profile.memory_namespace_id(),
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    let (_, max_summary_count) = measured_statements(&mut max_summary, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::list_episodic_summaries(
            &tx,
            max_summary_profile.memory_namespace_id(),
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((one_summary_count, max_summary_count), (8, 8));

    for purpose in [
        MemoryPurposeScope::General,
        MemoryPurposeScope::tagged(vec!["earnings".into()]).unwrap(),
    ] {
        let mut one_snapshot = database_with_current_entries(1);
        let mut max_snapshot = database_with_current_entries(32);
        let snapshot_profile = profile();
        for database in [&mut one_snapshot, &mut max_snapshot] {
            let tx = database.connection_mut().transaction().unwrap();
            insert_expected_version(&tx, 1, &snapshot_profile).unwrap();
            tx.commit().unwrap();
        }
        let request = MemoryRetrievalRequest::new(
            MemoryRetrievalScope::new(&snapshot_profile, purpose.clone()).unwrap(),
            Default::default(),
        )
        .unwrap();
        let (_, one_snapshot_count) = measured_statements(&mut one_snapshot, |database| {
            let tx = database.immediate_transaction().unwrap();
            MemoryRepository::build_snapshot(&tx, &request).unwrap();
            tx.rollback().unwrap();
        });
        let (_, max_snapshot_count) = measured_statements(&mut max_snapshot, |database| {
            let tx = database.immediate_transaction().unwrap();
            MemoryRepository::build_snapshot(&tx, &request).unwrap();
            tx.rollback().unwrap();
        });
        let expected = match purpose {
            MemoryPurposeScope::General => 17,
            MemoryPurposeScope::Tagged(_) => 19,
        };
        assert_eq!(
            (one_snapshot_count, max_snapshot_count),
            (expected, expected)
        );
    }

    let (mut one_summary_snapshot, summary_profile, _) = database_with_summaries(1);
    let (mut max_summary_snapshot, max_summary_profile, _) = database_with_summaries(8);
    let one_request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(&summary_profile, MemoryPurposeScope::General).unwrap(),
        Default::default(),
    )
    .unwrap();
    let max_request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(&max_summary_profile, MemoryPurposeScope::General).unwrap(),
        Default::default(),
    )
    .unwrap();
    let (_, one_snapshot_count) = measured_statements(&mut one_summary_snapshot, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::build_snapshot(&tx, &one_request).unwrap();
        tx.rollback().unwrap();
    });
    let (_, max_snapshot_count) = measured_statements(&mut max_summary_snapshot, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::build_snapshot(&tx, &max_request).unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((one_snapshot_count, max_snapshot_count), (11, 11));
}

#[test]
fn accepted_dependency_graph_statement_counts_are_constant_for_small_and_large_sets() {
    let (mut one_entry, one_profile, one_current) = database_with_accepted_dependency_chain(1);
    let (mut many_entries, many_profile, many_current) =
        database_with_accepted_dependency_chain(32);
    let key = one_current.reference().normalized_key().clone();
    let many_key = many_current.reference().normalized_key().clone();

    let (_, one_current_count) = measured_statements(&mut one_entry, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::list_current_entries(&tx, one_profile.memory_namespace_id(), 100)
            .unwrap();
        tx.rollback().unwrap();
    });
    let (_, many_current_count) = measured_statements(&mut many_entries, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::list_current_entries(&tx, many_profile.memory_namespace_id(), 100)
            .unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((one_current_count, many_current_count), (12, 12));

    let (_, one_active_count) = measured_statements(&mut one_entry, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::count_active_entries(&tx, one_profile.memory_namespace_id()).unwrap();
        tx.rollback().unwrap();
    });
    let (_, many_active_count) = measured_statements(&mut many_entries, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::count_active_entries(&tx, many_profile.memory_namespace_id()).unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((one_active_count, many_active_count), (11, 11));

    let (_, one_history_count) = measured_statements(&mut one_entry, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::load_entry_history(&tx, one_profile.memory_namespace_id(), &key, 100)
            .unwrap();
        tx.rollback().unwrap();
    });
    let (_, many_history_count) = measured_statements(&mut many_entries, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::load_entry_history(
            &tx,
            many_profile.memory_namespace_id(),
            &many_key,
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((one_history_count, many_history_count), (13, 13));

    let one_request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(&one_profile, MemoryPurposeScope::General).unwrap(),
        Default::default(),
    )
    .unwrap();
    let many_request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(&many_profile, MemoryPurposeScope::General).unwrap(),
        Default::default(),
    )
    .unwrap();
    let (_, one_snapshot_count) = measured_statements(&mut one_entry, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::build_snapshot(&tx, &one_request).unwrap();
        tx.rollback().unwrap();
    });
    let (_, many_snapshot_count) = measured_statements(&mut many_entries, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::build_snapshot(&tx, &many_request).unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((one_snapshot_count, many_snapshot_count), (17, 17));

    let (mut two_proposals, two_profile, two_current) = database_with_accepted_dependency_chain(2);
    let proposal_key = two_current.reference().normalized_key().clone();
    let (_, two_list_count) = measured_statements(&mut two_proposals, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::list_proposals(
            &tx,
            two_profile.memory_namespace_id(),
            MemoryProposalFilter::All,
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    let (_, many_list_count) = measured_statements(&mut many_entries, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::list_proposals(
            &tx,
            many_profile.memory_namespace_id(),
            MemoryProposalFilter::All,
            100,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((two_list_count, many_list_count), (15, 15));

    let (_, two_pending_count) = measured_statements(&mut two_proposals, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::count_pending_proposals(&tx, two_profile.memory_namespace_id()).unwrap();
        tx.rollback().unwrap();
    });
    let (_, many_pending_count) = measured_statements(&mut many_entries, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::count_pending_proposals(&tx, many_profile.memory_namespace_id()).unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((two_pending_count, many_pending_count), (14, 14));

    let (_, two_key_count) = measured_statements(&mut two_proposals, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::load_pending_proposals_for_key(
            &tx,
            two_profile.memory_namespace_id(),
            &proposal_key,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    let (_, many_key_count) = measured_statements(&mut many_entries, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::load_pending_proposals_for_key(
            &tx,
            many_profile.memory_namespace_id(),
            &many_key,
        )
        .unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((two_key_count, many_key_count), (14, 14));
}

#[test]
fn summary_detail_source_authentication_is_constant_for_one_and_128_sources() {
    let (mut one_source, one_summary) = database_with_summary_sources(1);
    let (mut max_sources, max_summary) = database_with_summary_sources(128);
    let (_, one_count) = measured_statements(&mut one_source, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::load_episodic_summary(&tx, one_summary)
            .unwrap()
            .unwrap();
        tx.rollback().unwrap();
    });
    let (_, max_count) = measured_statements(&mut max_sources, |database| {
        let tx = database.immediate_transaction().unwrap();
        MemoryRepository::load_episodic_summary(&tx, max_summary)
            .unwrap()
            .unwrap();
        tx.rollback().unwrap();
    });
    assert_eq!((one_count, max_count), (6, 6));
}

#[test]
fn every_summary_and_source_column_tamper_fails_closed() {
    for mutation in [
        "UPDATE episodic_summaries SET summary_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE episodic_summaries SET version=2",
        "UPDATE episodic_summaries SET memory_namespace_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE episodic_summaries SET profile_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE episodic_summaries SET profile_version_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE episodic_summaries SET profile_version=2",
        "UPDATE episodic_summaries SET profile_content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE episodic_summaries SET label='changed'",
        "UPDATE episodic_summaries SET body='changed'",
        "UPDATE episodic_summaries SET purpose_tags_json=CAST('[\"other\"]' AS BLOB)",
        "UPDATE episodic_summaries SET source_count=2",
        "UPDATE episodic_summaries SET plaintext_validation_version=2",
        "UPDATE episodic_summaries SET created_at_ms=created_at_ms+1",
        "UPDATE episodic_summaries SET creation_event_sequence=creation_event_sequence+1",
        "UPDATE episodic_summaries SET creation_event_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE episodic_summaries SET source_set_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE episodic_summaries SET content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE episodic_summaries SET record_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE episodic_summaries SET record_json=CAST('{}' AS BLOB)",
    ] {
        let (mut database, profile, _) = database_with_summaries(1);
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER episodic_summaries_no_update;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        assert_eq!(
            MemoryRepository::list_episodic_summaries(&tx, profile.memory_namespace_id(), 100,)
                .expect_err(mutation),
            PersistenceError::MemoryRowMismatch,
            "{mutation}",
        );
        tx.rollback().unwrap();
    }

    for mutation in [
        "UPDATE episodic_summary_sources SET summary_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE episodic_summary_sources SET source_ordinal=2 WHERE source_ordinal=0",
        "UPDATE episodic_summary_sources SET event_sequence=event_sequence+1000 WHERE source_ordinal=0",
        "UPDATE episodic_summary_sources SET event_id='00000000-0000-0000-0000-000000009999' WHERE source_ordinal=0",
        "UPDATE episodic_summary_sources SET event_type='changed' WHERE source_ordinal=0",
        "UPDATE episodic_summary_sources SET event_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE source_ordinal=0",
        "DELETE FROM episodic_summary_sources WHERE source_ordinal=0",
    ] {
        let (mut database, summary_id) = database_with_summary_sources(2);
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER episodic_summary_sources_no_update;
                 DROP TRIGGER episodic_summary_sources_no_delete;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        assert_eq!(
            MemoryRepository::load_episodic_summary(&tx, summary_id).expect_err(mutation),
            PersistenceError::MemoryRowMismatch,
            "{mutation}",
        );
        tx.rollback().unwrap();
    }
}

#[test]
fn snapshot_rejects_summary_tag_profile_and_namespace_mirror_drift() {
    for mutation in [
        "UPDATE episodic_summaries SET purpose_tags_json=CAST('[\"other\"]' AS BLOB)",
        "UPDATE episodic_summaries SET profile_id='00000000-0000-0000-0000-000000009999'",
        "UPDATE episodic_summaries SET memory_namespace_id='00000000-0000-0000-0000-000000009999'",
    ] {
        let (mut database, profile, _) = database_with_summaries(1);
        let request = MemoryRetrievalRequest::new(
            MemoryRetrievalScope::new(&profile, MemoryPurposeScope::General).unwrap(),
            Default::default(),
        )
        .unwrap();
        database
            .connection()
            .execute_batch(
                "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
                 DROP TRIGGER episodic_summaries_no_update;",
            )
            .unwrap();
        database.connection().execute(mutation, []).unwrap();
        let tx = database.immediate_transaction().unwrap();
        assert_eq!(
            MemoryRepository::build_snapshot(&tx, &request).expect_err(mutation),
            PersistenceError::MemoryRowMismatch,
            "{mutation}",
        );
        tx.rollback().unwrap();
    }
}

#[test]
fn summary_lists_and_details_preserve_count_limit_and_source_order() {
    let (mut database, profile, newest_id) = database_with_summaries(101);
    let tx = database.immediate_transaction().unwrap();
    let page =
        MemoryRepository::list_episodic_summaries(&tx, profile.memory_namespace_id(), 500).unwrap();
    assert_eq!(
        (page.total_count, page.returned_count, page.omitted_count),
        (101, 100, 1)
    );
    assert_eq!(
        page.records.first().unwrap().summary.summary_id(),
        newest_id
    );
    assert_eq!(page.records.first().unwrap().label, "Summary 100");
    assert_eq!(page.records.last().unwrap().label, "Summary 001");
    tx.rollback().unwrap();

    let (mut source_database, source_summary_id) = database_with_summary_sources(3);
    let expected_sequences = source_database
        .connection()
        .prepare(
            "SELECT event_sequence FROM episodic_summary_sources
             WHERE summary_id=?1 ORDER BY source_ordinal",
        )
        .unwrap()
        .query_map([source_summary_id.to_string()], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let tx = source_database.immediate_transaction().unwrap();
    let summary = MemoryRepository::load_episodic_summary(&tx, source_summary_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        summary
            .sources()
            .iter()
            .map(|source| i64::try_from(source.sequence()).unwrap())
            .collect::<Vec<_>>(),
        expected_sequences
    );
    tx.rollback().unwrap();
}

#[test]
fn tagged_snapshot_includes_a_multi_matching_summary_only_once() {
    let mut database = database();
    let profile = profile();
    let tx = database.connection_mut().transaction().unwrap();
    insert_expected_version(&tx, 1, &profile).unwrap();
    tx.commit().unwrap();
    let source_event = append_help(&mut database, 95_000);
    let source = event_source(&database, source_event);
    let creation_event = append_help(&mut database, 95_001);
    let creation_sequence = database
        .connection()
        .query_row(
            "SELECT sequence FROM event_stream WHERE event_id=?1",
            [creation_event.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let summary = EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::from_u128(95_002)),
        &profile,
        "Tagged summary".into(),
        "body".into(),
        vec!["earnings".into(), "macro".into()],
        vec![source],
        1,
        u64::try_from(creation_sequence).unwrap(),
        creation_event,
    )
    .unwrap();
    insert_summary_fixture(&database, &summary);
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &profile,
            MemoryPurposeScope::tagged(vec!["earnings".into(), "macro".into()]).unwrap(),
        )
        .unwrap(),
        Default::default(),
    )
    .unwrap();
    let tx = database.immediate_transaction().unwrap();
    let snapshot = MemoryRepository::build_snapshot(&tx, &request).unwrap();
    assert_eq!(snapshot.summaries().len(), 1);
    assert_eq!(
        snapshot.summaries()[0].summary().summary_id(),
        summary.reference().summary_id()
    );
    tx.rollback().unwrap();
}
