mod support;

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentRole},
    app::{
        AgentSkillAssignmentOperation, AppError, ApplicationCommand, CommandEnvelope, CommandView,
        SkillSelector,
    },
    domain::{Actor, CommandId, CorrelationId},
    skills::{SkillDraft, SkillResource},
};
use uuid::Uuid;

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id + 1_000_000)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 2_000_000)),
        actor: Actor::Human,
        command,
    }
}

fn skill(name: &str, instructions: &str) -> SkillDraft {
    SkillDraft::new(
        name.to_owned(),
        "Evidence-led research guidance.".to_owned(),
        "Use when reviewing an investment thesis.".to_owned(),
        vec!["research".to_owned()],
        instructions.to_owned(),
        vec![SkillResource {
            name: "Checklist".to_owned(),
            body: "Verify every material claim.".to_owned(),
        }],
    )
    .unwrap()
}

fn profile(name: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        "Evidence-led equity research.".to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical.".to_owned(),
        "Cite primary evidence.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn reviewed_create_version_reads_and_exact_assignment_lifecycle_are_typed() {
    let mut app = support::app();
    let create_candidate = skill("Evidence Review", "State evidence and disconfirming facts.");
    let create_preview = app
        .preview_skill_creation(create_candidate.clone())
        .unwrap();
    let created = app
        .execute(envelope(
            10_000,
            ApplicationCommand::CreateSkill {
                skill_id: create_preview.skill_id,
                candidate: create_candidate,
                review_token: create_preview.review_token,
                review_digest: create_preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::SkillCreated(created) = created.view else {
        panic!("skill created view")
    };
    assert_eq!(created.version.get(), 1);

    let listed = app
        .execute(envelope(10_001, ApplicationCommand::ListSkills))
        .unwrap();
    let CommandView::Skills(listed) = listed.view else {
        panic!("skills view")
    };
    assert_eq!(listed.total_count, 1);
    assert_eq!(listed.skills[0].display_name, "Evidence Review");

    let active = app
        .execute(envelope(
            10_002,
            ApplicationCommand::ShowSkill {
                selector: SkillSelector::from(created.skill_id),
            },
        ))
        .unwrap();
    let CommandView::Skill(active) = active.view else {
        panic!("skill view")
    };
    assert_eq!(active.content.resources[0].body, "Verify every material claim.");
    let mut rendered = Vec::new();
    ai_stock_forum::ui::command::TextRenderer::render_view(
        &CommandView::Skill(active.clone()),
        &mut rendered,
    )
    .unwrap();
    let rendered = String::from_utf8(rendered).unwrap();
    assert!(rendered.contains("Evidence Review"));
    assert!(rendered.contains(&created.skill_id.to_string()));
    assert!(!rendered.contains("Verify every material claim."));

    let version_candidate = skill("Evidence Review", "Require primary-source citations.");
    let version_preview = app
        .preview_skill_version(
            created.skill_id,
            created.skill_version_id,
            version_candidate.clone(),
        )
        .unwrap();
    let versioned = app
        .execute(envelope(
            10_003,
            ApplicationCommand::ActivateSkillVersion {
                skill_id: created.skill_id,
                expected_active_version_id: created.skill_version_id,
                candidate: version_candidate,
                review_token: version_preview.review_token,
                review_digest: version_preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::SkillVersionActivated(versioned) = versioned.view else {
        panic!("skill version activated view")
    };
    assert_eq!(versioned.version.get(), 2);

    let historical = app
        .execute(envelope(
            10_004,
            ApplicationCommand::ShowSkillVersion {
                selector: SkillSelector::from(created.skill_id),
                version: created.version,
            },
        ))
        .unwrap();
    let CommandView::SkillVersion(historical) = historical.view else {
        panic!("historical skill view")
    };
    assert_eq!(historical.skill_ref.skill_version_id(), created.skill_version_id);

    let history = app
        .execute(envelope(
            10_005,
            ApplicationCommand::ShowSkillHistory {
                selector: SkillSelector::from(created.skill_id),
            },
        ))
        .unwrap();
    let CommandView::SkillHistory(history) = history.view else {
        panic!("skill history view")
    };
    assert_eq!(history.versions.len(), 2);
    assert_eq!(history.active_version_id, versioned.skill_version_id);

    let profile = app
        .execute(envelope(
            10_006,
            ApplicationCommand::CreateAgentProfile {
                draft: profile("Research Analyst"),
                template_provenance: None,
            },
        ))
        .unwrap();
    let CommandView::AgentProfileCreated(profile) = profile.view else {
        panic!("profile created view")
    };

    let assign_preview = app
        .preview_agent_skill_assignment(
            profile.profile_id,
            profile.profile_version_id,
            historical.skill_ref.clone(),
        )
        .unwrap();
    assert!(matches!(assign_preview.operation, AgentSkillAssignmentOperation::Assign { .. }));
    let assigned = app
        .execute(envelope(
            10_007,
            ApplicationCommand::AssignAgentSkill {
                profile_id: profile.profile_id,
                expected_active_profile_version_id: profile.profile_version_id,
                skill: historical.skill_ref.clone(),
                review_token: assign_preview.review_token,
                review_digest: assign_preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::AgentSkillAssigned(assigned) = assigned.view else {
        panic!("agent skill assigned view")
    };

    let active_replacement = active_skill_ref(&mut app, created.skill_id, 10_008);
    let upgrade_preview = app
        .preview_agent_skill_upgrade(
            profile.profile_id,
            assigned.profile_version_id,
            historical.skill_ref.clone(),
            active_replacement,
        )
        .unwrap();
    let AgentSkillAssignmentOperation::Upgrade { replacement, .. } = &upgrade_preview.operation else {
        panic!("upgrade operation")
    };
    let replacement = replacement.clone();
    let upgraded = app
        .execute(envelope(
            10_009,
            ApplicationCommand::UpgradeAgentSkill {
                profile_id: profile.profile_id,
                expected_active_profile_version_id: assigned.profile_version_id,
                expected: historical.skill_ref,
                replacement: replacement.clone(),
                review_token: upgrade_preview.review_token,
                review_digest: upgrade_preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::AgentSkillUpgraded(upgraded) = upgraded.view else {
        panic!("agent skill upgraded view")
    };

    let unassign_preview = app
        .preview_agent_skill_unassignment(
            profile.profile_id,
            upgraded.profile_version_id,
            replacement.clone(),
        )
        .unwrap();
    let unassigned = app
        .execute(envelope(
            10_010,
            ApplicationCommand::UnassignAgentSkill {
                profile_id: profile.profile_id,
                expected_active_profile_version_id: upgraded.profile_version_id,
                expected: replacement,
                review_token: unassign_preview.review_token,
                review_digest: unassign_preview.review_digest,
            },
        ))
        .unwrap();
    assert!(matches!(unassigned.view, CommandView::AgentSkillUnassigned(_)));
}

fn active_skill_ref(
    app: &mut support::TestApp,
    skill_id: ai_stock_forum::domain::SkillId,
    command_id: u128,
) -> ai_stock_forum::skills::SkillVersionRef {
    let outcome = app
        .execute(envelope(
            command_id,
            ApplicationCommand::ShowSkill {
                selector: SkillSelector::from(skill_id),
            },
        ))
        .unwrap();
    let CommandView::Skill(view) = outcome.view else {
        panic!("skill view")
    };
    view.skill_ref
}

#[test]
fn create_and_version_reject_name_conflicts_and_stale_active_versions() {
    let mut app = support::app();
    let candidate = skill("Evidence Review", "State evidence.");
    let preview = app.preview_skill_creation(candidate.clone()).unwrap();
    let created = app
        .execute(envelope(
            20_000,
            ApplicationCommand::CreateSkill {
                skill_id: preview.skill_id,
                candidate,
                review_token: preview.review_token,
                review_digest: preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::SkillCreated(created) = created.view else {
        panic!("created")
    };

    assert_eq!(
        app.preview_skill_creation(skill("  EVIDENCE   review ", "Different content."))
            .unwrap_err(),
        AppError::DuplicateSkillName,
    );
    assert_eq!(
        app.preview_skill_version(
            created.skill_id,
            ai_stock_forum::domain::SkillVersionId::from_uuid(Uuid::from_u128(999)),
            skill("Evidence Review", "Changed."),
        )
        .unwrap_err(),
        AppError::StaleSkillVersion,
    );
}
