use std::{
    fs,
    path::PathBuf,
    sync::{mpsc, Arc, Barrier, Mutex},
    thread,
};

use ai_stock_forum::{
    app::ShutdownReason,
    config::{AppPaths, StartupError},
    domain::{canonical_json_bytes, sha256, CausationId, Clock, CorrelationId, EventId, IdGenerator, ObjectRef, SessionId, Sha256Digest},
    persistence::{Database, EventRepository, RecoveryError},
    recovery::{RecoveryCoordinator, RecoveryHook},
};
use rusqlite::params;
use serde::Serialize;
use tempfile::TempDir;
use uuid::Uuid;

#[cfg(unix)]
use std::{
    ffi::CString,
    os::unix::{fs::{symlink, MetadataExt, PermissionsExt}, net::UnixListener},
};

struct TestClock(Mutex<i64>);

impl TestClock {
    fn new() -> Self {
        Self(Mutex::new(1_700_000_000_000))
    }

    fn calls(&self) -> usize {
        (*self.0.lock().unwrap() - 1_700_000_000_000) as usize
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

struct TestIds {
    next_id: Mutex<u128>,
    initial_id: u128,
}

impl TestIds {
    fn new() -> Self {
        Self::starting_at(1)
    }

    fn starting_at(next_id: u128) -> Self {
        Self {
            next_id: Mutex::new(next_id),
            initial_id: next_id,
        }
    }

    fn calls(&self) -> usize {
        (*self.next_id.lock().unwrap() - self.initial_id) as usize
    }
}

impl IdGenerator for TestIds {
    fn next_uuid(&self) -> Uuid {
        let mut current = self.next_id.lock().unwrap();
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

    fn open_peer(&self) -> Database {
        Database::open(&AppPaths::for_test(self._temporary_directory.path())).unwrap()
    }

    fn lock_path(&self) -> PathBuf {
        self._temporary_directory.path().join("phase0-bootstrap.lock")
    }

    fn finish(
        &mut self,
        state: &mut ai_stock_forum::recovery::BootstrapState,
        reason: ShutdownReason,
    ) -> Vec<ai_stock_forum::app::EventEnvelope> {
        RecoveryCoordinator::finish_session(
            &mut self.database,
            state,
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

    fn raw_event_count(&self) -> i64 {
        self.database
            .connection()
            .query_row("SELECT COUNT(*) FROM event_stream", [], |row| row.get(0))
            .unwrap()
    }

    fn projection_marker(&self) -> (i64, Option<String>, String) {
        self.database
            .connection()
            .query_row(
                "SELECT last_event_sequence, last_event_digest, projection_digest FROM projection_metadata WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap()
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

    fn insert_invalid_event(&mut self, schema_version: i64, payload_json: &str) {
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
                "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, previous_event_digest, payload_json, event_digest) VALUES (?1, '00000000-0000-0000-0000-000000000997', ?2, 'status_viewed', 'system', 1700000000997, '00000000-0000-0000-0000-000000000996', ?3, ?4, '0000000000000000000000000000000000000000000000000000000000000000')",
                params![sequence + 1, schema_version, digest, payload_json],
            )
            .unwrap();
    }

    fn insert_valid_sequence_gap(&mut self) {
        let (sequence, previous_digest): (i64, String) = self
            .database
            .connection()
            .query_row(
                "SELECT sequence, event_digest FROM event_stream ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let event_id = EventId::from_uuid(Uuid::from_u128(0x995));
        let correlation_id = CorrelationId::from_uuid(Uuid::from_u128(0x994));
        let previous_digest = Sha256Digest::parse(&previous_digest).unwrap();
        let gap_sequence = u64::try_from(sequence + 2).unwrap();
        let digest = sha256(
            &canonical_json_bytes(&TestDigestMaterial {
                digest_format_version: 1,
                sequence: gap_sequence,
                event_id: &event_id,
                event_schema_version: 1,
                event_type: "status_viewed",
                actor_kind: "system",
                actor_id: None,
                occurred_at_ms: 1_700_000_000_995,
                correlation_id: &correlation_id,
                causation_id: None,
                object: None,
                previous_event_digest: Some(&previous_digest),
                payload_json: "{}".to_owned(),
            })
            .unwrap(),
        );
        self.database
            .connection()
            .execute(
                "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, previous_event_digest, payload_json, event_digest) VALUES (?1, ?2, 1, 'status_viewed', 'system', ?3, ?4, ?5, '{}', ?6)",
                params![
                    i64::try_from(gap_sequence).unwrap(),
                    event_id.to_string(),
                    1_700_000_000_995_i64,
                    correlation_id.to_string(),
                    previous_digest.as_str(),
                    digest.as_str(),
                ],
            )
            .unwrap();
    }
}

#[derive(Serialize)]
struct TestDigestMaterial<'a> {
    digest_format_version: u16,
    sequence: u64,
    event_id: &'a EventId,
    event_schema_version: u16,
    event_type: &'a str,
    actor_kind: &'a str,
    actor_id: Option<&'a str>,
    occurred_at_ms: i64,
    correlation_id: &'a CorrelationId,
    causation_id: Option<&'a CausationId>,
    object: Option<&'a ObjectRef>,
    previous_event_digest: Option<&'a Sha256Digest>,
    payload_json: String,
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

struct RepeatedIds;

impl IdGenerator for RepeatedIds {
    fn next_uuid(&self) -> Uuid {
        Uuid::from_u128(1)
    }
}

#[test]
fn empty_database_creates_one_installation_and_current_session() {
    let mut fixture = Fixture::new();

    let state = fixture.bootstrap();

    assert_eq!(fixture.event_count("installation_initialized"), 1);
    assert_eq!(fixture.event_count("process_session_started"), 1);
    assert_eq!(fixture.event_count("projection_rebuilt"), 0);
    assert_eq!(state.projection().installation.as_ref().unwrap().installation_id, state.installation_id());
    assert!(state.projection().sessions[&state.session_id()].ended.is_none());
}

#[test]
fn clean_restart_reuses_installation_without_reporting_an_interruption() {
    let mut fixture = Fixture::new();
    let mut first = fixture.bootstrap();
    fixture.finish(&mut first, ShutdownReason::InputClosed);
    let first_installation_id = first.installation_id();
    let first_session_id = first.session_id();
    drop(first);

    let second = fixture.bootstrap();

    assert_eq!(first_installation_id, second.installation_id());
    assert_ne!(first_session_id, second.session_id());
    assert!(!second.previous_session_interrupted());
    assert_eq!(fixture.event_count("previous_session_interrupted"), 0);
}

#[test]
fn interrupted_prior_session_is_closed_once_and_exposed_only_on_that_boot() {
    let mut fixture = Fixture::new();
    let abandoned = fixture.bootstrap();
    let abandoned_session_id = abandoned.session_id();
    drop(abandoned);
    let recovered = Arc::new(Mutex::new(Vec::new()));
    let hooks: Vec<Box<dyn RecoveryHook>> = vec![Box::new(RecordingHook {
        recovered: Arc::clone(&recovered),
    })];

    let second = fixture.bootstrap_with_hooks(&hooks).unwrap();

    assert!(second.previous_session_interrupted());
    assert_eq!(*recovered.lock().unwrap(), vec![abandoned_session_id]);
    assert_eq!(fixture.event_count("previous_session_interrupted"), 1);
    let mut second = second;
    fixture.finish(&mut second, ShutdownReason::UserQuit);
    drop(second);
    let third = fixture.bootstrap();
    assert!(!third.previous_session_interrupted());
    assert_eq!(fixture.event_count("previous_session_interrupted"), 1);
}

#[test]
fn missing_projections_rebuild_from_the_verified_event_stream() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap();
    let installation_id = state.installation_id();
    drop(state);
    fixture.remove_projections();

    let recovered = fixture.bootstrap();

    assert_eq!(installation_id, recovered.installation_id());
    assert_eq!(fixture.event_count("projection_rebuilt"), 1);
}

#[test]
fn stale_projection_metadata_rebuilds_from_the_verified_event_stream() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap();
    let installation_id = state.installation_id();
    drop(state);
    fixture.corrupt_projection_metadata();

    let recovered = fixture.bootstrap();

    assert_eq!(installation_id, recovered.installation_id());
    assert_eq!(fixture.event_count("projection_rebuilt"), 1);
}

#[test]
fn corrupt_projection_rows_rebuild_from_the_verified_event_stream() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap();
    let installation_id = state.installation_id();
    drop(state);
    fixture.corrupt_projection_rows();

    let recovered = fixture.bootstrap();

    assert_eq!(installation_id, recovered.installation_id());
    assert_eq!(fixture.event_count("projection_rebuilt"), 1);
}

#[test]
fn corrupt_event_stream_refuses_bootstrap_without_appending_recovery_events() {
    let mut fixture = Fixture::new();
    fixture.bootstrap();
    fixture.insert_bad_event();
    let clock_before = fixture.clock.calls();
    let ids_before = fixture.ids.calls();
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
    assert_eq!(fixture.clock.calls() - clock_before, 0);
    assert_eq!(fixture.ids.calls() - ids_before, 0);
}

#[test]
fn failed_interruption_hook_rolls_back_the_interruption_event_and_projection() {
    let mut fixture = Fixture::new();
    let abandoned = fixture.bootstrap();
    let abandoned_session_id = abandoned.session_id();
    drop(abandoned);
    let event_count_before = fixture.event_count("process_session_started")
        + fixture.event_count("previous_session_interrupted");
    let hooks: Vec<Box<dyn RecoveryHook>> = vec![Box::new(FailingHook)];
    let clock_before = fixture.clock.calls();
    let ids_before = fixture.ids.calls();

    let error = fixture.bootstrap_with_hooks(&hooks).unwrap_err();

    assert_eq!(error.code(), "event_query_failed");
    assert_eq!(fixture.clock.calls() - clock_before, 1);
    assert_eq!(fixture.ids.calls() - ids_before, 2);
    assert_eq!(
        fixture.event_count("process_session_started") + fixture.event_count("previous_session_interrupted"),
        event_count_before
    );
    assert!(fixture
        .database
        .connection()
        .query_row(
            "SELECT ended_event_id IS NULL FROM process_session_projection WHERE session_id = ?1",
            [abandoned_session_id.to_string()],
            |row| row.get::<_, bool>(0),
        )
        .unwrap());

    let retry_clock_before = fixture.clock.calls();
    let retry_ids_before = fixture.ids.calls();
    let retried = fixture.bootstrap();
    assert!(retried.previous_session_interrupted());
    assert_eq!(fixture.clock.calls() - retry_clock_before, 2);
    assert_eq!(fixture.ids.calls() - retry_ids_before, 5);
}

#[test]
fn orderly_shutdown_ends_the_current_session_once() {
    let mut fixture = Fixture::new();
    let mut state = fixture.bootstrap();

    let clock_before = fixture.clock.calls();
    let ids_before = fixture.ids.calls();
    let first = fixture.finish(&mut state, ShutdownReason::UserQuit);
    assert_eq!(fixture.clock.calls() - clock_before, 1);
    assert_eq!(fixture.ids.calls() - ids_before, 2);
    let retry_clock_before = fixture.clock.calls();
    let retry_ids_before = fixture.ids.calls();
    let second = fixture.finish(&mut state, ShutdownReason::UserQuit);

    assert_eq!(first, second);
    assert_eq!(fixture.event_count("process_session_ended"), 1);
    assert_eq!(state.projection().sessions[&state.session_id()].ended.as_ref().unwrap().reason, ShutdownReason::UserQuit);
    assert_eq!(fixture.clock.calls() - retry_clock_before, 0);
    assert_eq!(fixture.ids.calls() - retry_ids_before, 0);
}

#[test]
fn bootstrap_rejects_a_second_live_instance_and_releases_the_guard_when_owner_drops() {
    let mut fixture = Fixture::new();
    let first = fixture.bootstrap();
    let mut peer = fixture.open_peer();
    let calls_before = fixture.ids.calls();

    let error = RecoveryCoordinator::bootstrap(&mut peer, &fixture.clock, &fixture.ids, &[]).unwrap_err();

    assert_eq!(error.code(), "already_running");
    assert_eq!(fixture.ids.calls(), calls_before);
    drop(first);
    let resumed = RecoveryCoordinator::bootstrap(&mut peer, &fixture.clock, &fixture.ids, &[]).unwrap();
    assert!(resumed.previous_session_interrupted());
}

#[test]
fn invalid_projection_metadata_on_an_empty_stream_rebuilds_before_initialization() {
    let mut fixture = Fixture::new();
    fixture
        .database
        .connection()
        .execute(
            "INSERT INTO projection_metadata (singleton, last_event_sequence, last_event_digest, projection_digest) VALUES (1, 0, ?1, ?2)",
            [
                "0000000000000000000000000000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000000",
            ],
        )
        .unwrap();

    let clock_before = fixture.clock.calls();
    let ids_before = fixture.ids.calls();
    let state = fixture.bootstrap();

    assert_eq!(fixture.event_count("projection_rebuilt"), 1);
    assert_eq!(fixture.event_count("installation_initialized"), 1);
    assert_eq!(state.projection().last_sequence, 3);
    assert_eq!(fixture.clock.calls() - clock_before, 3);
    assert_eq!(fixture.ids.calls() - ids_before, 8);
}

#[test]
fn unsupported_event_schema_refuses_bootstrap_without_mutation() {
    let mut unsupported = Fixture::new();
    unsupported.bootstrap();
    let unsupported_before = unsupported.event_count("process_session_started");
    let event_count_before = unsupported.raw_event_count();
    let marker_before = unsupported.projection_marker();
    let clock_before = unsupported.clock.calls();
    let ids_before = unsupported.ids.calls();
    unsupported.insert_invalid_event(2, "{}");
    let error = RecoveryCoordinator::bootstrap(
        &mut unsupported.database,
        &unsupported.clock,
        &unsupported.ids,
        &[],
    )
    .unwrap_err();
    assert_eq!(error.code(), "unsupported_event_schema");
    assert_eq!(
        unsupported
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM event_stream WHERE event_type = 'process_session_started'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap() as usize,
        unsupported_before
    );
    assert_eq!(unsupported.raw_event_count(), event_count_before + 1);
    assert_eq!(unsupported.projection_marker(), marker_before);
    assert_eq!(unsupported.clock.calls() - clock_before, 0);
    assert_eq!(unsupported.ids.calls() - ids_before, 0);
}

#[test]
fn malformed_event_refuses_bootstrap_without_mutation() {
    let mut malformed = Fixture::new();
    malformed.bootstrap();
    let event_count_before = malformed.raw_event_count();
    let marker_before = malformed.projection_marker();
    let clock_before = malformed.clock.calls();
    let ids_before = malformed.ids.calls();
    malformed.insert_invalid_event(1, "{\"unexpected\":true}");
    let error = RecoveryCoordinator::bootstrap(
        &mut malformed.database,
        &malformed.clock,
        &malformed.ids,
        &[],
    )
    .unwrap_err();
    assert_eq!(error.code(), "invalid_event_record");
    assert_eq!(malformed.raw_event_count(), event_count_before + 1);
    assert_eq!(malformed.projection_marker(), marker_before);
    assert_eq!(malformed.clock.calls() - clock_before, 0);
    assert_eq!(malformed.ids.calls() - ids_before, 0);
}

#[test]
fn sequence_gap_refuses_bootstrap_without_mutation() {
    let mut gapped = Fixture::new();
    gapped.bootstrap();
    let event_count_before = gapped.raw_event_count();
    let marker_before = gapped.projection_marker();
    let clock_before = gapped.clock.calls();
    let ids_before = gapped.ids.calls();
    gapped.insert_valid_sequence_gap();
    let error = RecoveryCoordinator::bootstrap(&mut gapped.database, &gapped.clock, &gapped.ids, &[])
        .unwrap_err();
    assert_eq!(error.code(), "event_sequence_gap");
    assert_eq!(gapped.raw_event_count(), event_count_before + 1);
    assert_eq!(gapped.projection_marker(), marker_before);
    assert_eq!(gapped.clock.calls() - clock_before, 0);
    assert_eq!(gapped.ids.calls() - ids_before, 0);
}

#[test]
fn duplicate_generator_values_cannot_create_a_second_session() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    let clock = TestClock::new();
    let ids = RepeatedIds;

    let error = RecoveryCoordinator::bootstrap(&mut database, &clock, &ids, &[]).unwrap_err();

    assert_eq!(error.code(), "event_id_conflict");
    assert_eq!(
        EventRepository::load_all(database.connection())
            .unwrap()
            .iter()
            .filter(|event| matches!(event.event, ai_stock_forum::app::ApplicationEvent::ProcessSessionStarted { .. }))
            .count(),
        0
    );
}

#[test]
fn finish_session_returns_the_authoritative_terminal_event_for_a_different_reason_retry() {
    let mut fixture = Fixture::new();
    let mut state = fixture.bootstrap();

    let first = fixture.finish(&mut state, ShutdownReason::UserQuit);
    let retry_clock_before = fixture.clock.calls();
    let retry_ids_before = fixture.ids.calls();
    let retry = fixture.finish(&mut state, ShutdownReason::ApplicationError);

    assert_eq!(retry, first);
    assert_eq!(fixture.event_count("process_session_ended"), 1);
    assert_eq!(fixture.clock.calls() - retry_clock_before, 0);
    assert_eq!(fixture.ids.calls() - retry_ids_before, 0);
}

#[test]
fn concurrent_shutdown_attempts_return_one_authoritative_terminal_event() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap();
    let state = Arc::new(Mutex::new(state));
    let database_a = fixture.open_peer();
    let database_b = fixture.open_peer();
    let barrier = Arc::new(Barrier::new(3));
    let barrier_a = Arc::clone(&barrier);
    let barrier_b = Arc::clone(&barrier);
    let state_a = Arc::clone(&state);
    let state_b = Arc::clone(&state);

    let first = thread::spawn(move || {
        let mut database = database_a;
        barrier_a.wait();
        let mut lifecycle = state_a.lock().unwrap();
        RecoveryCoordinator::finish_session(
            &mut database,
            &mut lifecycle,
            ShutdownReason::UserQuit,
            &TestClock::new(),
            &TestIds::starting_at(10_000),
        )
        .unwrap()
    });
    let second = thread::spawn(move || {
        let mut database = database_b;
        barrier_b.wait();
        let mut lifecycle = state_b.lock().unwrap();
        RecoveryCoordinator::finish_session(
            &mut database,
            &mut lifecycle,
            ShutdownReason::ApplicationError,
            &TestClock::new(),
            &TestIds::starting_at(20_000),
        )
        .unwrap()
    });
    barrier.wait();

    let first = first.join().unwrap();
    let second = second.join().unwrap();
    assert_eq!(first, second);
    assert_eq!(fixture.event_count("process_session_ended"), 1);
}

#[test]
fn nonempty_rebuild_and_ordinary_startup_have_exact_dependency_deltas() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap();
    assert_eq!(fixture.clock.calls(), 2);
    assert_eq!(fixture.ids.calls(), 6);
    drop(state);
    fixture.corrupt_projection_metadata();
    let clock_before = fixture.clock.calls();
    let ids_before = fixture.ids.calls();

    let rebuilt = fixture.bootstrap();

    assert!(rebuilt.previous_session_interrupted());
    assert_eq!(fixture.clock.calls() - clock_before, 3);
    assert_eq!(fixture.ids.calls() - ids_before, 7);
}

#[test]
fn simultaneous_bootstrap_allows_one_owner_and_retains_its_guard_until_drop() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    Database::open(&paths).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let (sender, receiver) = mpsc::channel();
    let mut handles = Vec::new();
    for seed in [1_u128, 10_000] {
        let paths = paths.clone();
        let barrier = Arc::clone(&barrier);
        let sender = sender.clone();
        handles.push(thread::spawn(move || {
            let mut database = Database::open(&paths).unwrap();
            let clock = TestClock::new();
            let ids = TestIds::starting_at(seed);
            barrier.wait();
            let result = RecoveryCoordinator::bootstrap(&mut database, &clock, &ids, &[]);
            sender.send((result, clock.calls(), ids.calls())).unwrap();
        }));
    }
    drop(sender);
    barrier.wait();
    let mut winner = None;
    let mut loser_counts = None;
    for _ in 0..2 {
        let (result, clock_calls, id_calls) = receiver.recv().unwrap();
        match result {
            Ok(state) => {
                assert_eq!((clock_calls, id_calls), (2, 6));
                winner = Some(state);
            }
            Err(error) => {
                assert_eq!(error.code(), "already_running");
                assert_eq!((clock_calls, id_calls), (0, 0));
                loser_counts = Some((clock_calls, id_calls));
            }
        }
    }
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(loser_counts, Some((0, 0)));
    let mut peer = Database::open(&paths).unwrap();
    let peer_clock = TestClock::new();
    let peer_ids = TestIds::starting_at(20_000);
    assert_eq!(
        RecoveryCoordinator::bootstrap(&mut peer, &peer_clock, &peer_ids, &[])
            .unwrap_err()
            .code(),
        "already_running"
    );
    drop(winner);
    let resumed = RecoveryCoordinator::bootstrap(&mut peer, &peer_clock, &peer_ids, &[]).unwrap();
    assert!(resumed.previous_session_interrupted());
}

