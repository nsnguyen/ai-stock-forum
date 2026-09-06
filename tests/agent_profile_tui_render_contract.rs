use ai_stock_forum::{
    agents::{
        AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentReadiness, AgentRole,
        ProfileDiffField, ProfileEditPreview, ProfileFieldDiff, ProfileFieldValue,
        builtin_profile_templates,
    },
    app::{
        AgentProfileHistoryEntry, AgentProfileHistoryView, AgentProfileSummary,
        AgentProfileVersionView, AgentProfileView, AgentProfilesView, ApplicationCommand,
        DatabaseReadiness, PresentationSnapshot, ProcessGuardOwnership,
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
            layout::view_geometry,
            model::{AgentsPane, TuiModel, View},
            render,
            theme::Theme,
        },
    },
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
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
        let readiness = AgentReadiness::Unbound;
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
                total_count: 1,
                returned_count: 1,
                truncated: false,
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
                total_count: 1,
                returned_count: 1,
                truncated: false,
            }),
        )
    } else {
        (
            AgentProfilesView {
                profiles: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
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

fn render_rows(model: &TuiModel, width: u16, height: u16) -> Vec<String> {
    let terminal = rendered(model, width, height);
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
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

    let medium_low = render_text(&detail, 80, 18);
    assert!(medium_low.contains("Agent list"));
    assert!(medium_low.contains("Agent detail"));
    assert!(!medium_low.contains("Readiness & history"));

    let wide_low = render_text(&detail, 120, 18);
    assert!(wide_low.contains("Agent list"));
    assert!(wide_low.contains("Agent detail"));
    assert!(wide_low.contains("Readiness & history"));
}

#[test]
fn legacy_views_keep_height_aware_modes_at_low_supported_heights() {
    for view in [View::Overview, View::Setup, View::Audit, View::Help] {
        for width in [80, 120] {
            let mut legacy = model(false, AgentsPane::List);
            legacy.active_view = view;
            let text = render_text(&legacy, width, 18);
            assert!(text.contains("Narrow"), "view={view:?} width={width}");
            assert!(
                !text.contains(" Navigation "),
                "view={view:?} width={width}"
            );
        }
    }
}

#[test]
fn narrow_header_rows_are_complete_at_sixty_and_seventy_columns() {
    let model = model(true, AgentsPane::List);
    for width in [60, 70] {
        let rows = render_rows(&model, width, 18);
        assert_eq!(rows[0].trim_end(), "AI STOCK FORUM  /  Agents  /  Narrow");
        assert_eq!(rows[1].trim_end(), "Active 1  Ready 0  Not Ready 1");
        assert_eq!(
            rows[2].trim_end(),
            "1 Overview  2 Setup  3 Audit  4 Help  a Agents"
        );
        assert_eq!(rows[3], "-".repeat(usize::from(width)));
    }
}

#[test]
fn view_geometry_accounts_for_the_agents_header_height() {
    for width in [80, 120] {
        let area = Rect::new(0, 0, width, 18);
        let legacy = view_geometry(area, View::Overview, false);
        let agents = view_geometry(area, View::Agents, false);

        assert_eq!(legacy.cockpit.header.height, 3);
        assert_eq!(agents.cockpit.header.height, 4);
        assert_eq!(legacy.workspace_body_height, 9);
        assert_eq!(agents.workspace_body_height, 8);
        assert_eq!(
            legacy.workspace_body_height.saturating_sub(1),
            agents.workspace_body_height
        );
    }
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
        "long-duration, quality",
        "Active version",
        "Template",
        "builtin.bull",
        "Bindings",
        "Skill refs",
        "MCP refs",
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
fn list_scroll_is_an_item_offset_and_keeps_the_last_multiline_row_visible() {
    let mut model = model(true, AgentsPane::List);
    let base = model.agents.profiles.profiles[0].clone();
    model.agents.profiles.profiles = (0..12)
        .map(|index| {
            let mut profile = base.clone();
            profile.display_name = format!("Profile {index:02}");
            profile
        })
        .collect();
    model.agents.selected_profile = 11;
    model.agents.list_scroll = 11;

    let text = render_text(&model, 60, 18);
    assert!(text.contains("> Profile 11"));
    assert!(!text.contains("Profile 00"));
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
    assert!(confirmation.contains("Type exactly: activate"));
    assert!(confirmation.contains("Reviewed base"));
    assert!(confirmation.contains("Review digest"));
    assert!(confirmation.contains("Esc"));

    let mut create_editor =
        ProfileEditor::for_create(&builtin_profile_templates()[0]).expect("valid create editor");
    let mut create = model(false, AgentsPane::Editor);
    create.agents.editor = Some(create_editor.clone());
    let create_text = render_text(&create, 79, 24);
    assert!(create_text.contains("Step 1 of 7"));
    assert!(create_text.contains("Choose a template role"));

    for _ in 0..7 {
        assert_eq!(
            create_editor.submit_line(":next"),
            ProfileEditorEffect::None
        );
    }
    create.agents.editor = Some(create_editor);
    let create_review = render_text(&create, 100, 40);
    assert!(create_review.contains("Use :create"));
}

#[test]
fn long_safe_content_wraps_and_scrolls_while_escape_controls_never_reach_the_buffer() {
    let mut model = model(true, AgentsPane::Detail);
    model.agents.profiles.profiles[0].display_name =
        format!("\u{1b}[31m{}", "safe-long-name ".repeat(20));
    model.agents.detail_scroll = usize::MAX;

    for (width, height) in [(60, 18), (79, 24), (80, 24), (120, 30), (160, 44)] {
        let terminal = rendered(&model, width, height);
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| !cell.symbol().contains('\u{1b}'))
        );
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
    let text = render_text(&create, 100, 30);
    assert!(text.contains("Confirm Create"));
    assert!(text.contains("Type exactly: create"));
    assert!(text.contains("builtin.bull"));
    for chunk in builtin_profile_templates()[0]
        .digest
        .as_str()
        .as_bytes()
        .chunks(16)
    {
        let chunk = std::str::from_utf8(chunk).expect("digest chunks are UTF-8");
        assert!(text.contains(chunk), "missing digest chunk {chunk}");
    }
}

#[test]
fn selected_historical_version_renders_full_content_metadata_and_predecessor_diff() {
    let first = profile();
    let mut candidate = first.to_draft();
    candidate.description = "Second historical prose.".to_owned();
    let second = AgentProfileVersion::next_version(
        &first,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(13)),
        1_800_000_000_001,
        candidate,
    )
    .unwrap();
    let mut model = model(true, AgentsPane::History);
    model.agents.history = Some(AgentProfileHistoryView {
        profile_id: first.profile_id(),
        active_version_id: second.profile_version_id(),
        versions: vec![
            AgentProfileHistoryEntry {
                profile_version_id: second.profile_version_id(),
                version: second.version(),
                supersedes: second.supersedes(),
                created_at_ms: second.created_at_ms(),
                readiness: AgentReadiness::Unbound,
                content_digest: second.content_digest().clone(),
            },
            AgentProfileHistoryEntry {
                profile_version_id: first.profile_version_id(),
                version: first.version(),
                supersedes: first.supersedes(),
                created_at_ms: first.created_at_ms(),
                readiness: AgentReadiness::Unbound,
                content_digest: first.content_digest().clone(),
            },
        ],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    model.agents.selected_history_version = 0;
    model.agents.version_detail = Some(AgentProfileVersionView {
        profile: second.clone(),
        readiness: AgentReadiness::Unbound,
        predecessor_diff: vec![ProfileFieldDiff {
            field: ProfileDiffField::Description,
            before: ProfileFieldValue::Text(first.description().to_owned()),
            after: ProfileFieldValue::Text(second.description().to_owned()),
        }],
    });

    let rendered = render_text(&model, 79, 70);
    for expected in [
        "HISTORICAL VERSION",
        "Historical version",
        "Second historical prose.",
        "Template provenance",
        "Memory",
        "Policy",
        second.content_digest().as_str(),
        "PREDECESSOR DIFF",
        "Description",
        "Before",
        "After",
    ] {
        assert!(rendered.contains(expected), "missing {expected:?}");
    }
}
