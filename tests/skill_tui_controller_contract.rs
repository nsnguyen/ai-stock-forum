use ai_stock_forum::{
    app::{
        AgentProfileSummary, AgentProfileView, AgentProfilesView, ApplicationCommand,
        CommandOutcome, CommandView, DatabaseReadiness, PresentationSnapshot,
        ProcessGuardOwnership, ShutdownDisposition, SkillHistoryEntry, SkillHistoryView,
        SkillSummary, SkillView, SkillsView,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, InstallationId,
        MemoryNamespaceId, ObjectVersion, SessionId, SkillId, SkillReviewToken, SkillVersionId,
        sha256,
    },
    setup::SetupStatus,
    skills::{SkillDraft, SkillProvenance, SkillVersion},
    ui::tui::{
        ControllerEffect, TuiEvent, apply_outcome, handle_event,
        model::{
            AgentSkillAction, AgentsPane, AssignmentKind, Focus, SkillConfirmation,
            SkillDetailAction, SkillOperationOrigin, SkillWorkspaceOrigin, SkillsPane, TuiModel,
            View,
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

fn model() -> TuiModel {
    TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: Vec::new(),
            agent_profiles: AgentProfilesView {
                profiles: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            selected_agent_profile: None,
            selected_agent_profile_history: None,
        },
        false,
    )
}

#[test]
fn bare_four_switch_from_agent_panel_restores_the_agents_tab() {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.skill_panel_open = true;
    model.agents.detail = Some(profile_with_skills(10, Vec::new()));

    assert_eq!(
        handle_event(&mut model, navigation_key('4')),
        ControllerEffect::LoadSkills
    );
    assert_eq!(
        model.skills.workspace_origin,
        Some(ai_stock_forum::ui::tui::SkillWorkspaceOrigin::Cockpit(
            View::Agents
        ))
    );

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert!(!model.skills.active);
    assert_eq!(model.skills.workspace_origin, None);
    assert_eq!(model.active_view, View::Agents);
    assert!(model.agents.skill_panel_open);
}

#[test]
fn returning_from_an_agent_skill_keeps_the_agent_list_viewport() {
    let assigned = skill(10, "Assigned");
    let mut model = model();
    model.select_view(View::Agents);
    model.set_terminal_size(60, 18);
    model.agents.profiles = AgentProfilesView {
        profiles: vec![
            agent(100, "First"),
            agent(200, "Second"),
            agent(300, "Third"),
        ],
        total_count: 3,
        returned_count: 3,
        truncated: false,
    };
    model.agents.selected_profile = 2;
    model.agents.list_scroll = 1;
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail = Some(profile_with_skills(300, vec![assigned.reference()]));
    model.agents.skill_panel_open = true;

    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillVersion { .. }
    ));
    assert!(model.skills.active);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(
        (model.agents.selected_profile, model.agents.list_scroll),
        (2, 1)
    );
}

#[test]
fn hidden_skill_editor_blocks_agent_skill_actions_without_losing_its_draft() {
    let assigned = skill(20, "Protected editor skill");
    let mut model = model();
    model.skills.active = true;
    model.skills.library_loaded = true;
    model.skills.start_create(None);
    model.command.ingest("unfinished protected draft");
    model.select_view(View::Agents);
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail = Some(profile_with_skills(30, vec![assigned.reference()]));
    model.agents.skill_panel_open = true;
    model.agents.selected_skill_action_index = 0;
    let protected_skills = model.skills.clone();

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Agents);
    assert!(!model.skills.active);
    assert_eq!(model.skills, protected_skills);
    assert_eq!(
        model.message.as_ref().map(|message| message.text.as_str()),
        Some("Press 4 outside text input to finish the protected Skills workflow.")
    );

    assert_eq!(
        handle_event(&mut model, navigation_key('4')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.skills.pane, SkillsPane::Editor);
    assert_eq!(model.skills.editor, protected_skills.editor);
    assert_eq!(model.command.text(), "unfinished protected draft");
}

#[test]
fn hidden_skill_confirmation_blocks_agent_mutations_without_replacing_its_review() {
    let assigned = skill(40, "Protected confirmation skill");
    let mut model = model();
    model.skills.active = true;
    model.skills.library_loaded = true;
    model.skills.pane = SkillsPane::Confirmation;
    model.skills.pending_confirmation = Some(SkillConfirmation {
        command: ApplicationCommand::ShowHelp,
        origin: SkillOperationOrigin::Skills(SkillsPane::Detail),
    });
    model.skills.review_registered = true;

    assert_eq!(
        handle_event(&mut model, navigation_key('3')),
        ControllerEffect::Redraw
    );
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail = Some(profile_with_skills(50, vec![assigned.reference()]));
    model.agents.skill_panel_open = true;
    model.agents.selected_skill_action_index = 2;
    let protected_skills = model.skills.clone();

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Agents);
    assert!(!model.skills.active);
    assert_eq!(model.skills, protected_skills);
    assert_eq!(
        model.message.as_ref().map(|message| message.text.as_str()),
        Some("Press 4 outside text input to finish the protected Skills workflow.")
    );
}

