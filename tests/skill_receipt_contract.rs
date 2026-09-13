mod support;

use ai_stock_forum::{
    app::{ApplicationCommand, CommandEnvelope, CommandView, SkillSelector},
    domain::{Actor, CommandId, CorrelationId, ObjectVersion, SkillId, SkillVersionId},
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

mod fix_round_one_replay_matrix {
    use super::support;
    use ai_stock_forum::{
        agents::{AgentBindings, AgentProfileDraft, AgentRole},
        app::{
            AgentProfileSelector, ApplicationCommand, CommandEnvelope, CommandOutcome, CommandView,
            SkillSelector,
        },
        domain::{Actor, CommandId, CorrelationId, SkillId},
        skills::{SkillDraft, SkillResource},
    };
    use uuid::Uuid;

    fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
        CommandEnvelope {
            command_id: CommandId::from_uuid(Uuid::from_u128(8_000_000 + id)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(9_000_000 + id)),
            actor: Actor::Human,
            command,
        }
    }

    fn skill(name: &str, instructions: &str) -> SkillDraft {
        SkillDraft::new(
            name.to_owned(),
            "Receipt replay contract.".to_owned(),
            "Use for receipt replay verification.".to_owned(),
            vec!["receipt".to_owned()],
            instructions.to_owned(),
            vec![SkillResource {
                name: "Evidence".to_owned(),
                body: format!("{instructions} resource"),
            }],
        )
        .unwrap()
    }

    fn profile() -> AgentProfileDraft {
        AgentProfileDraft::new(
            "Receipt Analyst".to_owned(),
            "Receipt replay profile.".to_owned(),
            AgentRole::Custom,
            "receipt verification".to_owned(),
            vec!["receipt".to_owned()],
            "Precise.".to_owned(),
            "Keep exact identities.".to_owned(),
            AgentBindings::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    #[derive(Debug, PartialEq)]
    struct DurableSnapshot {
        skill_history: CommandOutcome,
        active_skill: CommandOutcome,
        profile_history: Option<CommandOutcome>,
        active_profile: Option<CommandOutcome>,
        active_profile_rows: Vec<(String, String, i64, String, String)>,
        mutation_event_payloads: Vec<Vec<String>>,
        mutation_receipts: Vec<String>,
        event_rows: i64,
        receipt_rows: i64,
        event_reference_rows: i64,
        profile_version_rows: i64,
    }

    fn snapshot(
        app: &mut support::TestApp,
        skill_id: SkillId,
        profile_id: Option<ai_stock_forum::domain::AgentProfileId>,
        mutation_ids: &[u128],
        read_id: u128,
    ) -> DurableSnapshot {
        let skill_history = app
            .execute(envelope(
                read_id,
                ApplicationCommand::ShowSkillHistory {
                    selector: SkillSelector::from(skill_id),
                },
            ))
            .unwrap();
        let active_skill = app
            .execute(envelope(
                read_id + 1,
                ApplicationCommand::ShowSkill {
                    selector: SkillSelector::from(skill_id),
                },
            ))
            .unwrap();
        let (profile_history, active_profile) = profile_id.map_or((None, None), |profile_id| {
            (
                Some(
                    app.execute(envelope(
                        read_id + 2,
                        ApplicationCommand::ShowAgentProfileHistory {
                            selector: AgentProfileSelector::from(profile_id),
                        },
                    ))
                    .unwrap(),
                ),
                Some(
                    app.execute(envelope(
                        read_id + 3,
                        ApplicationCommand::ShowAgentProfile {
                            selector: AgentProfileSelector::from(profile_id),
                        },
                    ))
                    .unwrap(),
                ),
            )
        });
        DurableSnapshot {
            skill_history,
            active_skill,
            profile_history,
            active_profile,
            active_profile_rows: app.active_profile_rows(),
            mutation_event_payloads: [
                "skill_created",
                "skill_version_activated",
                "agent_skill_assigned",
                "agent_skill_upgraded",
                "agent_skill_unassigned",
            ]
            .into_iter()
            .map(|kind| app.event_payloads(kind))
            .collect(),
            mutation_receipts: mutation_ids
                .iter()
                .map(|id| {
                    format!(
                        "{:?}",
                        app.receipt_row(CommandId::from_uuid(Uuid::from_u128(8_000_000 + id)))
                    )
                })
                .collect(),
            event_rows: app.count_rows("event_stream"),
            receipt_rows: app.count_rows("command_receipts"),
            event_reference_rows: app.count_rows("command_event_refs"),
            profile_version_rows: app.count_rows("agent_profile_versions"),
        }
    }

    fn replay_and_assert_unchanged(
        app: &mut support::TestApp,
        command: &CommandEnvelope,
        original: &CommandOutcome,
        skill_id: SkillId,
        profile_id: Option<ai_stock_forum::domain::AgentProfileId>,
        mutation_ids: &[u128],
        read_id: u128,
    ) {
        let before = snapshot(app, skill_id, profile_id, mutation_ids, read_id);
        let replay = app.execute(command.clone()).unwrap();
        assert_eq!(&replay, original);
        let after = snapshot(app, skill_id, profile_id, mutation_ids, read_id);
        assert_eq!(after, before);
    }

    #[test]
    fn every_skill_mutation_replays_exact_outcome_without_any_durable_change() {
        let mut app = support::app();

        let create_candidate = skill("Replay Skill", "version one");
        let create_preview = app
            .preview_skill_creation(create_candidate.clone())
            .unwrap();
        let create = envelope(
            100,
            ApplicationCommand::CreateSkill {
                skill_id: create_preview.skill_id,
                candidate: create_candidate,
                review_token: create_preview.review_token,
                review_digest: create_preview.review_digest,
            },
        );
        let created = app.execute(create.clone()).unwrap();
        let CommandView::SkillCreated(created_view) = &created.view else {
            panic!("skill created")
        };
        let skill_id = created_view.skill_id;
        replay_and_assert_unchanged(&mut app, &create, &created, skill_id, None, &[100], 1_100);

        let version_candidate = skill("Replay Skill", "version two");
        let version_preview = app
            .preview_skill_version(
                skill_id,
                created_view.skill_version_id,
                version_candidate.clone(),
            )
            .unwrap();
        let version = envelope(
            101,
            ApplicationCommand::ActivateSkillVersion {
                skill_id,
                expected_active_version_id: created_view.skill_version_id,
                candidate: version_candidate,
                review_token: version_preview.review_token,
                review_digest: version_preview.review_digest,
            },
        );
        let versioned = app.execute(version.clone()).unwrap();
        let CommandView::SkillVersionActivated(versioned_view) = &versioned.view else {
            panic!("skill versioned")
        };
        replay_and_assert_unchanged(
            &mut app,
            &version,
            &versioned,
            skill_id,
            None,
            &[100, 101],
            1_200,
        );

        let historical = app
            .execute(envelope(
                102,
                ApplicationCommand::ShowSkillVersion {
                    selector: SkillSelector::from(skill_id),
                    version: created_view.version,
                },
            ))
            .unwrap();
        let CommandView::SkillVersion(historical) = historical.view else {
            panic!("historical skill")
        };
        let active = app
            .execute(envelope(
                103,
                ApplicationCommand::ShowSkill {
                    selector: SkillSelector::from(skill_id),
                },
            ))
            .unwrap();
        let CommandView::Skill(active) = active.view else {
            panic!("active skill")
        };
        assert_eq!(
            active.skill_ref.skill_version_id(),
            versioned_view.skill_version_id
        );

        let profile = app
            .execute(envelope(
                104,
                ApplicationCommand::CreateAgentProfile {
                    draft: profile(),
                    template_provenance: None,
                },
            ))
            .unwrap();
        let CommandView::AgentProfileCreated(profile) = profile.view else {
            panic!("profile created")
        };

        let assign_preview = app
            .preview_agent_skill_assignment(
                profile.profile_id,
                profile.profile_version_id,
                historical.skill_ref.clone(),
            )
            .unwrap();
        let assign = envelope(
            105,
            ApplicationCommand::AssignAgentSkill {
                profile_id: profile.profile_id,
                expected_active_profile_version_id: profile.profile_version_id,
                skill: historical.skill_ref.clone(),
                review_token: assign_preview.review_token,
                review_digest: assign_preview.review_digest,
            },
        );
        let assigned = app.execute(assign.clone()).unwrap();
        let CommandView::AgentSkillAssigned(assigned_view) = &assigned.view else {
            panic!("skill assigned")
        };
        replay_and_assert_unchanged(
            &mut app,
            &assign,
            &assigned,
            skill_id,
            Some(profile.profile_id),
            &[100, 101, 105],
            1_300,
        );

        let upgrade_preview = app
            .preview_agent_skill_upgrade(
                profile.profile_id,
                assigned_view.profile_version_id,
                historical.skill_ref.clone(),
                active.skill_ref.clone(),
            )
            .unwrap();
        let upgrade = envelope(
            106,
            ApplicationCommand::UpgradeAgentSkill {
                profile_id: profile.profile_id,
                expected_active_profile_version_id: assigned_view.profile_version_id,
                expected: historical.skill_ref.clone(),
                replacement: active.skill_ref.clone(),
                review_token: upgrade_preview.review_token,
                review_digest: upgrade_preview.review_digest,
            },
        );
        let upgraded = app.execute(upgrade.clone()).unwrap();
        let CommandView::AgentSkillUpgraded(upgraded_view) = &upgraded.view else {
            panic!("skill upgraded")
        };
        replay_and_assert_unchanged(
            &mut app,
            &upgrade,
            &upgraded,
            skill_id,
            Some(profile.profile_id),
            &[100, 101, 105, 106],
            1_400,
        );

        let unassign_preview = app
            .preview_agent_skill_unassignment(
                profile.profile_id,
                upgraded_view.profile_version_id,
                active.skill_ref.clone(),
            )
            .unwrap();
        let unassign = envelope(
            107,
            ApplicationCommand::UnassignAgentSkill {
                profile_id: profile.profile_id,
                expected_active_profile_version_id: upgraded_view.profile_version_id,
                expected: active.skill_ref,
                review_token: unassign_preview.review_token,
                review_digest: unassign_preview.review_digest,
            },
        );
        let unassigned = app.execute(unassign.clone()).unwrap();
        replay_and_assert_unchanged(
            &mut app,
            &unassign,
            &unassigned,
            skill_id,
            Some(profile.profile_id),
            &[100, 101, 105, 106, 107],
            1_500,
        );
    }
}

fn draft(instructions: &str) -> SkillDraft {
    named_draft("Receipt Skill", instructions)
}

fn named_draft(name: &str, instructions: &str) -> SkillDraft {
    SkillDraft::new(
        name.to_owned(),
        "Receipt behavior.".to_owned(),
        "Use for receipt tests.".to_owned(),
        Vec::new(),
        instructions.to_owned(),
        Vec::new(),
    )
    .unwrap()
}

fn create_receipt_skill(
    app: &mut support::TestApp,
    command_id: u128,
) -> (SkillId, SkillVersionId, ObjectVersion) {
    let candidate = draft("Version one.");
    let preview = app.preview_skill_creation(candidate.clone()).unwrap();
    let created = app
        .execute(envelope(
            command_id,
            ApplicationCommand::CreateSkill {
                skill_id: preview.skill_id,
                candidate,
                review_token: preview.review_token,
                review_digest: preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::SkillCreated(created) = created.view else {
        panic!("skill created")
    };
    (created.skill_id, created.skill_version_id, created.version)
}

fn activate_renamed_receipt_skill(
    app: &mut support::TestApp,
    command_id: u128,
    skill_id: SkillId,
    active_version_id: SkillVersionId,
) -> SkillVersionId {
    let candidate = named_draft("Renamed Receipt Skill", "Version two.");
    let preview = app
        .preview_skill_version(skill_id, active_version_id, candidate.clone())
        .unwrap();
    let activated = app
        .execute(envelope(
            command_id,
            ApplicationCommand::ActivateSkillVersion {
                skill_id,
                expected_active_version_id: active_version_id,
                candidate,
                review_token: preview.review_token,
                review_digest: preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::SkillVersionActivated(activated) = activated.view else {
        panic!("skill version activated")
    };
    activated.skill_version_id
}

#[test]
fn show_skill_receipt_replays_original_outcome_after_later_activation_and_rename() {
    let mut app = support::app();
    let (skill_id, version_one_id, _) = create_receipt_skill(&mut app, 34_000);
    let command = envelope(
        34_001,
        ApplicationCommand::ShowSkill {
            selector: SkillSelector::from(skill_id),
        },
    );
    let original = app.execute(command.clone()).unwrap();

    let version_two_id = activate_renamed_receipt_skill(&mut app, 34_002, skill_id, version_one_id);

    assert_eq!(app.execute(command).unwrap(), original);

    let current = app
        .execute(envelope(
            34_003,
            ApplicationCommand::ShowSkill {
                selector: SkillSelector::from(skill_id),
            },
        ))
        .unwrap();
    let CommandView::Skill(current) = current.view else {
        panic!("active skill")
    };
    assert_eq!(current.skill_ref.skill_version_id(), version_two_id);
    assert_eq!(current.content.display_name, "Renamed Receipt Skill");
}

#[test]
fn show_skill_history_receipt_replays_original_outcome_after_later_activation_and_rename() {
    let mut app = support::app();
    let (skill_id, version_one_id, _) = create_receipt_skill(&mut app, 35_000);
    let command = envelope(
        35_001,
        ApplicationCommand::ShowSkillHistory {
            selector: SkillSelector::from(skill_id),
        },
    );
    let original = app.execute(command.clone()).unwrap();

    activate_renamed_receipt_skill(&mut app, 35_002, skill_id, version_one_id);

    assert_eq!(app.execute(command).unwrap(), original);
}

#[test]
fn eventless_builtin_show_skill_history_receipt_replays_exact_outcome() {
    let mut app = support::app();
    let command = envelope(
        35_100,
        ApplicationCommand::ShowSkillHistory {
            selector: SkillSelector::Name("Evidence Review".to_owned()),
        },
    );
    let original = app.execute(command.clone()).unwrap();

    assert_eq!(app.execute(command).unwrap(), original);
}

#[test]
fn name_based_show_skill_version_receipt_replays_original_outcome_after_later_rename() {
    let mut app = support::app();
    let (skill_id, version_one_id, version_one) = create_receipt_skill(&mut app, 36_000);
    let command = envelope(
        36_001,
        ApplicationCommand::ShowSkillVersion {
            selector: SkillSelector::Name("Receipt Skill".to_owned()),
            version: version_one,
        },
    );
    let original = app.execute(command.clone()).unwrap();

    activate_renamed_receipt_skill(&mut app, 36_002, skill_id, version_one_id);

    assert_eq!(app.execute(command).unwrap(), original);
}

#[test]
fn create_receipt_replays_after_a_later_version_becomes_active() {
    let mut app = support::app();
    let candidate = draft("Version one.");
    let preview = app.preview_skill_creation(candidate.clone()).unwrap();
    let create = envelope(
        32_000,
        ApplicationCommand::CreateSkill {
            skill_id: preview.skill_id,
            candidate,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    );
    let created = app.execute(create.clone()).unwrap();
    let CommandView::SkillCreated(created_view) = &created.view else {
        panic!("skill created")
    };

    let candidate = draft("Version two.");
    let preview = app
        .preview_skill_version(
            created_view.skill_id,
            created_view.skill_version_id,
            candidate.clone(),
        )
        .unwrap();
    app.execute(envelope(
        32_001,
        ApplicationCommand::ActivateSkillVersion {
            skill_id: created_view.skill_id,
            expected_active_version_id: created_view.skill_version_id,
            candidate,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    ))
    .unwrap();

    let durable_rows = (
        app.count_rows("event_stream"),
        app.count_rows("command_receipts"),
        app.count_rows("command_event_refs"),
    );
    assert_eq!(app.execute(create).unwrap(), created);
    assert_eq!(
        (
            app.count_rows("event_stream"),
            app.count_rows("command_receipts"),
            app.count_rows("command_event_refs"),
        ),
        durable_rows
    );
}

#[test]
fn version_receipt_replays_after_a_newer_version_becomes_active() {
    let mut app = support::app();
    let candidate = draft("Version one.");
    let preview = app.preview_skill_creation(candidate.clone()).unwrap();
    let created = app
        .execute(envelope(
            33_000,
            ApplicationCommand::CreateSkill {
                skill_id: preview.skill_id,
                candidate,
                review_token: preview.review_token,
                review_digest: preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::SkillCreated(created_view) = &created.view else {
        panic!("skill created")
    };

    let candidate = draft("Version two.");
    let preview = app
        .preview_skill_version(
            created_view.skill_id,
            created_view.skill_version_id,
            candidate.clone(),
        )
        .unwrap();
    let version = envelope(
        33_001,
        ApplicationCommand::ActivateSkillVersion {
            skill_id: created_view.skill_id,
            expected_active_version_id: created_view.skill_version_id,
            candidate,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    );
    let versioned = app.execute(version.clone()).unwrap();
    let CommandView::SkillVersionActivated(versioned_view) = &versioned.view else {
        panic!("skill version activated")
    };

    let candidate = draft("Version three.");
    let preview = app
        .preview_skill_version(
            versioned_view.skill_id,
            versioned_view.skill_version_id,
            candidate.clone(),
        )
        .unwrap();
    app.execute(envelope(
        33_002,
        ApplicationCommand::ActivateSkillVersion {
            skill_id: versioned_view.skill_id,
            expected_active_version_id: versioned_view.skill_version_id,
            candidate,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    ))
    .unwrap();

    let durable_rows = (
        app.count_rows("event_stream"),
        app.count_rows("command_receipts"),
        app.count_rows("command_event_refs"),
    );
    assert_eq!(app.execute(version).unwrap(), versioned);
    assert_eq!(
        (
            app.count_rows("event_stream"),
            app.count_rows("command_receipts"),
            app.count_rows("command_event_refs"),
        ),
        durable_rows
    );
}

#[test]
fn skill_mutation_receipts_replay_the_original_typed_outcome_without_duplicate_writes() {
    let mut app = support::app();
    let preview = app.preview_skill_creation(draft("Version one.")).unwrap();
    let command = envelope(
        30_000,
        ApplicationCommand::CreateSkill {
            skill_id: preview.skill_id,
            candidate: draft("Version one."),
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    );
    let before_events = app.count_rows("event_stream");
    let before_receipts = app.count_rows("command_receipts");
    let first = app.execute(command.clone()).unwrap();
    let after_first_events = app.count_rows("event_stream");
    let after_first_receipts = app.count_rows("command_receipts");
    let replay = app.execute(command).unwrap();

    assert_eq!(replay, first);
    assert!(matches!(first.view, CommandView::SkillCreated(_)));
    assert_eq!(after_first_events, before_events + 1);
    assert_eq!(after_first_receipts, before_receipts + 1);
    assert_eq!(app.count_rows("event_stream"), after_first_events);
    assert_eq!(app.count_rows("command_receipts"), after_first_receipts);
}

#[test]
fn a_replayed_command_id_with_a_changed_skill_candidate_is_a_stable_conflict() {
    let mut app = support::app();
    let preview = app.preview_skill_creation(draft("Version one.")).unwrap();
    let command = envelope(
        31_000,
        ApplicationCommand::CreateSkill {
            skill_id: preview.skill_id,
            candidate: draft("Version one."),
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    );
    app.execute(command.clone()).unwrap();
    let mut changed = command;
    let ApplicationCommand::CreateSkill { candidate, .. } = &mut changed.command else {
        panic!("create skill")
    };
    *candidate = draft("Changed after commit.");

    let error = app.execute(changed).unwrap_err();
    assert_eq!(error.code(), "command_conflict");
}
