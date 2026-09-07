mod support;

use std::thread;

use ai_stock_forum::{
    app::{AppError, ApplicationCommand, CommandEnvelope, CommandView},
    domain::{Actor, CommandId, CorrelationId},
    skills::SkillDraft,
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

mod fix_round_one_agent_skill_races {
    use super::support;
    use ai_stock_forum::{
        agents::{AgentBindings, AgentProfileDraft, AgentRole},
        app::{
            AppError, ApplicationCommand, CommandEnvelope, CommandOutcome, CommandView,
            SkillSelector,
        },
        domain::{Actor, AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId},
        skills::{SkillDraft, SkillResource, SkillVersionRef},
    };
    use uuid::Uuid;

    fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
        CommandEnvelope {
            command_id: CommandId::from_uuid(Uuid::from_u128(20_000_000 + id)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(21_000_000 + id)),
            actor: Actor::Human,
            command,
        }
    }

    fn skill(name: &str, instructions: &str) -> SkillDraft {
        SkillDraft::new(
            name.to_owned(),
            "Concurrent assignment contract.".to_owned(),
            "Use for concurrent profile mutation.".to_owned(),
            vec!["race-safe".to_owned()],
            instructions.to_owned(),
            vec![SkillResource {
                name: "Race".to_owned(),
                body: format!("{instructions} resource"),
            }],
        )
        .unwrap()
    }

    fn profile() -> AgentProfileDraft {
        AgentProfileDraft::new(
            "Concurrent Agent".to_owned(),
            "Concurrent mutation profile.".to_owned(),
            AgentRole::Custom,
            "concurrency".to_owned(),
            vec!["race-profile".to_owned()],
            "Deterministic.".to_owned(),
            "One winner only.".to_owned(),
            AgentBindings::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    fn create_skill(app: &mut support::TestApp, id: u128, name: &str) -> SkillVersionRef {
        let candidate = skill(name, "version one");
        let preview = app.preview_skill_creation(candidate.clone()).unwrap();
        app.execute(envelope(
            id,
            ApplicationCommand::CreateSkill {
                skill_id: preview.skill_id,
                candidate,
                review_token: preview.review_token,
                review_digest: preview.review_digest,
            },
        ))
        .unwrap();
        active_skill(app, id + 1, preview.skill_id)
    }

    fn active_skill(
        app: &mut support::TestApp,
        id: u128,
        skill_id: ai_stock_forum::domain::SkillId,
    ) -> SkillVersionRef {
        let outcome = app
            .execute(envelope(
                id,
                ApplicationCommand::ShowSkill {
                    selector: SkillSelector::from(skill_id),
                },
            ))
            .unwrap();
        match outcome.view {
            CommandView::Skill(skill) => skill.skill_ref,
            _ => panic!("skill view"),
        }
    }

    fn activate_skill(
        app: &mut support::TestApp,
        id: u128,
        current: &SkillVersionRef,
        instructions: &str,
    ) -> SkillVersionRef {
        let candidate = skill("Upgradeable Skill", instructions);
        let preview = app
            .preview_skill_version(
                current.skill_id(),
                current.skill_version_id(),
                candidate.clone(),
            )
            .unwrap();
        app.execute(envelope(
            id,
            ApplicationCommand::ActivateSkillVersion {
                skill_id: current.skill_id(),
                expected_active_version_id: current.skill_version_id(),
                candidate,
                review_token: preview.review_token,
                review_digest: preview.review_digest,
            },
        ))
        .unwrap();
        active_skill(app, id + 1, current.skill_id())
    }

    fn create_profile(
        app: &mut support::TestApp,
        id: u128,
    ) -> (AgentProfileId, AgentProfileVersionId) {
        let outcome = app
            .execute(envelope(
                id,
                ApplicationCommand::CreateAgentProfile {
                    draft: profile(),
                    template_provenance: None,
                },
            ))
            .unwrap();
        match outcome.view {
            CommandView::AgentProfileCreated(profile) => {
                (profile.profile_id, profile.profile_version_id)
            }
            _ => panic!("profile created"),
        }
    }

    fn assign(
        app: &mut support::TestApp,
        id: u128,
        profile_id: AgentProfileId,
        profile_version_id: AgentProfileVersionId,
        skill: SkillVersionRef,
    ) -> AgentProfileVersionId {
        let preview = app
            .preview_agent_skill_assignment(profile_id, profile_version_id, skill.clone())
            .unwrap();
        let outcome = app
            .execute(envelope(
                id,
                ApplicationCommand::AssignAgentSkill {
                    profile_id,
                    expected_active_profile_version_id: profile_version_id,
                    skill,
                    review_token: preview.review_token,
                    review_digest: preview.review_digest,
                },
            ))
            .unwrap();
        match outcome.view {
            CommandView::AgentSkillAssigned(profile) => profile.profile_version_id,
            _ => panic!("skill assigned"),
        }
    }

    fn assert_one_winner_one_stale(
        results: [Result<CommandOutcome, AppError>; 2],
    ) -> AgentProfileVersionId {
        let mut winner = None;
        let mut stale = 0;
        for result in results {
            match result {
                Ok(outcome) => {
                    assert!(winner.is_none(), "more than one mutation committed");
                    winner = Some(match outcome.view {
                        CommandView::AgentSkillAssigned(view)
                        | CommandView::AgentSkillUpgraded(view)
                        | CommandView::AgentSkillUnassigned(view) => view.profile_version_id,
                        other => panic!("unexpected winner outcome: {other:?}"),
                    });
                }
                Err(AppError::StaleAgentProfileVersion) => stale += 1,
                Err(other) => panic!("unexpected loser: {other:?}"),
            }
        }
        assert_eq!(stale, 1);
        winner.expect("one winner")
    }

    fn assert_atomic_delta(
        app: &support::TestApp,
        before: (i64, i64, i64, i64),
        event_kind: &str,
        winner: AgentProfileVersionId,
    ) {
        assert_eq!(app.count_rows("event_stream"), before.0 + 1);
        assert_eq!(app.count_rows("command_receipts"), before.1 + 1);
        assert_eq!(app.count_rows("command_event_refs"), before.2 + 1);
        assert_eq!(app.count_rows("agent_profile_versions"), before.3 + 1);
        let active = app.active_profile_rows();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].1, winner.to_string());
        let payloads = app.event_payloads(event_kind);
        assert_eq!(payloads.len(), 1);
        assert!(payloads[0].contains(&winner.to_string()));
    }

    fn counts(app: &support::TestApp) -> (i64, i64, i64, i64) {
        (
            app.count_rows("event_stream"),
            app.count_rows("command_receipts"),
            app.count_rows("command_event_refs"),
            app.count_rows("agent_profile_versions"),
        )
    }

    #[test]
    fn concurrent_assignments_have_one_winner_and_no_partial_writes() {
        let mut app = support::app();
        let first = create_skill(&mut app, 300, "Assignment Race A");
        let second = create_skill(&mut app, 310, "Assignment Race B");
        let (profile_id, profile_version_id) = create_profile(&mut app, 320);
        let mut left = app.independent_skill_instance().unwrap();
        let mut right = app.independent_skill_instance().unwrap();
        let left_preview = left
            .preview_agent_skill_assignment(profile_id, profile_version_id, first.clone())
            .unwrap();
        let right_preview = right
            .preview_agent_skill_assignment(profile_id, profile_version_id, second.clone())
            .unwrap();
        let before = counts(&app);
        let left_command = envelope(
            321,
            ApplicationCommand::AssignAgentSkill {
                profile_id,
                expected_active_profile_version_id: profile_version_id,
                skill: first,
                review_token: left_preview.review_token,
                review_digest: left_preview.review_digest,
            },
        );
        let right_command = envelope(
            322,
            ApplicationCommand::AssignAgentSkill {
                profile_id,
                expected_active_profile_version_id: profile_version_id,
                skill: second,
                review_token: right_preview.review_token,
                review_digest: right_preview.review_digest,
            },
        );
        let results = std::thread::scope(|scope| {
            let left = scope.spawn(move || left.execute(left_command));
            let right = scope.spawn(move || right.execute(right_command));
            [left.join().unwrap(), right.join().unwrap()]
        });
        let winner = assert_one_winner_one_stale(results);
        assert_atomic_delta(&app, before, "agent_skill_assigned", winner);
    }

    #[test]
    fn concurrent_upgrades_have_one_winner_and_no_partial_writes() {
        let mut app = support::app();
        let version_one = create_skill(&mut app, 400, "Upgradeable Skill");
        let version_two = activate_skill(&mut app, 410, &version_one, "version two");
        let version_three = activate_skill(&mut app, 420, &version_two, "version three");
        let (profile_id, initial_profile_version_id) = create_profile(&mut app, 430);
        let assigned_profile_version_id = assign(
            &mut app,
            431,
            profile_id,
            initial_profile_version_id,
            version_one.clone(),
        );
        let mut left = app.independent_skill_instance().unwrap();
        let mut right = app.independent_skill_instance().unwrap();
        let left_preview = left
            .preview_agent_skill_upgrade(
                profile_id,
                assigned_profile_version_id,
                version_one.clone(),
                version_two.clone(),
            )
            .unwrap();
        let right_preview = right
            .preview_agent_skill_upgrade(
                profile_id,
                assigned_profile_version_id,
                version_one.clone(),
                version_three.clone(),
            )
            .unwrap();
        let before = counts(&app);
        let results = std::thread::scope(|scope| {
            let left_expected = version_one.clone();
            let left = scope.spawn(move || {
                left.execute(envelope(
                    432,
                    ApplicationCommand::UpgradeAgentSkill {
                        profile_id,
                        expected_active_profile_version_id: assigned_profile_version_id,
                        expected: left_expected,
                        replacement: version_two,
                        review_token: left_preview.review_token,
                        review_digest: left_preview.review_digest,
                    },
                ))
            });
            let right = scope.spawn(move || {
                right.execute(envelope(
                    433,
                    ApplicationCommand::UpgradeAgentSkill {
                        profile_id,
                        expected_active_profile_version_id: assigned_profile_version_id,
                        expected: version_one,
                        replacement: version_three,
                        review_token: right_preview.review_token,
                        review_digest: right_preview.review_digest,
                    },
                ))
            });
            [left.join().unwrap(), right.join().unwrap()]
        });
        let winner = assert_one_winner_one_stale(results);
        assert_atomic_delta(&app, before, "agent_skill_upgraded", winner);
    }

    #[test]
    fn concurrent_unassignments_have_one_winner_and_no_partial_writes() {
        let mut app = support::app();
        let skill = create_skill(&mut app, 500, "Unassignment Race");
        let (profile_id, initial_profile_version_id) = create_profile(&mut app, 510);
        let assigned_profile_version_id = assign(
            &mut app,
            511,
            profile_id,
            initial_profile_version_id,
            skill.clone(),
        );
        let mut left = app.independent_skill_instance().unwrap();
        let mut right = app.independent_skill_instance().unwrap();
        let left_preview = left
            .preview_agent_skill_unassignment(
                profile_id,
                assigned_profile_version_id,
                skill.clone(),
            )
            .unwrap();
        let right_preview = right
            .preview_agent_skill_unassignment(
                profile_id,
                assigned_profile_version_id,
                skill.clone(),
            )
            .unwrap();
        let before = counts(&app);
        let results = std::thread::scope(|scope| {
            let left_skill = skill.clone();
            let left = scope.spawn(move || {
                left.execute(envelope(
                    512,
                    ApplicationCommand::UnassignAgentSkill {
                        profile_id,
                        expected_active_profile_version_id: assigned_profile_version_id,
                        expected: left_skill,
                        review_token: left_preview.review_token,
                        review_digest: left_preview.review_digest,
                    },
                ))
            });
            let right = scope.spawn(move || {
                right.execute(envelope(
                    513,
                    ApplicationCommand::UnassignAgentSkill {
                        profile_id,
                        expected_active_profile_version_id: assigned_profile_version_id,
                        expected: skill,
                        review_token: right_preview.review_token,
                        review_digest: right_preview.review_digest,
                    },
                ))
            });
            [left.join().unwrap(), right.join().unwrap()]
        });
        let winner = assert_one_winner_one_stale(results);
        assert_atomic_delta(&app, before, "agent_skill_unassigned", winner);
    }
}