#[test]
fn conflicting_slash_workflows_keep_protected_tabs_and_command_drafts_intact() {
    let mut skill_model = model();
    skill_model.skills.active = true;
    skill_model.skills.library_loaded = true;
    skill_model.skills.start_create(None);
    skill_model.select_view(View::Overview);
    let protected_skills = skill_model.skills.clone();

    assert_eq!(
        submit_command(&mut skill_model, "/skill add"),
        ControllerEffect::Redraw
    );
    assert_eq!(skill_model.command.text(), "/skill add");
    assert!(!skill_model.command_in_flight);
    assert_eq!(skill_model.skills, protected_skills);

    let mut agent_model = model();
    agent_model.skills.library_loaded = true;
    agent_model.select_view(View::Agents);
    assert!(
        agent_model
            .agents
            .start_profile_create(0, ai_stock_forum::agents::builtin_profile_templates(),)
    );
    agent_model.select_view(View::Overview);
    let protected_agents = agent_model.agents.clone();

    assert_eq!(
        submit_command(&mut agent_model, "/agent create"),
        ControllerEffect::Redraw
    );
    assert_eq!(agent_model.command.text(), "/agent create");
    assert!(!agent_model.command_in_flight);
    assert_eq!(agent_model.agents, protected_agents);
}

#[test]
fn delayed_read_results_hydrate_without_displacing_protected_workflows() {
    let listed_skill = skill(60, "Hydrated skill");
    let mut skill_model = model();
    skill_model.skills.active = true;
    skill_model.skills.library_loaded = true;
    skill_model.skills.pane = SkillsPane::Confirmation;
    skill_model.skills.pending_confirmation = Some(SkillConfirmation {
        command: ApplicationCommand::ShowHelp,
        origin: SkillOperationOrigin::Skills(SkillsPane::Detail),
    });
    skill_model.skills.review_registered = true;
    let protected_confirmation = skill_model.skills.pending_confirmation.clone();

    assert_eq!(
        handle_event(&mut skill_model, navigation_key('1')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        submit_command(&mut skill_model, "/skills"),
        ControllerEffect::Submit(ApplicationCommand::ListSkills)
    );
    assert_eq!(
        handle_event(&mut skill_model, key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut skill_model, navigation_key('4')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        apply_outcome(
            &mut skill_model,
            command_outcome(CommandView::Skills(SkillsView {
                skills: vec![summary(&listed_skill)],
                total_count: 1,
                returned_count: 1,
                truncated: false,
            })),
        ),
        ControllerEffect::Redraw
    );
    assert!(skill_model.skills.active);
    assert_eq!(skill_model.skills.pane, SkillsPane::Confirmation);
    assert_eq!(
        skill_model.skills.pending_confirmation,
        protected_confirmation
    );
    assert!(skill_model.skills.review_registered);
    assert_eq!(skill_model.skills.library.skills.len(), 1);

    let mut agent_model = model();
    agent_model.skills.library_loaded = true;
    agent_model.select_view(View::Agents);
    assert!(
        agent_model
            .agents
            .start_profile_create(0, ai_stock_forum::agents::builtin_profile_templates(),)
    );
    let protected_editor = agent_model.agents.editor.clone();
    agent_model.select_view(View::Overview);
    assert_eq!(
        submit_command(&mut agent_model, "/agent list"),
        ControllerEffect::Submit(ApplicationCommand::ListAgentProfiles)
    );
    assert_eq!(
        handle_event(&mut agent_model, key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut agent_model, navigation_key('3')),
        ControllerEffect::Redraw
    );
    let listed_agent = agent(70, "Hydrated agent");
    assert_eq!(
        apply_outcome(
            &mut agent_model,
            command_outcome(CommandView::AgentProfiles(AgentProfilesView {
                profiles: vec![listed_agent.clone()],
                total_count: 1,
                returned_count: 1,
                truncated: false,
            })),
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(agent_model.active_view, View::Agents);
    assert_eq!(agent_model.agents.pane, AgentsPane::Editor);
    assert_eq!(agent_model.agents.editor, protected_editor);
    assert_eq!(agent_model.agents.profiles.profiles, vec![listed_agent]);
}

#[test]
fn delayed_agent_detail_does_not_replace_a_newer_selection_or_context_after_returning() {
    let agent_a = agent(72, "Agent A");
    let agent_b = agent(76, "Agent B");
    let detail_a = profile_with_skills(72, Vec::new());
    let detail_b = profile_with_skills(76, Vec::new());
    let mut model = model();
    model.skills.library_loaded = true;
    model.select_view(View::Agents);
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![agent_a.clone(), agent_b.clone()],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    model.agents.selected_profile = 1;
    model.agents.list_scroll = 1;
    model.agents.detail = Some(detail_b);
    model.agents.pane = AgentsPane::List;

    assert!(matches!(
        submit_command(&mut model, &format!("/agent show {}", agent_a.profile_id)),
        ControllerEffect::Submit(ApplicationCommand::ShowAgentProfile { .. })
    ));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, navigation_key('9')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, navigation_key('3')),
        ControllerEffect::Redraw
    );
    model.set_focus(Focus::Workspace);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Up)),
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Detail
        }
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Detail
        }
    );
    model.agents.detail_scroll = 4;
    let expected_agents = model.agents.clone();

    assert_eq!(
        apply_outcome(
            &mut model,
            command_outcome(CommandView::AgentProfile(detail_a))
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(model.agents, expected_agents);
}

#[test]
fn delayed_skill_detail_does_not_replace_a_newer_selection_or_context_after_returning() {
    let skill_a = skill(82, "Skill A");
    let skill_b = skill(86, "Skill B");
    let detail_a = skill_view(&skill_a);
    let detail_b = skill_view(&skill_b);
    let mut model = model();
    model.skills.active = true;
    model.skills.library_loaded = true;
    model.skills.library = SkillsView {
        skills: vec![summary(&skill_a), summary(&skill_b)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    };
    model.skills.selected_skill = 1;
    model.skills.detail = Some(detail_b);
    model.skills.pane = SkillsPane::List;

    assert!(matches!(
        submit_command(&mut model, &format!("/skill show {}", skill_a.skill_id())),
        ControllerEffect::Submit(ApplicationCommand::ShowSkill { .. })
    ));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, navigation_key('9')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, navigation_key('4')),
        ControllerEffect::Redraw
    );
    model.set_focus(Focus::Workspace);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Up)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    let expected_skills = model.skills.clone();

    assert_eq!(
        apply_outcome(&mut model, command_outcome(CommandView::Skill(detail_a))),
        ControllerEffect::Redraw
    );
    assert!(model.skills.active);
    assert_eq!(model.skills, expected_skills);
}

