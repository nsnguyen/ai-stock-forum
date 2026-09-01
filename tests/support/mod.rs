#![allow(dead_code)]

use ai_stock_forum::{
    app::{
        ApplicationEvent, ApplicationService, AuthorizationDecision, CommandPolicy, PendingEvent,
        EVENT_SCHEMA_VERSION,
    },
    config::AppPaths,
    domain::{Actor, Clock, CorrelationId, EventId, IdGenerator},
    persistence::{Database, EventRepository, ProjectionRepository},
    policy::Capability,
    recovery::reduce,
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
    decision: AuthorizationDecision,
    calls: Arc<AtomicUsize>,
    capabilities: Arc<Mutex<Vec<Capability>>>,
}

impl RecordingPolicy {
    pub fn new(decision: AuthorizationDecision) -> Self {
        Self {
            decision,
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
}

impl CommandPolicy for RecordingPolicy {
    fn authorize(&self, capability: Capability) -> AuthorizationDecision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.capabilities.lock().unwrap().push(capability);
        self.decision
    }
}

pub struct TestApp {
    _temporary_directory: TempDir,
    paths: AppPaths,
    pub clock: Arc<TestClock>,
    pub ids: Arc<TestIds>,
    service: ApplicationService,
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

    pub fn append_external_help_event(&self) {
        let mut database = Database::open(&self.paths).unwrap();
        let transaction = database.immediate_transaction().unwrap();
        let mut projection = ProjectionRepository::load_in(&transaction).unwrap();
        let event = EventRepository::append(
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
        reduce(&mut projection, &event).unwrap();
        ProjectionRepository::store(&transaction, &projection).unwrap();
        transaction.commit().unwrap();
    }
}

pub fn app() -> TestApp {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let clock = Arc::new(TestClock::new());
    let ids = Arc::new(TestIds::new());
    let service = ApplicationService::bootstrap(&paths, clock.clone(), ids.clone()).unwrap();
    TestApp {
        _temporary_directory: temporary_directory,
        paths,
        clock,
        ids,
        service,
    }
}

pub fn app_with_policy(policy: Arc<dyn CommandPolicy>) -> TestApp {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let clock = Arc::new(TestClock::new());
    let ids = Arc::new(TestIds::new());
    let service = ApplicationService::bootstrap_with_policy(
        &paths,
        clock.clone(),
        ids.clone(),
        policy,
    )
    .unwrap();
    TestApp {
        _temporary_directory: temporary_directory,
        paths,
        clock,
        ids,
        service,
    }
}
