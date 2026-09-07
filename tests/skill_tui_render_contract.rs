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
            model::{AgentsPane, Focus, Severity, SkillsPane, TuiModel, View},
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

fn profile_with_skills(
    skill_refs: Vec<ai_stock_forum::skills::SkillVersionRef>,
) -> AgentProfileView {
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
fn open_skills_inspector_is_rendered_at_medium_and_narrow_breakpoints() {
    let mut detail = skills_model(SkillsPane::Detail);
    detail.inspector_open = true;
    detail.focus = Focus::Inspector;

    for (width, height) in [(80, 24), (70, 20)] {
        let text = render_text(&detail, width, height);
        assert!(
            text.contains("Skill context"),
            "missing Skills inspector at {width}x{height}"
        );
    }
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
    assert!(not_assigned.contains("Review"));
    assert!(not_assigned.contains("current"));
    assert!(not_assigned.contains("state"));
    assert!(not_assigned.contains("retry"));
}

#[test]
fn detail_and_history_make_active_historical_and_inert_content_unambiguous() {
    let first = builtin(70, "Long Guidance");
    let mut candidate = first.content().clone();
    candidate.instructions = format!("{} END-INSTRUCTIONS", "Long inert instruction. ".repeat(80));
    candidate.resources = vec![SkillResource {
        name: "Very long reference note name for deterministic wrapping".to_owned(),
        body: format!(
            "{} END-REFERENCE",
            "Reference body remains inert text. ".repeat(80)
        ),
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
        assert!(
            rows.iter()
                .all(|row| row.chars().count() == usize::from(width))
        );
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

    let text = render_text(&model, 180, 60);
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
    for chunk in first
        .reference()
        .skill_version_id()
        .to_string()
        .as_bytes()
        .chunks(8)
    {
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
    assert!(help_text.contains("1-4 / a / s"));
    assert!(help_text.contains("Open a view outside active text entry"));
    assert!(help_text.contains("a Agents / s Skills"));
    assert!(!help_text.contains("Option/Alt+1-6"));
    assert!(help_text.contains("q                   Inert"));
    assert!(help_text.contains("/quit"));
    assert!(!help_text.contains("q                   Request shutdown"));

    let navigation = render_text(&help, 120, 36);
    assert!(navigation.contains("s Skills"));
    assert!(navigation.contains("a Agents"));
    assert!(!navigation.contains("Alt+"));
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

fn terminal_for(model: &TuiModel, width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .expect("render contract");
    terminal
}

fn create_confirmation_model(candidate: &SkillVersion, unrelated: &SkillVersion) -> TuiModel {
    let mut editor = SkillEditor::for_create(Some(candidate.content().clone()));
    editor.go_to_review().expect("valid review candidate");
    let ai_stock_forum::ui::skill_editor::SkillEditorEffect::Preview(request) =
        editor.submit_keyboard_line("")
    else {
        panic!("preview request")
    };
    assert!(editor.apply_preview(
        request.generation(),
        ai_stock_forum::skills::SkillEditPreview {
            skill_id: candidate.skill_id(),
            expected_active_version_id: None,
            candidate_digest: candidate.content_digest().clone(),
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(9_001)),
            review_digest: sha256(b"authoritative-create-review"),
        },
    ));
    let ai_stock_forum::ui::skill_editor::SkillEditorEffect::Execute(command) =
        editor.submit_keyboard_line("")
    else {
        panic!("create command")
    };
    let mut model = skills_model(SkillsPane::Confirmation);
    model.skills.detail = Some(view(unrelated));
    model.skills.version_detail = Some(view(unrelated));
    model.skills.editor = Some(editor);
    model.skills.pending_confirmation = Some(SkillConfirmation {
        command,
        origin: SkillOperationOrigin::Skills(SkillsPane::Editor),
    });
    model
}

#[test]
fn compact_review_and_confirmation_keep_identity_and_actions_in_fixed_visible_regions() {
    let target = builtin(1_100, "Compact Target");
    let agent = profile_with_skills(Vec::new());
    let mut review = skills_model(SkillsPane::AssignmentReview);
    review.skills.detail = Some(view(&target));
    review.skills.version_detail = Some(view(&target));
    review.skills.selected_agent_detail = Some(agent.clone());
    review.skills.assignment = Some(AssignmentKind::Add);

    let compact_review = render_text(&review, 60, 18);
    for expected in ["Assign", "v1", "Enter", "Esc"] {
        assert!(
            compact_review.contains(expected),
            "review missing {expected}"
        );
    }

    let mut confirmation = review;
    confirmation.skills.pane = SkillsPane::Confirmation;
    confirmation.skills.pending_confirmation = Some(SkillConfirmation {
        command: ApplicationCommand::AssignAgentSkill {
            profile_id: agent.profile.profile_id(),
            expected_active_profile_version_id: agent.profile.profile_version_id(),
            skill: target.reference(),
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(1_101)),
            review_digest: sha256(b"compact-confirmation"),
        },
        origin: SkillOperationOrigin::Skills(SkillsPane::AssignmentReview),
    });
    let compact_confirmation = render_text(&confirmation, 60, 18);
    for expected in ["Assign", "v1", "Enter: confirm", "Esc: return"] {
        assert!(
            compact_confirmation.contains(expected),
            "confirmation missing {expected}"
        );
    }
}

#[test]
fn create_confirmation_uses_authoritative_candidate_state_and_names_version_one() {
    let candidate = skill(1_200, "Authoritative Candidate", SkillProvenance::User);
    let unrelated = builtin(1_300, "Unrelated Loaded Detail");
    let model = create_confirmation_model(&candidate, &unrelated);
    let text = render_text(&model, 120, 44);

    for expected in ["Authoritative Candidate", "Digest", "Version v1"] {
        assert!(text.contains(expected), "missing {expected}");
    }
    for exact_identity in [
        candidate.skill_id().to_string(),
        candidate.content_digest().to_string(),
    ] {
        assert!(text.contains(&exact_identity[..8]));
        assert!(text.contains(&exact_identity[exact_identity.len() - 8..]));
    }
    assert!(!text.contains("Unrelated Loaded Detail"));
    assert!(!text.contains("builtin.unrelated-loaded-detail"));
    assert!(!text.contains("Version Pending"));
}

#[test]
fn list_focus_is_exclusive_and_skills_suppresses_agents_navigation_focus() {
    let mut model = skills_model(SkillsPane::List);
    model.active_view = View::Agents;
    let terminal = terminal_for(&model, 80, 24);
    let buffer = terminal.backend().buffer();

    let library_corner = buffer.cell((20, 4)).expect("library corner");
    let detail_corner = buffer.cell((45, 4)).expect("detail corner");
    assert!(
        library_corner
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    assert!(
        !detail_corner
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );

    let text = render_text(&model, 120, 36);
    assert!(text.contains("> s Skills"));
    assert!(!text.contains("> a Agents"));
}

#[test]
fn unknown_active_status_is_not_rendered_as_historical() {
    let target = skill(1_400, "Unknown Active", SkillProvenance::User);
    let mut model = skills_model(SkillsPane::Detail);
    model.skills.library = SkillsView {
        skills: Vec::new(),
        total_count: 0,
        returned_count: 0,
        truncated: true,
    };
    model.skills.detail = Some(view(&target));
    model.skills.version_detail = Some(view(&target));

    let text = render_text(&model, 120, 40);
    assert!(text.contains("UNKNOWN"));
    assert!(text.contains("active"));
    assert!(text.contains("version not loaded"));
    assert!(!text.contains("Status        HISTORICAL"));
}

#[test]
fn editor_review_names_create_v1_and_the_next_object_version_without_fabricating_an_id() {
    let first = skill(1_500, "Pending Version", SkillProvenance::User);
    let mut create = SkillEditor::for_create(Some(first.content().clone()));
    create.go_to_review().expect("create review");
    let mut create_model = skills_model(SkillsPane::Editor);
    create_model.skills.editor = Some(create);
    let create_text = render_text(&create_model, 180, 60);
    assert!(create_text.contains("Exact version v1"));
    assert!(create_text.contains("version ID assigned on commit"));

    let mut editor = SkillEditor::for_version(
        first.skill_id(),
        first.skill_version_id(),
        first.content().clone(),
    );
    editor.go_to_review().expect("version review");
    let mut model = skills_model(SkillsPane::Editor);
    model.skills.detail = Some(view(&first));
    model.skills.editor = Some(editor);

    let text = render_text(&model, 180, 60);
    assert!(text.contains("Exact version v2"));
    assert!(text.contains("version ID assigned on commit"));
    assert!(!text.contains(&SkillVersionId::from_uuid(Uuid::from_u128(1_502)).to_string()));
}

#[test]
fn assignment_review_distinguishes_forward_upgrade_from_historical_reassignment() {
    let first = skill(1_550, "Version Direction", SkillProvenance::User);
    let mut next_draft = first.content().clone();
    next_draft.instructions = "Forward version guidance.".to_owned();
    let second = SkillVersion::next_version(
        &first,
        SkillVersionId::from_uuid(Uuid::from_u128(1_552)),
        1_800_000_001_552,
        next_draft,
    )
    .expect("second version");

    let mut forward = skills_model(SkillsPane::AssignmentReview);
    forward.skills.detail = Some(view(&second));
    forward.skills.selected_agent_detail = Some(profile_with_skills(vec![first.reference()]));
    forward.skills.assignment = Some(AssignmentKind::classify(
        &second.reference(),
        Some(&first.reference()),
    ));
    let forward_text = render_text(&forward, 180, 60);
    assert!(forward_text.contains("Upgrade explicit exact version"));

    let mut historical = skills_model(SkillsPane::AssignmentReview);
    historical.skills.detail = Some(view(&first));
    historical.skills.selected_agent_detail = Some(profile_with_skills(vec![second.reference()]));
    historical.skills.assignment = Some(AssignmentKind::classify(
        &first.reference(),
        Some(&second.reference()),
    ));
    let historical_text = render_text(&historical, 180, 60);
    assert!(historical_text.contains("Reassign historical exact version"));
    assert!(!historical_text.contains("Upgrade explicit exact version"));
}

#[test]
fn already_assigned_context_omits_enter_validation_and_matches_escape_only_main_action() {
    let target = skill(1_600, "Already Pinned", SkillProvenance::User);
    let mut model = skills_model(SkillsPane::AssignmentReview);
    model.skills.detail = Some(view(&target));
    model.skills.selected_agent_detail = Some(profile_with_skills(vec![target.reference()]));
    model.skills.assignment = Some(AssignmentKind::AlreadyAssigned);

    let text = render_text(&model, 120, 38);
    assert!(text.contains("No operation"));
    assert!(text.contains("Esc: choose another version"));
    assert!(!text.contains("Enter: validate"));
    assert!(!text.contains("Next action Enter validate"));
}

#[test]
fn create_source_agent_picker_and_result_keep_complete_contextual_keys_compact_and_wide() {
    let mut create = skills_model(SkillsPane::CreateSource);
    create.skills.selected_create_source = 0;
    let mut picker = skills_model(SkillsPane::AgentPicker);
    picker.agents.profiles = AgentProfilesView {
        profiles: vec![AgentProfileSummary {
            profile_id: AgentProfileId::from_uuid(Uuid::from_u128(1_700)),
            profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(1_701)),
            version: ObjectVersion::new(1).unwrap(),
            display_name: "Picker Agent".to_owned(),
            role: builtin_profile_templates()[0].role,
            primary_specialty: "Research".to_owned(),
            readiness: AgentReadiness::Unbound,
            content_digest: sha256(b"picker-agent"),
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };
    let result = skills_model(SkillsPane::Result);

    for (name, model, expected) in [
        (
            "create",
            &create,
            ["Up/Down", "Enter: continue", "Esc: library"],
        ),
        ("picker", &picker, ["Up/Down", "Enter: review", "Esc:"]),
        (
            "result",
            &result,
            [
                "Skill action completed",
                "Enter or Esc",
                "return to skill detail",
            ],
        ),
    ] {
        for (width, height) in [(60, 18), (120, 36)] {
            let text = render_text(model, width, height);
            for value in expected {
                assert!(
                    text.contains(value),
                    "{name} {width}x{height} missing {value}"
                );
            }
        }
    }
}

#[test]
fn wide_long_content_declares_truncation_without_hiding_actions() {
    let mut long = skill(1_800, "Long Complete", SkillProvenance::User);
    let mut draft = long.content().clone();
    draft.instructions = format!("{} INSTRUCTION-END", "wrapped guidance ".repeat(80));
    draft.resources = vec![SkillResource {
        name: "Long inert note".to_owned(),
        body: format!("{} REFERENCE-END", "wrapped reference ".repeat(80)),
    }];
    long = SkillVersion::create(
        long.skill_id(),
        SkillVersionId::from_uuid(Uuid::from_u128(1_802)),
        1_800_000_001_800,
        SkillProvenance::User,
        draft,
    )
    .expect("long skill");
    let mut model = skills_model(SkillsPane::Detail);
    model.skills.library = library_for_render(&long);
    model.skills.detail = Some(view(&long));

    let text = render_text(&model, 160, 44);
    assert!(!text.contains("INSTRUCTION-END"));
    assert!(!text.contains("REFERENCE-END"));
    assert!(text.contains("Long content may be truncated"));
    assert!(text.contains("Left/Right: choose action"));
    assert!(text.contains("Enter: open"));
    assert!(text.contains("Esc: library"));
}

fn library_for_render(skill: &SkillVersion) -> SkillsView {
    SkillsView {
        skills: vec![summary(skill)],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    }
}

#[test]
fn agents_multi_skill_panel_shows_position_rows_and_contextual_available_actions() {
    let first = skill(1_900, "First Pin", SkillProvenance::User);
    let first_active = SkillVersion::next_version(
        &first,
        SkillVersionId::from_uuid(Uuid::from_u128(1_902)),
        1_800_000_001_902,
        {
            let mut draft = first.content().clone();
            draft.instructions = "New first pin.".to_owned();
            draft
        },
    )
    .expect("newer first");
    let second = skill(2_000, "Second Pin", SkillProvenance::User);
    let detail = profile_with_skills(vec![first.reference(), second.reference()]);
    let mut model = TuiModel::new(snapshot(), false);
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    model.agents.skill_panel_open = true;
    model.agents.detail = Some(detail);
    model.skills.library = SkillsView {
        skills: vec![summary(&first_active), summary(&second)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    };

    for (width, height) in [(60, 24), (120, 44)] {
        let available = render_text(&model, width, height);
        for expected in [
            "Skill 1 of 2",
            "AVAILABLE",
            "View",
            "Upgrade",
            "Unassign",
            "Enter:",
            "Esc: detail",
            "Up/Down",
        ] {
            assert!(
                available.contains(expected),
                "{width}x{height} missing {expected}"
            );
        }
    }

    model.agents.selected_assigned_skill = 1;
    model.agents.selected_skill_action_index = 0;
    let current = render_text(&model, 120, 44);
    assert!(current.contains("Skill 2 of 2"));
    assert!(current.contains("CURRENT"));
    assert!(!current.contains("[Upgrade]"));
    assert!(current.contains("Enter: View"));

    model.agents.selected_skill_action_index = 2;
    let unassign = render_text(&model, 120, 44);
    assert!(unassign.contains("Enter: Unassign"));
}

#[test]
fn skills_navigation_has_exclusive_focus_style_when_opened_from_agents() {
    for (width, height) in [(60, 24), (80, 28), (120, 44)] {
        let mut model = skills_model(SkillsPane::List);
        model.active_view = View::Agents;
        model.skills.active = true;
        let terminal = terminal_for(&model, width, height);
        let buffer = terminal.backend().buffer();
        let locate = |needle: &str| {
            (0..height)
                .find_map(|y| {
                    let row = (0..width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>();
                    row.find(needle).map(|x| (u16::try_from(x).unwrap(), y))
                })
                .unwrap_or_else(|| panic!("missing {needle} navigation label"))
        };
        let (agents_x, agents_y) = locate("Agents");
        let (skills_x, skills_y) = locate("Skills");

        let agents_style = buffer[(agents_x, agents_y)].style();
        let skills_style = buffer[(skills_x, skills_y)].style();
        assert!(
            !agents_style
                .add_modifier
                .contains(ratatui::style::Modifier::REVERSED),
            "{width}x{height} Agents retained focus style"
        );
        assert!(
            skills_style
                .add_modifier
                .contains(ratatui::style::Modifier::REVERSED),
            "{width}x{height} Skills lacked focus style"
        );
    }
}

#[test]
fn medium_assignment_review_reserves_complete_contextual_controls_and_identity() {
    let target = builtin(1_500, "Medium Target");
    let current = builtin(1_501, "Medium Current");
    let agent = profile_with_skills(vec![current.reference()]);

    for assignment in [
        AssignmentKind::Add,
        AssignmentKind::Upgrade {
            expected: current.reference(),
        },
        AssignmentKind::Unassign {
            expected: current.reference(),
        },
    ] {
        let mut model = skills_model(SkillsPane::AssignmentReview);
        model.skills.detail = Some(view(&target));
        model.skills.version_detail = Some(view(&target));
        model.skills.selected_agent_detail = Some(agent.clone());
        model.skills.assignment = Some(assignment.clone());
        let text = render_text(&model, 80, 28);

        for expected in [
            "Operation",
            "Target",
            "Enter: validate",
            "Esc: agent picker",
        ] {
            assert!(
                text.contains(expected),
                "{:?} missing {expected}",
                assignment
            );
        }
        let version_id = target.skill_version_id().to_string();
        assert!(text.contains(&version_id[..8]));
        assert!(text.contains(&version_id[version_id.len() - 8..]));
    }

    let mut already = skills_model(SkillsPane::AssignmentReview);
    already.skills.detail = Some(view(&target));
    already.skills.version_detail = Some(view(&target));
    already.skills.selected_agent_detail = Some(agent);
    already.skills.assignment = Some(AssignmentKind::AlreadyAssigned);
    let text = render_text(&already, 80, 28);
    assert!(text.contains("No operation"));
    assert!(text.contains("Esc: choose another version"));
    assert!(!text.contains("Enter: validate"));
}

#[test]
fn medium_assignment_confirmations_keep_enter_escape_and_exact_identity_visible() {
    let target = builtin(1_600, "Confirmation Target");
    let current = builtin(1_601, "Confirmation Current");
    let agent = profile_with_skills(vec![current.reference()]);
    let profile_id = agent.profile.profile_id();
    let profile_version_id = agent.profile.profile_version_id();
    let review_token = SkillReviewToken::from_uuid(Uuid::from_u128(9_600));
    let review_digest = sha256(b"medium-confirmation-review");
    let commands = [
        ApplicationCommand::AssignAgentSkill {
            profile_id,
            expected_active_profile_version_id: profile_version_id,
            skill: target.reference(),
            review_token,
            review_digest: review_digest.clone(),
        },
        ApplicationCommand::UpgradeAgentSkill {
            profile_id,
            expected_active_profile_version_id: profile_version_id,
            expected: current.reference(),
            replacement: target.reference(),
            review_token,
            review_digest: review_digest.clone(),
        },
        ApplicationCommand::UnassignAgentSkill {
            profile_id,
            expected_active_profile_version_id: profile_version_id,
            expected: current.reference(),
            review_token,
            review_digest,
        },
    ];

    for command in commands {
        let mut model = skills_model(SkillsPane::Confirmation);
        model.skills.pending_confirmation = Some(SkillConfirmation {
            command,
            origin: SkillOperationOrigin::Skills(SkillsPane::AssignmentReview),
        });
        let text = render_text(&model, 80, 28);
        assert!(text.contains("Enter: confirm"));
        assert!(text.contains("Esc: return"));
        let expected_ref = if text.contains("unassignment") {
            current.reference()
        } else {
            target.reference()
        };
        let version_id = expected_ref.skill_version_id().to_string();
        assert!(text.contains(&version_id[..8]));
        assert!(text.contains(&version_id[version_id.len() - 8..]));
    }
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
