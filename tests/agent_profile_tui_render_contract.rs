use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentReadiness, AgentRole,
        ProfileDiffField, ProfileEditPreview, ProfileFieldDiff, ProfileFieldValue,
        builtin_profile_templates,
    },
    app::{
        AgentProfileHistoryEntry, AgentProfileHistoryView, AgentProfileSummary, AgentProfileView,
        AgentProfilesView, ApplicationCommand, DatabaseReadiness, PresentationSnapshot,
        ProcessGuardOwnership,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, InstallationId, MemoryNamespaceId,
        ProfileReviewToken, SessionId, sha256,
    },
    setup::SetupStatus,
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorEffect},
        tui::{
            ProfileConfirmation,
            model::{AgentsPane, TuiModel, View},
            render,
            theme::Theme,
        },
    },
};
use ratatui::{Terminal, backend::TestBackend};
use uuid::Uuid;

fn profile() -> AgentProfileVersion {
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().expect("valid builtin template");
    draft.display_name = "Long Horizon Analyst".to_owned();
    draft.description = "Builds patient, evidence-led theses.".to_owned();
    draft.primary_specialty = "fundamental compounders".to_owned();
    draft.specialty_tags = vec!["quality".to_owned(), "long-duration".to_owned()];
    draft.personality = "Patient, skeptical, and explicit about uncertainty.".to_owned();
    draft.instructions = "Separate facts from assumptions and cite primary evidence.".to_owned();
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(10)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(11)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(12)),
        1_800_000_000_000,
        draft,
        Some(template.provenance()),
    )
    .expect("valid profile")
}

fn snapshot(with_profile: bool) -> PresentationSnapshot {
    let (agent_profiles, selected_agent_profile, selected_agent_profile_history) = if with_profile {
        let profile = profile();
        let readiness = AgentReadiness::NotReady;
        (
            AgentProfilesView {
                profiles: vec![AgentProfileSummary {
                    profile_id: profile.profile_id(),
                    profile_version_id: profile.profile_version_id(),
                    version: profile.version(),
                    display_name: profile.display_name().to_owned(),
                    role: profile.role(),
                    primary_specialty: profile.primary_specialty().to_owned(),
                    readiness,
                    content_digest: profile.content_digest().clone(),
                }],
            },
            Some(AgentProfileView {
                profile: profile.clone(),
                readiness,
            }),
            Some(AgentProfileHistoryView {
                profile_id: profile.profile_id(),
                active_version_id: profile.profile_version_id(),
                versions: vec![AgentProfileHistoryEntry {
                    profile_version_id: profile.profile_version_id(),
                    version: profile.version(),
                    supersedes: profile.supersedes(),
                    created_at_ms: profile.created_at_ms(),
                    readiness,
                    content_digest: profile.content_digest().clone(),
                }],
            }),
        )
    } else {
        (
            AgentProfilesView {
                profiles: Vec::new(),
            },
            None,
            None,
        )
    };

    PresentationSnapshot {
        installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
        session_id: SessionId::from_uuid(Uuid::from_u128(2)),
        database_readiness: DatabaseReadiness::Ready,
        process_guard_ownership: ProcessGuardOwnership::Held,
        setup_status: SetupStatus::NotStarted,
        recent_audit: Vec::new(),
        agent_profiles,
        selected_agent_profile,
        selected_agent_profile_history,
    }
}

fn model(with_profile: bool, pane: AgentsPane) -> TuiModel {
    let mut model = TuiModel::new(snapshot(with_profile), false);
    model.active_view = View::Agents;
    model.agents.pane = pane;
    model
}

fn rendered(model: &TuiModel, width: u16, height: u16) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(false)))
        .expect("agents render remains total");
    terminal
}