fn draft(instructions: &str) -> SkillDraft {
    SkillDraft::new(
        "Concurrent Skill".to_owned(),
        "Concurrency behavior.".to_owned(),
        "Use for concurrency tests.".to_owned(),
        Vec::new(),
        instructions.to_owned(),
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn concurrent_versions_against_one_active_version_have_one_winner_and_one_stale_loser() {
    let mut app = support::app();
    let create_preview = app.preview_skill_creation(draft("Version one.")).unwrap();
    let created = app
        .execute(envelope(
            50_000,
            ApplicationCommand::CreateSkill {
                skill_id: create_preview.skill_id,
                candidate: draft("Version one."),
                review_token: create_preview.review_token,
                review_digest: create_preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::SkillCreated(created) = created.view else {
        panic!("created")
    };
    let mut first = app.independent_skill_instance().unwrap();
    let mut second = app.independent_skill_instance().unwrap();
    let first_candidate = draft("First version two.");
    let second_candidate = draft("Second version two.");
    let first_preview = first
        .preview_skill_version(
            created.skill_id,
            created.skill_version_id,
            first_candidate.clone(),
        )
        .unwrap();
    let second_preview = second
        .preview_skill_version(
            created.skill_id,
            created.skill_version_id,
            second_candidate.clone(),
        )
        .unwrap();

    let first_command = envelope(
        50_001,
        ApplicationCommand::ActivateSkillVersion {
            skill_id: created.skill_id,
            expected_active_version_id: created.skill_version_id,
            candidate: first_candidate,
            review_token: first_preview.review_token,
            review_digest: first_preview.review_digest,
        },
    );
    let second_command = envelope(
        50_002,
        ApplicationCommand::ActivateSkillVersion {
            skill_id: created.skill_id,
            expected_active_version_id: created.skill_version_id,
            candidate: second_candidate,
            review_token: second_preview.review_token,
            review_digest: second_preview.review_digest,
        },
    );
    let first_thread = thread::spawn(move || first.execute(first_command));
    let second_thread = thread::spawn(move || second.execute(second_command));
    let results = [first_thread.join().unwrap(), second_thread.join().unwrap()];

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(AppError::StaleSkillVersion)))
            .count(),
        1,
    );
    assert_eq!(app.event_count("skill_version_activated"), 1);
}