#[test]
fn skill_read_outcome_replaces_a_stale_cockpit_escape_origin() {
    let mut model = model();
    model.skills.library_loaded = true;

    assert_eq!(
        handle_event(&mut model, navigation_key('4')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.skills.workspace_origin,
        Some(SkillWorkspaceOrigin::Cockpit(View::Overview))
    );
    assert_eq!(
        handle_event(&mut model, navigation_key('7')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        submit_command(&mut model, "/skills"),
        ControllerEffect::Submit(ApplicationCommand::ListSkills)
    );
    apply_outcome(
        &mut model,
        command_outcome(CommandView::Skills(SkillsView {
            skills: Vec::new(),
            total_count: 0,
            returned_count: 0,
            truncated: false,
        })),
    );

    assert!(model.skills.active);
    assert_eq!(
        model.skills.workspace_origin,
        Some(SkillWorkspaceOrigin::Cockpit(View::Setup))
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Setup);
}

#[test]
fn cancelled_agent_skill_origin_cannot_capture_a_later_cockpit_open() {
    let profile_id = AgentProfileId::from_uuid(Uuid::from_u128(80));
    let mut model = model();
    model.skills.active = true;
    model.skills.library_loaded = true;
    model.skills.workspace_origin = Some(SkillWorkspaceOrigin::AgentSkills { profile_id });
    model.skills.operation_origin = SkillOperationOrigin::AgentSkills { profile_id };
    model.skills.pane = SkillsPane::Confirmation;
    model.skills.pending_confirmation = Some(SkillConfirmation {
        command: ApplicationCommand::ShowHelp,
        origin: SkillOperationOrigin::AgentSkills { profile_id },
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::CancelSkillReview
    );
    assert_eq!(model.active_view, View::Agents);
    assert!(!model.skills.active);
    assert_eq!(model.skills.workspace_origin, None);

    assert_eq!(
        handle_event(&mut model, navigation_key('7')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, navigation_key('4')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.skills.workspace_origin,
        Some(SkillWorkspaceOrigin::Cockpit(View::Setup))
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Setup);
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn navigation_key(character: char) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
}

fn submit_command(model: &mut TuiModel, command: &str) -> ControllerEffect {
    model.command.clear();
    model.command.ingest(command);
    model.set_focus(Focus::Command);
    handle_event(model, key(KeyCode::Enter))
}

#[test]
fn direct_agent_commands_from_active_skills_follow_the_requested_destination() {
    let mut model = model();
    model.skills.active = true;
    model.skills.pane = SkillsPane::Detail;

    assert!(matches!(
        submit_command(&mut model, "/agent list"),
        ControllerEffect::Submit(ApplicationCommand::ListAgentProfiles)
    ));
    apply_outcome(
        &mut model,
        command_outcome(CommandView::AgentProfiles(AgentProfilesView {
            profiles: vec![agent(900, "Direct Agent")],
            total_count: 1,
            returned_count: 1,
            truncated: false,
        })),
    );
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(model.agents.pane, AgentsPane::List);

    model.skills.active = true;
    model.skills.pane = SkillsPane::Detail;
    let direct_profile_id = AgentProfileId::from_uuid(Uuid::from_u128(900));
    assert!(matches!(
        submit_command(&mut model, &format!("/agent show {direct_profile_id}")),
        ControllerEffect::Submit(ApplicationCommand::ShowAgentProfile { .. })
    ));
    let detail = profile_with_skills(900, Vec::new());
    apply_outcome(
        &mut model,
        command_outcome(CommandView::AgentProfile(detail.clone())),
    );
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(model.agents.pane, AgentsPane::Detail);
    assert_eq!(model.agents.detail, Some(detail));
}

#[test]
fn picker_profile_response_keeps_its_assignment_intent_when_workspace_state_changes() {
    let target = skill(910, "Picker Target");
    let detail = profile_with_skills(920, Vec::new());
    let mut model = model();
    model.skills.active = true;
    model.skills.replace_detail(skill_view(&target));
    model.skills.pane = SkillsPane::AgentPicker;
    model.agents.profiles = AgentProfilesView {
        profiles: vec![agent(920, "Picker Agent")],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };

    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillAgent { .. }
    ));
    model.skills.active = false;
    apply_outcome(
        &mut model,
        command_outcome(CommandView::AgentProfile(detail.clone())),
    );

    assert!(model.skills.active);
    assert_eq!(model.skills.pane, SkillsPane::AssignmentReview);
    assert_eq!(model.skills.selected_agent_detail, Some(detail));
    assert_eq!(model.skills.assignment, Some(AssignmentKind::Add));
}

fn skill(seed: u128, name: &str) -> SkillVersion {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(seed)),
        SkillVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        1,
        SkillProvenance::User,
        SkillDraft::new(
            name.to_owned(),
            format!("Purpose for {name}"),
            "Use for deterministic tests.".to_owned(),
            Vec::new(),
            "Follow the evidence.".to_owned(),
            Vec::new(),
        )
        .expect("draft"),
    )
    .expect("skill")
}

