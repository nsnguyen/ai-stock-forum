use ai_stock_forum::{
    agents::{AgentProfileVersion, builtin_profile_templates},
    app::{
        AgentProfileSummary, AgentProfileView, AgentProfilesView, ApplicationCommand,
        CommandOutcome, CommandView, DatabaseReadiness, PresentationSnapshot,
        ProcessGuardOwnership, ShutdownDisposition, SkillSummary, SkillView, SkillsView,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, InstallationId,
        MemoryNamespaceId, SessionId, SkillId, SkillReviewToken, SkillVersionId, sha256,
    },
    setup::SetupStatus,
    skills::{SkillDraft, SkillEditPreview, SkillProvenance, SkillVersion},
    ui::tui::{
        AssignmentKind, ControllerEffect, TuiEvent, apply_outcome, handle_event,
        model::{AgentsPane, SkillConfirmation, SkillOperationOrigin, SkillsPane, TuiModel, View},
        render,
        theme::Theme,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use uuid::Uuid;

fn snapshot(profile: Option<AgentProfileView>) -> PresentationSnapshot {
    let profiles = profile.as_ref().map_or_else(Vec::new, |detail| {
        vec![AgentProfileSummary {
            profile_id: detail.profile.profile_id(),
            profile_version_id: detail.profile.profile_version_id(),
            version: detail.profile.version(),
            display_name: detail.profile.display_name().to_owned(),
            role: detail.profile.role(),
            primary_specialty: detail.profile.primary_specialty().to_owned(),
            readiness: detail.readiness,
            content_digest: detail.profile.content_digest().clone(),
        }]
    });
    PresentationSnapshot {
        installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
        session_id: SessionId::from_uuid(Uuid::from_u128(2)),
        database_readiness: DatabaseReadiness::Ready,
        process_guard_ownership: ProcessGuardOwnership::Held,
        setup_status: SetupStatus::NotStarted,
        recent_audit: Vec::new(),
        agent_profiles: AgentProfilesView {
            total_count: profiles.len() as u32,
            returned_count: profiles.len() as u32,
            profiles,
            truncated: false,
        },
        selected_agent_profile: profile,
        selected_agent_profile_history: None,
    }
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn navigation_key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn type_line(model: &mut TuiModel, value: &str) -> ControllerEffect {
    model.command.clear();
    model.command.ingest(value);
    handle_event(model, key(KeyCode::Enter))
}

fn outcome(view: CommandView) -> CommandOutcome {
    CommandOutcome {
        command_id: CommandId::from_uuid(Uuid::from_u128(700)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(701)),
        committed_events: Vec::new(),
        view,
        shutdown: ShutdownDisposition::Continue,
    }
}

fn skill(seed: u128, version: Option<&SkillVersion>) -> SkillVersion {
    let draft = SkillDraft::new(
        "Decision Journal".to_owned(),
        "Records the evidence behind an investment decision.".to_owned(),
        "Use before committing to a position.".to_owned(),
        vec!["decision".to_owned(), "evidence".to_owned()],
        if version.is_some() {
            "Record evidence, disconfirming facts, and the explicit exit condition.".to_owned()
        } else {
            "Record evidence and disconfirming facts.".to_owned()
        },
        Vec::new(),
    )
    .expect("skill draft");
    match version {
        None => SkillVersion::create(
            SkillId::from_uuid(Uuid::from_u128(seed)),
            SkillVersionId::from_uuid(Uuid::from_u128(seed + 1)),
            1_800_000_000_000,
            SkillProvenance::User,
            draft,
        )
        .expect("version one"),
        Some(previous) => SkillVersion::next_version(
            previous,
            SkillVersionId::from_uuid(Uuid::from_u128(seed + 2)),
            1_800_000_000_001,
            draft,
        )
        .expect("version two"),
    }
}

fn skill_view(skill: &SkillVersion) -> SkillView {
    SkillView {
        skill_ref: skill.reference(),
        content: skill.content().clone(),
        created_at_ms: skill.created_at_ms(),
        provenance: skill.provenance().clone(),
        predecessor_version_id: skill.predecessor(),
    }
}

fn library(skills: &[&SkillVersion]) -> SkillsView {
    SkillsView {
        skills: skills
            .iter()
            .map(|skill| SkillSummary {
                skill_ref: skill.reference(),
                display_name: skill.content().display_name.clone(),
                provenance: skill.provenance().clone(),
            })
            .collect(),
        total_count: skills.len() as u32,
        returned_count: skills.len() as u32,
        truncated: false,
    }
}

fn profile(seed: u128, skills: Vec<ai_stock_forum::skills::SkillVersionRef>) -> AgentProfileView {
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().expect("profile draft");
    draft.display_name = "Restarted Research Agent".to_owned();
    draft.skill_refs = skills;
    let profile = AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(seed)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(seed + 2)),
        1_800_000_000_010,
        draft,
        Some(template.provenance()),
    )
    .expect("agent profile");
    AgentProfileView {
        readiness: profile.readiness(),
        profile,
    }
}

fn render_text(model: &TuiModel, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .expect("render workflow state");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn keyboard_workflow_creates_versions_pins_upgrades_unassigns_and_restores_exact_refs() {
    let first = skill(100, None);
    let second = skill(100, Some(&first));
    let empty_agent = profile(200, Vec::new());
    let mut model = TuiModel::new(snapshot(Some(empty_agent.clone())), false);

    assert_eq!(
        handle_event(&mut model, navigation_key(KeyCode::Char('s'))),
        ControllerEffect::LoadSkills
    );
    apply_outcome(&mut model, outcome(CommandView::Skills(library(&[&first]))));
    assert!(render_text(&model, 80, 24).contains("Skill library"));

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('c'))),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    for value in [
        "Decision Journal",
        "Records the evidence behind an investment decision.",
        "Use before committing to a position.",
        "decision, evidence",
        "Record evidence and disconfirming facts.",
        "",
    ] {
        assert_eq!(type_line(&mut model, value), ControllerEffect::Redraw);
    }
    let ControllerEffect::RequestSkillPreview(request) =
        handle_event(&mut model, key(KeyCode::Enter))
    else {
        panic!("create review request")
    };
    assert!(model.skills.editor.as_mut().expect("editor").apply_preview(
        request.generation(),
        SkillEditPreview {
            skill_id: first.skill_id(),
            expected_active_version_id: None,
            candidate_digest: first.content_digest().clone(),
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(300)),
            review_digest: sha256(b"create-review"),
        },
    ));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::ExecuteSkill(ApplicationCommand::CreateSkill { .. })
    ));

    model.skills.pending_confirmation = None;
    model.skills.editor = None;
    model.skills.replace_skills(library(&[&first]));
    model.skills.replace_detail(skill_view(&first));
    model.skills.selected_action_index = 1;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    for value in [
        "Decision Journal",
        "Records the evidence behind an investment decision.",
        "Use before committing to a position.",
        "decision, evidence",
        "Record evidence, disconfirming facts, and the explicit exit condition.",
        "",
    ] {
        assert_eq!(type_line(&mut model, value), ControllerEffect::Redraw);
    }
    let ControllerEffect::RequestSkillPreview(request) =
        handle_event(&mut model, key(KeyCode::Enter))
    else {
        panic!("version review request")
    };
    assert!(
        model
            .skills
            .editor
            .as_mut()
            .expect("version editor")
            .apply_preview(
                request.generation(),
                SkillEditPreview {
                    skill_id: second.skill_id(),
                    expected_active_version_id: Some(first.skill_version_id()),
                    candidate_digest: second.content_digest().clone(),
                    review_token: SkillReviewToken::from_uuid(Uuid::from_u128(301)),
                    review_digest: sha256(b"version-review"),
                },
            )
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::ExecuteSkill(ApplicationCommand::ActivateSkillVersion { .. })
    ));

    model.skills.pending_confirmation = None;
    model.skills.editor = None;
    model.skills.replace_skills(library(&[&second]));
    model.skills.replace_version_detail(skill_view(&first));
    model.skills.pane = SkillsPane::Detail;
    model.skills.selected_action_index = 0;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillAgents
    );
    model.agents.profiles = snapshot(Some(empty_agent.clone())).agent_profiles;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillAgent {
            profile_id: empty_agent.profile.profile_id(),
        }
    );
    apply_outcome(
        &mut model,
        outcome(CommandView::AgentProfile(empty_agent.clone())),
    );
    let ControllerEffect::RequestSkillAssignmentPreview {
        target, assignment, ..
    } = handle_event(&mut model, key(KeyCode::Enter))
    else {
        panic!("pin review")
    };
    assert_eq!(target, first.reference());
    assert_eq!(assignment, AssignmentKind::Add);

    let pinned_agent = profile(210, vec![first.reference()]);
    model.skills.replace_detail(skill_view(&second));
    model.skills.selected_agent_detail = Some(pinned_agent.clone());
    model.skills.assignment = Some(AssignmentKind::classify(
        &second.reference(),
        pinned_agent.profile.skill_refs().first(),
    ));
    model.skills.pane = SkillsPane::AssignmentReview;
    let ControllerEffect::RequestSkillAssignmentPreview {
        target, assignment, ..
    } = handle_event(&mut model, key(KeyCode::Enter))
    else {
        panic!("upgrade review")
    };
    assert_eq!(target, second.reference());
    assert_eq!(
        assignment,
        AssignmentKind::Upgrade {
            expected: first.reference(),
        }
    );

    let upgraded_agent = profile(220, vec![second.reference()]);
    model.skills.active = false;
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail = Some(upgraded_agent.clone());
    model.agents.skill_panel_open = true;
    model.agents.selected_skill_action_index = 2;
    let ControllerEffect::RequestSkillAssignmentPreview {
        target, assignment, ..
    } = handle_event(&mut model, key(KeyCode::Enter))
    else {
        panic!("unassign review")
    };
    assert_eq!(target, second.reference());
    assert_eq!(
        assignment,
        AssignmentKind::Unassign {
            expected: second.reference(),
        }
    );

    let unassign_command = ApplicationCommand::UnassignAgentSkill {
        profile_id: upgraded_agent.profile.profile_id(),
        expected_active_profile_version_id: upgraded_agent.profile.profile_version_id(),
        expected: second.reference(),
        review_token: SkillReviewToken::from_uuid(Uuid::from_u128(302)),
        review_digest: sha256(b"unassign-review"),
    };
    model.skills.pending_confirmation = Some(SkillConfirmation {
        command: unassign_command,
        origin: SkillOperationOrigin::AgentSkills {
            profile_id: upgraded_agent.profile.profile_id(),
        },
    });
    model.skills.active = true;
    model.skills.pane = SkillsPane::Confirmation;
    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::ExecuteSkill(ApplicationCommand::UnassignAgentSkill { .. })
    ));

    let mut restarted = TuiModel::new(snapshot(Some(upgraded_agent)), false);
    restarted.active_view = View::Agents;
    restarted.agents.pane = AgentsPane::Detail;
    restarted.agents.skill_panel_open = true;
    restarted.skills.library = library(&[&second]);
    let restarted_text = render_text(&restarted, 120, 40);
    assert!(restarted_text.contains("PINNED EXACT VERSION"));
    assert!(restarted_text.contains("v2"));
    assert!(restarted_text.contains("No automatic upgrades"));
}

