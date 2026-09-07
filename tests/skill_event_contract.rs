use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        ApplicationEvent, EVENT_SCHEMA_VERSION, EventEnvelope, PendingEvent, SkillEventSummary,
        SkillHistoryEventEntry,
    },
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CorrelationId, EventId, MemoryNamespaceId,
        SkillId, SkillVersionId,
    },
    persistence::{Database, EventRepository},
    skills::{SkillDraft, SkillProvenance, SkillVersion},
};
use uuid::Uuid;

fn skill() -> SkillVersion {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(1)),
        SkillVersionId::from_uuid(Uuid::from_u128(2)),
        1_726_000_000_000,
        SkillProvenance::User,
        SkillDraft::new(
            "Evidence Review".to_owned(),
            "Evidence-led research.".to_owned(),
            "Use for thesis review.".to_owned(),
            vec!["research".to_owned()],
            "SECRET-INSTRUCTION".to_owned(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn profile() -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(10)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(11)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(12)),
        1_726_000_000_001,
        AgentProfileDraft::new(
            "Research Analyst".to_owned(),
            "Evidence-led research.".to_owned(),
            AgentRole::Custom,
            "equity research".to_owned(),
            Vec::new(),
            "Calm and skeptical.".to_owned(),
            "Cite primary evidence.".to_owned(),
            AgentBindings::default(),
            vec![skill().reference()],
            Vec::new(),
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

#[test]
fn skill_events_round_trip_on_the_existing_schema_and_read_payloads_are_metadata_only() {
    let temp = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let skill = skill();
    let events = [
        ApplicationEvent::SkillsListed {
            skills: vec![SkillEventSummary {
                skill: skill.reference(),
                display_name: skill.content().display_name.clone(),
                provenance: skill.provenance().clone(),
            }],
            total_count: 1,
            returned_count: 1,
            truncated: false,
        },
        ApplicationEvent::SkillViewed {
            skill: skill.reference(),
            display_name: skill.content().display_name.clone(),
            provenance: skill.provenance().clone(),
        },
        ApplicationEvent::SkillHistoryViewed {
            skill_id: skill.skill_id(),
            active: skill.reference(),
            versions: vec![SkillHistoryEventEntry {
                skill: skill.reference(),
                created_at_ms: skill.created_at_ms(),
                predecessor_version_id: None,
            }],
            total_count: 1,
            returned_count: 1,
            truncated: false,
        },
        ApplicationEvent::SkillVersionViewed {
            skill: skill.reference(),
            display_name: skill.content().display_name.clone(),
            provenance: skill.provenance().clone(),
            predecessor_version_id: None,
        },
    ];

    for (index, event) in events.into_iter().enumerate() {
        let transaction = database.immediate_transaction().unwrap();
        let committed = EventRepository::append(
            &transaction,
            PendingEvent {
                event_id: EventId::from_uuid(Uuid::from_u128(100 + index as u128)),
                event_schema_version: EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: 1_726_000_000_000 + index as i64,
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(200 + index as u128)),
                causation_id: None,
                object: None,
                event,
            },
        )
        .unwrap();
        transaction.commit().unwrap();
        let wire = serde_json::to_string(&committed).unwrap();
        assert!(!wire.contains("SECRET-INSTRUCTION"));
        assert_eq!(
            serde_json::from_str::<EventEnvelope>(&wire).unwrap(),
            committed
        );
        assert_eq!(committed.event_schema_version, 1);
    }
    assert_eq!(EVENT_SCHEMA_VERSION, 1);
}

#[test]
fn all_new_event_kinds_are_stable() {
    let skill = skill();
    let profile = profile();
    let reference = skill.reference();
    let events = [
        ApplicationEvent::SkillCreated {
            skill: skill.reference(),
            display_name: skill.content().display_name.clone(),
            provenance: skill.provenance().clone(),
        },
        ApplicationEvent::SkillVersionActivated {
            skill: skill.reference(),
            previous_version_id: skill.skill_version_id(),
            display_name: skill.content().display_name.clone(),
            provenance: skill.provenance().clone(),
        },
        ApplicationEvent::AgentSkillAssigned {
            profile: profile.clone(),
            previous_profile_version_id: profile.profile_version_id(),
            skill: reference.clone(),
        },
        ApplicationEvent::AgentSkillUpgraded {
            profile: profile.clone(),
            previous_profile_version_id: profile.profile_version_id(),
            expected: reference.clone(),
            replacement: reference.clone(),
        },
        ApplicationEvent::AgentSkillUnassigned {
            profile: profile.clone(),
            previous_profile_version_id: profile.profile_version_id(),
            expected: reference,
        },
    ];
    assert_eq!(
        events.map(|event| event.kind()),
        [
            "skill_created",
            "skill_version_activated",
            "agent_skill_assigned",
            "agent_skill_upgraded",
            "agent_skill_unassigned",
        ]
    );
}
