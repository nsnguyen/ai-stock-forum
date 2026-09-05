use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, EventEnvelope, PendingEvent},
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CorrelationId, EventId, MemoryNamespaceId,
        sha256,
    },
    persistence::{Database, EventRepository, ProjectionRepository, RecoveryError},
    recovery::{ProjectionState, reduce},
};
use serde_json::json;
use uuid::Uuid;

fn profile_id(value: u128) -> AgentProfileId {
    AgentProfileId::from_uuid(Uuid::from_u128(value))
}

fn version_id(value: u128) -> AgentProfileVersionId {
    AgentProfileVersionId::from_uuid(Uuid::from_u128(value))
}

fn memory_namespace_id(value: u128) -> MemoryNamespaceId {
    MemoryNamespaceId::from_uuid(Uuid::from_u128(value))
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

fn profile(profile: u128, version: u128, name: &str) -> AgentProfileVersion {
    AgentProfileVersion::create(
        profile_id(profile),
        version_id(version),
        memory_namespace_id(profile + 100),
        1_726_000_000_000,
        draft(name, "Evidence-led equity research."),
        None,
    )
    .unwrap()
}

fn append(database: &mut Database, event_id: u128, event: ApplicationEvent) -> EventEnvelope {
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

fn database() -> Database {
    let temporary_directory = tempfile::tempdir().unwrap();
    // Keep the directory alive for the database lifetime in each test by leaking this tiny fixture.
    let path = temporary_directory.keep();
    Database::open(&AppPaths::for_test(&path)).unwrap()
}

fn reduce_all(events: &[EventEnvelope]) -> ProjectionState {
    let mut state = ProjectionState::default();
    for event in events {
        reduce(&mut state, event).unwrap();
    }
    state
}

#[test]
fn profile_events_round_trip_canonically_with_schema_v1_and_legacy_events_stay_compatible() {
    let mut database = database();
    let profile_v1 = profile(1, 2, "Research Analyst");
    let profile_v2 = AgentProfileVersion::next_version(
        &profile_v1,
        version_id(3),
        1_726_000_000_001,
        draft("Research Analyst", "Independent equity research."),
    )
    .unwrap();
    let legacy = append(&mut database, 10, ApplicationEvent::HelpViewed);
    let created = append(
        &mut database,
        11,
        ApplicationEvent::AgentProfileCreated {
            profile: profile_v1.clone(),
        },
    );
    let activated = append(
        &mut database,
        12,
        ApplicationEvent::AgentProfileVersionActivated {
            profile: profile_v2.clone(),
            previous_version_id: profile_v1.profile_version_id(),
        },
    );

    for event in [&legacy, &created, &activated] {
        let wire = serde_json::to_string(event).unwrap();
        let decoded: EventEnvelope = serde_json::from_str(&wire).unwrap();
        assert_eq!(decoded, *event);
        assert_eq!(decoded.event_schema_version, 1);
    }
    assert_eq!(EVENT_SCHEMA_VERSION, 1);
    assert_eq!(created.event.kind(), "agent_profile_created");
    assert_eq!(activated.event.kind(), "agent_profile_version_activated");
}

#[test]
fn unknown_profile_event_kind_fails_safely() {
    let mut database = database();
    let known = append(&mut database, 10, ApplicationEvent::HelpViewed);
    let mut wire = serde_json::to_value(known).unwrap();
    wire["event_type"] = json!("agent_profile_deleted");

    assert!(serde_json::from_value::<EventEnvelope>(wire).is_err());
}

#[test]
fn profile_wire_rejects_a_normalized_name_that_does_not_match_the_visible_name() {
    let profile = profile(1, 2, "Research Analyst");
    let mut profile_json = serde_json::to_value(profile).unwrap();
    profile_json["normalized_name"] = json!("forged unique name");

    assert!(serde_json::from_value::<AgentProfileVersion>(profile_json).is_err());
}

#[test]
fn profile_wire_rejects_an_altered_content_digest_before_reduction() {
    let profile = profile(1, 2, "Research Analyst");
    let mut profile_json = serde_json::to_value(profile).unwrap();
    profile_json["content_digest"] = json!(sha256(b"altered-profile"));

    assert!(serde_json::from_value::<AgentProfileVersion>(profile_json).is_err());
}

#[test]
fn reducer_rejects_duplicate_active_normalized_names_without_mutation() {
    let mut database = database();
    let first = append(
        &mut database,
        10,
        ApplicationEvent::AgentProfileCreated {
            profile: profile(1, 2, "Research Analyst"),
        },
    );
    let duplicate = append(
        &mut database,
        11,
        ApplicationEvent::AgentProfileCreated {
            profile: profile(3, 4, "research analyst"),
        },
    );
    let mut state = reduce_all(&[first]);
    let before = state.clone();

    assert_eq!(
        reduce(&mut state, &duplicate),
        Err(RecoveryError::InvalidEventRecord)
    );
    assert_eq!(state, before);
}

#[test]
fn reducer_rejects_version_two_without_its_active_version_one() {
    let mut database = database();
    let profile_v1 = profile(1, 2, "Research Analyst");
    let profile_v2 = AgentProfileVersion::next_version(
        &profile_v1,
        version_id(3),
        1_726_000_000_001,
        draft("Research Analyst", "Independent equity research."),
    )
    .unwrap();
    let activation = append(
        &mut database,
        10,
        ApplicationEvent::AgentProfileVersionActivated {
            profile: profile_v2,
            previous_version_id: profile_v1.profile_version_id(),
        },
    );

    assert_eq!(
        reduce(&mut ProjectionState::default(), &activation),
        Err(RecoveryError::InvalidEventRecord)
    );
}

#[test]
fn create_then_edit_has_one_active_version_two_historical_versions_and_recovery_equivalence() {
    let mut database = database();
    let profile_v1 = profile(1, 2, "Research Analyst");
    let profile_v2 = AgentProfileVersion::next_version(
        &profile_v1,
        version_id(3),
        1_726_000_000_001,
        draft("Research Analyst", "Independent equity research."),
    )
    .unwrap();
    let created = append(
        &mut database,
        10,
        ApplicationEvent::AgentProfileCreated {
            profile: profile_v1.clone(),
        },
    );
    let activated = append(
        &mut database,
        11,
        ApplicationEvent::AgentProfileVersionActivated {
            profile: profile_v2.clone(),
            previous_version_id: profile_v1.profile_version_id(),
        },
    );

    let events = vec![created, activated];
    let direct = reduce_all(&events);
    let recovered = ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();
    let profiles = &direct.agent_profiles;

    assert_eq!(profiles.active_profiles(), vec![profile_v2.clone()]);
    assert_eq!(
        profiles.active_profile(profile_v1.profile_id()),
        Some(&profile_v2)
    );
    assert_eq!(
        profiles.history(profile_v1.profile_id()),
        vec![profile_v1.clone(), profile_v2.clone()]
    );
    assert_eq!(
        profiles.version(profile_v1.profile_version_id()),
        Some(&profile_v1)
    );
    assert_eq!(
        profiles.version(profile_v2.profile_version_id()),
        Some(&profile_v2)
    );
    assert_eq!(
        serde_json::to_vec(&direct.agent_profiles).unwrap(),
        serde_json::to_vec(&recovered.agent_profiles).unwrap()
    );
}
