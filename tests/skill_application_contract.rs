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

mod fix_round_one_review_and_exact_refs {
    use super::support;
    use ai_stock_forum::{
        agents::{AgentBindings, AgentProfileDraft, AgentRole},
        app::{
            AgentSkillAssignmentOperation, AppError, ApplicationCommand, CommandEnvelope,
            CommandOutcome, CommandView, SkillSelector,
        },
        domain::{Actor, AgentProfileId, CommandId, CorrelationId, SkillId},
        skills::{SkillDraft, SkillResource, SkillVersionRef},
    };
    use uuid::Uuid;

    fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
        CommandEnvelope {
            command_id: CommandId::from_uuid(Uuid::from_u128(10_000_000 + id)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(11_000_000 + id)),
            actor: Actor::Human,
            command,
        }
    }

    fn skill(name: &str, instructions: &str) -> SkillDraft {
        SkillDraft::new(
            name.to_owned(),
            "Review binding contract.".to_owned(),
            "Use for exact review binding.".to_owned(),
            vec!["review".to_owned()],
            instructions.to_owned(),
            vec![SkillResource {
                name: "Reference".to_owned(),
                body: format!("{instructions} body"),
            }],
        )
        .unwrap()
    }

    fn profile(name: &str) -> AgentProfileDraft {
        AgentProfileDraft::new(
            name.to_owned(),
            "Review binding profile.".to_owned(),
            AgentRole::Custom,
            "review binding".to_owned(),
            vec!["review".to_owned()],
            "Exact.".to_owned(),
            "Reject substitutions.".to_owned(),
            AgentBindings::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    #[derive(Debug, PartialEq)]
    struct DurableSnapshot {
        skill: Option<CommandOutcome>,
        skill_history: Option<CommandOutcome>,
        profile: Option<CommandOutcome>,
        profile_history: Option<CommandOutcome>,
        active_profile_rows: Vec<(String, String, i64, String, String)>,
        mutation_payloads: Vec<Vec<String>>,
        event_rows: i64,
        receipt_rows: i64,
        event_reference_rows: i64,
        profile_version_rows: i64,
    }

    fn snapshot(
        app: &mut support::TestApp,
        skill_id: Option<SkillId>,
        profile_id: Option<AgentProfileId>,
        read_id: u128,
    ) -> DurableSnapshot {
        let skill = skill_id.map(|skill_id| {
            app.execute(envelope(
                read_id,
                ApplicationCommand::ShowSkill {
                    selector: SkillSelector::from(skill_id),
                },
            ))
            .unwrap()
        });
        let skill_history = skill_id.map(|skill_id| {
            app.execute(envelope(
                read_id + 1,
                ApplicationCommand::ShowSkillHistory {
                    selector: SkillSelector::from(skill_id),
                },
            ))
            .unwrap()
        });
        let profile = profile_id.map(|profile_id| {
            app.execute(envelope(
                read_id + 2,
                ApplicationCommand::ShowAgentProfile {
                    selector: ai_stock_forum::app::AgentProfileSelector::from(profile_id),
                },
            ))
            .unwrap()
        });
        let profile_history = profile_id.map(|profile_id| {
            app.execute(envelope(
                read_id + 3,
                ApplicationCommand::ShowAgentProfileHistory {
                    selector: ai_stock_forum::app::AgentProfileSelector::from(profile_id),
                },
            ))
            .unwrap()
        });
        DurableSnapshot {
            skill,
            skill_history,
            profile,
            profile_history,
            active_profile_rows: app.active_profile_rows(),
            mutation_payloads: [
                "skill_created",
                "skill_version_activated",
                "agent_skill_assigned",
                "agent_skill_upgraded",
                "agent_skill_unassigned",
            ]
            .into_iter()
            .map(|kind| app.event_payloads(kind))
            .collect(),
            event_rows: app.count_rows("event_stream"),
            receipt_rows: app.count_rows("command_receipts"),
            event_reference_rows: app.count_rows("command_event_refs"),
            profile_version_rows: app.count_rows("agent_profile_versions"),
        }
    }

    fn assert_rejected_unchanged(
        app: &mut support::TestApp,
        before: DurableSnapshot,
        skill_id: Option<SkillId>,
        profile_id: Option<AgentProfileId>,
        read_id: u128,
    ) {
        assert_eq!(snapshot(app, skill_id, profile_id, read_id), before);
    }

    fn create_skill(
        app: &mut support::TestApp,
        id: u128,
        candidate: SkillDraft,
    ) -> SkillVersionRef {
        let preview = app.preview_skill_creation(candidate.clone()).unwrap();
        let outcome = app
            .execute(envelope(
                id,
                ApplicationCommand::CreateSkill {
                    skill_id: preview.skill_id,
                    candidate,
                    review_token: preview.review_token,
                    review_digest: preview.review_digest,
                },
            ))
            .unwrap();
        let CommandView::SkillCreated(created) = outcome.view else {
            panic!("skill created")
        };
        app.execute(envelope(
            id + 1,
            ApplicationCommand::ShowSkill {
                selector: SkillSelector::from(created.skill_id),
            },
        ))
        .map(|outcome| match outcome.view {
            CommandView::Skill(skill) => skill.skill_ref,
            _ => panic!("skill view"),
        })
        .unwrap()
    }

    #[test]
    fn review_tokens_bind_actors_candidates_operations_refs_agents_and_single_use() {
        let mut app = support::app();

        let original = skill("Bound Skill", "original candidate");
        let substituted = skill("Bound Skill", "substituted candidate");
        let create_preview = app.preview_skill_creation(original.clone()).unwrap();
        let before = snapshot(&mut app, None, None, 2_000);
        assert_eq!(
            app.execute(envelope(
                200,
                ApplicationCommand::CreateSkill {
                    skill_id: create_preview.skill_id,
                    candidate: substituted,
                    review_token: create_preview.review_token,
                    review_digest: create_preview.review_digest.clone(),
                },
            )),
            Err(AppError::SkillReviewMismatch)
        );
        assert_rejected_unchanged(&mut app, before, None, None, 2_000);

        let create_command = envelope(
            201,
            ApplicationCommand::CreateSkill {
                skill_id: create_preview.skill_id,
                candidate: original,
                review_token: create_preview.review_token,
                review_digest: create_preview.review_digest,
            },
        );
        let created = app.execute(create_command.clone()).unwrap();
        assert_eq!(app.execute(create_command.clone()).unwrap(), created);
        let CommandView::SkillCreated(created_view) = &created.view else {
            panic!("skill created")
        };
        let before = snapshot(&mut app, Some(created_view.skill_id), None, 2_100);
        let mut reused = create_command.clone();
        reused.command_id = CommandId::from_uuid(Uuid::from_u128(10_000_202));
        reused.correlation_id = CorrelationId::from_uuid(Uuid::from_u128(11_000_202));
        assert_eq!(app.execute(reused), Err(AppError::SkillReviewUnavailable));
        assert_rejected_unchanged(
            &mut app,
            before,
            Some(created_view.skill_id),
            None,
            2_100,
        );

        let version_candidate = skill("Bound Skill", "accepted version two");
        let version_preview = app
            .preview_skill_version(
                created_view.skill_id,
                created_view.skill_version_id,
                version_candidate.clone(),
            )
            .unwrap();
        let before = snapshot(&mut app, Some(created_view.skill_id), None, 2_200);
        assert_eq!(
            app.execute(envelope(
                203,
                ApplicationCommand::ActivateSkillVersion {
                    skill_id: created_view.skill_id,
                    expected_active_version_id: created_view.skill_version_id,
                    candidate: skill("Bound Skill", "substituted version two"),
                    review_token: version_preview.review_token,
                    review_digest: version_preview.review_digest.clone(),
                },
            )),
            Err(AppError::SkillReviewMismatch)
        );
        assert_rejected_unchanged(
            &mut app,
            before,
            Some(created_view.skill_id),
            None,
            2_200,
        );
        let versioned = app
            .execute(envelope(
                204,
                ApplicationCommand::ActivateSkillVersion {
                    skill_id: created_view.skill_id,
                    expected_active_version_id: created_view.skill_version_id,
                    candidate: version_candidate,
                    review_token: version_preview.review_token,
                    review_digest: version_preview.review_digest,
                },
            ))
            .unwrap();
        let CommandView::SkillVersionActivated(versioned) = versioned.view else {
            panic!("skill versioned")
        };
        let active_ref = app
            .execute(envelope(
                205,
                ApplicationCommand::ShowSkill {
                    selector: SkillSelector::from(created_view.skill_id),
                },
            ))
            .map(|outcome| match outcome.view {
                CommandView::Skill(skill) => skill.skill_ref,
                _ => panic!("skill view"),
            })
            .unwrap();
        assert_eq!(active_ref.skill_version_id(), versioned.skill_version_id);
        let second_ref = create_skill(
            &mut app,
            210,
            skill("Substitution Skill", "alternate exact ref"),
        );

        let profile_a = app
            .execute(envelope(
                220,
                ApplicationCommand::CreateAgentProfile {
                    draft: profile("Bound Agent"),
                    template_provenance: None,
                },
            ))
            .unwrap();
        let CommandView::AgentProfileCreated(profile_a) = profile_a.view else {
            panic!("profile A")
        };
        let profile_b = app
            .execute(envelope(
                221,
                ApplicationCommand::CreateAgentProfile {
                    draft: profile("Other Agent"),
                    template_provenance: None,
                },
            ))
            .unwrap();
        let CommandView::AgentProfileCreated(profile_b) = profile_b.view else {
            panic!("profile B")
        };

        let original_ref = app
            .execute(envelope(
                219,
                ApplicationCommand::ShowSkillVersion {
                    selector: SkillSelector::from(created_view.skill_id),
                    version: created_view.version,
                },
            ))
            .map(|outcome| match outcome.view {
                CommandView::SkillVersion(skill) => skill.skill_ref,
                _ => panic!("historical skill view"),
            })
            .unwrap();
        let assign_preview = app
            .preview_agent_skill_assignment(
                profile_a.profile_id,
                profile_a.profile_version_id,
                original_ref.clone(),
            )
            .unwrap();
        assert!(matches!(
            &assign_preview.operation,
            AgentSkillAssignmentOperation::Assign { skill } if skill == &original_ref
        ));
        let mut actor_swapped_assign = envelope(
            12_225,
            ApplicationCommand::AssignAgentSkill {
                profile_id: profile_a.profile_id,
                expected_active_profile_version_id: profile_a.profile_version_id,
                skill: original_ref.clone(),
                review_token: assign_preview.review_token,
                review_digest: assign_preview.review_digest.clone(),
            },
        );
        actor_swapped_assign.actor = Actor::System;
        assert_eq!(
            app.execute(actor_swapped_assign),
            Err(AppError::SkillReviewMismatch)
        );
        for (id, profile_id, command) in [
            (
                222,
                profile_a.profile_id,
                ApplicationCommand::AssignAgentSkill {
                    profile_id: profile_a.profile_id,
                    expected_active_profile_version_id: profile_a.profile_version_id,
                    skill: second_ref.clone(),
                    review_token: assign_preview.review_token,
                    review_digest: assign_preview.review_digest.clone(),
                },
            ),
            (
                223,
                profile_b.profile_id,
                ApplicationCommand::AssignAgentSkill {
                    profile_id: profile_b.profile_id,
                    expected_active_profile_version_id: profile_b.profile_version_id,
                    skill: original_ref.clone(),
                    review_token: assign_preview.review_token,
                    review_digest: assign_preview.review_digest.clone(),
                },
            ),
            (
                224,
                profile_a.profile_id,
                ApplicationCommand::UpgradeAgentSkill {
                    profile_id: profile_a.profile_id,
                    expected_active_profile_version_id: profile_a.profile_version_id,
                    expected: original_ref.clone(),
                    replacement: active_ref.clone(),
                    review_token: assign_preview.review_token,
                    review_digest: assign_preview.review_digest.clone(),
                },
            ),
        ] {
            let before = snapshot(
                &mut app,
                Some(created_view.skill_id),
                Some(profile_id),
                3_000 + id * 10,
            );
            assert_eq!(
                app.execute(envelope(id, command)),
                Err(AppError::SkillReviewMismatch)
            );
            assert_rejected_unchanged(
                &mut app,
                before,
                Some(created_view.skill_id),
                Some(profile_id),
                3_000 + id * 10,
            );
        }

        let assign = envelope(
            225,
            ApplicationCommand::AssignAgentSkill {
                profile_id: profile_a.profile_id,
                expected_active_profile_version_id: profile_a.profile_version_id,
                skill: original_ref.clone(),
                review_token: assign_preview.review_token,
                review_digest: assign_preview.review_digest,
            },
        );
        let assigned = app.execute(assign.clone()).unwrap();
        assert_eq!(app.execute(assign.clone()).unwrap(), assigned);
        let CommandView::AgentSkillAssigned(assigned_view) = &assigned.view else {
            panic!("assigned")
        };
        let before = snapshot(
            &mut app,
            Some(created_view.skill_id),
            Some(profile_a.profile_id),
            2_600,
        );
        let mut reused_assign = assign;
        reused_assign.command_id = CommandId::from_uuid(Uuid::from_u128(10_000_226));
        reused_assign.correlation_id = CorrelationId::from_uuid(Uuid::from_u128(11_000_226));
        assert_eq!(
            app.execute(reused_assign),
            Err(AppError::SkillReviewUnavailable)
        );
        assert_rejected_unchanged(
            &mut app,
            before,
            Some(created_view.skill_id),
            Some(profile_a.profile_id),
            2_600,
        );

        let before = snapshot(
            &mut app,
            Some(created_view.skill_id),
            Some(profile_a.profile_id),
            2_700,
        );
        assert_eq!(
            app.preview_agent_skill_upgrade(
                profile_a.profile_id,
                assigned_view.profile_version_id,
                active_ref.clone(),
                second_ref.clone(),
            ),
            Err(AppError::SkillNotAssigned)
        );
        assert_rejected_unchanged(
            &mut app,
            before,
            Some(created_view.skill_id),
            Some(profile_a.profile_id),
            2_700,
        );
        let before = snapshot(
            &mut app,
            Some(created_view.skill_id),
            Some(profile_a.profile_id),
            2_800,
        );
        assert_eq!(
            app.preview_agent_skill_unassignment(
                profile_a.profile_id,
                assigned_view.profile_version_id,
                active_ref.clone(),
            ),
            Err(AppError::SkillNotAssigned)
        );
        assert_rejected_unchanged(
            &mut app,
            before,
            Some(created_view.skill_id),
            Some(profile_a.profile_id),
            2_800,
        );

        let mut stale = app.independent_skill_instance().unwrap();
        let stale_preview = stale
            .preview_agent_skill_assignment(
                profile_a.profile_id,
                assigned_view.profile_version_id,
                second_ref.clone(),
            )
            .unwrap();
        let upgrade_preview = app
            .preview_agent_skill_upgrade(
                profile_a.profile_id,
                assigned_view.profile_version_id,
                original_ref.clone(),
                active_ref.clone(),
            )
            .unwrap();
        let mut actor_swapped_upgrade = envelope(
            12_227,
            ApplicationCommand::UpgradeAgentSkill {
                profile_id: profile_a.profile_id,
                expected_active_profile_version_id: assigned_view.profile_version_id,
                expected: original_ref.clone(),
                replacement: active_ref.clone(),
                review_token: upgrade_preview.review_token,
                review_digest: upgrade_preview.review_digest.clone(),
            },
        );
        actor_swapped_upgrade.actor = Actor::System;
        assert_eq!(
            app.execute(actor_swapped_upgrade),
            Err(AppError::SkillReviewMismatch)
        );
        let upgraded = app
            .execute(envelope(
                227,
                ApplicationCommand::UpgradeAgentSkill {
                    profile_id: profile_a.profile_id,
                    expected_active_profile_version_id: assigned_view.profile_version_id,
                    expected: original_ref.clone(),
                    replacement: active_ref.clone(),
                    review_token: upgrade_preview.review_token,
                    review_digest: upgrade_preview.review_digest,
                },
            ))
            .unwrap();
        let CommandView::AgentSkillUpgraded(upgraded) = upgraded.view else {
            panic!("upgraded")
        };
        let unassign_preview = app
            .preview_agent_skill_unassignment(
                profile_a.profile_id,
                upgraded.profile_version_id,
                active_ref.clone(),
            )
            .unwrap();
        let mut actor_swapped_unassign = envelope(
            12_228,
            ApplicationCommand::UnassignAgentSkill {
                profile_id: profile_a.profile_id,
                expected_active_profile_version_id: upgraded.profile_version_id,
                expected: active_ref.clone(),
                review_token: unassign_preview.review_token,
                review_digest: unassign_preview.review_digest.clone(),
            },
        );
        actor_swapped_unassign.actor = Actor::System;
        assert_eq!(
            app.execute(actor_swapped_unassign),
            Err(AppError::SkillReviewMismatch)
        );
        let unassigned = app
            .execute(envelope(
                12_229,
                ApplicationCommand::UnassignAgentSkill {
                    profile_id: profile_a.profile_id,
                    expected_active_profile_version_id: upgraded.profile_version_id,
                    expected: active_ref,
                    review_token: unassign_preview.review_token,
                    review_digest: unassign_preview.review_digest,
                },
            ))
            .unwrap();
        assert!(matches!(
            unassigned.view,
            CommandView::AgentSkillUnassigned(_)
        ));
        let before = snapshot(
            &mut app,
            Some(created_view.skill_id),
            Some(profile_a.profile_id),
            2_900,
        );
        assert_eq!(
            stale.execute(envelope(
                228,
                ApplicationCommand::AssignAgentSkill {
                    profile_id: profile_a.profile_id,
                    expected_active_profile_version_id: assigned_view.profile_version_id,
                    skill: second_ref,
                    review_token: stale_preview.review_token,
                    review_digest: stale_preview.review_digest,
                },
            )),
            Err(AppError::StaleAgentProfileVersion)
        );
        assert_rejected_unchanged(
            &mut app,
            before,
            Some(created_view.skill_id),
            Some(profile_a.profile_id),
            2_900,
        );
        assert!(upgraded.profile_version_id != assigned_view.profile_version_id);
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
    let create_candidate = skill(
        "Independent Evidence Review",
        "State evidence and disconfirming facts.",
    );
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
    assert_eq!(listed.total_count, 5);
    assert!(
        listed
            .skills
            .iter()
            .any(|skill| skill.display_name == "Independent Evidence Review")
    );

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
    assert!(rendered.contains("Independent Evidence Review"));
    assert!(rendered.contains(&created.skill_id.to_string()));
    assert!(!rendered.contains("Verify every material claim."));

    let version_candidate = skill(
        "Independent Evidence Review",
        "Require primary-source citations.",
    );
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
    let candidate = skill("Independent Evidence Review", "State evidence.");
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
        app.preview_skill_creation(skill(
            "  INDEPENDENT   EVIDENCE   review ",
            "Different content.",
        ))
            .unwrap_err(),
        AppError::DuplicateSkillName,
    );
    assert_eq!(
        app.preview_skill_version(
            created.skill_id,
            ai_stock_forum::domain::SkillVersionId::from_uuid(Uuid::from_u128(999)),
            skill("Independent Evidence Review", "Changed."),
        )
        .unwrap_err(),
        AppError::StaleSkillVersion,
    );
}