fn render_text(model: &TuiModel, width: u16, height: u16) -> String {
    rendered(model, width, height)
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[test]
fn agents_layout_uses_one_two_and_three_panes_at_exact_width_breakpoints() {
    let list = model(true, AgentsPane::List);
    let narrow_list = render_text(&list, 79, 24);
    assert!(narrow_list.contains("Agent list"));
    assert!(!narrow_list.contains("Agent detail"));

    let detail = model(true, AgentsPane::Detail);
    let narrow_detail = render_text(&detail, 79, 24);
    assert!(!narrow_detail.contains("Agent list"));
    assert!(narrow_detail.contains("Agent detail"));

    let medium = render_text(&detail, 80, 24);
    assert!(medium.contains("Agent list"));
    assert!(medium.contains("Agent detail"));
    assert!(!medium.contains("Readiness & history"));

    let wide = render_text(&detail, 120, 30);
    assert!(wide.contains("Agent list"));
    assert!(wide.contains("Agent detail"));
    assert!(wide.contains("Readiness & history"));
}

#[test]
fn agents_empty_and_populated_states_render_counts_readiness_and_complete_metadata() {
    let empty = render_text(&model(false, AgentsPane::List), 100, 30);
    assert!(empty.contains("No agent profiles yet"));
    assert!(empty.contains("Press c to create your first profile"));
    assert!(empty.contains("Active 0"));

    let populated = render_text(&model(true, AgentsPane::Detail), 160, 44);
    for expected in [
        "Active 1",
        "Ready 0",
        "Not Ready 1",
        "Long Horizon Analyst",
        "bull",
        "fundamental compounders",
        "quality, long-duration",
        "Active version",
        "Template",
        "builtin.bull",
        "Bindings",
        "Created ms",
        "Memory",
        "Policy",
        "Digest",
    ] {
        assert!(populated.contains(expected), "missing {expected:?}");
    }
    assert!(!populated.contains("Failed"));
    assert!(!populated.contains("Error: Not Ready"));
}

#[test]
fn editor_renders_progress_guidance_ordered_review_diffs_and_explicit_confirmation() {
    let profile = profile();
    let draft = AgentProfileDraft::new(
        "Revised Horizon Analyst".to_owned(),
        profile.description().to_owned(),
        AgentRole::Chief,
        profile.primary_specialty().to_owned(),
        profile.specialty_tags().to_vec(),
        profile.personality().to_owned(),
        profile.instructions().to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .expect("valid edit draft");
    let mut editor = ProfileEditor::for_edit(
        profile.profile_id(),
        profile.profile_version_id(),
        draft.clone(),
    );
    for _ in 0..7 {
        assert_eq!(editor.submit_line(":next"), ProfileEditorEffect::None);
    }
    let ProfileEditorEffect::PreviewEdit(request) = editor.submit_line(":review") else {
        panic!("review request")
    };
    editor.apply_preview(
        request.generation,
        ProfileEditPreview {
            profile_id: request.profile_id,
            expected_active_version_id: request.expected_active_version_id,
            diffs: vec![
                ProfileFieldDiff {
                    field: ProfileDiffField::DisplayName,
                    before: ProfileFieldValue::Text("Long Horizon Analyst".to_owned()),
                    after: ProfileFieldValue::Text("Revised Horizon Analyst".to_owned()),
                },
                ProfileFieldDiff {
                    field: ProfileDiffField::Role,
                    before: ProfileFieldValue::Role(AgentRole::Bull),
                    after: ProfileFieldValue::Role(AgentRole::Chief),
                },
            ],
            review_token: ProfileReviewToken::from_uuid(Uuid::from_u128(20)),
            review_digest: sha256(b"review"),
        },
    );

    let mut editor_model = model(true, AgentsPane::Editor);
    editor_model.agents.editor = Some(editor.clone());
    let review = render_text(&editor_model, 100, 30);
    assert!(review.contains("Step 7 of 7"));
    assert!(review.contains("Review"));
    assert!(review.contains("Display name"));
    assert!(review.contains("Before"));
    assert!(review.contains("After"));
    assert!(
        review.find("Display name").expect("display diff")
            < review.find("Role").expect("role diff")
    );

    let ProfileEditorEffect::Execute(command) = editor.submit_line(":activate") else {
        panic!("activation command")
    };
    let mut confirmation = model(true, AgentsPane::Confirmation);
    confirmation.agents.editor = Some(editor);
    confirmation.agents.pending_confirmation = Some(ProfileConfirmation { command });
    let confirmation = render_text(&confirmation, 100, 30);
    assert!(confirmation.contains("Confirm Activate"));
    assert!(confirmation.contains("Enter"));
    assert!(confirmation.contains("Esc"));

    let mut create = model(false, AgentsPane::Editor);
    create.agents.editor = Some(
        ProfileEditor::for_create(&builtin_profile_templates()[0]).expect("valid create editor"),
    );
    let create = render_text(&create, 79, 24);
    assert!(create.contains("Step 1 of 7"));
    assert!(create.contains("Choose a template role"));
}

#[test]
fn long_safe_content_wraps_and_scrolls_while_escape_controls_never_reach_the_buffer() {
    let mut model = model(true, AgentsPane::Detail);
    model.agents.profiles.profiles[0].display_name =
        format!("\u{1b}[31m{}", "safe-long-name ".repeat(20));
    model.agents.detail_scroll = usize::MAX;

    for (width, height) in [(60, 18), (79, 24), (80, 24), (120, 30), (160, 44)] {
        let terminal = rendered(&model, width, height);
        assert!(terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .all(|cell| !cell.symbol().contains('\u{1b}')));
    }

    let tiny = render_text(&model, 59, 18);
    assert!(tiny.contains("Terminal too small"));
    assert!(tiny.contains("Minimum: 60 x 18"));
}

#[test]
fn confirmation_distinguishes_create_from_activate() {
    let mut create = model(false, AgentsPane::Confirmation);
    create.agents.pending_confirmation = Some(ProfileConfirmation {
        command: ApplicationCommand::CreateAgentProfile {
            draft: builtin_profile_templates()[0]
                .copy_to_draft()
                .expect("valid create draft"),
            template_provenance: Some(builtin_profile_templates()[0].provenance()),
        },
    });
    assert!(render_text(&create, 100, 30).contains("Confirm Create"));
}
