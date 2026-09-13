use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{ApplicationEvent, EVENT_SCHEMA_VERSION, EventEnvelope, PendingEvent},
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CausationId, CorrelationId, EventId,
        MemoryNamespaceId, ObjectRef, ObjectVersion, Sha256Digest, canonical_json_bytes, sha256,
    },
    persistence::{Database, EventRepository, RecoveryError},
    recovery::{ProjectionState, reduce},
};
use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

const LEGACY_HUMAN_HELP_DIGEST: &str =
    "af22836f89502372423c3c3224cc08ad7ea09deeba25c518c6976aeb3ff0ab62";
const LEGACY_SYSTEM_STATUS_DIGEST: &str =
    "0c92c7079c931bd17c0b1a436477cad6708df0accff08bf945fcdc8f8c3bb6d1";

fn profile_id(value: u128) -> AgentProfileId {
    AgentProfileId::from_uuid(Uuid::from_u128(value))
}

fn profile_version_id(value: u128) -> AgentProfileVersionId {
    AgentProfileVersionId::from_uuid(Uuid::from_u128(value))
}

fn profile() -> AgentProfileVersion {
    AgentProfileVersion::create(
        profile_id(10),
        profile_version_id(11),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(12)),
        1_700_000_000_000,
        AgentProfileDraft::new(
            "Research Analyst".to_owned(),
            "Immutable analyst profile.".to_owned(),
            AgentRole::Custom,
            "equity research".to_owned(),
            vec!["valuation".to_owned()],
            "Deliberate and concise.".to_owned(),
            "Assess evidence before answering.".to_owned(),
            AgentBindings::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn pending_with_actor(actor: Actor) -> PendingEvent {
    PendingEvent {
        event_id: EventId::from_uuid(Uuid::from_u128(1)),
        event_schema_version: EVENT_SCHEMA_VERSION,
        actor,
        occurred_at_ms: 1_700_000_000_000,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(2)),
        causation_id: None,
        object: None,
        event: ApplicationEvent::HelpViewed,
    }
}

fn seal(pending: PendingEvent) -> EventEnvelope {
    let temporary_directory = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    let transaction = database.immediate_transaction().unwrap();
    let envelope = EventRepository::append(&transaction, pending).unwrap();
    transaction.commit().unwrap();
    envelope
}

#[derive(Serialize)]
struct EventDigestFixture<'a> {
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

fn canonical_agent_event_digest(
    event_id: &EventId,
    correlation_id: &CorrelationId,
    canonical_actor_id: &str,
) -> Sha256Digest {
    sha256(
        &canonical_json_bytes(&EventDigestFixture {
            digest_format_version: 1,
            sequence: 1,
            event_id,
            event_schema_version: EVENT_SCHEMA_VERSION,
            event_type: "help_viewed",
            actor_kind: "agent",
            actor_id: Some(canonical_actor_id),
            occurred_at_ms: 1_700_000_000_000,
            correlation_id,
            causation_id: None,
            object: None,
            previous_event_digest: None,
            payload_json: "{}".to_owned(),
        })
        .unwrap(),
    )
}

#[test]
fn agent_actor_id_is_part_of_the_event_digest() {
    let first = pending_with_actor(Actor::Agent(profile_id(1)));
    let second = pending_with_actor(Actor::Agent(profile_id(2)));

    assert_ne!(seal(first).event_digest, seal(second).event_digest);
}

#[test]
fn actor_database_shapes_round_trip_and_reject_invalid_pairs() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
    let actors = [Actor::Human, Actor::System, Actor::Agent(profile_id(23))];
    let expected_actors = actors.clone();

    for (index, actor) in actors.into_iter().enumerate() {
        let transaction = database.immediate_transaction().unwrap();
        let envelope = EventRepository::append(
            &transaction,
            PendingEvent {
                event_id: EventId::from_uuid(Uuid::from_u128(100 + index as u128)),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor,
                occurred_at_ms: 1_700_000_000_000 + index as i64,
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(200 + index as u128)),
                causation_id: None,
                object: None,
                event: ApplicationEvent::HelpViewed,
            },
        )
        .unwrap();
        transaction.commit().unwrap();

        let (kind, id): (String, Option<String>) = database
            .connection()
            .query_row(
                "SELECT actor_kind, actor_id FROM event_stream WHERE sequence = ?1",
                [i64::try_from(envelope.sequence).unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let expected = match envelope.actor {
            Actor::Human => ("human", None),
            Actor::System => ("system", None),
            Actor::Agent(id) => ("agent", Some(id.to_string())),
        };
        assert_eq!((kind.as_str(), id), expected);
    }

    assert_eq!(
        EventRepository::load_all(database.connection())
            .unwrap()
            .into_iter()
            .map(|event| event.actor)
            .collect::<Vec<_>>(),
        expected_actors
    );

    for (kind, id) in [
        ("human", Some(profile_id(30).to_string())),
        ("system", Some(profile_id(31).to_string())),
        ("agent", None),
        ("agent", Some("not-a-uuid".to_owned())),
        ("unknown", None),
    ] {
        let temporary_directory = tempfile::tempdir().unwrap();
        let database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, actor_id, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (1, ?1, 1, 'help_viewed', ?2, ?3, 1, ?4, '{}', ?5)",
                params![
                    EventId::from_uuid(Uuid::from_u128(99)).to_string(),
                    kind,
                    id,
                    CorrelationId::from_uuid(Uuid::from_u128(98)).to_string(),
                    "0".repeat(64),
                ],
            )
            .unwrap();
        assert_eq!(
            EventRepository::load_all(database.connection()).unwrap_err(),
            RecoveryError::InvalidEventRecord
        );
    }
}

