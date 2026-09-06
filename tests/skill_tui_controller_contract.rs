use ai_stock_forum::{
    app::{
        AgentProfileSummary, AgentProfileView, AgentProfilesView, DatabaseReadiness,
        PresentationSnapshot, ProcessGuardOwnership, SkillHistoryView, SkillSummary, SkillsView,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, InstallationId, MemoryNamespaceId, ObjectVersion,
        SessionId, SkillId, SkillVersionId,
    },
    setup::SetupStatus,
    skills::{SkillDraft, SkillProvenance, SkillVersion},
    ui::tui::{
        ControllerEffect, TuiEvent, handle_event,
        model::{
            AgentSkillAction, AgentsPane, AssignmentKind, SkillDetailAction, SkillsPane, TuiModel,
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

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
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

fn summary(skill: &SkillVersion) -> SkillSummary {
    SkillSummary {
        skill_ref: skill.reference(),
        display_name: skill.content().display_name.clone(),
        provenance: SkillProvenance::User,
    }
}

#[test]
fn s_opens_skills_without_mutating_library_and_bare_q_is_inert() {
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
        handle_event(&mut model, key(KeyCode::Char('s'))),
        ControllerEffect::LoadSkills
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

    assert_eq!(handle_event(&mut model, key(KeyCode::Down)), ControllerEffect::Redraw);
    assert_eq!(model.skills.selected_skill, 1);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkill { selected_skill: 1 }
    );

    model.skills.pane = SkillsPane::Detail;
    assert_eq!(model.skills.selected_action(), SkillDetailAction::Assign);
    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(model.skills.selected_action(), SkillDetailAction::CreateVersion);
    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(model.skills.selected_action(), SkillDetailAction::History);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSkillHistory { selected_skill: 1 }
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
        ControllerEffect::LoadSkillAgent { selected_agent: 1 }
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

    assert_eq!(AssignmentKind::classify(&first.reference(), None), AssignmentKind::Add);
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

    assert_eq!(handle_event(&mut model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert!(model.skills.active);
    assert_eq!(model.skills.pane, SkillsPane::CreateSource);
    assert!(model.skills.editor.is_none());
    assert_eq!(model.command.text(), "");

    assert_eq!(handle_event(&mut model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(model.skills.pane, SkillsPane::List);
    assert_eq!(handle_event(&mut model, key(KeyCode::Esc)), ControllerEffect::Redraw);
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
    model.agents.detail = Some(AgentProfileView {
        readiness: profile.readiness(),
        profile,
    });

    assert_eq!(handle_event(&mut model, key(KeyCode::Enter)), ControllerEffect::Redraw);
    assert!(model.agents.skill_panel_open);
    assert_eq!(model.agents.selected_skill_action(), AgentSkillAction::View);
    handle_event(&mut model, key(KeyCode::Right));
    handle_event(&mut model, key(KeyCode::Right));
    assert_eq!(model.agents.selected_skill_action(), AgentSkillAction::Unassign);

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
