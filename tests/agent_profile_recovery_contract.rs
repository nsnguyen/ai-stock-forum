use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, PendingEvent},
    config::{AppPaths, StartupError},
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, Clock, CorrelationId, EventId, IdGenerator,
        MemoryNamespaceId,
    },
    persistence::{
        Database, EventRepository, RecoveryError, insert_expected_version, load_all_versions,
    },
    recovery::{BootstrapState, RecoveryCoordinator},
};
use tempfile::TempDir;
use uuid::Uuid;

struct TestClock(AtomicI64);

impl TestClock {
    fn new() -> Self {
        Self(AtomicI64::new(1_726_000_100_000))
    }
}

impl Clock for TestClock {
    fn now_millis(&self) -> i64 {
        self.0.fetch_add(1, Ordering::SeqCst)
    }
}

struct TestIds(AtomicU64);

impl TestIds {
    fn new() -> Self {
        Self(AtomicU64::new(10_000))
    }
}

impl IdGenerator for TestIds {
    fn next_uuid(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.0.fetch_add(1, Ordering::SeqCst)))
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

    fn append(&mut self, event_id: u128, event: ApplicationEvent) {
        let transaction = self.database.immediate_transaction().unwrap();
        EventRepository::append(
            &transaction,
            PendingEvent {
                event_id: EventId::from_uuid(Uuid::from_u128(event_id)),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 1_726_000_000_000 + i64::try_from(event_id).unwrap(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(event_id + 1_000)),
                causation_id: None,
                object: None,
                event,
            },
        )
        .unwrap();
        transaction.commit().unwrap();
    }

    fn bootstrap(&mut self) -> Result<BootstrapState, StartupError> {
        RecoveryCoordinator::bootstrap(&mut self.database, &self.clock, &self.ids, &[])
    }

    fn materialize(&mut self, event_sequence: i64, profile: &AgentProfileVersion) {
        let transaction = self.database.connection_mut().transaction().unwrap();
        insert_expected_version(&transaction, event_sequence, profile).unwrap();
        transaction.commit().unwrap();
    }

    fn insert_active(&mut self, profile: &AgentProfileVersion) {
        self.database
            .connection()
            .execute(
                "INSERT INTO active_agent_profiles (
                    profile_id, profile_version_id, version, normalized_name, readiness
                 ) VALUES (?1, ?2, ?3, ?4, 'not_ready')",
                rusqlite::params![
                    profile.profile_id().to_string(),
                    profile.profile_version_id().to_string(),
                    i64::try_from(profile.version().get()).unwrap(),
                    profile.normalized_name().as_str(),
                ],
            )
            .unwrap();
    }

    fn active_version_id(&self) -> Option<String> {
        self.database
            .connection()
            .query_row(
                "SELECT profile_version_id FROM active_agent_profiles",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
    }
}

use rusqlite::OptionalExtension;

fn profile_id(value: u128) -> AgentProfileId {
    AgentProfileId::from_uuid(Uuid::from_u128(value))
}

fn version_id(value: u128) -> AgentProfileVersionId {
    AgentProfileVersionId::from_uuid(Uuid::from_u128(value))
}

fn draft(name: &str, description: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        description.to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical.".to_owned(),
        "Cite evidence before making a claim.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn profile_v1() -> AgentProfileVersion {
    AgentProfileVersion::create(
        profile_id(1),
        version_id(11),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(101)),
        1_726_000_000_000,
        draft("Research Analyst", "Sensitive profile prose."),
        None,
    )
    .unwrap()
}

fn profile_v2(first: &AgentProfileVersion) -> AgentProfileVersion {
    AgentProfileVersion::next_version(
        first,
        version_id(12),
        1_726_000_000_001,
        draft("Senior Research Analyst", "Independent equity research."),
    )
    .unwrap()
}

fn append_create(fixture: &mut Fixture, profile: &AgentProfileVersion) {
    fixture.append(
        201,
        ApplicationEvent::AgentProfileCreated {
            profile: profile.clone(),
        },
    );
}

fn append_activation(
    fixture: &mut Fixture,
    first: &AgentProfileVersion,
    second: &AgentProfileVersion,
) {
    fixture.append(
        202,
        ApplicationEvent::AgentProfileVersionActivated {
            profile: second.clone(),
            previous_version_id: first.profile_version_id(),
        },
    );
}

fn immutable_snapshot(database: &Database) -> Vec<(String, String, i64, String, String, Vec<u8>, i64, i64)> {
    let mut statement = database
        .connection()
        .prepare(
            "SELECT profile_id, profile_version_id, version, normalized_name, content_digest,
                    payload_json, source_event_sequence, created_at_ms
             FROM agent_profile_versions ORDER BY source_event_sequence",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn missing_immutable_row_is_backfilled_and_activated_from_verified_events() {
    let mut fixture = Fixture::new();
    let profile = profile_v1();
    append_create(&mut fixture, &profile);

    let state = fixture.bootstrap().unwrap();

    let stored = load_all_versions(fixture.database.connection()).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].event_sequence, 1);
    assert_eq!(stored[0].profile, profile);
    assert_eq!(
        state
            .projection()
            .agent_profiles
            .active_profile(profile.profile_id()),
        Some(&profile)
    );
    assert_eq!(fixture.active_version_id(), Some(profile.profile_version_id().to_string()));
}