#[cfg(unix)]
#[test]
fn process_guard_refuses_terminal_and_intermediate_symlinks_without_touching_targets() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let root = temporary_directory.path().join("state");
    let paths = AppPaths::for_test(&root);
    let database = Database::open(&paths).unwrap();
    let target = temporary_directory.path().join("target");
    fs::write(&target, b"unchanged").unwrap();
    symlink(&target, root.join("phase0-bootstrap.lock")).unwrap();
    assert_eq!(database.acquire_process_guard().unwrap_err().code(), "state_permissions");
    assert_eq!(fs::read(&target).unwrap(), b"unchanged");

    fs::remove_file(root.join("phase0-bootstrap.lock")).unwrap();
    let redirected = temporary_directory.path().join("redirected-state");
    fs::rename(&root, &redirected).unwrap();
    fs::write(redirected.join("target-marker"), b"unchanged").unwrap();
    symlink(&redirected, &root).unwrap();
    assert_eq!(database.acquire_process_guard().unwrap_err().code(), "state_directory_unavailable");
    assert_eq!(fs::read(redirected.join("target-marker")).unwrap(), b"unchanged");
}

#[cfg(unix)]
#[test]
fn process_guard_refuses_fifo_and_socket_when_socket_fixture_is_available() {
    let fixture = Fixture::new();
    let lock = fixture.lock_path();
    let fifo = CString::new(lock.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert_eq!(fixture.database.acquire_process_guard().unwrap_err().code(), "state_permissions");
    fs::remove_file(&lock).unwrap();
    #[cfg(target_os = "macos")]
    let socket_path = PathBuf::from("/private").join(lock.strip_prefix("/").unwrap());
    #[cfg(not(target_os = "macos"))]
    let socket_path = lock.clone();
    match UnixListener::bind(&socket_path) {
        Ok(listener) => {
            assert_eq!(fixture.database.acquire_process_guard().unwrap_err().code(), "state_permissions");
            drop(listener);
            fs::remove_file(&lock).unwrap();
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
        Err(error) => panic!("could not create socket fixture: {error}"),
    }
}

#[cfg(unix)]
#[test]
fn process_guard_corrects_existing_regular_lock_file_to_owner_only_mode() {
    let fixture = Fixture::new();
    let lock = fixture.lock_path();
    fs::write(&lock, b"").unwrap();
    fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
    let guard = fixture.database.acquire_process_guard().unwrap();
    assert_eq!(fs::metadata(&lock).unwrap().mode() & 0o777, 0o600);
    drop(guard);
}
