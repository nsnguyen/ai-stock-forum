use std::sync::{
    Arc,
    atomic::{AtomicI64, AtomicU64, Ordering},
};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        ApplicationCommand, ApplicationEvent, ApplicationService, CommandEnvelope, CommandView,
        EVENT_SCHEMA_VERSION, PendingEvent,
    },
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, Clock, CommandId, CorrelationId, EventId,
        IdGenerator, MemoryNamespaceId, ObjectRef, SkillId, SkillVersionId, canonical_json_bytes,
        sha256,
    },
    persistence::{
        Database, EventRepository, insert_skill_version, load_skill_version_by_id,
    },
    recovery::RecoveryCoordinator,
    skills::{SkillDraft, SkillProvenance, SkillVersion, SkillVersionRef, builtin_manifests},
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
    next_event: u128,
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
            next_event: 71_000,
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

    fn append(&mut self, event: ApplicationEvent, object: Option<ObjectRef>) {
        let id = self.next_event;
        self.next_event += 2;
        let transaction = self.database.immediate_transaction().unwrap();
        EventRepository::append(
            &transaction,
            PendingEvent {
                event_id: EventId::from_uuid(Uuid::from_u128(id)),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 1_726_300_000_000 + i64::try_from(id).unwrap(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 1)),
                causation_id: None,
                object,
                event,
            },
        )
        .unwrap();
        transaction.commit().unwrap();
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

fn skill_object(skill: &SkillVersion) -> ObjectRef {
    ObjectRef::new(
        "skill_version",
        skill.skill_version_id().to_string(),
        skill.version(),
        sha256(&canonical_json_bytes(skill).unwrap()),
    )
    .unwrap()
}

fn recovery_counts(database: &Database) -> (i64, i64, i64, i64, i64, i64, i64) {
    let count = |table: &str| {
        database
            .connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap()
    };
    (
        count("event_stream"),
        count("skill_versions"),
        count("active_skills"),
        count("agent_profile_versions"),
        count("active_agent_profiles"),
        count("projection_metadata"),
        count("process_session_projection"),
    )
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

#[test]
fn coherently_rehashed_noncontent_tampering_fails_event_anchored_authentication() {
    let mut fixture = Fixture::new();
    let original = unexpected_skill();
    let transaction = fixture.database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &original).unwrap();
    transaction.commit().unwrap();
    fixture.append(
        ApplicationEvent::SkillCreated {
            skill: original.reference(),
            display_name: original.content().display_name.clone(),
            provenance: original.provenance().clone(),
        },
        Some(skill_object(&original)),
    );
    let altered = SkillVersion::create(
        original.skill_id(),
        original.skill_version_id(),
        original.created_at_ms() + 1,
        original.provenance().clone(),
        original.content().clone(),
    )
    .unwrap();
    let altered_record = canonical_json_bytes(&altered).unwrap();
    fixture
        .database
        .connection()
        .execute_batch("DROP TRIGGER skill_versions_no_update")
        .unwrap();
    fixture
        .database
        .connection()
        .execute(
            "UPDATE skill_versions
             SET created_at_ms = ?1, record_json = ?2, record_digest = ?3
             WHERE skill_version_id = ?4",
            rusqlite::params![
                altered.created_at_ms(),
                altered_record,
                sha256(&canonical_json_bytes(&altered).unwrap()).as_str(),
                altered.skill_version_id().to_string(),
            ],
        )
        .unwrap();
    let before = recovery_counts(&fixture.database);

    let error = fixture.bootstrap().unwrap_err();

    assert_eq!(error.code(), "skill_version_integrity_mismatch");
    assert_eq!(recovery_counts(&fixture.database), before);
}

#[test]
fn mismatched_skill_event_digest_fails_before_projection_mutation() {
    let mut fixture = Fixture::new();
    let skill = unexpected_skill();
    let transaction = fixture.database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &skill).unwrap();
    transaction.commit().unwrap();
    fixture.append(
        ApplicationEvent::SkillCreated {
            skill: skill.reference(),
            display_name: skill.content().display_name.clone(),
            provenance: skill.provenance().clone(),
        },
        Some(
            ObjectRef::new(
                "skill_version",
                skill.skill_version_id().to_string(),
                skill.version(),
                sha256(b"wrong-full-record-digest"),
            )
            .unwrap(),
        ),
    );
    let before = recovery_counts(&fixture.database);

    let error = fixture.bootstrap().unwrap_err();

    assert_eq!(error.code(), "skill_version_integrity_mismatch");
    assert_eq!(recovery_counts(&fixture.database), before);
}

#[test]
fn missing_custom_immutable_row_fails_closed_without_builtin_reconstruction() {
    let mut fixture = Fixture::new();
    let skill = unexpected_skill();
    fixture.append(
        ApplicationEvent::SkillCreated {
            skill: skill.reference(),
            display_name: skill.content().display_name.clone(),
            provenance: skill.provenance().clone(),
        },
        Some(skill_object(&skill)),
    );
    let before = recovery_counts(&fixture.database);

    let error = fixture.bootstrap().unwrap_err();

    assert_eq!(error.code(), "skill_version_integrity_mismatch");
    assert_eq!(recovery_counts(&fixture.database), before);
}

