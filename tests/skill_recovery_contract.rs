use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, PendingEvent},
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, Clock, CorrelationId, EventId, IdGenerator,
        MemoryNamespaceId, SkillId, SkillVersionId, canonical_json_bytes, sha256,
    },
    persistence::{
        Database, EventRepository, ProjectionRepository, insert_skill_version,
    },
    recovery::RecoveryCoordinator,
    skills::{SkillDraft, SkillProvenance, SkillVersion, builtin_manifests},
};
use tempfile::TempDir;
use uuid::Uuid;

struct TestClock(AtomicI64);

impl TestClock {
    fn new() -> Self {
        Self(AtomicI64::new(1_726_100_000_000))
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
        Self(AtomicU64::new(50_000))
    }
}

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
    next_event: u128,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
        Self {
            _temp: temp,
            database,
            clock: TestClock::new(),
            ids: TestIds::new(),
            next_event: 80_000,
        }
    }

    fn append(&mut self, event: ApplicationEvent) {
        let id = self.next_event;
        self.next_event += 2;
        let transaction = self.database.immediate_transaction().unwrap();
        EventRepository::append(
            &transaction,
            PendingEvent {
                event_id: EventId::from_uuid(Uuid::from_u128(id)),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 1_726_000_000_000 + i64::try_from(id).unwrap(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 1)),
                causation_id: None,
                object: None,
                event,
            },
        )
        .unwrap();
        transaction.commit().unwrap();
    }
}

fn profile_with_skill(skill: &SkillVersion) -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(90_001)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(90_002)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(90_003)),
        1_726_000_000_000,
        AgentProfileDraft::new(
            "Recovery Analyst".to_owned(),
            "Validates deterministic startup recovery.".to_owned(),
            AgentRole::Custom,
            "recovery".to_owned(),
            vec!["integrity".to_owned()],
            "Careful and exact.".to_owned(),
            "Fail closed on inconsistent state.".to_owned(),
            AgentBindings::default(),
            vec![skill.reference()],
            Vec::new(),
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn custom_skill_v1() -> SkillVersion {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(91_001)),
        SkillVersionId::from_uuid(Uuid::from_u128(91_011)),
        1_726_000_000_000,
        SkillProvenance::User,
        SkillDraft::new(
            "Recovery Review".to_owned(),
            "Review recovered state.".to_owned(),
            "Use after an interrupted write.".to_owned(),
            vec!["recovery".to_owned()],
            "Compare immutable records with authoritative events.".to_owned(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn skill_rows(database: &Database) -> Vec<(String, String, i64)> {
    let mut statement = database
        .connection()
        .prepare(
            "SELECT skill_id, skill_version_id, version
             FROM skill_versions ORDER BY skill_id, version",
        )
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn active_rows(database: &Database) -> Vec<(String, String, i64, String, String, String)> {
    let mut statement = database
        .connection()
        .prepare(
            "SELECT skill_id, skill_version_id, version, normalized_name,
                    content_digest, record_digest
             FROM active_skills ORDER BY skill_id",
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
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn builtin_versions_reconcile_before_profile_skill_references_and_assignment_rebuild() {
    let mut fixture = Fixture::new();
    let builtin = builtin_manifests().unwrap()[0].skill().clone();
    let profile = profile_with_skill(&builtin);
    fixture.append(ApplicationEvent::AgentProfileCreated {
        profile: profile.clone(),
    });

    let state = RecoveryCoordinator::bootstrap(
        &mut fixture.database,
        &fixture.clock,
        &fixture.ids,
        &[],
    )
    .unwrap();

    assert_eq!(skill_rows(&fixture.database).len(), 4);
    assert_eq!(active_rows(&fixture.database).len(), 4);
    assert_eq!(
        state
            .projection()
            .agent_profiles
            .active_profile(profile.profile_id()),
        Some(&profile),
    );
    assert_eq!(
        fixture
            .database
            .connection()
            .query_row(
                "SELECT profile_version_id FROM active_agent_profiles WHERE profile_id = ?1",
                [profile.profile_id().to_string()],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        profile.profile_version_id().to_string(),
    );
}

#[test]
fn active_skills_rebuild_from_events_and_repeated_rebuild_is_deterministic() {
    let mut fixture = Fixture::new();
    let first = custom_skill_v1();
    let second = SkillVersion::next_version(
        &first,
        SkillVersionId::from_uuid(Uuid::from_u128(91_012)),
        1_726_000_000_001,
        SkillDraft::new(
            "Recovery Review".to_owned(),
            "Review recovered state deterministically.".to_owned(),
            "Use after an interrupted write.".to_owned(),
            vec!["recovery".to_owned()],
            "Rebuild only from verified immutable inputs.".to_owned(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();
    let transaction = fixture.database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &first).unwrap();
    insert_skill_version(&transaction, &second).unwrap();
    transaction.commit().unwrap();
    fixture.append(ApplicationEvent::SkillCreated {
        skill: first.reference(),
        display_name: first.content().display_name.clone(),
        provenance: first.provenance().clone(),
    });
    fixture.append(ApplicationEvent::SkillVersionActivated {
        skill: second.reference(),
        previous_version_id: first.skill_version_id(),
        display_name: second.content().display_name.clone(),
        provenance: second.provenance().clone(),
    });
    let events = EventRepository::load_all(fixture.database.connection()).unwrap();

    let first_state = ProjectionRepository::rebuild(fixture.database.connection_mut(), &events)
        .unwrap();
    let first_skills = skill_rows(&fixture.database);
    let first_active = active_rows(&fixture.database);
    let first_digest = first_state.digest().unwrap();
    let first_metadata = fixture
        .database
        .connection()
        .query_row(
            "SELECT last_event_sequence, last_event_digest, projection_digest
             FROM projection_metadata WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?)),
        )
        .unwrap();

    let second_state = ProjectionRepository::rebuild(fixture.database.connection_mut(), &events)
        .unwrap();

    assert_eq!(skill_rows(&fixture.database), first_skills);
    assert_eq!(active_rows(&fixture.database), first_active);
    assert_eq!(second_state.digest().unwrap(), first_digest);
    assert_eq!(
        fixture
            .database
            .connection()
            .query_row(
                "SELECT last_event_sequence, last_event_digest, projection_digest
                 FROM projection_metadata WHERE singleton = 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?)),
            )
            .unwrap(),
        first_metadata,
    );
    assert_eq!(first_skills.len(), 6);
    assert_eq!(first_active.len(), 5);
    assert!(first_active.iter().any(|row| {
        row.0 == second.skill_id().to_string()
            && row.1 == second.skill_version_id().to_string()
            && row.2 == 2
    }));
}

#[test]
fn empty_skill_state_preserves_legacy_projection_bytes_and_digest() {
    let state = ai_stock_forum::recovery::ProjectionState::default();
    let legacy = br#"{"installation":null,"last_event_digest":null,"last_sequence":0,"sessions":{},"setup_status":"not_started"}"#;

    assert_eq!(canonical_json_bytes(&state).unwrap(), legacy);
    assert_eq!(state.digest().unwrap(), sha256(legacy));
}
