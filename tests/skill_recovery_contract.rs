use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, PendingEvent},
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, Clock, CorrelationId, EventId, IdGenerator,
        MemoryNamespaceId, ObjectRef, SkillId, SkillVersionId, canonical_json_bytes, sha256,
    },
    persistence::{
        Database, EventRepository, ProjectionRepository, insert_skill_version,
    },
    recovery::{ProjectionState, RecoveryCoordinator, reduce},
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

    fn append(&mut self, event: ApplicationEvent) -> ai_stock_forum::app::EventEnvelope {
        self.append_with_object(event, None)
    }

    fn append_with_object(
        &mut self,
        event: ApplicationEvent,
        object: Option<ObjectRef>,
    ) -> ai_stock_forum::app::EventEnvelope {
        let id = self.next_event;
        self.next_event += 2;
        let transaction = self.database.immediate_transaction().unwrap();
        let envelope = EventRepository::append(
            &transaction,
            PendingEvent {
                event_id: EventId::from_uuid(Uuid::from_u128(id)),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 1_726_000_000_000 + i64::try_from(id).unwrap(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 1)),
                causation_id: None,
                object,
                event,
            },
        )
        .unwrap();
        transaction.commit().unwrap();
        envelope
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

fn skill_object(skill: &SkillVersion) -> ObjectRef {
    ObjectRef::new(
        "skill_version",
        skill.skill_version_id().to_string(),
        skill.version(),
        sha256(&canonical_json_bytes(skill).unwrap()),
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
    fixture.append_with_object(
        ApplicationEvent::SkillCreated {
            skill: first.reference(),
            display_name: first.content().display_name.clone(),
            provenance: first.provenance().clone(),
        },
        Some(skill_object(&first)),
    );
    fixture.append_with_object(
        ApplicationEvent::SkillVersionActivated {
            skill: second.reference(),
            previous_version_id: first.skill_version_id(),
            display_name: second.content().display_name.clone(),
            provenance: second.provenance().clone(),
        },
        Some(skill_object(&second)),
    );
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

#[test]
fn skill_mutation_events_require_an_exact_object_before_reduction() {
    let skill = custom_skill_v1();
    let event = ApplicationEvent::SkillCreated {
        skill: skill.reference(),
        display_name: skill.content().display_name.clone(),
        provenance: skill.provenance().clone(),
    };
    let bad_objects = [
        None,
        Some(
            ObjectRef::new(
                "wrong_kind",
                skill.skill_version_id().to_string(),
                skill.version(),
                sha256(&canonical_json_bytes(&skill).unwrap()),
            )
            .unwrap(),
        ),
        Some(
            ObjectRef::new(
                "skill_version",
                SkillVersionId::from_uuid(Uuid::from_u128(999_001)).to_string(),
                skill.version(),
                sha256(&canonical_json_bytes(&skill).unwrap()),
            )
            .unwrap(),
        ),
        Some(
            ObjectRef::new(
                "skill_version",
                skill.skill_version_id().to_string(),
                ai_stock_forum::domain::ObjectVersion::new(2).unwrap(),
                sha256(&canonical_json_bytes(&skill).unwrap()),
            )
            .unwrap(),
        ),
    ];

    for object in bad_objects {
        let mut fixture = Fixture::new();
        let envelope = fixture.append_with_object(event.clone(), object);
        let mut state = ProjectionState::default();

        assert_eq!(
            reduce(&mut state, &envelope).unwrap_err(),
            ai_stock_forum::persistence::RecoveryError::InvalidEventRecord,
        );
        assert_eq!(state, ProjectionState::default());
    }
}

#[test]
fn duplicate_skill_event_object_is_rejected_without_projection_mutation() {
    let mut fixture = Fixture::new();
    let skill = custom_skill_v1();
    let event = ApplicationEvent::SkillCreated {
        skill: skill.reference(),
        display_name: skill.content().display_name.clone(),
        provenance: skill.provenance().clone(),
    };
    let first = fixture.append_with_object(event.clone(), Some(skill_object(&skill)));
    let duplicate = fixture.append_with_object(event, Some(skill_object(&skill)));
    let mut state = ProjectionState::default();
    reduce(&mut state, &first).unwrap();
    let before = state.clone();

    assert_eq!(
        reduce(&mut state, &duplicate).unwrap_err(),
        ai_stock_forum::persistence::RecoveryError::InvalidEventRecord,
    );
    assert_eq!(state, before);
}

#[test]
fn orphan_and_noncanonical_root_activations_are_rejected() {
    let first = custom_skill_v1();
    let second = SkillVersion::next_version(
        &first,
        SkillVersionId::from_uuid(Uuid::from_u128(91_099)),
        1_726_000_000_099,
        SkillDraft::new(
            "Recovery Review".to_owned(),
            "Orphaned custom version.".to_owned(),
            "Never valid without version one.".to_owned(),
            vec!["recovery".to_owned()],
            "Reject this chain.".to_owned(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut fixture = Fixture::new();
    let orphan = fixture.append_with_object(
        ApplicationEvent::SkillVersionActivated {
            skill: second.reference(),
            previous_version_id: first.skill_version_id(),
            display_name: second.content().display_name.clone(),
            provenance: second.provenance().clone(),
        },
        Some(skill_object(&second)),
    );
    let mut state = ProjectionState::default();
    assert_eq!(
        reduce(&mut state, &orphan).unwrap_err(),
        ai_stock_forum::persistence::RecoveryError::InvalidEventRecord,
    );

    let builtin = builtin_manifests().unwrap()[0].skill().clone();
    let builtin_v2 = SkillVersion::next_version(
        &builtin,
        SkillVersionId::from_uuid(Uuid::from_u128(92_002)),
        1_726_000_000_100,
        SkillDraft::new(
            builtin.content().display_name.clone(),
            builtin.content().description.clone(),
            builtin.content().use_when.clone(),
            builtin.content().tags.clone(),
            "Canonical built-in version two.".to_owned(),
            builtin.content().resources.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut builtin_fixture = Fixture::new();
    let wrong_predecessor = builtin_fixture.append_with_object(
        ApplicationEvent::SkillVersionActivated {
            skill: builtin_v2.reference(),
            previous_version_id: SkillVersionId::from_uuid(Uuid::from_u128(92_999)),
            display_name: builtin_v2.content().display_name.clone(),
            provenance: builtin_v2.provenance().clone(),
        },
        Some(skill_object(&builtin_v2)),
    );
    assert_eq!(
        reduce(&mut ProjectionState::default(), &wrong_predecessor).unwrap_err(),
        ai_stock_forum::persistence::RecoveryError::InvalidEventRecord,
    );
}

#[test]
fn cross_skill_canonical_predecessor_is_rejected() {
    let manifests = builtin_manifests().unwrap();
    let builtin = manifests[0].skill().clone();
    let other_builtin = manifests[1].skill().clone();
    let successor = SkillVersion::next_version(
        &builtin,
        SkillVersionId::from_uuid(Uuid::from_u128(92_102)),
        1_726_000_000_102,
        SkillDraft::new(
            builtin.content().display_name.clone(),
            builtin.content().description.clone(),
            builtin.content().use_when.clone(),
            builtin.content().tags.clone(),
            "Reject a predecessor from another canonical skill.".to_owned(),
            builtin.content().resources.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut fixture = Fixture::new();
    let envelope = fixture.append_with_object(
        ApplicationEvent::SkillVersionActivated {
            skill: successor.reference(),
            previous_version_id: other_builtin.skill_version_id(),
            display_name: successor.content().display_name.clone(),
            provenance: successor.provenance().clone(),
        },
        Some(skill_object(&successor)),
    );
    let mut state = ProjectionState::default();

    assert_eq!(
        reduce(&mut state, &envelope).unwrap_err(),
        ai_stock_forum::persistence::RecoveryError::InvalidEventRecord,
    );
    assert_eq!(state, ProjectionState::default());
}

#[test]
fn wrong_version_for_canonical_predecessor_is_rejected() {
    let builtin = builtin_manifests().unwrap()[0].skill().clone();
    let version_two = SkillVersion::next_version(
        &builtin,
        SkillVersionId::from_uuid(Uuid::from_u128(92_202)),
        1_726_000_000_202,
        SkillDraft::new(
            builtin.content().display_name.clone(),
            builtin.content().description.clone(),
            builtin.content().use_when.clone(),
            builtin.content().tags.clone(),
            "Canonical immediate successor.".to_owned(),
            builtin.content().resources.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    let version_three = SkillVersion::next_version(
        &version_two,
        SkillVersionId::from_uuid(Uuid::from_u128(92_203)),
        1_726_000_000_203,
        SkillDraft::new(
            builtin.content().display_name.clone(),
            builtin.content().description.clone(),
            builtin.content().use_when.clone(),
            builtin.content().tags.clone(),
            "Reject a non-immediate successor.".to_owned(),
            builtin.content().resources.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut fixture = Fixture::new();
    let envelope = fixture.append_with_object(
        ApplicationEvent::SkillVersionActivated {
            skill: version_three.reference(),
            previous_version_id: builtin.skill_version_id(),
            display_name: version_three.content().display_name.clone(),
            provenance: version_three.provenance().clone(),
        },
        Some(skill_object(&version_three)),
    );
    let mut state = ProjectionState::default();

    assert_eq!(
        reduce(&mut state, &envelope).unwrap_err(),
        ai_stock_forum::persistence::RecoveryError::InvalidEventRecord,
    );
    assert_eq!(state, ProjectionState::default());
}

fn assignment_upgrade_fixture() -> (Fixture, AgentProfileVersion, SkillVersion) {
    let mut fixture = Fixture::new();
    let builtin = builtin_manifests().unwrap()[0].skill().clone();
    let builtin_v2 = SkillVersion::next_version(
        &builtin,
        SkillVersionId::from_uuid(Uuid::from_u128(93_002)),
        1_726_000_000_101,
        SkillDraft::new(
            builtin.content().display_name.clone(),
            builtin.content().description.clone(),
            builtin.content().use_when.clone(),
            builtin.content().tags.clone(),
            "Use canonical evidence and record contradictions.".to_owned(),
            builtin.content().resources.clone(),
        )
        .unwrap(),
    )
    .unwrap();
    let initial = AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(93_101)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(93_111)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(93_121)),
        1_726_000_000_102,
        AgentProfileDraft::new(
            "Assignment Recovery Analyst".to_owned(),
            "Exercises assignment recovery.".to_owned(),
            AgentRole::Custom,
            "recovery".to_owned(),
            vec!["assignment".to_owned()],
            "Exact.".to_owned(),
            "Keep pinned versions exact.".to_owned(),
            AgentBindings::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let assigned = AgentProfileVersion::next_version(
        &initial,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(93_112)),
        1_726_000_000_103,
        initial.assign_skill(builtin.reference()).unwrap(),
    )
    .unwrap();
    let upgraded = AgentProfileVersion::next_version(
        &assigned,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(93_113)),
        1_726_000_000_104,
        assigned
            .upgrade_skill(builtin.reference(), builtin_v2.reference())
            .unwrap(),
    )
    .unwrap();
    let transaction = fixture.database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &builtin).unwrap();
    insert_skill_version(&transaction, &builtin_v2).unwrap();
    transaction.commit().unwrap();
    fixture.append(ApplicationEvent::AgentProfileCreated {
        profile: initial.clone(),
    });
    fixture.append(ApplicationEvent::AgentSkillAssigned {
        profile: assigned.clone(),
        previous_profile_version_id: initial.profile_version_id(),
        skill: builtin.reference(),
    });
    fixture.append_with_object(
        ApplicationEvent::SkillVersionActivated {
            skill: builtin_v2.reference(),
            previous_version_id: builtin.skill_version_id(),
            display_name: builtin_v2.content().display_name.clone(),
            provenance: builtin_v2.provenance().clone(),
        },
        Some(skill_object(&builtin_v2)),
    );
    fixture.append(ApplicationEvent::AgentSkillUpgraded {
        profile: upgraded.clone(),
        previous_profile_version_id: assigned.profile_version_id(),
        expected: builtin.reference(),
        replacement: builtin_v2.reference(),
    });
    (fixture, upgraded, builtin_v2)
}

#[test]
fn assignment_and_upgrade_events_rebuild_the_active_exact_profile_version() {
    let (mut fixture, upgraded, builtin_v2) = assignment_upgrade_fixture();

    let state = RecoveryCoordinator::bootstrap(
        &mut fixture.database,
        &fixture.clock,
        &fixture.ids,
        &[],
    )
    .unwrap();

    let active = state
        .projection()
        .agent_profiles
        .active_profile(upgraded.profile_id())
        .unwrap();
    assert_eq!(active, &upgraded);
    assert_eq!(active.skill_refs(), &[builtin_v2.reference()]);
    assert!(active_rows(&fixture.database).iter().any(|row| {
        row.0 == builtin_v2.skill_id().to_string()
            && row.1 == builtin_v2.skill_version_id().to_string()
            && row.2 == 2
    }));
}

#[test]
fn identical_durable_inputs_produce_identical_whole_startup_state() {
    let (mut left, _, _) = assignment_upgrade_fixture();
    let (mut right, _, _) = assignment_upgrade_fixture();

    let left_state =
        RecoveryCoordinator::bootstrap(&mut left.database, &left.clock, &left.ids, &[]).unwrap();
    let right_state =
        RecoveryCoordinator::bootstrap(&mut right.database, &right.clock, &right.ids, &[]).unwrap();

    assert_eq!(left_state.projection(), right_state.projection());
    assert_eq!(skill_rows(&left.database), skill_rows(&right.database));
    assert_eq!(active_rows(&left.database), active_rows(&right.database));
    assert_eq!(
        EventRepository::load_all(left.database.connection()).unwrap(),
        EventRepository::load_all(right.database.connection()).unwrap(),
    );
    let metadata = |database: &Database| {
        database
            .connection()
            .query_row(
                "SELECT last_event_sequence, last_event_digest, projection_digest
                 FROM projection_metadata WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .unwrap()
    };
    assert_eq!(metadata(&left.database), metadata(&right.database));
}