#[test]
fn agent_actor_serde_preserves_legacy_human_and_system_bytes() {
    let actor = Actor::Agent(profile_id(42));
    assert_eq!(
        serde_json::from_str::<Actor>(&serde_json::to_string(&actor).unwrap()).unwrap(),
        actor
    );
    assert_eq!(serde_json::to_string(&Actor::Human).unwrap(), "\"Human\"");
    assert_eq!(serde_json::to_string(&Actor::System).unwrap(), "\"System\"");
    assert!(serde_json::from_str::<Actor>("\"Agent\"").is_err());
}

#[test]
fn exact_profile_version_references_resolve_only_authoritative_fields() {
    let profile = profile();
    let envelope = EventEnvelope {
        sequence: 1,
        event_id: EventId::from_uuid(Uuid::from_u128(20)),
        event_schema_version: EVENT_SCHEMA_VERSION,
        actor: Actor::Human,
        occurred_at_ms: 1_700_000_000_000,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(21)),
        causation_id: None,
        object: None,
        event: ApplicationEvent::AgentProfileCreated {
            profile: profile.clone(),
        },
        previous_event_digest: None,
        event_digest: sha256(b"profile-created"),
    };
    let mut state = ProjectionState::default();
    reduce(&mut state, &envelope).unwrap();

    let reference = profile.reference();
    let serialized = serde_json::to_string(&reference).unwrap();
    assert_eq!(
        serde_json::from_str::<ai_stock_forum::agents::AgentProfileVersionRef>(&serialized)
            .unwrap(),
        reference
    );
    assert_eq!(
        state
            .agent_profiles
            .resolve_reference(&reference)
            .unwrap()
            .memory_namespace_id(),
        profile.memory_namespace_id()
    );

    let wrong_version = ai_stock_forum::agents::AgentProfileVersionRef::new(
        profile.profile_id(),
        profile.profile_version_id(),
        ObjectVersion::new(profile.version().get() + 1).unwrap(),
        profile.content_digest().clone(),
    )
    .unwrap();
    let wrong_digest = ai_stock_forum::agents::AgentProfileVersionRef::new(
        profile.profile_id(),
        profile.profile_version_id(),
        profile.version(),
        sha256(b"wrong digest"),
    )
    .unwrap();
    let wrong_profile = ai_stock_forum::agents::AgentProfileVersionRef::new(
        profile_id(99),
        profile.profile_version_id(),
        profile.version(),
        profile.content_digest().clone(),
    )
    .unwrap();

    assert!(
        state
            .agent_profiles
            .resolve_reference(&wrong_version)
            .is_err()
    );
    assert!(
        state
            .agent_profiles
            .resolve_reference(&wrong_digest)
            .is_err()
    );
    assert!(
        state
            .agent_profiles
            .resolve_reference(&wrong_profile)
            .is_err()
    );
}

