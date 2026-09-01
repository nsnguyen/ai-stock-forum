use std::sync::{Arc, Mutex};

use ai_stock_forum::{
    app::ShutdownReason,
    config::{AppPaths, StartupError},
    domain::{Clock, IdGenerator, SessionId},
    persistence::{Database, EventRepository, RecoveryError},
    recovery::{RecoveryCoordinator, RecoveryHook},
};
use rusqlite::params;
use tempfile::TempDir;
use uuid::Uuid;

struct TestClock(Mutex<i64>);

impl TestClock {
    fn new() -> Self {
        Self(Mutex::new(1_700_000_000_000))
    }
}

impl Clock for TestClock {
    fn now_millis(&self) -> i64 {
        let mut current = self.0.lock().unwrap();
        let value = *current;
        *current += 1;
        value
    }
}

struct TestIds(Mutex<u128>);

impl TestIds {
    fn new() -> Self {
        Self(Mutex::new(1))
    }
}

impl IdGenerator for TestIds {
    fn next_uuid(&self) -> Uuid {
        let mut current = self.0.lock().unwrap();
        let value = Uuid::from_u128(*current);
        *current += 1;
        value
    }
}

struct Fixture {
    _temporary_directory: TempDir,
    database: Database,
    clock: TestClock,
    ids: TestIds,
}

impl Fixture {
    fn new() -> Self {
        let temporary_directory = tempfile::tempdir().unwrap();
        let database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
        Self {
            _temporary_directory: temporary_directory,
            database,
            clock: TestClock::new(),
            ids: TestIds::new(),
        }
    }

    fn bootstrap(&mut self) -> ai_stock_forum::recovery::BootstrapState {
        RecoveryCoordinator::bootstrap(&mut self.database, &self.clock, &self.ids, &[]).unwrap()
    }

    fn bootstrap_with_hooks(
        &mut self,
        hooks: &[Box<dyn RecoveryHook>],
    ) -> Result<ai_stock_forum::recovery::BootstrapState, StartupError> {
        RecoveryCoordinator::bootstrap(&mut self.database, &self.clock, &self.ids, hooks)
    }

    fn finish(
        &mut self,
        state: &mut ai_stock_forum::recovery::BootstrapState,
        reason: ShutdownReason,
    ) -> Vec<ai_stock_forum::app::EventEnvelope> {
        RecoveryCoordinator::finish_session(
            &mut self.database,
            &mut state.projection,
            state.session_id,
            reason,
            &self.clock,
            &self.ids,
        )
        .unwrap()
    }

    fn event_count(&self, kind: &str) -> usize {
        EventRepository::load_all(self.database.connection())
            .unwrap()
            .iter()
            .filter(|event| event.event.kind() == kind)
            .count()
    }

    fn corrupt_projection_metadata(&mut self) {
        self.database
            .connection()
            .execute(
                "UPDATE projection_metadata SET projection_digest = ?1 WHERE singleton = 1",
                ["0000000000000000000000000000000000000000000000000000000000000000"],
            )
            .unwrap();
    }

    fn remove_projections(&mut self) {
        self.database
            .connection()
            .execute("DELETE FROM projection_metadata", [])
            .unwrap();
        self.database
            .connection()
            .execute("DELETE FROM process_session_projection", [])
            .unwrap();
        self.database
            .connection()
            .execute("DELETE FROM installation_projection", [])
            .unwrap();
    }

    fn corrupt_projection_rows(&mut self) {
        self.database
            .connection()
            .execute(
                "UPDATE installation_projection SET installation_id = 'not-a-uuid' WHERE singleton = 1",
                [],
            )
            .unwrap();
    }

