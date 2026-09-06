mod support;

use ai_stock_forum::{
    app::{ApplicationCommand, CommandEnvelope, SkillSelector},
    domain::{Actor, CommandId, CorrelationId},
    policy::Capability,
    skills::{SkillDraft, SkillResource},
};
use uuid::Uuid;

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 1_000)),
        actor: Actor::Human,
        command,
    }
}

fn draft() -> SkillDraft {
    SkillDraft::new(
        "Audit Skill".to_owned(),
        "Safe metadata.".to_owned(),
        "Use for audit tests.".to_owned(),
        Vec::new(),
        "TOP-SECRET-INSTRUCTION-BODY".to_owned(),
        vec![SkillResource {
            name: "Private Notes".to_owned(),
            body: "TOP-SECRET-REFERENCE-BODY".to_owned(),
        }],
    )
    .unwrap()
}

#[test]
fn skill_read_events_and_audit_summaries_never_duplicate_full_content() {
    let mut app = support::app();
    let preview = app.preview_skill_creation(draft()).unwrap();
    app.execute(envelope(
        40_000,
        ApplicationCommand::CreateSkill {
            skill_id: preview.skill_id,
            candidate: draft(),
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    ))
    .unwrap();
    app.execute(envelope(40_001, ApplicationCommand::ListSkills))
        .unwrap();
    app.execute(envelope(
        40_002,
        ApplicationCommand::ShowSkill {
            selector: SkillSelector::from(preview.skill_id),
        },
    ))
    .unwrap();
    app.execute(envelope(
        40_003,
        ApplicationCommand::ShowSkillHistory {
            selector: SkillSelector::from(preview.skill_id),
        },
    ))
    .unwrap();

    for kind in ["skills_listed", "skill_viewed", "skill_history_viewed"] {
        for payload in app.event_payloads(kind) {
            assert!(!payload.contains("TOP-SECRET-INSTRUCTION-BODY"));
            assert!(!payload.contains("TOP-SECRET-REFERENCE-BODY"));
        }
    }
    let audit = app
        .execute_user(ApplicationCommand::audit_tail(100).unwrap())
        .unwrap();
    let rendered = serde_json::to_string(&audit).unwrap();
    assert!(!rendered.contains("TOP-SECRET-INSTRUCTION-BODY"));
    assert!(!rendered.contains("TOP-SECRET-REFERENCE-BODY"));
}

#[test]
fn capabilities_are_exactly_narrow_and_execution_like_values_are_rejected() {
    assert_eq!(serde_json::to_string(&Capability::SkillRead).unwrap(), "\"skill_read\"");
    assert_eq!(serde_json::to_string(&Capability::SkillCreate).unwrap(), "\"skill_create\"");
    assert_eq!(serde_json::to_string(&Capability::SkillVersion).unwrap(), "\"skill_version\"");
    assert_eq!(serde_json::to_string(&Capability::AgentSkillAssign).unwrap(), "\"agent_skill_assign\"");
    assert_eq!(serde_json::to_string(&Capability::AgentSkillUnassign).unwrap(), "\"agent_skill_unassign\"");
    assert!(serde_json::from_str::<Capability>("\"skill_execute\"").is_err());
    assert!(serde_json::from_str::<Capability>("\"execute_skill\"").is_err());
    assert!(serde_json::from_str::<Capability>("\"unknown_skill_capability\"").is_err());
}
