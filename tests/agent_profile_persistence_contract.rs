use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{AppError, ApplicationEvent, EVENT_SCHEMA_VERSION, PendingEvent},
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CorrelationId, EventId, MemoryNamespaceId,
    },
    persistence::{
        Database, EventRepository, PersistenceError, insert_expected_version, load_all_versions,
        replace_active_profiles,
    },
    recovery::{ProjectionState, reduce},
};
use uuid::Uuid;

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

fn database() -> Database {
    let temporary_directory = tempfile::tempdir().unwrap();
    let path = temporary_directory.keep();
    Database::open(&AppPaths::for_test(&path)).unwrap()
}

fn append_profile_event(
    database: &mut Database,
    event_id: u128,
    event: ApplicationEvent,
) -> ai_stock_forum::app::EventEnvelope {
    let transaction = database.immediate_transaction().unwrap();
    let envelope = EventRepository::append(
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
    envelope
}

#[test]
fn append_and_exact_duplicate_are_idempotent_and_history_is_event_ordered() {
    let mut database = database();
    let first = profile_v1();
    let second = profile_v2(&first);
    let transaction = database.connection_mut().transaction().unwrap();

    insert_expected_version(&transaction, 20, &second).unwrap();
    insert_expected_version(&transaction, 10, &first).unwrap();
    insert_expected_version(&transaction, 10, &first).unwrap();
    transaction.commit().unwrap();

    let stored = load_all_versions(database.connection()).unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].event_sequence, 10);
    assert_eq!(stored[0].profile, first);
    assert_eq!(stored[1].event_sequence, 20);
    assert_eq!(stored[1].profile, second);
}

#[test]
fn duplicate_logical_version_with_different_stored_bytes_is_rejected_safely() {
    let mut database = database();
    let profile = profile_v1();
    let transaction = database.connection_mut().transaction().unwrap();
    insert_expected_version(&transaction, 1, &profile).unwrap();
    transaction.commit().unwrap();
    database
        .connection()
        .execute_batch(
            "DROP TRIGGER agent_profile_versions_no_update;
             UPDATE agent_profile_versions
             SET payload_json = CAST(payload_json || X'20' AS BLOB)
             WHERE source_event_sequence = 1;",
        )
        .unwrap();

    let transaction = database.connection_mut().transaction().unwrap();
    let error = insert_expected_version(&transaction, 1, &profile).unwrap_err();
    transaction.rollback().unwrap();

    assert_eq!(error, PersistenceError::AgentProfileHistoryMismatch);
    assert_eq!(error.code(), "database_agent_profile_history_mismatch");
    assert_eq!(AppError::from(error).code(), error.code());
    assert!(!error.to_string().contains("Sensitive profile prose"));
}

#[test]
fn malformed_profile_payload_has_a_stable_redacted_repository_error() {
    let mut database = database();
    let profile = profile_v1();
    database
        .connection_mut()
        .execute(
            "INSERT INTO agent_profile_versions (
                profile_id, profile_version_id, version, supersedes_version_id,
                template_id, template_version, template_digest, role, display_name,
                normalized_name, memory_namespace_id, policy_profile_ref, content_digest,
                payload_json, source_event_sequence, created_at_ms
             ) VALUES (
                ?1, ?2, 1, NULL, NULL, NULL, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10
             )",
            rusqlite::params![
                profile.profile_id().to_string(),
                profile.profile_version_id().to_string(),
                profile.role().as_str(),
                profile.display_name(),
                profile.normalized_name().as_str(),
                profile.memory_namespace_id().to_string(),
                profile.default_policy_ref(),
                profile.content_digest().as_str(),
                br#"{"secret":"profile payload"}"#.as_slice(),
                profile.created_at_ms(),
            ],
        )
        .unwrap();

    let error = load_all_versions(database.connection()).unwrap_err();

    assert_eq!(error, PersistenceError::InvalidAgentProfilePayload);
    assert_eq!(error.code(), "invalid_agent_profile_payload");
    assert!(!error.to_string().contains("secret profile payload"));
}

#[test]
fn active_pointer_replacement_tracks_the_latest_projection_version() {
    let mut database = database();
    let first = profile_v1();
    let second = profile_v2(&first);
    let created = append_profile_event(
        &mut database,
        201,
        ApplicationEvent::AgentProfileCreated {
            profile: first.clone(),
        },
    );
    let mut state = ProjectionState::default();
    reduce(&mut state, &created).unwrap();
    let transaction = database.connection_mut().transaction().unwrap();
    insert_expected_version(&transaction, 1, &first).unwrap();
    replace_active_profiles(&transaction, &state.agent_profiles).unwrap();
    transaction.commit().unwrap();

    let activated = append_profile_event(
        &mut database,
        202,
        ApplicationEvent::AgentProfileVersionActivated {
            profile: second.clone(),
            previous_version_id: first.profile_version_id(),
        },
    );
    reduce(&mut state, &activated).unwrap();
    let transaction = database.connection_mut().transaction().unwrap();
    insert_expected_version(&transaction, 2, &second).unwrap();
    replace_active_profiles(&transaction, &state.agent_profiles).unwrap();
    transaction.commit().unwrap();

    let active: (String, i64, String) = database
        .connection()
        .query_row(
            "SELECT profile_version_id, version, content_digest FROM active_agent_profiles",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        active,
        (
            second.profile_version_id().to_string(),
            2,
            second.content_digest().to_string()
        )
    );
    assert_eq!(
        database
            .connection()
            .query_row("SELECT COUNT(*) FROM active_agent_profiles", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
}

#[test]
fn immutable_and_active_writes_roll_back_together() {
    let mut database = database();
    let profile = profile_v1();
    let event = append_profile_event(
        &mut database,
        301,
        ApplicationEvent::AgentProfileCreated {
            profile: profile.clone(),
        },
    );
    let mut state = ProjectionState::default();
    reduce(&mut state, &event).unwrap();
    let transaction = database.connection_mut().transaction().unwrap();
    insert_expected_version(&transaction, 1, &profile).unwrap();
    replace_active_profiles(&transaction, &state.agent_profiles).unwrap();
    transaction.rollback().unwrap();

    let counts = (
        database
            .connection()
            .query_row("SELECT COUNT(*) FROM agent_profile_versions", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        database
            .connection()
            .query_row("SELECT COUNT(*) FROM active_agent_profiles", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
    );
    assert_eq!(counts, (0, 0));
}