    fn insert_bad_event(&mut self) {
        let (sequence, digest): (i64, String) = self
            .database
            .connection()
            .query_row(
                "SELECT sequence, event_digest FROM event_stream ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        self.database
            .connection()
            .execute(
                "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, previous_event_digest, payload_json, event_digest) VALUES (?1, '00000000-0000-0000-0000-000000000999', 1, 'status_viewed', 'system', 1700000000999, '00000000-0000-0000-0000-000000000998', ?2, '{}', '0000000000000000000000000000000000000000000000000000000000000000')",
                params![sequence + 1, digest],
            )
            .unwrap();
    }
}

struct RecordingHook {
    recovered: Arc<Mutex<Vec<SessionId>>>,
}

impl RecoveryHook for RecordingHook {
    fn recover(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
        interrupted_session: SessionId,
    ) -> Result<(), RecoveryError> {
        self.recovered.lock().unwrap().push(interrupted_session);
        Ok(())
    }
}

struct FailingHook;

impl RecoveryHook for FailingHook {
    fn recover(
        &self,
        _transaction: &rusqlite::Transaction<'_>,
        _interrupted_session: SessionId,
    ) -> Result<(), RecoveryError> {
        Err(RecoveryError::QueryFailed)
    }
}

#[test]
fn empty_database_creates_one_installation_and_current_session() {
    let mut fixture = Fixture::new();

    let state = fixture.bootstrap();

    assert_eq!(fixture.event_count("installation_initialized"), 1);
    assert_eq!(fixture.event_count("process_session_started"), 1);
    assert_eq!(fixture.event_count("projection_rebuilt"), 0);
    assert_eq!(state.projection.installation.unwrap().installation_id, state.installation_id);
    assert!(state.projection.sessions[&state.session_id].ended.is_none());
}

#[test]
fn clean_restart_reuses_installation_without_reporting_an_interruption() {
    let mut fixture = Fixture::new();
    let mut first = fixture.bootstrap();
    fixture.finish(&mut first, ShutdownReason::InputClosed);

    let second = fixture.bootstrap();

    assert_eq!(first.installation_id, second.installation_id);
    assert_ne!(first.session_id, second.session_id);
    assert!(!second.previous_session_interrupted);
    assert_eq!(fixture.event_count("previous_session_interrupted"), 0);
}

#[test]
fn interrupted_prior_session_is_closed_once_and_exposed_only_on_that_boot() {
    let mut fixture = Fixture::new();
    let abandoned = fixture.bootstrap();
    let recovered = Arc::new(Mutex::new(Vec::new()));
    let hooks: Vec<Box<dyn RecoveryHook>> = vec![Box::new(RecordingHook {
        recovered: Arc::clone(&recovered),
    })];

    let second = fixture.bootstrap_with_hooks(&hooks).unwrap();

    assert!(second.previous_session_interrupted);
    assert_eq!(*recovered.lock().unwrap(), vec![abandoned.session_id]);
    assert_eq!(fixture.event_count("previous_session_interrupted"), 1);
    let mut second = second;
    fixture.finish(&mut second, ShutdownReason::UserQuit);
    let third = fixture.bootstrap();
    assert!(!third.previous_session_interrupted);
    assert_eq!(fixture.event_count("previous_session_interrupted"), 1);
}

#[test]
fn missing_projections_rebuild_from_the_verified_event_stream() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap();
    fixture.remove_projections();

    let recovered = fixture.bootstrap();

    assert_eq!(state.installation_id, recovered.installation_id);
    assert_eq!(fixture.event_count("projection_rebuilt"), 1);
}

#[test]
fn stale_projection_metadata_rebuilds_from_the_verified_event_stream() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap();
    fixture.corrupt_projection_metadata();

    let recovered = fixture.bootstrap();

    assert_eq!(state.installation_id, recovered.installation_id);
    assert_eq!(fixture.event_count("projection_rebuilt"), 1);
}

#[test]
fn corrupt_projection_rows_rebuild_from_the_verified_event_stream() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap();
    fixture.corrupt_projection_rows();

    let recovered = fixture.bootstrap();

    assert_eq!(state.installation_id, recovered.installation_id);
    assert_eq!(fixture.event_count("projection_rebuilt"), 1);
}

#[test]
fn corrupt_event_stream_refuses_bootstrap_without_appending_recovery_events() {
    let mut fixture = Fixture::new();
    fixture.bootstrap();
    fixture.insert_bad_event();
    let count_before: i64 = fixture
        .database
        .connection()
        .query_row("SELECT COUNT(*) FROM event_stream", [], |row| row.get(0))
        .unwrap();

    let error = RecoveryCoordinator::bootstrap(&mut fixture.database, &fixture.clock, &fixture.ids, &[])
        .unwrap_err();

    assert_eq!(error.code(), "event_digest_mismatch");
    assert_eq!(
        fixture
            .database
            .connection()
            .query_row("SELECT COUNT(*) FROM event_stream", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        count_before
    );
}

#[test]
fn failed_interruption_hook_rolls_back_the_interruption_event_and_projection() {
    let mut fixture = Fixture::new();
    let abandoned = fixture.bootstrap();
    let event_count_before = fixture.event_count("process_session_started")
        + fixture.event_count("previous_session_interrupted");
    let hooks: Vec<Box<dyn RecoveryHook>> = vec![Box::new(FailingHook)];

    let error = fixture.bootstrap_with_hooks(&hooks).unwrap_err();

    assert_eq!(error.code(), "event_query_failed");
    assert_eq!(
        fixture.event_count("process_session_started") + fixture.event_count("previous_session_interrupted"),
        event_count_before
    );
    assert!(fixture
        .database
        .connection()
        .query_row(
            "SELECT ended_event_id IS NULL FROM process_session_projection WHERE session_id = ?1",
            [abandoned.session_id.to_string()],
            |row| row.get::<_, bool>(0),
        )
        .unwrap());
}

#[test]
fn orderly_shutdown_ends_the_current_session_once() {
    let mut fixture = Fixture::new();
    let mut state = fixture.bootstrap();

    let first = fixture.finish(&mut state, ShutdownReason::UserQuit);
    let second = fixture.finish(&mut state, ShutdownReason::UserQuit);

    assert_eq!(first, second);
    assert_eq!(fixture.event_count("process_session_ended"), 1);
    assert_eq!(state.projection.sessions[&state.session_id].ended.as_ref().unwrap().reason, ShutdownReason::UserQuit);
}
