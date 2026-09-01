#![allow(dead_code)]

use ai_stock_forum::{
    app::{
        ApplicationEvent, ApplicationService, AuthorizationDecision, CommandPolicy,
        CommandTransactionHook, PendingEvent, EVENT_SCHEMA_VERSION,
    },
    config::AppPaths,
    domain::{Actor, Clock, CorrelationId, EventId, IdGenerator},
    persistence::{Database, EventRepository, PersistenceError},
    policy::Capability,
};
use rusqlite::Connection;
use std::{
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};
use tempfile::TempDir;
use uuid::Uuid;

pub struct DatabaseFixture {
    _temporary_directory: TempDir,
    pub database: Database,
    next_id: u128,
}

impl DatabaseFixture {
    pub fn append(&mut self, event: ApplicationEvent) -> ai_stock_forum::app::EventEnvelope {
        let pending = self.pending(event);
        let transaction = self.database.immediate_transaction().unwrap();
        let envelope = EventRepository::append(&transaction, pending).unwrap();
        transaction.commit().unwrap();
        envelope
    }

    pub fn pending(&mut self, event: ApplicationEvent) -> PendingEvent {
        let event_id = EventId::from_uuid(self.next_uuid());
        let correlation_id = CorrelationId::from_uuid(self.next_uuid());
        PendingEvent {
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: 1_700_000_000_000,
            correlation_id,
            causation_id: None,
            object: None,
            event,
        }
    }

    fn next_uuid(&mut self) -> Uuid {
        let value = Uuid::from_u128(self.next_id);
        self.next_id += 1;
        value
    }
}

pub fn database() -> DatabaseFixture {
    let temporary_directory = tempfile::tempdir().unwrap();
    let database = Database::open(&ai_stock_forum::config::AppPaths::for_test(
        temporary_directory.path(),
    ))
    .unwrap();
    DatabaseFixture {
        _temporary_directory: temporary_directory,
        database,
        next_id: 1,
    }
}

pub fn rejected_event(input: &[u8]) -> ai_stock_forum::app::EventEnvelope {
    let mut fixture = database();
    fixture.append(ApplicationEvent::CommandRejected {
        rejection: ai_stock_forum::app::InputRejection::from_input(
            ai_stock_forum::app::InputRejectionCategory::Malformed,
            Some(ai_stock_forum::app::SafeToken::new("/secret").unwrap()),
            input,
        ),
    })
}

pub struct TestClock {
    calls: AtomicUsize,
}