#[test]
fn missing_skill_event_object_fails_before_any_recovery_mutation() {
    let mut fixture = Fixture::new();
    let skill = unexpected_skill();
    let transaction = fixture.database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &skill).unwrap();
    transaction.commit().unwrap();
    fixture.append(
        ApplicationEvent::SkillCreated {
            skill: skill.reference(),
            display_name: skill.content().display_name.clone(),
            provenance: skill.provenance().clone(),
        },
        None,
    );
    let before = recovery_counts(&fixture.database);

    let error = fixture.bootstrap().unwrap_err();

    assert_eq!(error.code(), "invalid_event_record");
    assert_eq!(recovery_counts(&fixture.database), before);
}

fn profile_without_skills() -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(72_001)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(72_011)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(72_021)),
        1_726_300_000_000,
        AgentProfileDraft::new(
            "Atomic Recovery Analyst".to_owned(),
            "Tests later-stage rollback.".to_owned(),
            AgentRole::Custom,
            "recovery".to_owned(),
            vec!["atomicity".to_owned()],
            "Exact.".to_owned(),
            "Reject invalid exact references.".to_owned(),
            AgentBindings::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

#[test]
fn invalid_assignment_exact_ref_rolls_back_the_entire_startup_reconciliation() {
    let mut fixture = Fixture::new();
    let initial = profile_without_skills();
    let builtin = builtin_manifests().unwrap()[0].skill().clone();
    let invalid_ref: SkillVersionRef = serde_json::from_value(serde_json::json!({
        "skill_id": builtin.skill_id(),
        "skill_version_id": builtin.skill_version_id(),
        "version": builtin.version(),
        "content_digest": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    }))
    .unwrap();
    let assigned = AgentProfileVersion::next_version(
        &initial,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(72_012)),
        1_726_300_000_001,
        initial.assign_skill(invalid_ref.clone()).unwrap(),
    )
    .unwrap();
    fixture.append(
        ApplicationEvent::AgentProfileCreated {
            profile: initial.clone(),
        },
        None,
    );
    fixture.append(
        ApplicationEvent::AgentSkillAssigned {
            profile: assigned,
            previous_profile_version_id: initial.profile_version_id(),
            skill: invalid_ref,
        },
        None,
    );
    let before = recovery_counts(&fixture.database);

    let error = fixture.bootstrap().unwrap_err();

    assert_eq!(error.code(), "skill_version_reference_mismatch");
    assert_eq!(recovery_counts(&fixture.database), before);
}

fn command_envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 1)),
        actor: Actor::Human,
        command,
    }
}

#[test]
fn skill_create_and_version_receipts_bind_the_same_persisted_version_object() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    let mut service = ApplicationService::bootstrap(
        &paths,
        Arc::new(TestClock(AtomicI64::new(1_726_400_000_000))),
        Arc::new(TestIds(AtomicU64::new(73_000))),
    )
    .unwrap();
    let first_draft = SkillDraft::new(
        "Receipt-bound Skill".to_owned(),
        "Authenticates the accepted immutable record.".to_owned(),
        "Use for receipt recovery checks.".to_owned(),
        vec!["receipt".to_owned()],
        "Bind one accepted version everywhere.".to_owned(),
        Vec::new(),
    )
    .unwrap();
    let preview = service.preview_skill_creation(first_draft.clone()).unwrap();
    let create_envelope = command_envelope(
        73_100,
        ApplicationCommand::CreateSkill {
            skill_id: preview.skill_id,
            candidate: first_draft,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    );
    let created = service.execute(create_envelope.clone()).unwrap();
    let CommandView::SkillCreated(created_view) = &created.view else {
        panic!("skill created view");
    };
    let database = Database::open(&paths).unwrap();
    let accepted =
        load_skill_version_by_id(database.connection(), created_view.skill_version_id)
            .unwrap()
            .unwrap();
    assert_eq!(
        created.committed_events[0].object.as_ref(),
        Some(&skill_object(&accepted)),
    );
    assert_eq!(service.execute(create_envelope).unwrap(), created);

    let second_draft = SkillDraft::new(
        "Receipt-bound Skill".to_owned(),
        "Authenticates every immutable version.".to_owned(),
        "Use for receipt recovery checks.".to_owned(),
        vec!["receipt".to_owned()],
        "Bind the replacement version everywhere.".to_owned(),
        Vec::new(),
    )
    .unwrap();
    let preview = service
        .preview_skill_version(
            created_view.skill_id,
            created_view.skill_version_id,
            second_draft.clone(),
        )
        .unwrap();
    let version_envelope = command_envelope(
        73_200,
        ApplicationCommand::ActivateSkillVersion {
            skill_id: created_view.skill_id,
            expected_active_version_id: created_view.skill_version_id,
            candidate: second_draft,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    );
    let activated = service.execute(version_envelope.clone()).unwrap();
    let CommandView::SkillVersionActivated(activated_view) = &activated.view else {
        panic!("skill version activated view");
    };
    let database = Database::open(&paths).unwrap();
    let accepted =
        load_skill_version_by_id(database.connection(), activated_view.skill_version_id)
            .unwrap()
            .unwrap();
    assert_eq!(
        activated.committed_events[0].object.as_ref(),
        Some(&skill_object(&accepted)),
    );
    assert_eq!(service.execute(version_envelope).unwrap(), activated);
}