#[test]
fn logical_list_focus_routes_skills_keys_to_the_visible_library() {
    let first = skill(90_000, "First visible skill");
    let second = skill(90_100, "Second visible skill");
    let mut model = model();
    model.skills.active = true;
    model.skills.library_loaded = true;
    model.skills.library = SkillsView {
        skills: vec![summary(&first), summary(&second)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    };
    model.skills.detail = Some(skill_view(&first));
    model.skills.pane = SkillsPane::Detail;
    model.skills.selected_action_index = 0;
    model.set_focus(Focus::List);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('s'))),
        ControllerEffect::Redraw
    );
    assert_eq!(model.skills.selected_skill, 1);
    assert_eq!(model.skills.selected_action_index, 0);
    assert_eq!(model.skills.pane, SkillsPane::Detail);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkill { selected_skill: 1 }
    );
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.skills.pane, SkillsPane::Detail);
}

fn summary(skill: &SkillVersion) -> SkillSummary {
    SkillSummary {
        skill_ref: skill.reference(),
        display_name: skill.content().display_name.clone(),
        provenance: SkillProvenance::User,
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

fn command_outcome(view: CommandView) -> CommandOutcome {
    CommandOutcome {
        command_id: CommandId::from_uuid(Uuid::from_u128(800)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(801)),
        committed_events: Vec::new(),
        view,
        shutdown: ShutdownDisposition::Continue,
    }
}

fn profile_with_skills(
    seed: u128,
    skill_refs: Vec<ai_stock_forum::skills::SkillVersionRef>,
) -> AgentProfileView {
    let template = &ai_stock_forum::agents::builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().unwrap();
    draft.skill_refs = skill_refs;
    let profile = ai_stock_forum::agents::AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(seed)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(seed + 2)),
        1,
        draft,
        Some(template.provenance()),
    )
    .unwrap();
    AgentProfileView {
        readiness: profile.readiness(),
        profile,
    }
}