#[test]
fn altered_immutable_payload_digest_sequence_or_identity_refuses_startup() {
    for mutation in [
        "UPDATE agent_profile_versions SET payload_json = CAST(payload_json || X'20' AS BLOB)",
        "UPDATE agent_profile_versions SET content_digest = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE agent_profile_versions SET source_event_sequence = 99",
        "UPDATE agent_profile_versions SET profile_version_id = '00000000-0000-0000-0000-000000000099'",
    ] {
        let mut fixture = Fixture::new();
        let profile = profile_v1();
        append_create(&mut fixture, &profile);
        fixture.materialize(1, &profile);
        fixture
            .database
            .connection()
            .execute_batch("DROP TRIGGER agent_profile_versions_no_update")
            .unwrap();
        fixture.database.connection().execute(mutation, []).unwrap();

        let error = fixture.bootstrap().unwrap_err();

        assert_eq!(error.code(), "database_agent_profile_history_mismatch");
        assert!(!error.to_string().contains("Sensitive profile prose"));
        assert_eq!(
            fixture
                .database
                .connection()
                .query_row("SELECT COUNT(*) FROM event_stream", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn unexpected_extra_immutable_row_refuses_startup_without_rewriting_history() {
    let mut fixture = Fixture::new();
    let expected = profile_v1();
    let extra = AgentProfileVersion::create(
        profile_id(2),
        version_id(21),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(102)),
        1_726_000_000_002,
        draft("Extra Analyst", "Unexpected history."),
        None,
    )
    .unwrap();
    append_create(&mut fixture, &expected);
    fixture.materialize(1, &expected);
    fixture.materialize(99, &extra);
    let before = immutable_snapshot(&fixture.database);

    let error = fixture.bootstrap().unwrap_err();

    assert_eq!(error.code(), "unexpected_agent_profile_history");
    assert_eq!(immutable_snapshot(&fixture.database), before);
}

#[test]
fn missing_stale_and_corrupt_active_rows_are_rebuilt_from_verified_events() {
    for active_fixture in ["missing", "stale", "corrupt"] {
        let mut fixture = Fixture::new();
        let first = profile_v1();
        let second = profile_v2(&first);
        append_create(&mut fixture, &first);
        append_activation(&mut fixture, &first, &second);
        fixture.materialize(1, &first);
        fixture.materialize(2, &second);
        if active_fixture != "missing" {
            fixture.insert_active(if active_fixture == "stale" { &first } else { &second });
        }
        if active_fixture == "corrupt" {
            fixture
                .database
                .connection()
                .execute(
                    "UPDATE active_agent_profiles SET normalized_name = 'corrupt-name'",
                    [],
                )
                .unwrap();
        }

        let immutable_before = immutable_snapshot(&fixture.database);
        let state = fixture.bootstrap().unwrap();

        assert_eq!(fixture.active_version_id(), Some(second.profile_version_id().to_string()));
        assert_eq!(
            state
                .projection()
                .agent_profiles
                .active_profile(first.profile_id()),
            Some(&second)
        );
        assert_eq!(immutable_snapshot(&fixture.database), immutable_before);
    }
}

#[test]
fn active_rebuild_failure_rolls_back_pointer_replacement_and_missing_history_backfill() {
    let mut fixture = Fixture::new();
    let first = profile_v1();
    let second = profile_v2(&first);
    append_create(&mut fixture, &first);
    append_activation(&mut fixture, &first, &second);
    fixture.materialize(1, &first);
    fixture.insert_active(&first);
    fixture
        .database
        .connection()
        .execute_batch(
            "CREATE TRIGGER fail_active_profile_rebuild
             BEFORE INSERT ON active_agent_profiles BEGIN
                 SELECT RAISE(ABORT, 'injected active rebuild failure');
             END;",
        )
        .unwrap();

    let error = fixture.bootstrap().unwrap_err();

    assert_eq!(error.code(), "active_agent_profile_rebuild_failed");
    assert_eq!(fixture.active_version_id(), Some(first.profile_version_id().to_string()));
    assert_eq!(load_all_versions(fixture.database.connection()).unwrap().len(), 1);
    assert_eq!(immutable_snapshot(&fixture.database)[0].1, first.profile_version_id().to_string());
}

#[test]
fn legacy_event_stream_without_profile_events_recovers_as_empty_profile_state() {
    let mut fixture = Fixture::new();
    fixture.append(501, ApplicationEvent::HelpViewed);

    let state = fixture.bootstrap().unwrap();

    assert!(state.projection().agent_profiles.active_profiles().is_empty());
    assert!(load_all_versions(fixture.database.connection()).unwrap().is_empty());
    assert_eq!(
        fixture
            .database
            .connection()
            .query_row("SELECT COUNT(*) FROM active_agent_profiles", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn recovery_error_codes_and_display_are_stable_and_redacted() {
    for (error, code) in [
        (
            RecoveryError::AgentProfileHistoryMismatch,
            "database_agent_profile_history_mismatch",
        ),
        (
            RecoveryError::UnexpectedAgentProfileHistory,
            "unexpected_agent_profile_history",
        ),
        (
            RecoveryError::InvalidAgentProfilePayload,
            "invalid_agent_profile_payload",
        ),
        (
            RecoveryError::ActiveAgentProfileRebuildFailed,
            "active_agent_profile_rebuild_failed",
        ),
    ] {
        assert_eq!(error.code(), code);
        assert!(!error.to_string().contains("Sensitive profile prose"));
    }
}
