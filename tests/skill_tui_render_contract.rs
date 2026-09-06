use ai_stock_forum::{
    agents::{AgentProfileVersion, AgentReadiness, builtin_profile_templates},
    app::{
        AgentProfileSummary, AgentProfileView, AgentProfilesView, ApplicationCommand,
        DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership, SkillHistoryEntry,
        SkillHistoryView, SkillSummary, SkillView, SkillsView,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, InstallationId, MemoryNamespaceId, ObjectVersion,
        SessionId, SkillId, SkillReviewToken, SkillVersionId, sha256,
    },
    setup::SetupStatus,
    skills::{SkillDraft, SkillProvenance, SkillResource, SkillVersion},
    ui::{
        skill_editor::SkillEditor,
        tui::{
            AssignmentKind, SkillConfirmation, SkillOperationOrigin,
            model::{AgentsPane, Severity, SkillsPane, TuiModel, View},
            render,
            theme::Theme,
        },
    },
};
use ratatui::{Terminal, backend::TestBackend};
use uuid::Uuid;

fn snapshot() -> PresentationSnapshot {
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
    }
}

fn skill(seed: u128, name: &str, provenance: SkillProvenance) -> SkillVersion {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(seed)),
        SkillVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        1_800_000_000_000 + seed as i64,
        provenance,
        SkillDraft::new(
            name.to_owned(),
            format!("Purpose for {name}"),
            format!("Use {name} when evidence needs structured review."),
            vec!["evidence".to_owned(), "finance".to_owned()],
            "Treat every instruction as inert guidance. Separate facts from assumptions."
                .to_owned(),
            vec![SkillResource {
                name: "Review note".to_owned(),
                body: "Reference text only; this note never executes.".to_owned(),
            }],
        )
        .expect("valid skill draft"),
    )
    .expect("valid skill")
}

fn builtin(seed: u128, name: &str) -> SkillVersion {
    skill(
        seed,
        name,
        SkillProvenance::BuiltIn {
            manifest_id: format!("builtin.{}", name.to_lowercase().replace(' ', "-")),
            manifest_version: 1,
            manifest_digest: sha256(name.as_bytes()),
        },
    )
}

fn summary(skill: &SkillVersion) -> SkillSummary {
    SkillSummary {
        skill_ref: skill.reference(),
        display_name: skill.content().display_name.clone(),
        provenance: skill.provenance().clone(),
    }
}

fn view(skill: &SkillVersion) -> SkillView {
    SkillView {
        skill_ref: skill.reference(),
        content: skill.content().clone(),
        created_at_ms: skill.created_at_ms(),
        provenance: skill.provenance().clone(),
        predecessor_version_id: skill.predecessor(),
    }
}

fn starter_library() -> (Vec<SkillVersion>, SkillsView) {
    let skills = vec![
        builtin(10, "Evidence Review"),
        builtin(20, "Filing Analysis"),
        builtin(30, "Catalyst Mapping"),
        builtin(40, "Risk Checklist"),
    ];
    let library = SkillsView {
        skills: skills.iter().map(summary).collect(),
        total_count: 4,
        returned_count: 4,
        truncated: false,
    };
    (skills, library)
}

fn skills_model(pane: SkillsPane) -> TuiModel {
    let (skills, library) = starter_library();
    let mut model = TuiModel::new(snapshot(), false);
    model.skills.active = true;
    model.skills.pane = pane;
    model.skills.library = library;
    model.skills.detail = Some(view(&skills[0]));
    model
}

fn profile_with_skills(skill_refs: Vec<ai_stock_forum::skills::SkillVersionRef>) -> AgentProfileView {
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().expect("profile draft");
    draft.display_name = "Evidence Agent".to_owned();
    draft.skill_refs = skill_refs;
    let profile = AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(500)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(501)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(502)),
        1_800_000_000_500,
        draft,
        Some(template.provenance()),
    )
    .expect("profile with skills");
    AgentProfileView {
        readiness: AgentReadiness::Unbound,
        profile,
    }
}