#[test]
fn agent_view_replaces_stale_skill_context_and_escape_returns_to_agent_skills() {
    let skill_a = skill(810, "Skill A");
    let skill_b = skill(820, "Skill B");
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail = Some(profile_with_skills(830, vec![skill_b.reference()]));
    model.agents.skill_panel_open = true;
    model.skills.detail = Some(skill_view(&skill_a));
    model.skills.version_detail = Some(skill_view(&skill_a));
    model.skills.history = Some(SkillHistoryView {
        skill_id: skill_a.skill_id(),
        active_version_id: skill_a.skill_version_id(),
        versions: vec![SkillHistoryEntry {
            skill_ref: skill_a.reference(),
            created_at_ms: skill_a.created_at_ms(),
            predecessor_version_id: skill_a.predecessor(),
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillVersion {
            skill_id: skill_b.skill_id(),
            version: skill_b.reference().version(),
        }
    );
    assert_eq!(
        model.skills.workspace_origin,
        Some(SkillWorkspaceOrigin::AgentSkills {
            profile_id: AgentProfileId::from_uuid(Uuid::from_u128(830)),
        })
    );
    assert!(model.skills.detail.is_none());
    assert!(model.skills.history.is_none());
    assert!(model.skills.version_detail.is_none());

    assert_eq!(
        apply_outcome(
            &mut model,
            command_outcome(CommandView::SkillVersion(skill_view(&skill_b))),
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(model.skills.pane, SkillsPane::Detail);
    assert_eq!(
        model.skills.selected_skill_ref(),
        Some(&skill_b.reference())
    );
    assert_eq!(
        model
            .skills
            .detail
            .as_ref()
            .map(|detail| detail.skill_ref.clone()),
        Some(skill_b.reference())
    );
    assert_eq!(
        model
            .skills
            .version_detail
            .as_ref()
            .map(|detail| detail.skill_ref.clone()),
        Some(skill_b.reference())
    );
    assert!(model.skills.history.is_none());

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Agents);
    assert!(model.agents.skill_panel_open);
}

#[test]
fn agent_view_history_and_historical_assignment_never_reuse_selected_skill_a() {
    let skill_a = skill(870, "Skill A");
    let skill_b_v1 = skill(880, "Skill B");
    let mut next_draft = skill_b_v1.content().clone();
    next_draft.instructions = "Second B version".to_owned();
    let skill_b_v2 = SkillVersion::next_version(
        &skill_b_v1,
        SkillVersionId::from_uuid(Uuid::from_u128(882)),
        2,
        next_draft,
    )
    .unwrap();
    let assigned_agent = profile_with_skills(890, vec![skill_b_v2.reference()]);
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail = Some(assigned_agent.clone());
    model.agents.skill_panel_open = true;
    model.skills.library = SkillsView {
        skills: vec![summary(&skill_a), summary(&skill_b_v2)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    };
    model.skills.selected_skill = 0;
    model.skills.detail = Some(skill_view(&skill_a));
    model.skills.version_detail = Some(skill_view(&skill_a));
    model.skills.history = Some(SkillHistoryView {
        skill_id: skill_a.skill_id(),
        active_version_id: skill_a.skill_version_id(),
        versions: vec![SkillHistoryEntry {
            skill_ref: skill_a.reference(),
            created_at_ms: skill_a.created_at_ms(),
            predecessor_version_id: skill_a.predecessor(),
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillVersion {
            skill_id: skill_b_v2.skill_id(),
            version: skill_b_v2.version(),
        }
    );
    apply_outcome(
        &mut model,
        command_outcome(CommandView::SkillVersion(skill_view(&skill_b_v2))),
    );
    assert_eq!(
        model
            .skills
            .selected_summary()
            .map(|summary| summary.skill_ref.clone()),
        Some(skill_b_v2.reference())
    );

    handle_event(&mut model, key(KeyCode::Right));
    handle_event(&mut model, key(KeyCode::Right));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillHistory {
            skill_id: skill_b_v2.skill_id(),
        }
    );
    apply_outcome(
        &mut model,
        command_outcome(CommandView::SkillHistory(SkillHistoryView {
            skill_id: skill_b_v2.skill_id(),
            active_version_id: skill_b_v2.skill_version_id(),
            versions: vec![
                SkillHistoryEntry {
                    skill_ref: skill_b_v1.reference(),
                    created_at_ms: skill_b_v1.created_at_ms(),
                    predecessor_version_id: skill_b_v1.predecessor(),
                },
                SkillHistoryEntry {
                    skill_ref: skill_b_v2.reference(),
                    created_at_ms: skill_b_v2.created_at_ms(),
                    predecessor_version_id: skill_b_v2.predecessor(),
                },
            ],
            total_count: 2,
            returned_count: 2,
            truncated: false,
        })),
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillVersion {
            skill_id: skill_b_v1.skill_id(),
            version: skill_b_v1.version(),
        }
    );
    apply_outcome(
        &mut model,
        command_outcome(CommandView::SkillVersion(skill_view(&skill_b_v1))),
    );
    assert_eq!(
        model.skills.selected_skill_ref(),
        Some(&skill_b_v1.reference())
    );

    handle_event(&mut model, key(KeyCode::Left));
    handle_event(&mut model, key(KeyCode::Left));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillAgent {
            profile_id: AgentProfileId::from_uuid(Uuid::from_u128(890)),
        }
    );
    apply_outcome(
        &mut model,
        command_outcome(CommandView::AgentProfile(assigned_agent)),
    );
    let ControllerEffect::RequestSkillAssignmentPreview {
        target, assignment, ..
    } = handle_event(&mut model, key(KeyCode::Enter))
    else {
        panic!("historical B assignment preview")
    };
    assert_eq!(target, skill_b_v1.reference());
    assert_eq!(
        assignment,
        AssignmentKind::Reassign {
            expected: skill_b_v2.reference(),
        }
    );
    assert_ne!(target.skill_id(), skill_a.skill_id());
}

#[test]
fn opened_historical_version_assigns_its_exact_ref_and_classifies_historical_reassignment() {
    let first = skill(840, "Historical");
    let active = SkillVersion::create(
        first.skill_id(),
        SkillVersionId::from_uuid(Uuid::from_u128(842)),
        2,
        SkillProvenance::User,
        first.content().clone(),
    )
    .unwrap();
    let mut model = model();
    model.skills.active = true;
    model.skills.replace_detail(skill_view(&active));
    model.skills.replace_history(SkillHistoryView {
        skill_id: first.skill_id(),
        active_version_id: active.skill_version_id(),
        versions: vec![
            SkillHistoryEntry {
                skill_ref: first.reference(),
                created_at_ms: first.created_at_ms(),
                predecessor_version_id: first.predecessor(),
            },
            SkillHistoryEntry {
                skill_ref: active.reference(),
                created_at_ms: active.created_at_ms(),
                predecessor_version_id: active.predecessor(),
            },
        ],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    model.skills.pane = SkillsPane::History;

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillVersion {
            skill_id: first.skill_id(),
            version: first.reference().version(),
        }
    );
    apply_outcome(
        &mut model,
        command_outcome(CommandView::SkillVersion(skill_view(&first))),
    );
    assert_eq!(model.skills.pane, SkillsPane::Detail);
    assert_eq!(model.skills.selected_skill_ref(), Some(&first.reference()));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillAgents
    );

    let assigned = profile_with_skills(850, vec![active.reference()]);
    model.skills.selected_agent_detail = Some(assigned.clone());
    model.skills.assignment = Some(AssignmentKind::classify(
        &first.reference(),
        assigned.profile.skill_refs().first(),
    ));
    model.skills.pane = SkillsPane::AssignmentReview;
    let ControllerEffect::RequestSkillAssignmentPreview {
        target, assignment, ..
    } = handle_event(&mut model, key(KeyCode::Enter))
    else {
        panic!("historical assignment preview")
    };
    assert_eq!(target, first.reference());
    assert!(
        format!("{assignment:?}").starts_with("Reassign"),
        "historical reassignment was mislabeled as {assignment:?}"
    );

    let skill_b = skill(860, "Skill B");
    model.skills.replace_detail(skill_view(&skill_b));
    assert_eq!(
        model.skills.selected_skill_ref(),
        Some(&skill_b.reference())
    );
}

#[test]
fn bare_four_opens_skills_without_mutating_library_and_bare_q_is_inert() {
    let first = skill(10, "First");
    let mut model = model();
    model.skills.replace_skills(SkillsView {
        skills: vec![summary(&first)],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    });
    let before = model.skills.library.clone();

    assert_eq!(
        handle_event(&mut model, navigation_key('4')),
        ControllerEffect::Redraw
    );
    assert!(model.skills.active);
    assert_eq!(model.skills.library, before);

    let before_q = model.clone();
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('q'))),
        ControllerEffect::None
    );
    assert_eq!(model, before_q);
}

#[test]
fn active_skills_workspace_owns_keys_even_when_the_rendered_view_is_agents() {
    let assigned = skill(500, "Assigned");
    let first = skill(510, "First");
    let second = skill(520, "Second");
    let template = &ai_stock_forum::agents::builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().unwrap();
    draft.skill_refs = vec![assigned.reference()];
    let profile = ai_stock_forum::agents::AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(530)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(531)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(532)),
        1,
        draft,
        Some(template.provenance()),
    )
    .unwrap();
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    model.agents.skill_panel_open = true;
    model.agents.detail = Some(AgentProfileView {
        readiness: profile.readiness(),
        profile,
    });
    model.skills.active = true;
    model.skills.replace_skills(SkillsView {
        skills: vec![summary(&first), summary(&second)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.skills.selected_skill, 1);
    assert_eq!(model.agents.selected_assigned_skill, 0);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('/'))),
        ControllerEffect::Redraw
    );
    assert_eq!(model.command.text(), "/");
}