#[test]
fn malformed_projection_key_cannot_substitute_an_embedded_profile_version_id() {
    let profile = profile();
    let envelope = EventEnvelope {
        sequence: 1,
        event_id: EventId::from_uuid(Uuid::from_u128(40)),
        event_schema_version: EVENT_SCHEMA_VERSION,
        actor: Actor::Human,
        occurred_at_ms: 1_700_000_000_000,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(41)),
        causation_id: None,
        object: None,
        event: ApplicationEvent::AgentProfileCreated {
            profile: profile.clone(),
        },
        previous_event_digest: None,
        event_digest: sha256(b"profile-created"),
    };
    let mut state = ProjectionState::default();
    reduce(&mut state, &envelope).unwrap();

    let mut serialized = serde_json::to_value(&state.agent_profiles).unwrap();
    let versions = serialized
        .get_mut("versions_by_id")
        .unwrap()
        .as_object_mut()
        .unwrap();
    let embedded_id = profile.profile_version_id().to_string();
    let mismatched_key = profile_version_id(99).to_string();
    let embedded_profile = versions.remove(&embedded_id).unwrap();
    versions.insert(mismatched_key, embedded_profile);
    let malformed: ai_stock_forum::agents::AgentProfilesProjection =
        serde_json::from_value(serialized).unwrap();
    let forged = ai_stock_forum::agents::AgentProfileVersionRef::new(
        profile.profile_id(),
        profile_version_id(99),
        profile.version(),
        profile.content_digest().clone(),
    )
    .unwrap();

    assert!(malformed.resolve_reference(&forged).is_err());
}

#[test]
fn alternate_agent_uuid_spellings_are_rejected_before_digest_verification() {
    let actor_id = profile_id(0xab);
    let canonical_actor_id = actor_id.to_string();
    let event_id = EventId::from_uuid(Uuid::from_u128(50));
    let correlation_id = CorrelationId::from_uuid(Uuid::from_u128(51));
    let digest = canonical_agent_event_digest(&event_id, &correlation_id, &canonical_actor_id);

    for alternate in [
        "000000000000000000000000000000ab",
        "00000000-0000-0000-0000-0000000000AB",
        "{00000000-0000-0000-0000-0000000000ab}",
        "urn:uuid:00000000-0000-0000-0000-0000000000ab",
    ] {
        let temporary_directory = tempfile::tempdir().unwrap();
        let database = Database::open(&AppPaths::for_test(temporary_directory.path())).unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, actor_id, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (1, ?1, 1, 'help_viewed', 'agent', ?2, 1700000000000, ?3, '{}', ?4)",
                params![
                    event_id.to_string(),
                    alternate,
                    correlation_id.to_string(),
                    digest.as_str(),
                ],
            )
            .unwrap();

        assert_eq!(
            EventRepository::load_all(database.connection()).unwrap_err(),
            RecoveryError::InvalidEventRecord
        );
    }
}

#[test]
fn legacy_human_and_system_event_goldens_do_not_change() {
    assert_eq!(serde_json::to_string(&Actor::Human).unwrap(), "\"Human\"");
    assert_eq!(serde_json::to_string(&Actor::System).unwrap(), "\"System\"");

    let human = seal(pending_with_actor(Actor::Human));
    let system = seal(PendingEvent {
        event_id: EventId::from_uuid(Uuid::from_u128(3)),
        event_schema_version: EVENT_SCHEMA_VERSION,
        actor: Actor::System,
        occurred_at_ms: 1_700_000_000_001,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(4)),
        causation_id: None,
        object: None,
        event: ApplicationEvent::StatusViewed,
    });

    assert_eq!(human.event_digest.as_str(), LEGACY_HUMAN_HELP_DIGEST);
    assert_eq!(system.event_digest.as_str(), LEGACY_SYSTEM_STATUS_DIGEST);
}