fn agent_action_model(
    pins: Vec<ai_stock_forum::skills::SkillVersionRef>,
    active: Vec<&SkillVersion>,
    truncated: bool,
) -> TuiModel {
    let detail = profile(1_000 + pins.len() as u128 * 10, pins);
    let mut model = TuiModel::new(snapshot(Some(detail.clone())), false);
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail = Some(detail);
    model.agents.skill_panel_open = true;
    model.skills.library = library(&active);
    model.skills.library.truncated = truncated;
    model
}

fn divergent_equal_version(previous: &SkillVersion, seed: u128) -> SkillVersion {
    let mut draft = previous.content().clone();
    draft.instructions = "Divergent equal-version content.".to_owned();
    SkillVersion::create(
        previous.skill_id(),
        SkillVersionId::from_uuid(Uuid::from_u128(seed)),
        1_800_000_000_900,
        SkillProvenance::User,
        draft,
    )
    .expect("divergent equal object version")
}

fn availability_text(model: &TuiModel) -> String {
    render_text(model, 120, 44)
}

#[test]
fn upgrade_availability_derivation_rejects_unknown_and_inconsistent_library_states() {
    let pinned_v1 = skill(400, None);
    let active_v2 = skill(400, Some(&pinned_v1));
    let unrelated = skill(500, None);
    let divergent = divergent_equal_version(&pinned_v1, 499);

    let cases = [
        (
            "absent",
            agent_action_model(vec![pinned_v1.reference()], Vec::new(), false),
            "UNKNOWN",
        ),
        (
            "truncated missing match",
            agent_action_model(vec![pinned_v1.reference()], vec![&unrelated], true),
            "UNKNOWN",
        ),
        (
            "unrelated",
            agent_action_model(vec![pinned_v1.reference()], vec![&unrelated], false),
            "UNKNOWN",
        ),
        (
            "exact current",
            agent_action_model(vec![pinned_v1.reference()], vec![&pinned_v1], false),
            "CURRENT",
        ),
        (
            "higher same skill",
            agent_action_model(vec![pinned_v1.reference()], vec![&active_v2], false),
            "AVAILABLE",
        ),
        (
            "lower same skill",
            agent_action_model(vec![active_v2.reference()], vec![&pinned_v1], false),
            "INCONSISTENT",
        ),
        (
            "equal divergent ref",
            agent_action_model(vec![pinned_v1.reference()], vec![&divergent], false),
            "INCONSISTENT",
        ),
        (
            "truncated exact match",
            agent_action_model(vec![pinned_v1.reference()], vec![&active_v2], true),
            "AVAILABLE",
        ),
    ];

    for (case, model, expected) in cases {
        let text = availability_text(&model);
        assert!(text.contains(expected), "case={case}, expected={expected}");
        if expected != "AVAILABLE" {
            assert!(!text.contains("[Upgrade]"), "case={case}");
        }
    }

    let mut inconsistent = agent_action_model(vec![pinned_v1.reference()], vec![&divergent], false);
    inconsistent.skills.library_loaded = true;
    let text = availability_text(&inconsistent);
    assert!(text.contains("INCONSISTENT"));
    assert!(text.contains("r: reload"));
    assert!(text.contains("Upgrade is hidden"));
    assert!(!text.contains("press s"));

    let before = inconsistent.clone();
    assert_eq!(
        handle_event(
            &mut inconsistent,
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)),
        ),
        ControllerEffect::None
    );
    assert_eq!(inconsistent, before);
    assert_eq!(
        handle_event(&mut inconsistent, key(KeyCode::Char('r'))),
        ControllerEffect::LoadAgentSkillLibrary
    );
    assert_eq!(inconsistent.active_view, View::Agents);
    assert_eq!(inconsistent.agents.pane, AgentsPane::Detail);
    assert!(inconsistent.agents.skill_panel_open);
    assert_eq!(inconsistent.agents.selected_assigned_skill, 0);
}