#[test]
fn loading_skill_b_after_skill_a_history_clears_the_historical_a_reference() {
    let skill_a = skill(540, "Skill A");
    let skill_b = skill(550, "Skill B");
    let mut model = model();
    model.skills.active = true;
    model.skills.replace_detail(skill_view(&skill_a));
    model.skills.replace_history(SkillHistoryView {
        skill_id: skill_a.skill_id(),
        active_version_id: skill_a.skill_version_id(),
        versions: vec![ai_stock_forum::app::SkillHistoryEntry {
            skill_ref: skill_a.reference(),
            created_at_ms: 1,
            predecessor_version_id: None,
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    });
    model.skills.replace_version_detail(skill_view(&skill_a));

    model.skills.replace_detail(skill_view(&skill_b));

    assert_eq!(
        model.skills.selected_skill_ref(),
        Some(&skill_b.reference())
    );
    assert!(model.skills.version_detail.is_none());
    assert!(model.skills.history.is_none());
}

#[test]
fn skill_editor_input_seeds_version_fields_and_enter_accepts_unchanged_values() {
    let version = skill(560, "Seeded Skill");
    let mut model = model();
    model.skills.active = true;
    model.skills.replace_detail(skill_view(&version));
    model.skills.selected_action_index = 1;

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.command.text(), "Seeded Skill");

    let expected = [
        "Purpose for Seeded Skill",
        "Use for deterministic tests.",
        "",
        "Follow the evidence.",
        "",
        "",
    ];
    for value in expected {
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Enter)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.command.text(), value);
    }
    assert_eq!(
        model.skills.editor.as_ref().unwrap().step(),
        ai_stock_forum::ui::skill_editor::SkillEditorStep::Review
    );
}