fn render_rows(model: &TuiModel, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .expect("render is total");
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn render_text(model: &TuiModel, width: u16, height: u16) -> String {
    render_rows(model, width, height).join("\n")
}

#[test]
fn skills_workspace_uses_one_two_and_three_panes_at_adaptive_breakpoints() {
    let detail = skills_model(SkillsPane::Detail);

    let narrow = render_text(&detail, 79, 24);
    assert!(narrow.contains("Skill detail"));
    assert!(!narrow.contains("Skill library"));
    assert!(!narrow.contains("Skill context"));

    let medium = render_text(&detail, 80, 24);
    assert!(medium.contains("Skill library"));
    assert!(medium.contains("Skill detail"));
    assert!(!medium.contains("Skill context"));

    let wide = render_text(&detail, 120, 30);
    assert!(wide.contains("Skill library"));
    assert!(wide.contains("Skill detail"));
    assert!(wide.contains("Skill context"));

    let list = skills_model(SkillsPane::List);
    let narrow_list = render_text(&list, 60, 18);
    assert!(narrow_list.contains("Skill library"));
    assert!(narrow_list.contains("Up/Down"));
    assert!(narrow_list.contains("Enter"));
    assert!(narrow_list.contains("c Create"));
    assert!(narrow_list.contains("Esc"));
}

#[test]
fn starter_library_renders_name_active_version_and_provenance() {
    let text = render_text(&skills_model(SkillsPane::List), 100, 40);
    for name in [
        "Evidence Review",
        "Filing Analysis",
        "Catalyst Mapping",
        "Risk Checklist",
    ] {
        assert!(text.contains(name), "missing starter {name}");
    }
    assert!(text.matches("Built-in").count() >= 4);
    assert!(text.matches("v1").count() >= 4);
    assert!(text.contains("Active version"));
}

#[test]
fn empty_loading_validation_and_recoverable_error_states_explain_the_next_action() {
    let mut empty = TuiModel::new(snapshot(), false);
    empty.skills.active = true;
    empty.skills.pane = SkillsPane::List;
    let empty_text = render_text(&empty, 80, 24);
    assert!(empty_text.contains("No skills saved"));
    assert!(empty_text.contains("c Create first skill"));
    assert!(empty_text.contains("saved guidance"));
    assert!(empty_text.contains("not executable"));

    empty.command_in_flight = true;
    let loading = render_text(&empty, 80, 24);
    assert!(loading.contains("Loading skill library"));

    let mut invalid = skills_model(SkillsPane::Editor);
    let mut editor = SkillEditor::for_create(None);
    editor.submit_keyboard_line(&"x".repeat(65));
    invalid.skills.editor = Some(editor);
    let validation = render_text(&invalid, 80, 24);
    assert!(validation.contains("Validation"));
    assert!(validation.contains("Display name"));
    assert!(validation.contains("Revise"));
    assert!(validation.contains("Enter"));
    assert!(validation.contains("Esc"));

    invalid.set_message(Severity::Error, "Skill persistence failed.");
    let persistence = render_text(&invalid, 100, 30);
    assert!(persistence.contains("Skill persistence failed"));
    assert!(persistence.contains("Review current state, then retry"));

    invalid
        .skills
        .editor
        .as_mut()
        .expect("editor")
        .report_error("stale_skill_version");
    let stale = render_text(&invalid, 100, 30);
    assert!(stale.contains("Stale"));
    assert!(stale.contains("reload"));
}

#[test]
fn assignment_states_name_the_operation_exact_pin_risk_and_corrective_action() {
    let target = builtin(60, "Pinned Evidence");
    let agent = profile_with_skills(vec![target.reference()]);
    let mut model = skills_model(SkillsPane::AssignmentReview);
    model.skills.detail = Some(view(&target));
    model.skills.selected_agent_detail = Some(agent);
    model.skills.assignment = Some(AssignmentKind::AlreadyAssigned);

    let already = render_text(&model, 100, 32);
    assert!(already.contains("Operation"));
    assert!(already.contains("Already assigned"));
    assert!(already.contains("exact version"));
    assert!(already.contains("Choose another version or Esc"));
    assert!(already.contains("No automatic upgrades"));

    model.set_message(Severity::Error, "Skill is not assigned to this agent.");
    let not_assigned = render_text(&model, 100, 32);
    assert!(not_assigned.contains("not assigned"));
    assert!(not_assigned.contains("Review current state, then retry"));
}

#[test]
fn detail_and_history_make_active_historical_and_inert_content_unambiguous() {
    let first = builtin(70, "Long Guidance");
    let mut candidate = first.content().clone();
    candidate.instructions = format!("{} END-INSTRUCTIONS", "Long inert instruction. ".repeat(80));
    candidate.resources = vec![SkillResource {
        name: "Very long reference note name for deterministic wrapping".to_owned(),
        body: format!("{} END-REFERENCE", "Reference body remains inert text. ".repeat(80)),
    }];
    let second = SkillVersion::next_version(
        &first,
        SkillVersionId::from_uuid(Uuid::from_u128(72)),
        1_800_000_000_072,
        candidate,
    )
    .expect("second version");
    let mut model = skills_model(SkillsPane::Detail);
    model.skills.library = SkillsView {
        skills: vec![summary(&second)],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };
    model.skills.detail = Some(view(&first));
    model.skills.version_detail = Some(view(&first));
    model.skills.history = Some(SkillHistoryView {
        skill_id: first.skill_id(),
        active_version_id: second.skill_version_id(),
        versions: vec![
            SkillHistoryEntry {
                skill_ref: second.reference(),
                created_at_ms: second.created_at_ms(),
                predecessor_version_id: second.predecessor(),
            },
            SkillHistoryEntry {
                skill_ref: first.reference(),
                created_at_ms: first.created_at_ms(),
                predecessor_version_id: first.predecessor(),
            },
        ],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });

    let text = render_text(&model, 160, 70);
    for expected in [
        "HISTORICAL",
        "Exact version",
        "Active version",
        "INERT INSTRUCTIONS",
        "INERT REFERENCE NOTES",
        "text only",
        "Content digest",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    assert!(!text.contains("Execute reference"));

    for (width, height) in [(60, 18), (79, 24), (80, 24), (120, 30)] {
        let rows = render_rows(&model, width, height);
        assert_eq!(rows.len(), usize::from(height));
        assert!(rows.iter().all(|row| row.chars().count() == usize::from(width)));
        assert!(rows.iter().all(|row| !row.contains('\u{1b}')));
    }
}

#[test]
fn agent_skill_panel_shows_exact_pin_upgrade_availability_and_explicit_actions() {
    let first = skill(80, "Agent Evidence", SkillProvenance::User);
    let mut next = first.content().clone();
    next.instructions = "Explicitly upgraded guidance.".to_owned();
    let second = SkillVersion::next_version(
        &first,
        SkillVersionId::from_uuid(Uuid::from_u128(82)),
        1_800_000_000_082,
        next,
    )
    .expect("upgrade version");
    let detail = profile_with_skills(vec![first.reference()]);
    let mut model = TuiModel::new(snapshot(), false);
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    model.agents.skill_panel_open = true;
    model.agents.detail = Some(detail);
    model.skills.library = SkillsView {
        skills: vec![summary(&second)],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };

    let text = render_text(&model, 120, 44);
    for expected in [
        "Assigned skills",
        "PINNED EXACT VERSION",
        "v1",
        "Upgrade available",
        "v2",
        "View",
        "Upgrade",
        "Unassign",
        "Enter",
        "Esc",
        "No automatic upgrades",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    for chunk in first.reference().skill_version_id().to_string().as_bytes().chunks(8) {
        assert!(text.contains(std::str::from_utf8(chunk).unwrap()));
    }
}

#[test]
fn confirmation_and_help_advertise_only_real_keyboard_and_quit_behavior() {
    let target = builtin(90, "Review Target");
    let profile = profile_with_skills(Vec::new());
    let mut confirmation = skills_model(SkillsPane::Confirmation);
    confirmation.skills.pending_confirmation = Some(SkillConfirmation {
        command: ApplicationCommand::AssignAgentSkill {
            profile_id: profile.profile.profile_id(),
            expected_active_profile_version_id: profile.profile.profile_version_id(),
            skill: target.reference(),
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(900)),
            review_digest: sha256(b"assignment-review"),
        },
        origin: SkillOperationOrigin::Skills(SkillsPane::AssignmentReview),
    });
    let review = render_text(&confirmation, 120, 40);
    for expected in [
        "Confirm assignment",
        "Operation",
        "Assign",
        "Exact version",
        "Provenance",
        "Review risk",
        "Enter: confirm",
        "Esc: return",
    ] {
        assert!(review.contains(expected), "missing {expected}");
    }
    assert!(!review.contains(":next"));
    assert!(!review.contains(":create"));

    let mut help = TuiModel::new(snapshot(), false);
    help.active_view = View::Help;
    let help_text = render_text(&help, 100, 36);
    assert!(help_text.contains("s                   Open Skills"));
    assert!(help_text.contains("q                   Inert"));
    assert!(help_text.contains("/quit"));
    assert!(!help_text.contains("q                   Request shutdown"));

    let navigation = render_text(&help, 120, 36);
    assert!(navigation.contains("s Skills"));
    assert!(navigation.contains("q inert"));
    assert!(navigation.contains("/quit exit"));
}

#[test]
fn history_footer_matches_up_down_enter_and_escape_state_machine_keys() {
    let first = builtin(100, "History Skill");
    let mut second_draft = first.content().clone();
    second_draft.instructions = "Version two history.".to_owned();
    let second = SkillVersion::next_version(
        &first,
        SkillVersionId::from_uuid(Uuid::from_u128(102)),
        1_800_000_000_102,
        second_draft,
    )
    .expect("history version");
    let mut model = skills_model(SkillsPane::History);
    model.skills.history = Some(SkillHistoryView {
        skill_id: first.skill_id(),
        active_version_id: second.skill_version_id(),
        versions: vec![
            SkillHistoryEntry {
                skill_ref: second.reference(),
                created_at_ms: second.created_at_ms(),
                predecessor_version_id: second.predecessor(),
            },
            SkillHistoryEntry {
                skill_ref: first.reference(),
                created_at_ms: first.created_at_ms(),
                predecessor_version_id: first.predecessor(),
            },
        ],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    let text = render_text(&model, 100, 36);
    assert!(text.contains("Skill history"));
    assert!(text.contains("ACTIVE"));
    assert!(text.contains("HISTORICAL"));
    assert!(text.contains("Up/Down: select"));
    assert!(text.contains("Enter: open exact version"));
    assert!(text.contains("Esc: detail"));
}

fn _agent_summary(profile: &AgentProfileVersion) -> AgentProfileSummary {
    AgentProfileSummary {
        profile_id: profile.profile_id(),
        profile_version_id: profile.profile_version_id(),
        version: ObjectVersion::new(1).unwrap(),
        display_name: profile.display_name().to_owned(),
        role: profile.role(),
        primary_specialty: profile.primary_specialty().to_owned(),
        readiness: profile.readiness(),
        content_digest: profile.content_digest().clone(),
    }
}