#[test]
fn unknown_and_current_agent_actions_traverse_and_dispatch_only_view_or_unassign() {
    let pinned = skill(600, None);
    for (case, active) in [("unknown", Vec::new()), ("current", vec![&pinned])] {
        let mut model = agent_action_model(vec![pinned.reference()], active, false);

        assert_eq!(
            model.agents.selected_skill_action(),
            ai_stock_forum::ui::tui::AgentSkillAction::View
        );
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Enter)),
            ControllerEffect::LoadSkillVersion {
                skill_id: pinned.skill_id(),
                version: pinned.version(),
            },
            "case={case}"
        );

        model.skills.active = false;
        model.active_view = View::Agents;
        model.agents.skill_panel_open = true;
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Right)),
            ControllerEffect::Redraw
        );
        assert_eq!(
            model.agents.selected_skill_action(),
            ai_stock_forum::ui::tui::AgentSkillAction::Unassign,
            "case={case}"
        );
        assert!(matches!(
            handle_event(&mut model, key(KeyCode::Enter)),
            ControllerEffect::RequestSkillAssignmentPreview {
                target,
                assignment: AssignmentKind::Unassign { expected },
                ..
            } if target == pinned.reference() && expected == pinned.reference()
        ));
    }
}

#[test]
fn available_agent_actions_dispatch_the_exact_derived_replacement_ref() {
    let pinned = skill(700, None);
    let active = skill(700, Some(&pinned));
    let mut model = agent_action_model(vec![pinned.reference()], vec![&active], false);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Right)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.selected_skill_action(),
        ai_stock_forum::ui::tui::AgentSkillAction::Upgrade
    );
    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::RequestSkillAssignmentPreview {
            target,
            assignment: AssignmentKind::Upgrade { expected },
            ..
        } if target == active.reference() && expected == pinned.reference()
    ));
}

#[test]
fn changing_assigned_skill_rows_recomputes_availability_and_normalizes_upgrade() {
    let first_v1 = skill(800, None);
    let first_v2 = skill(800, Some(&first_v1));
    let second = skill(900, None);
    let mut model = agent_action_model(
        vec![first_v1.reference(), second.reference()],
        vec![&first_v2, &second],
        false,
    );

    handle_event(&mut model, key(KeyCode::Right));
    assert_eq!(
        model.agents.selected_skill_action(),
        ai_stock_forum::ui::tui::AgentSkillAction::Upgrade
    );
    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(model.agents.selected_assigned_skill, 1);
    assert_eq!(
        model.agents.selected_skill_action(),
        ai_stock_forum::ui::tui::AgentSkillAction::View
    );
}
