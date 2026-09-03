#![allow(dead_code)]

use ai_stock_forum::{
    app::{
        ApplicationEvent, ApplicationService, ApplicationWorker, AuthorizationDecision,
        CommandPolicy, CommandTransactionHook, EVENT_SCHEMA_VERSION, PendingEvent, ShutdownReason,
    },
    config::AppPaths,
    domain::{Actor, Clock, CorrelationId, EventId, IdGenerator},
    persistence::{
        Database, EventRepository, PersistenceError, ProjectionRepository, RecoveryError,
    },
    policy::Capability,
    runtime::{ApplicationRuntime, RuntimeClient},
};
use rusqlite::Connection;
use std::{
    ops::{Deref, DerefMut},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tempfile::TempDir;
use uuid::Uuid;

pub struct RuntimeFixture {
    _temporary_directory: TempDir,
    paths: AppPaths,
    session_id: ai_stock_forum::domain::SessionId,
    runtime: ApplicationRuntime,
}

impl RuntimeFixture {
    pub fn client(&self) -> RuntimeClient {
        self.runtime.client()
    }

    pub fn finish_and_join(&self, reason: ShutdownReason) {
        self.runtime.finish_and_join(reason).unwrap();
    }

    pub fn last_shutdown_reason(&self) -> Option<String> {
        Connection::open(self.paths.database_path())
            .unwrap()
            .query_row(
                "SELECT end_reason FROM process_session_projection WHERE session_id = ?1",
                [self.session_id.to_string()],
                |row| row.get(0),
            )
            .unwrap()
    }
}

pub fn runtime() -> RuntimeFixture {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let service =
        ApplicationService::bootstrap(&paths, Arc::new(TestClock::new()), Arc::new(TestIds::new()))
            .unwrap();
    let session_id = service.session_id();
    let runtime = ApplicationRuntime::spawn_application(service, 32).unwrap();
    RuntimeFixture {
        _temporary_directory: temporary_directory,
        paths,
        session_id,
        runtime,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedEvent {
    pub sequence: u64,
    pub event_id: EventId,
    pub kind: String,
    pub payload_json: String,
}

pub struct PersistentFixture {
    _temporary_directory: TempDir,
    paths: AppPaths,
    clock: Arc<TestClock>,
    ids: Arc<TestIds>,
}

impl PersistentFixture {
    pub fn runtime(&self) -> ApplicationRuntime {
        let service =
            ApplicationService::bootstrap(&self.paths, self.clock.clone(), self.ids.clone())
                .unwrap();
        ApplicationRuntime::spawn_application(service, 32).unwrap()
    }

    pub fn installation_id(&self) -> ai_stock_forum::domain::InstallationId {
        let database = Database::open(&self.paths).unwrap();
        ProjectionRepository::load(database.connection())
            .unwrap()
            .installation
            .unwrap()
            .installation_id
    }

    pub fn events(&self) -> Vec<PersistedEvent> {
        let database = Database::open(&self.paths).unwrap();
        let mut statement = database
            .connection()
            .prepare(
                "SELECT sequence, event_id, event_type, payload_json
                 FROM event_stream ORDER BY sequence",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .map(|row| {
                let (sequence, event_id, kind, payload_json) = row.unwrap();
                PersistedEvent {
                    sequence: u64::try_from(sequence).unwrap(),
                    event_id: event_id.parse().unwrap(),
                    kind,
                    payload_json,
                }
            })
            .collect()
    }

    pub fn event_count(&self, kind: &str) -> i64 {
        Database::open(&self.paths)
            .unwrap()
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM event_stream WHERE event_type = ?1",
                [kind],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn count_rows(&self, table: &str) -> i64 {
        assert!(matches!(
            table,
            "event_stream"
                | "installation_projection"
                | "process_session_projection"
                | "projection_metadata"
                | "setup_drafts"
                | "installation_configuration_versions"
                | "active_installation_configuration"
                | "setup_step_outcomes"
                | "capability_readiness"
                | "approval_records"
        ));
        Database::open(&self.paths)
            .unwrap()
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    pub fn table_exists(&self, table: &str) -> bool {
        Database::open(&self.paths)
            .unwrap()
            .connection()
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
                )",
                [table],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub fn remove_recoverable_projection_state(&self) {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        transaction
            .execute_batch(
                "DELETE FROM projection_metadata;
                 DELETE FROM process_session_projection;
                 DELETE FROM installation_projection;",
            )
            .unwrap();
        transaction.commit().unwrap();
    }

    pub fn verify_event_stream(&self) -> Result<(), RecoveryError> {
        let database = Database::open(&self.paths).unwrap();
        EventRepository::verify(database.connection())
    }

    pub fn assert_projection_rows_match_event_stream(&self) {
        let database = Database::open(&self.paths).unwrap();
        let connection = database.connection();
        let (last_sequence, last_digest): (i64, String) = connection
            .query_row(
                "SELECT sequence, event_digest FROM event_stream
                 ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let (projected_sequence, projected_digest): (i64, Option<String>) = connection
            .query_row(
                "SELECT last_event_sequence, last_event_digest
                 FROM projection_metadata WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(projected_sequence, last_sequence);
        assert_eq!(projected_digest.as_deref(), Some(last_digest.as_str()));

        let (installation_id, created_event_id, created_at_ms, event_kind, payload_json, event_at):
            (String, String, i64, String, String, i64) = connection
                .query_row(
                    "SELECT p.installation_id, p.created_event_id, p.created_at_ms,
                            e.event_type, e.payload_json, e.occurred_at_ms
                     FROM installation_projection p
                     JOIN event_stream e ON e.event_id = p.created_event_id
                     WHERE p.singleton = 1",
                    [],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .unwrap();
        assert_eq!(event_kind, "installation_initialized");
        assert_eq!(created_at_ms, event_at);
        assert_eq!(
            payload_field(&payload_json, "installation_id"),
            installation_id
        );
        assert!(self.events().iter().any(|event| {
            event.event_id.to_string() == created_event_id
                && event.kind == "installation_initialized"
        }));

        let started_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM event_stream WHERE event_type = 'process_session_started'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let mut statement = connection
            .prepare(
                "SELECT session_id, started_event_id, started_at_ms,
                        ended_event_id, ended_at_ms, end_reason
                 FROM process_session_projection ORDER BY started_at_ms",
            )
            .unwrap();
        let sessions = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(i64::try_from(sessions.len()).unwrap(), started_count);
        for (session_id, started_event_id, started_at_ms, ended_event_id, ended_at_ms, reason) in
            sessions
        {
            let (started_kind, started_payload, event_started_at): (String, String, i64) =
                connection
                    .query_row(
                        "SELECT event_type, payload_json, occurred_at_ms
                         FROM event_stream WHERE event_id = ?1",
                        [&started_event_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .unwrap();
            assert_eq!(started_kind, "process_session_started");
            assert_eq!(payload_field(&started_payload, "session_id"), session_id);
            assert_eq!(started_at_ms, event_started_at);

            let ended_event_id = ended_event_id.expect("finished acceptance session");
            let ended_at_ms = ended_at_ms.expect("finished acceptance session");
            let reason = reason.expect("finished acceptance session");
            let (ended_kind, ended_payload, event_ended_at): (String, String, i64) = connection
                .query_row(
                    "SELECT event_type, payload_json, occurred_at_ms
                     FROM event_stream WHERE event_id = ?1",
                    [&ended_event_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(payload_field(&ended_payload, "session_id"), session_id);
            assert_eq!(ended_at_ms, event_ended_at);
            match ended_kind.as_str() {
                "process_session_ended" => {
                    assert_eq!(payload_field(&ended_payload, "reason"), reason)
                }
                "previous_session_interrupted" => assert_eq!(reason, "interrupted"),
                _ => panic!("unexpected session terminal event {ended_kind}"),
            }
        }
    }
}

fn payload_field(payload_json: &str, field: &str) -> String {
    serde_json::from_str::<serde_json::Value>(payload_json).unwrap()[field]
        .as_str()
        .unwrap()
        .to_owned()
}

pub fn persistent_fixture() -> PersistentFixture {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    PersistentFixture {
        _temporary_directory: temporary_directory,
        paths,
        clock: Arc::new(TestClock::new()),
        ids: Arc::new(TestIds::new()),
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptTamper {
    NoncanonicalRequest,
    NoncanonicalOutcome,
    TypedInvalidRequest,
    TypedInvalidOutcome,
    FingerprintMismatch,
    CapabilityMismatch,
    PolicyDecisionMismatch,
    EventRefOutcomeMismatch,
    OrdinalGap,
    MalformedReference,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleRacePoint {
    BeforeUserRead,
    AfterUserRead,
}

pub struct TestLifecycleHook {
    point: LifecycleRacePoint,
    command_entered: Arc<Barrier>,
    command_release: Arc<Barrier>,
    finish_attempted: Option<Arc<Barrier>>,
}

impl TestLifecycleHook {
    pub fn new(
        point: LifecycleRacePoint,
        command_entered: Arc<Barrier>,
        command_release: Arc<Barrier>,
        finish_attempted: Option<Arc<Barrier>>,
    ) -> Self {
        Self {
            point,
            command_entered,
            command_release,
            finish_attempted,
        }
    }

    fn block_command(&self) {
        self.command_entered.wait();
        self.command_release.wait();
    }
}

impl CommandTransactionHook for TestLifecycleHook {
    fn before_user_lifecycle_read(&self) {
        if self.point == LifecycleRacePoint::BeforeUserRead {
            self.block_command();
        }
    }

    fn after_user_lifecycle_read(&self) {
        if self.point == LifecycleRacePoint::AfterUserRead {
            self.block_command();
        }
    }

    fn before_finish_lifecycle_write(&self) {
        if let Some(barrier) = &self.finish_attempted {
            barrier.wait();
        }
    }

    fn before_outcome_materialization(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
    }

    fn before_receipt_write(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
    ) -> Result<(), PersistenceError> {
        Ok(())
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
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
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

    pub fn receipt_row(
        &self,
        command_id: ai_stock_forum::domain::CommandId,
    ) -> (String, String, String, String, String) {
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

    pub fn event_ref_rows(
        &self,
        command_id: ai_stock_forum::domain::CommandId,
    ) -> Vec<(i64, String)> {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        let mut statement = connection
            .prepare(
                "SELECT event_ordinal, event_id FROM command_event_refs
                 WHERE command_id = ?1 ORDER BY event_ordinal",
            )
            .unwrap();
        statement
            .query_map([command_id.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
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

    pub fn shift_event_ref_to_ordinal_one(&self, command_id: ai_stock_forum::domain::CommandId) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .execute_batch(&format!(
                "DROP TRIGGER command_event_refs_no_update;
                 UPDATE command_event_refs SET event_ordinal = 1 WHERE command_id = '{}';",
                command_id
            ))
            .unwrap();
    }

    pub fn mark_current_session_finished_in_database(&self) {
        Connection::open(self.paths.database_path())
            .unwrap()
            .execute(
                "UPDATE process_session_projection
                 SET ended_event_id = started_event_id, ended_at_ms = started_at_ms + 1,
                     end_reason = 'application_error'
                 WHERE session_id = ?1",
                [self.session_id().to_string()],
            )
            .unwrap();
    }

    pub fn tamper_receipt(
        &self,
        command_id: ai_stock_forum::domain::CommandId,
        tamper: ReceiptTamper,
    ) {
        let connection = Connection::open(self.paths.database_path()).unwrap();
        connection
            .execute_batch(
                "DROP TRIGGER IF EXISTS command_receipts_no_update;
                 DROP TRIGGER IF EXISTS command_event_refs_no_update;",
            )
            .unwrap();
        let command_id = command_id.to_string();

        match tamper {
            ReceiptTamper::NoncanonicalRequest => {
                let value: serde_json::Value = serde_json::from_str(
                    &connection
                        .query_row(
                            "SELECT request_json FROM command_receipts WHERE command_id = ?1",
                            [&command_id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                )
                .unwrap();
                let json = serde_json::to_string_pretty(&value).unwrap();
                let fingerprint = ai_stock_forum::domain::sha256(json.as_bytes()).to_string();
                connection
                    .execute(
                        "UPDATE command_receipts SET request_json = ?1, command_fingerprint = ?2 WHERE command_id = ?3",
                        rusqlite::params![json, fingerprint, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::NoncanonicalOutcome => {
                let value: serde_json::Value = serde_json::from_str(
                    &connection
                        .query_row(
                            "SELECT outcome_json FROM command_receipts WHERE command_id = ?1",
                            [&command_id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                )
                .unwrap();
                let json = serde_json::to_string_pretty(&value).unwrap();
                connection
                    .execute(
                        "UPDATE command_receipts SET outcome_json = ?1 WHERE command_id = ?2",
                        rusqlite::params![json, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::TypedInvalidRequest => {
                let mut value: serde_json::Value = serde_json::from_str(
                    &connection
                        .query_row(
                            "SELECT request_json FROM command_receipts WHERE command_id = ?1",
                            [&command_id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                )
                .unwrap();
                value
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".into(), serde_json::Value::Bool(true));
                let json = serde_json::to_string(&value).unwrap();
                let fingerprint = ai_stock_forum::domain::sha256(json.as_bytes()).to_string();
                connection
                    .execute(
                        "UPDATE command_receipts SET request_json = ?1, command_fingerprint = ?2 WHERE command_id = ?3",
                        rusqlite::params![json, fingerprint, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::TypedInvalidOutcome => {
                let mut value: serde_json::Value = serde_json::from_str(
                    &connection
                        .query_row(
                            "SELECT outcome_json FROM command_receipts WHERE command_id = ?1",
                            [&command_id],
                            |row| row.get::<_, String>(0),
                        )
                        .unwrap(),
                )
                .unwrap();
                value
                    .as_object_mut()
                    .unwrap()
                    .insert("unexpected".into(), serde_json::Value::Bool(true));
                let json = serde_json::to_string(&value).unwrap();
                connection
                    .execute(
                        "UPDATE command_receipts SET outcome_json = ?1 WHERE command_id = ?2",
                        rusqlite::params![json, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::FingerprintMismatch => {
                connection
                    .execute(
                        "UPDATE command_receipts SET command_fingerprint = ?1 WHERE command_id = ?2",
                        rusqlite::params!["b".repeat(64), command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::CapabilityMismatch => {
                connection
                    .execute(
                        "UPDATE command_receipts SET capability = 'help_read' WHERE command_id = ?1",
                        [&command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::PolicyDecisionMismatch => {
                connection
                    .execute(
                        "UPDATE command_receipts SET policy_decision = 'denied' WHERE command_id = ?1",
                        [&command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::EventRefOutcomeMismatch => {
                let replacement = connection
                    .query_row(
                        "SELECT event_id FROM event_stream
                         WHERE event_id NOT IN (SELECT event_id FROM command_event_refs)
                         ORDER BY sequence LIMIT 1",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap();
                connection
                    .execute(
                        "UPDATE command_event_refs SET event_id = ?1 WHERE command_id = ?2",
                        rusqlite::params![replacement, command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::OrdinalGap => {
                connection
                    .execute(
                        "UPDATE command_event_refs SET event_ordinal = 1 WHERE command_id = ?1",
                        [&command_id],
                    )
                    .unwrap();
            }
            ReceiptTamper::MalformedReference => {
                connection
                    .pragma_update(None, "foreign_keys", "OFF")
                    .unwrap();
                connection
                    .execute(
                        "UPDATE command_event_refs SET event_id = 'not-an-event-id' WHERE command_id = ?1",
                        [&command_id],
                    )
                    .unwrap();
            }
        }
    }

    pub fn peer(&self) -> ApplicationWorker {
        self.service.worker().unwrap()
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