#[test]
fn skill_editor_input_keeps_invalid_text_in_the_visible_buffer() {
    let mut model = model();
    model.skills.active = true;
    model.skills.start_create(None);
    let invalid = "x".repeat(65);
    model.command.ingest(&invalid);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.command.text(), invalid);
    assert_eq!(
        model.skills.editor.as_ref().unwrap().field(),
        ai_stock_forum::ui::skill_editor::SkillEditorField::DisplayName
    );
}

#[test]
fn skill_editor_input_escape_restores_the_previous_field_value() {
    let mut model = model();
    model.skills.active = true;
    model.skills.start_create(None);
    model.command.ingest("Draft Skill");
    handle_event(&mut model, key(KeyCode::Enter));
    model.command.ingest("Draft purpose");
    handle_event(&mut model, key(KeyCode::Enter));
    assert_eq!(
        model.skills.editor.as_ref().unwrap().field(),
        ai_stock_forum::ui::skill_editor::SkillEditorField::UseWhen
    );

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.skills.editor.as_ref().unwrap().field(),
        ai_stock_forum::ui::skill_editor::SkillEditorField::Purpose
    );
    assert_eq!(model.command.text(), "Draft purpose");
}

#[test]
fn editor_confirmation_escape_discards_the_cancelled_preview_before_retry() {
    let version = skill(570, "Fresh Preview");
    let mut editor = ai_stock_forum::ui::skill_editor::SkillEditor::for_version(
        version.skill_id(),
        version.skill_version_id(),
        version.content().clone(),
    );
    editor.go_to_review().unwrap();
    let ai_stock_forum::ui::skill_editor::SkillEditorEffect::Preview(request) =
        editor.submit_keyboard_line("")
    else {
        panic!("preview request")
    };
    assert!(editor.apply_preview(
        request.generation(),
        ai_stock_forum::skills::SkillEditPreview {
            skill_id: version.skill_id(),
            expected_active_version_id: Some(version.skill_version_id()),
            candidate_digest: sha256(b"candidate"),
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(571)),
            review_digest: sha256(b"review"),
        },
    ));
    let ai_stock_forum::ui::skill_editor::SkillEditorEffect::Execute(command) =
        editor.submit_keyboard_line("")
    else {
        panic!("protected command")
    };
    let mut model = model();
    model.skills.active = true;
    model.skills.pane = SkillsPane::Confirmation;
    model.skills.editor = Some(editor);
    model.skills.pending_confirmation = Some(SkillConfirmation {
        command,
        origin: SkillOperationOrigin::Skills(SkillsPane::Editor),
    });
    model.skills.review_registered = true;

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::CancelSkillReview
    );
    assert!(model.skills.editor.as_ref().unwrap().review().is_none());
    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::RequestSkillPreview(_)
    ));
}

