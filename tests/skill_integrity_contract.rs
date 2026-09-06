use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use ai_stock_forum::{
    config::AppPaths,
    domain::{Clock, IdGenerator, SkillId, SkillVersionId},
    persistence::{Database, insert_skill_version},
    recovery::RecoveryCoordinator,
    skills::{SkillDraft, SkillProvenance, SkillVersion, builtin_manifests},
};
use tempfile::TempDir;
use uuid::Uuid;

struct TestClock(AtomicI64);

impl Clock for TestClock {
    fn now_millis(&self) -> i64 {
        self.0.fetch_add(1, Ordering::SeqCst)
    }
}

struct TestIds(AtomicU64);

impl IdGenerator for TestIds {
    fn next_uuid(&self) -> Uuid {
        Uuid::from_u128(u128::from(self.0.fetch_add(1, Ordering::SeqCst)))
    }
}

struct Fixture {
    _temp: TempDir,
    database: Database,
    clock: TestClock,
    ids: TestIds,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
        Self {
            _temp: temp,
            database,
            clock: TestClock(AtomicI64::new(1_726_200_000_000)),
            ids: TestIds(AtomicU64::new(60_000)),
        }
    }

    fn bootstrap(&mut self) -> Result<ai_stock_forum::recovery::BootstrapState, ai_stock_forum::config::StartupError> {
        RecoveryCoordinator::bootstrap(&mut self.database, &self.clock, &self.ids, &[])
    }

    fn count(&self, table: &str) -> i64 {
        assert!(matches!(table, "skill_versions" | "active_skills" | "event_stream"));
        self.database
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    }
}

fn unexpected_skill() -> SkillVersion {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(70_001)),
        SkillVersionId::from_uuid(Uuid::from_u128(70_002)),
        1_726_000_000_000,
        SkillProvenance::User,
        SkillDraft::new(
            "Unexpected Skill".to_owned(),
            "Has no authoritative event.".to_owned(),
            "Never active without an event.".to_owned(),
            Vec::new(),
            "Recovery must reject this row.".to_owned(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn builtin_seeding_is_idempotent_and_restores_a_missing_canonical_row() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap().unwrap();
    drop(state);
    assert_eq!(fixture.count("skill_versions"), 4);
    assert_eq!(fixture.count("active_skills"), 4);
    let missing_id = builtin_manifests().unwrap()[0].skill().skill_id();
    fixture
        .database
        .connection()
        .execute_batch("DROP TRIGGER skill_versions_no_delete")
        .unwrap();
    fixture
        .database
        .connection()
        .execute(
            "DELETE FROM active_skills WHERE skill_id = ?1",
            [missing_id.to_string()],
        )
        .unwrap();
    fixture
        .database
        .connection()
        .execute(
            "DELETE FROM skill_versions WHERE skill_id = ?1",
            [missing_id.to_string()],
        )
        .unwrap();

    let state = fixture.bootstrap().unwrap();
    drop(state);
    assert_eq!(fixture.count("skill_versions"), 4);
    assert_eq!(fixture.count("active_skills"), 4);

    let state = fixture.bootstrap().unwrap();
    drop(state);
    assert_eq!(fixture.count("skill_versions"), 4);
    assert_eq!(fixture.count("active_skills"), 4);
}

#[test]
fn altered_immutable_builtin_row_fails_closed_before_startup_writes() {
    let mut fixture = Fixture::new();
    let state = fixture.bootstrap().unwrap();
    drop(state);
    fixture
        .database
        .connection()
        .execute_batch("DROP TRIGGER skill_versions_no_update")
        .unwrap();
    fixture
        .database
        .connection()
        .execute(
            "DELETE FROM active_skills WHERE skill_id = ?1",
            [builtin_manifests().unwrap()[0].skill().skill_id().to_string()],
        )
        .unwrap();
    fixture
        .database
        .connection()
        .execute(
            "UPDATE skill_versions
             SET content_digest = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
             WHERE skill_id = ?1",
            [builtin_manifests().unwrap()[0].skill().skill_id().to_string()],
        )
        .unwrap();
    let events_before = fixture.count("event_stream");

    let error = fixture.bootstrap().unwrap_err();

    assert_eq!(error.code(), "skill_version_integrity_mismatch");
    assert_eq!(fixture.count("event_stream"), events_before);
}

#[test]
fn unexpected_immutable_skill_row_fails_closed_without_deleting_it() {
    let mut fixture = Fixture::new();
    let unexpected = unexpected_skill();
    let transaction = fixture.database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &unexpected).unwrap();
    transaction.commit().unwrap();

    let error = fixture.bootstrap().unwrap_err();

    assert_eq!(error.code(), "skill_version_integrity_mismatch");
    assert_eq!(fixture.count("event_stream"), 0);
    assert_eq!(fixture.count("skill_versions"), 1);
}