impl TestClock {
    pub fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Clock for TestClock {
    fn now_millis(&self) -> i64 {
        1_700_000_000_000 + self.calls.fetch_add(1, Ordering::SeqCst) as i64
    }
}

pub struct TestIds {
    calls: AtomicUsize,
}

impl TestIds {
    pub fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl IdGenerator for TestIds {
    fn next_uuid(&self) -> Uuid {
        Uuid::from_u128(10_000 + self.calls.fetch_add(1, Ordering::SeqCst) as u128)
    }
}

#[derive(Clone)]
pub struct RecordingPolicy {
    decision: Arc<Mutex<AuthorizationDecision>>,
    calls: Arc<AtomicUsize>,
    capabilities: Arc<Mutex<Vec<Capability>>>,
}

impl RecordingPolicy {
    pub fn new(decision: AuthorizationDecision) -> Self {
        Self {
            decision: Arc::new(Mutex::new(decision)),
            calls: Arc::new(AtomicUsize::new(0)),
            capabilities: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn capabilities(&self) -> Vec<Capability> {
        self.capabilities.lock().unwrap().clone()
    }

    pub fn set_decision(&self, decision: AuthorizationDecision) {
        *self.decision.lock().unwrap() = decision;
    }
}

impl CommandPolicy for RecordingPolicy {
    fn authorize(&self, capability: Capability) -> AuthorizationDecision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.capabilities.lock().unwrap().push(capability);
        *self.decision.lock().unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFailure {
    OutcomeMaterialization,
    ReceiptWrite,
}

pub struct TestCommandHook {
    failure: Option<HookFailure>,
}

impl TestCommandHook {
    pub fn passing() -> Self {
        Self { failure: None }
    }

    pub fn failing(failure: HookFailure) -> Self {
        Self {
            failure: Some(failure),
        }
    }
}

impl CommandTransactionHook for TestCommandHook {
    fn before_outcome_materialization(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        if self.failure == Some(HookFailure::OutcomeMaterialization) {
            Err(PersistenceError::QueryFailed)
        } else {
            Ok(())
        }
    }

    fn before_receipt_write(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        if self.failure == Some(HookFailure::ReceiptWrite) {
            Err(PersistenceError::QueryFailed)
        } else {
            Ok(())
        }
    }
}

pub struct TestApp {
    _temporary_directory: TempDir,
    paths: AppPaths,
    pub clock: Arc<TestClock>,
    pub ids: Arc<TestIds>,
    service: ApplicationService,
    policy: Option<Arc<dyn CommandPolicy>>,
    hook: Arc<dyn CommandTransactionHook>,
}

impl Deref for TestApp {
    type Target = ApplicationService;

    fn deref(&self) -> &Self::Target {
        &self.service
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.service
    }
}

impl TestApp {
    pub fn count_rows(&self, table: &str) -> i64 {
        assert!(matches!(
            table,
            "event_stream"
                | "setup_drafts"
                | "installation_configuration_versions"
                | "capability_readiness"
                | "approval_records"
                | "command_receipts"
                | "command_event_refs"
        ));
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    }

    pub fn event_count(&self, kind: &str) -> i64 {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM event_stream WHERE event_type = ?1",
                [kind],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn persisted_last_sequence(&self) -> u64 {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT last_event_sequence FROM projection_metadata WHERE singleton = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value as u64)
            .unwrap()
    }

    pub fn max_event_sequence(&self) -> u64 {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row("SELECT MAX(sequence) FROM event_stream", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|value| value as u64)
            .unwrap()
    }

    pub fn receipt_row(&self, command_id: ai_stock_forum::domain::CommandId) -> (String, String, String, String, String) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT command_fingerprint, request_json, capability, policy_decision, outcome_json FROM command_receipts WHERE command_id = ?1",
                [command_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap()
    }

    pub fn event_ref_ordinals(&self, command_id: ai_stock_forum::domain::CommandId) -> Vec<i64> {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        let mut statement = connection
            .prepare("SELECT event_ordinal FROM command_event_refs WHERE command_id = ?1 ORDER BY event_ordinal")
            .unwrap();
        statement
            .query_map([command_id.to_string()], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    pub fn install_projection_failure(&self) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_projection_metadata_update
                 BEFORE UPDATE ON projection_metadata BEGIN
                     SELECT RAISE(ABORT, 'injected projection failure');
                 END;",
            )
            .unwrap();
    }

    pub fn last_payload_json(&self) -> String {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT payload_json FROM event_stream ORDER BY sequence DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn append_authoritative_help_event_without_projection(&self) {
        let mut database = Database::open(&self.paths).unwrap();
        let transaction = database.immediate_transaction().unwrap();
        EventRepository::append(
            &transaction,
            PendingEvent {
                event_id: EventId::from_uuid(Uuid::from_u128(u128::MAX - 1)),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::System,
                occurred_at_ms: 1_800_000_000_000,
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(u128::MAX - 2)),
                causation_id: None,
                object: None,
                event: ApplicationEvent::HelpViewed,
            },
        )
        .unwrap();
        transaction.commit().unwrap();
    }

    pub fn shift_event_ref_to_ordinal_one(
        &self,
        command_id: ai_stock_forum::domain::CommandId,
    ) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .execute_batch(&format!(
                "DROP TRIGGER command_event_refs_no_update;
                 UPDATE command_event_refs SET event_ordinal = 1 WHERE command_id = '{}';",
                command_id
            ))
            .unwrap();
    }

    pub fn peer(&self) -> ApplicationService {
        ApplicationService::open_peer_for_test(
            &self.paths,
            self.clock.clone(),
            self.ids.clone(),
            self.policy.as_ref().expect("peer requires injected policy").clone(),
            self.hook.clone(),
        )
        .unwrap()
    }
}

pub fn app() -> TestApp {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let clock = Arc::new(TestClock::new());
    let ids = Arc::new(TestIds::new());
    let service = ApplicationService::bootstrap(&paths, clock.clone(), ids.clone()).unwrap();
    let hook: Arc<dyn CommandTransactionHook> = Arc::new(TestCommandHook::passing());
    TestApp {
        _temporary_directory: temporary_directory,
        paths,
        clock,
        ids,
        service,
        policy: None,
        hook,
    }
}

pub fn app_with_policy(policy: Arc<dyn CommandPolicy>) -> TestApp {
    app_with_policy_and_hook(policy, Arc::new(TestCommandHook::passing()))
}

pub fn app_with_policy_and_hook(
    policy: Arc<dyn CommandPolicy>,
    hook: Arc<dyn CommandTransactionHook>,
) -> TestApp {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let clock = Arc::new(TestClock::new());
    let ids = Arc::new(TestIds::new());
    let service = ApplicationService::bootstrap_with_dependencies(
        &paths,
        clock.clone(),
        ids.clone(),
        policy.clone(),
        hook.clone(),
    )
    .unwrap();
    TestApp {
        _temporary_directory: temporary_directory,
        paths,
        clock,
        ids,
        service,
        policy: Some(policy),
        hook,
    }
}