#[test]
fn arrows_and_enter_drive_list_detail_actions_history_and_agent_picker() {
    let first = skill(20, "First");
    let second = skill(30, "Second");
    let mut model = model();
    model.skills.active = true;
    model.skills.replace_skills(SkillsView {
        skills: vec![summary(&first), summary(&second)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.skills.selected_skill, 1);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkill { selected_skill: 1 }
    );

    model.skills.pane = SkillsPane::Detail;
    assert_eq!(model.skills.selected_action(), SkillDetailAction::Assign);
    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(
        model.skills.selected_action(),
        SkillDetailAction::CreateVersion
    );
    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(model.skills.selected_action(), SkillDetailAction::History);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillHistory {
            skill_id: second.skill_id(),
        }
    );

    model.skills.pane = SkillsPane::History;
    model.skills.replace_history(SkillHistoryView {
        skill_id: second.skill_id(),
        active_version_id: second.skill_version_id(),
        versions: vec![
            ai_stock_forum::app::SkillHistoryEntry {
                skill_ref: second.reference(),
                created_at_ms: 2,
                predecessor_version_id: None,
            },
            ai_stock_forum::app::SkillHistoryEntry {
                skill_ref: first.reference(),
                created_at_ms: 1,
                predecessor_version_id: None,
            },
        ],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(model.skills.selected_history_version, 1);
    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillVersion { version, .. } if version.get() == 1
    ));

    model.skills.pane = SkillsPane::AgentPicker;
    model.agents.profiles = AgentProfilesView {
        profiles: vec![agent(100, "One"), agent(200, "Two")],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    };
    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(model.skills.selected_agent, 1);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillAgent {
            profile_id: model.agents.profiles.profiles[1].profile_id,
        }
    );
}

#[test]
fn assignment_classification_never_auto_upgrades_an_exact_reference() {
    let first = skill(40, "Pinned");
    let second_version = SkillVersion::next_version(
        &first,
        SkillVersionId::from_uuid(Uuid::from_u128(42)),
        2,
        SkillDraft::new(
            "Pinned".to_owned(),
            "Changed purpose".to_owned(),
            "Use for deterministic tests.".to_owned(),
            Vec::new(),
            "Follow newer evidence.".to_owned(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        AssignmentKind::classify(&first.reference(), None),
        AssignmentKind::Add
    );
    assert_eq!(
        AssignmentKind::classify(&first.reference(), Some(&first.reference())),
        AssignmentKind::AlreadyAssigned
    );
    assert_eq!(
        AssignmentKind::classify(&second_version.reference(), Some(&first.reference())),
        AssignmentKind::Upgrade {
            expected: first.reference()
        }
    );
}

#[test]
fn escape_unwinds_one_skills_level_and_preserves_editor_state() {
    let mut model = model();
    model.skills.active = true;
    model.skills.start_create(None);
    model.command.ingest("Draft name");

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert!(model.skills.active);
    assert_eq!(model.skills.pane, SkillsPane::CreateSource);
    assert!(model.skills.editor.is_none());
    assert_eq!(model.command.text(), "");

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.skills.pane, SkillsPane::List);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert!(!model.skills.active);
}

#[test]
fn agent_detail_unassigns_the_selected_exact_current_reference_through_preview() {
    let assigned = skill(300, "Assigned").reference();
    let template = &ai_stock_forum::agents::builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().unwrap();
    draft.skill_refs = vec![assigned.clone()];
    let profile = ai_stock_forum::agents::AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(400)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(401)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(402)),
        1,
        draft,
        Some(template.provenance()),
    )
    .unwrap();
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    let mut row = agent(400, profile.display_name());
    row.content_digest = profile.content_digest().clone();
    model.agents.profiles.profiles = vec![row];
    model.agents.selected_detail_action =
        ai_stock_forum::ui::tui::model::AgentDetailAction::AssignedSkills;
    model.agents.detail = Some(AgentProfileView {
        readiness: profile.readiness(),
        profile,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert!(model.agents.skill_panel_open);
    assert_eq!(model.agents.selected_skill_action(), AgentSkillAction::View);
    handle_event(&mut model, key(KeyCode::Right));
    handle_event(&mut model, key(KeyCode::Right));
    assert_eq!(
        model.agents.selected_skill_action(),
        AgentSkillAction::Unassign
    );

    let ControllerEffect::RequestSkillAssignmentPreview {
        profile_id,
        expected_active_profile_version_id,
        target,
        assignment: AssignmentKind::Unassign { expected },
    } = handle_event(&mut model, key(KeyCode::Enter))
    else {
        panic!("protected unassignment preview")
    };
    assert_eq!(profile_id, AgentProfileId::from_uuid(Uuid::from_u128(400)));
    assert_eq!(
        expected_active_profile_version_id,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(401))
    );
    assert_eq!(target, assigned);
    assert_eq!(expected, assigned);
    assert_eq!(
        model.skills.operation_origin,
        SkillOperationOrigin::AgentSkills {
            profile_id: AgentProfileId::from_uuid(Uuid::from_u128(400)),
        }
    );
}

fn agent(seed: u128, name: &str) -> AgentProfileSummary {
    AgentProfileSummary {
        profile_id: AgentProfileId::from_uuid(Uuid::from_u128(seed)),
        profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        version: ObjectVersion::new(1).unwrap(),
        display_name: name.to_owned(),
        role: ai_stock_forum::agents::builtin_profile_templates()[0].role,
        primary_specialty: "Research".to_owned(),
        readiness: ai_stock_forum::agents::AgentReadiness::Unbound,
        content_digest: ai_stock_forum::domain::sha256(name.as_bytes()),
    }
}
