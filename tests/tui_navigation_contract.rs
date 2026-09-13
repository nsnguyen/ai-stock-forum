use ai_stock_forum::{
    agents::{AgentProfileVersion, AgentReadiness, builtin_profile_templates},
    app::{
        AgentProfileSelector, AgentProfileSummary, AgentProfileView, AgentProfilesView,
        ApplicationCommand, CommandOutcome, CommandView, DatabaseReadiness, MemoryEditPreview,
        MemoryEntriesView, MemoryProfileIdentityView, MemoryProposalResolutionReview,
        MemoryProposalSummary, MemoryProposalView, MemoryProposalsView, PresentationSnapshot,
        ProcessGuardOwnership, ShutdownDisposition, SkillsView, StatusView,
    },
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CommandId, CorrelationId,
        EventId, InstallationId, MemoryNamespaceId, MemoryProposalId, MemoryReviewToken, SessionId,
        SkillId, sha256,
    },
    memory::{
        ExpectedMemoryEntryState, MemoryEditReview, MemoryEntryDraft, MemoryField, MemoryFieldDiff,
        MemoryFieldValue, MemoryMutationKind, MemoryPlaintextAcknowledgement, MemoryProposal,
        MemoryProposalFilter, MemoryProposalOperation, MemoryProposalOperationKind,
        MemoryProposalStatus, MemoryResolutionAction,
    },
    policy::ApprovalStatus,
    setup::SetupStatus,
    ui::tui::{
        ControllerEffect, TuiEvent, apply_outcome, handle_event,
        layout::view_geometry_for_state,
        model::{
            AgentsPane, Focus, LayoutMode, MemoryConfirmation, MemoryPane,
            MemoryProposalDetailAction, ProfileConfirmation, SkillConfirmation,
            SkillOperationOrigin, SkillsPane, TuiModel, View,
        },
        render,
        theme::Theme,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
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

fn modified_character(character: char, modifiers: KeyModifiers) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(KeyCode::Char(character), modifiers))
}

fn plain_character(character: char) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
}

fn plain_key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn status_outcome(installation_id: InstallationId, session_id: SessionId) -> CommandOutcome {
    CommandOutcome {
        command_id: CommandId::from_uuid(Uuid::from_u128(3)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(4)),
        committed_events: Vec::new(),
        view: CommandView::Status(StatusView {
            installation_id,
            session_id,
        }),
        shutdown: ShutdownDisposition::Continue,
    }
}

fn memory_profile(seed: u128) -> AgentProfileVersion {
    let template = &builtin_profile_templates()[0];
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(seed)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(seed + 2)),
        1_800_000_000_000,
        template.copy_to_draft().expect("builtin profile draft"),
        Some(template.provenance()),
    )
    .expect("memory profile")
}

fn memory_identity(profile: &AgentProfileVersion) -> MemoryProfileIdentityView {
    MemoryProfileIdentityView {
        profile: profile.reference(),
        display_name: profile.display_name().to_owned(),
    }
}

fn memory_profile_summary(profile: &AgentProfileVersion) -> AgentProfileSummary {
    AgentProfileSummary {
        profile_id: profile.profile_id(),
        profile_version_id: profile.profile_version_id(),
        version: profile.version(),
        display_name: profile.display_name().to_owned(),
        role: profile.role(),
        primary_specialty: profile.primary_specialty().to_owned(),
        readiness: AgentReadiness::Unbound,
        content_digest: profile.content_digest().clone(),
    }
}

fn memory_model(profile: &AgentProfileVersion) -> TuiModel {
    let mut model = model();
    model.select_view(View::Agents);
    model.skills.library_loaded = true;
    model.agents.profiles = AgentProfilesView {
        profiles: vec![memory_profile_summary(profile)],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };
    model.agents.detail = Some(AgentProfileView {
        profile: profile.clone(),
        readiness: AgentReadiness::Unbound,
    });
    model.agents.pane = AgentsPane::Memory;
    model
        .agents
        .memory
        .bind_profile(memory_identity(profile), profile.memory_namespace_id())
        .expect("bind memory profile");
    model.agents.memory.entries = Some(MemoryEntriesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        entries: Vec::new(),
        total_count: 0,
        returned_count: 0,
        omitted_count: 0,
    });
    model.set_terminal_size(140, 30);
    model
}

fn absent_set_diff(candidate: &MemoryEntryDraft) -> Vec<MemoryFieldDiff> {
    vec![
        MemoryFieldDiff {
            field: MemoryField::DisplayKey,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Text(candidate.display_key().to_owned()),
        },
        MemoryFieldDiff {
            field: MemoryField::State,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Present),
        },
        MemoryFieldDiff {
            field: MemoryField::Value,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Text(candidate.value().to_owned()),
        },
        MemoryFieldDiff {
            field: MemoryField::PurposeTags,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
        },
    ]
}

const MEMORY_NAVIGATION_PROSE_SENTINELS: [&str; 6] =
    ["NK701X", "NV702X", "NT703X", "PK704X", "PV705X", "PR706X"];

fn memory_set_review(profile: &AgentProfileVersion, seed: u128) -> MemoryEditReview {
    let candidate = MemoryEntryDraft::new(
        MEMORY_NAVIGATION_PROSE_SENTINELS[0].to_owned(),
        MEMORY_NAVIGATION_PROSE_SENTINELS[1].to_owned(),
        vec![MEMORY_NAVIGATION_PROSE_SENTINELS[2].to_owned()],
    )
    .expect("memory candidate");
    MemoryEditReview {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        expected: ExpectedMemoryEntryState::Absent,
        operation: MemoryMutationKind::Set,
        candidate: Some(candidate.clone()),
        diff: absent_set_diff(&candidate),
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed)),
        review_digest: sha256(format!("navigation review {seed}").as_bytes()),
    }
}

fn memory_set_command(review: &MemoryEditReview) -> ApplicationCommand {
    ApplicationCommand::SetMemoryEntry {
        profile: review.profile.clone(),
        expected: review.expected.clone(),
        candidate: review.candidate.clone().expect("set candidate"),
        review_token: review.review_token,
        review_digest: review.review_digest.clone(),
    }
}

fn install_memory_set_review(model: &mut TuiModel, review: &MemoryEditReview) {
    let candidate = review.candidate.as_ref().expect("set candidate");
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(review.profile.profile_id()))
        .expect("open memory editor");
    let editor = model.agents.memory.editor.as_mut().expect("memory editor");
    editor
        .submit_line(candidate.display_key().to_owned())
        .expect("submit memory key");
    editor
        .submit_line(candidate.value().to_owned())
        .expect("submit memory value");
    editor
        .submit_line(candidate.purpose_tags().join(", "))
        .expect("submit memory tags");
    model
        .agents
        .memory
        .begin_review_request()
        .expect("begin memory review");
    let editor = model.agents.memory.editor.as_mut().expect("memory editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    ));
    model.agents.memory.edit_review = Some(review.clone());
    model.agents.memory.review_registered = true;
    model.agents.memory.pane = MemoryPane::MutationReview;
}

fn memory_proposal(profile: &AgentProfileVersion, seed: u128) -> MemoryProposal {
    MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(seed)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                MEMORY_NAVIGATION_PROSE_SENTINELS[3].to_owned(),
                MEMORY_NAVIGATION_PROSE_SENTINELS[4].to_owned(),
                vec![MEMORY_NAVIGATION_PROSE_SENTINELS[2].to_owned()],
            )
            .expect("proposal candidate"),
        },
        MEMORY_NAVIGATION_PROSE_SENTINELS[3].to_owned(),
        ExpectedMemoryEntryState::Absent,
        MEMORY_NAVIGATION_PROSE_SENTINELS[5].to_owned(),
        1_800_000_000_100,
        EventId::from_uuid(Uuid::from_u128(seed + 1)),
        ApprovalId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .expect("memory proposal")
}

fn install_memory_resolution_review(
    model: &mut TuiModel,
    profile: &AgentProfileVersion,
    seed: u128,
) {
    let proposal = memory_proposal(profile, seed);
    model.agents.memory.proposals = Some(MemoryProposalsView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        filter: MemoryProposalFilter::Pending,
        proposals: vec![MemoryProposalSummary {
            proposal: proposal.reference(),
            namespace_id: proposal.namespace_id(),
            proposer: proposal.proposer().clone(),
            operation: MemoryProposalOperationKind::Set,
            display_key: proposal.display_key().to_owned(),
            status: MemoryProposalStatus::Pending,
            created_at_ms: proposal.created_at_ms(),
        }],
        total_count: 1,
        returned_count: 1,
        omitted_count: 0,
    });
    model.agents.memory.proposal_detail = Some(MemoryProposalView {
        proposal: proposal.clone(),
        status: MemoryProposalStatus::Pending,
        resolution: None,
        current_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: false,
        proposer_identity: memory_identity(profile),
        namespace_owner_identity: memory_identity(profile),
    });
    model.agents.memory.resolution_review = Some(MemoryProposalResolutionReview {
        action: MemoryResolutionAction::Approve,
        approval_id: proposal.approval_id(),
        proposal,
        expected_approval_status: ApprovalStatus::Pending,
        expected_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: false,
        proposer_identity: memory_identity(profile),
        namespace_owner_identity: memory_identity(profile),
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed + 3)),
        review_digest: sha256(format!("navigation resolution review {seed}").as_bytes()),
    });
    model.agents.memory.selected_proposal_detail_action = MemoryProposalDetailAction::Approve;
    model.agents.memory.review_registered = true;
    model.agents.memory.pane = MemoryPane::ProposalResolutionReview;
}

fn authentic_memory_non_text_models() -> Vec<(&'static str, TuiModel)> {
    let profile = memory_profile(50_000);
    let entry_list = memory_model(&profile);

    let review = memory_set_review(&profile, 50_100);
    let mut editor_at_review = memory_model(&profile);
    install_memory_set_review(&mut editor_at_review, &review);
    editor_at_review.agents.memory.pane = MemoryPane::Editor;

    let mut mutation_review = editor_at_review.clone();
    mutation_review.agents.memory.pane = MemoryPane::MutationReview;

    let mut resolution_review = memory_model(&profile);
    install_memory_resolution_review(&mut resolution_review, &profile, 50_200);

    let mut confirmation = mutation_review.clone();
    confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
        command: memory_set_command(&review),
        generation: confirmation.agents.memory.generation,
    });
    confirmation.agents.memory.pane = MemoryPane::Confirmation;

    vec![
        ("entry-list", entry_list),
        ("editor-at-review", editor_at_review),
        ("mutation-review", mutation_review),
        ("proposal-resolution-review", resolution_review),
        ("confirmation", confirmation),
    ]
}

fn authentic_memory_text_editor_models() -> Vec<(&'static str, TuiModel)> {
    let profile = memory_profile(51_000);
    let mut key = memory_model(&profile);
    key.agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(profile.profile_id()))
        .expect("open key editor");

    let mut value = key.clone();
    value
        .agents
        .memory
        .editor
        .as_mut()
        .expect("value editor")
        .submit_keyboard_line("navigation key")
        .expect("submit key");

    let mut purpose_tags = value.clone();
    purpose_tags
        .agents
        .memory
        .editor
        .as_mut()
        .expect("purpose-tags editor")
        .submit_line("navigation value".to_owned())
        .expect("submit value");

    vec![
        ("key", key),
        ("value", value),
        ("purpose-tags", purpose_tags),
    ]
}

fn rendered_memory_navigation_labels(model: &TuiModel) -> Vec<String> {
    let width = 140;
    let height = 30;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .expect("render Memory navigation");
    let navigation = view_geometry_for_state(
        Rect::new(0, 0, width, height),
        model.active_view,
        model.inspector_open,
        true,
    )
    .cockpit
    .navigation
    .expect("navigation region");
    let rows = terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>();
    let inner_rows = rows
        .iter()
        .skip(usize::from(navigation.y.saturating_add(1)))
        .take(usize::from(navigation.height.saturating_sub(2)))
        .map(|row| {
            row.chars()
                .skip(usize::from(navigation.x.saturating_add(1)))
                .take(usize::from(navigation.width.saturating_sub(2)))
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let views_row = inner_rows
        .iter()
        .position(|row| row.trim() == "VIEWS")
        .expect("VIEWS heading");
    let entire_navigation = inner_rows.join("\n");
    assert!(
        MEMORY_NAVIGATION_PROSE_SENTINELS
            .iter()
            .all(|sentinel| !entire_navigation.contains(sentinel)),
        "navigation pane exposed Memory prose"
    );
    let mut labels = Vec::new();
    let mut started = false;
    for row in inner_rows.iter().skip(views_row + 1) {
        let row = row.trim();
        if row.is_empty() {
            if started {
                break;
            }
            continue;
        }
        started = true;
        labels.push(row.strip_prefix("> ").unwrap_or(row).to_owned());
    }
    labels
}

#[test]
fn bare_shortcuts_switch_every_tab_from_non_text_workspaces() {
    let mut model = model();
    model.skills.active = true;
    model.skills.library_loaded = true;
    model.skills.pane = SkillsPane::History;
    model.skills.selected_history_version = 3;
    model.set_focus(Focus::Workspace);

    assert_eq!(
        handle_event(&mut model, plain_character('1')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Overview);
    assert!(!model.skills.active);

    assert_eq!(
        handle_event(&mut model, plain_character('s')),
        ControllerEffect::Redraw
    );
    assert!(model.skills.active);
    assert_eq!(model.skills.pane, SkillsPane::History);
    assert_eq!(model.skills.selected_history_version, 3);

    for (shortcut, view) in [
        ('2', View::Setup),
        ('a', View::Agents),
        ('3', View::Audit),
        ('4', View::Help),
    ] {
        assert_eq!(
            handle_event(&mut model, plain_character(shortcut)),
            ControllerEffect::Redraw,
            "shortcut={shortcut}"
        );
        assert_eq!(model.active_view, view, "shortcut={shortcut}");
        assert!(!model.skills.active, "shortcut={shortcut}");
    }
}

#[test]
fn every_bare_shortcut_works_from_every_tab_and_non_text_focus_region() {
    let sources = [
        Some(View::Overview),
        Some(View::Setup),
        Some(View::Audit),
        Some(View::Help),
        Some(View::Agents),
        None,
    ];
    let targets = [
        ('1', Some(View::Overview)),
        ('2', Some(View::Setup)),
        ('3', Some(View::Audit)),
        ('4', Some(View::Help)),
        ('a', Some(View::Agents)),
        ('s', None),
    ];

    for source in sources {
        for focus in [Focus::Workspace, Focus::Navigation, Focus::Inspector] {
            for (shortcut, target) in targets {
                let mut model = model();
                model.skills.library_loaded = true;
                if let Some(view) = source {
                    model.select_view(view);
                } else {
                    assert_eq!(
                        handle_event(&mut model, plain_character('s')),
                        ControllerEffect::Redraw
                    );
                }
                if focus == Focus::Inspector {
                    model.inspector_open = true;
                }
                model.set_focus(focus);

                assert_eq!(
                    handle_event(&mut model, plain_character(shortcut)),
                    ControllerEffect::Redraw,
                    "source={source:?} focus={focus:?} shortcut={shortcut}"
                );
                if let Some(view) = target {
                    assert_eq!(
                        model.active_view, view,
                        "source={source:?} focus={focus:?} shortcut={shortcut}"
                    );
                    assert!(!model.skills.active);
                } else {
                    assert!(model.skills.active);
                }
            }
        }
    }
}

#[test]
fn option_alt_shortcuts_are_not_navigation_fallbacks() {
    for character in ['1', '2', '3', '4', '5', '6', 'a', 's'] {
        let mut model = model();
        let before = model.clone();

        assert_eq!(
            handle_event(&mut model, modified_character(character, KeyModifiers::ALT),),
            ControllerEffect::None,
            "character={character}"
        );
        assert_eq!(model, before, "character={character}");
    }
}

#[test]
fn unassigned_bare_keys_and_function_keys_never_switch_tabs() {
    for character in ['5', '6', '7', 'm', 'q'] {
        let mut model = model();
        let before = model.clone();

        assert_eq!(
            handle_event(&mut model, plain_character(character)),
            ControllerEffect::None,
            "character={character}"
        );
        assert_eq!(model, before, "character={character}");
    }

    for function_key in 1..=6 {
        let mut model = model();
        let before = model.clone();

        assert_eq!(
            handle_event(
                &mut model,
                TuiEvent::Key(KeyEvent::new(KeyCode::F(function_key), KeyModifiers::NONE)),
            ),
            ControllerEffect::None,
            "function_key=F{function_key}"
        );
        assert_eq!(model, before, "function_key=F{function_key}");
    }
}

#[test]
fn bare_shortcuts_work_from_confirmations_and_preserve_pending_actions() {
    let mut profile_model = model();
    profile_model.select_view(View::Agents);
    profile_model.skills.library_loaded = true;
    profile_model.agents.pane = AgentsPane::Confirmation;
    profile_model.agents.pending_confirmation = Some(ProfileConfirmation {
        command: ApplicationCommand::ShowHelp,
    });
    let expected_profile_confirmation = profile_model.agents.pending_confirmation.clone();

    assert_eq!(
        handle_event(&mut profile_model, plain_character('s')),
        ControllerEffect::Redraw
    );
    assert!(profile_model.skills.active);
    assert_eq!(
        profile_model.agents.pending_confirmation,
        expected_profile_confirmation
    );

    let mut skill_model = model();
    skill_model.skills.active = true;
    skill_model.skills.library_loaded = true;
    skill_model.skills.pane = SkillsPane::Confirmation;
    skill_model.skills.pending_confirmation = Some(SkillConfirmation {
        command: ApplicationCommand::ShowHelp,
        origin: SkillOperationOrigin::Skills(SkillsPane::Detail),
    });
    let expected_skill_confirmation = skill_model.skills.pending_confirmation.clone();

    assert_eq!(
        handle_event(&mut skill_model, plain_character('2')),
        ControllerEffect::Redraw
    );
    assert_eq!(skill_model.active_view, View::Setup);
    assert!(!skill_model.skills.active);
    assert_eq!(
        skill_model.skills.pending_confirmation,
        expected_skill_confirmation
    );
}

#[test]
fn bare_shortcut_characters_remain_text_for_each_active_text_owner() {
    let characters = "1234as";

    let mut command_model = model();
    command_model.set_focus(Focus::Command);
    for character in characters.chars() {
        assert_eq!(
            handle_event(&mut command_model, plain_character(character)),
            ControllerEffect::Redraw,
            "command character={character}"
        );
    }
    assert_eq!(command_model.active_view, View::Overview);
    assert_eq!(command_model.command.text(), characters);

    let mut profile_model = model();
    profile_model.select_view(View::Agents);
    assert!(
        profile_model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );
    for character in characters.chars() {
        assert_eq!(
            handle_event(&mut profile_model, plain_character(character)),
            ControllerEffect::Redraw,
            "profile character={character}"
        );
    }
    assert_eq!(profile_model.active_view, View::Agents);
    assert!(!profile_model.skills.active);
    assert_eq!(profile_model.agents.pane, AgentsPane::Editor);
    assert_eq!(profile_model.command.text(), characters);

    let mut skill_model = model();
    skill_model.skills.active = true;
    skill_model.skills.library_loaded = true;
    skill_model.skills.start_create(None);
    for character in characters.chars() {
        assert_eq!(
            handle_event(&mut skill_model, plain_character(character)),
            ControllerEffect::Redraw,
            "skill character={character}"
        );
    }
    assert!(skill_model.skills.active);
    assert_eq!(skill_model.skills.pane, SkillsPane::Editor);
    assert_eq!(skill_model.command.text(), characters);
}

#[test]
fn switching_tabs_restores_each_tabs_focus_scroll_and_unsubmitted_input() {
    let mut model = model();
    model.set_terminal_size(60, 18);
    model.set_focus(Focus::Command);
    model.inspector_open = true;
    model.command.ingest("overview draft");
    model.command.move_left();
    model.command.move_left();
    let overview_cursor = model.command.cursor_byte();
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Workspace);

    assert_eq!(
        handle_event(&mut model, plain_character('4')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Help);
    assert_eq!(model.focus, Focus::Workspace);
    assert!(!model.inspector_open);
    assert_eq!(model.workspace_scroll, 0);
    assert_eq!(model.command.text(), "");

    model.set_focus(Focus::Command);
    model.workspace_scroll = 7;
    model.command.ingest("help draft");
    model.command.move_home();
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Workspace);

    assert_eq!(
        handle_event(&mut model, plain_character('1')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Overview);
    assert_eq!(model.focus, Focus::Workspace);
    assert!(model.inspector_open);
    assert_eq!(model.workspace_scroll, 0);
    assert_eq!(model.command.text(), "overview draft");
    assert_eq!(model.command.cursor_byte(), overview_cursor);

    assert_eq!(
        handle_event(&mut model, plain_character('4')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Help);
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.workspace_scroll, 7);
    assert_eq!(model.command.text(), "help draft");
    assert_eq!(model.command.cursor_byte(), 0);
}

#[test]
fn switching_away_from_a_profile_editor_preserves_its_exact_draft_and_input() {
    let mut model = model();
    model.select_view(View::Agents);
    model.skills.library_loaded = true;
    assert!(
        model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );
    model.command.ingest("Draft Analyst");
    let expected_editor = model.agents.editor.clone();

    model.select_view(View::Setup);
    assert_eq!(model.active_view, View::Setup);
    assert!(!model.skills.active);
    assert_eq!(model.command.text(), "");
    assert_eq!(
        handle_event(&mut model, plain_character('x')),
        ControllerEffect::None
    );
    assert_eq!(model.command.text(), "");
    assert_eq!(model.agents.editor, expected_editor);

    assert_eq!(
        handle_event(&mut model, plain_character('a')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(model.agents.pane, AgentsPane::Editor);
    assert_eq!(model.agents.editor, expected_editor);
    assert_eq!(model.command.text(), "Draft Analyst");
}

#[test]
fn switching_away_from_a_skill_editor_preserves_its_exact_draft_and_input() {
    let mut model = model();
    model.skills.active = true;
    model.skills.library_loaded = true;
    model.skills.start_create(None);
    model.command.ingest("Draft Skill");
    let expected_editor = model.skills.editor.clone();

    model.select_view(View::Setup);
    assert_eq!(model.active_view, View::Setup);
    assert!(!model.skills.active);
    assert_eq!(model.command.text(), "");

    assert_eq!(
        handle_event(&mut model, plain_character('s')),
        ControllerEffect::Redraw
    );
    assert!(model.skills.active);
    assert_eq!(model.skills.pane, SkillsPane::Editor);
    assert_eq!(model.skills.editor, expected_editor);
    assert_eq!(model.command.text(), "Draft Skill");
}

#[test]
fn switching_away_from_confirmations_keeps_the_exact_pending_actions() {
    let mut profile_model = model();
    profile_model.select_view(View::Agents);
    profile_model.skills.library_loaded = true;
    profile_model.agents.pane = AgentsPane::Confirmation;
    profile_model.agents.pending_confirmation = Some(ProfileConfirmation {
        command: ApplicationCommand::ShowHelp,
    });
    let expected_confirmation = profile_model.agents.pending_confirmation.clone();

    assert_eq!(
        handle_event(&mut profile_model, plain_character('s')),
        ControllerEffect::Redraw
    );
    assert!(profile_model.skills.active);
    assert_eq!(
        handle_event(&mut profile_model, plain_character('c')),
        ControllerEffect::Redraw
    );
    assert_eq!(profile_model.skills.pane, SkillsPane::CreateSource);
    assert_eq!(
        profile_model.agents.pending_confirmation,
        expected_confirmation
    );

    assert_eq!(
        handle_event(&mut profile_model, plain_character('a')),
        ControllerEffect::Redraw
    );
    assert_eq!(profile_model.active_view, View::Agents);
    assert_eq!(profile_model.agents.pane, AgentsPane::Confirmation);
    assert_eq!(
        profile_model.agents.pending_confirmation,
        expected_confirmation
    );

    let mut skill_model = model();
    skill_model.skills.active = true;
    skill_model.skills.library_loaded = true;
    skill_model.skills.pane = SkillsPane::Confirmation;
    skill_model.skills.pending_confirmation = Some(SkillConfirmation {
        command: ApplicationCommand::ShowHelp,
        origin: SkillOperationOrigin::Skills(SkillsPane::Detail),
    });
    let expected_skill_confirmation = skill_model.skills.pending_confirmation.clone();

    assert_eq!(
        handle_event(&mut skill_model, plain_character('2')),
        ControllerEffect::Redraw
    );
    assert_eq!(skill_model.active_view, View::Setup);
    assert_eq!(
        skill_model.skills.pending_confirmation,
        expected_skill_confirmation
    );
    assert_eq!(
        handle_event(
            &mut skill_model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        ),
        ControllerEffect::None
    );
    assert_eq!(skill_model.active_view, View::Setup);
    assert_eq!(
        skill_model.skills.pending_confirmation,
        expected_skill_confirmation
    );

    assert_eq!(
        handle_event(&mut skill_model, plain_character('s')),
        ControllerEffect::Redraw
    );
    assert!(skill_model.skills.active);
    assert_eq!(skill_model.skills.pane, SkillsPane::Confirmation);
    assert_eq!(
        skill_model.skills.pending_confirmation,
        expected_skill_confirmation
    );
}

#[test]
fn escape_from_skills_restores_the_originating_tabs_saved_state() {
    let mut model = model();
    model.select_view(View::Agents);
    model.skills.library_loaded = true;
    model.set_terminal_size(60, 18);
    model.set_focus(Focus::Command);
    model.inspector_open = true;
    model.agents.detail_scroll = 5;
    model.command.ingest("unfinished agent draft");
    model.command.move_home();
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Workspace);

    assert_eq!(
        handle_event(&mut model, plain_character('s')),
        ControllerEffect::Redraw
    );
    assert!(model.skills.active);
    assert_eq!(model.skills.pane, SkillsPane::List);

    assert_eq!(
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        ),
        ControllerEffect::Redraw
    );
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(model.focus, Focus::Workspace);
    assert!(model.inspector_open);
    assert_eq!(model.agents.detail_scroll, 5);
    assert_eq!(model.command.text(), "unfinished agent draft");
    assert_eq!(model.command.cursor_byte(), 0);
}

#[test]
fn help_shortcut_does_not_discard_the_previous_tabs_saved_state() {
    let mut model = model();
    model.set_terminal_size(60, 18);
    model.set_focus(Focus::Inspector);
    model.inspector_open = true;
    model.workspace_scroll = 2;
    model.command.ingest("overview draft");

    assert_eq!(
        handle_event(&mut model, plain_character('4')),
        ControllerEffect::Redraw
    );
    model.set_focus(Focus::Inspector);
    model.inspector_open = true;
    model.workspace_scroll = 5;

    assert_eq!(
        handle_event(&mut model, plain_character('1')),
        ControllerEffect::Redraw
    );

    assert_eq!(
        handle_event(&mut model, plain_character('?')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Help);

    assert_eq!(
        handle_event(&mut model, plain_character('1')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Overview);
    assert_eq!(model.focus, Focus::Inspector);
    assert!(model.inspector_open);
    assert_eq!(model.workspace_scroll, 2);
    assert_eq!(model.command.text(), "overview draft");

    assert_eq!(
        handle_event(&mut model, plain_character('?')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Help);
    assert_eq!(model.focus, Focus::Inspector);
    assert!(model.inspector_open);
    assert_eq!(model.workspace_scroll, 5);

    let help_state = model.clone();
    assert_eq!(
        handle_event(&mut model, plain_character('?')),
        ControllerEffect::Redraw
    );
    assert_eq!(model, help_state);
}

#[test]
fn reopening_skills_from_a_new_cockpit_tab_updates_escape_origin() {
    let mut model = model();
    model.skills.library_loaded = true;

    assert_eq!(
        handle_event(&mut model, plain_character('s')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, plain_character('2')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, plain_character('s')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        ),
        ControllerEffect::Redraw
    );

    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Setup);
}

#[test]
fn focused_navigation_walks_all_six_tabs_and_home_end_reach_the_bounds() {
    let mut model = model();
    model.skills.library_loaded = true;
    model.set_focus(Focus::Navigation);

    for expected in [
        Some(View::Setup),
        Some(View::Audit),
        Some(View::Help),
        Some(View::Agents),
        None,
    ] {
        assert_eq!(
            handle_event(&mut model, plain_key(KeyCode::Down)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.focus, Focus::Navigation);
        if let Some(view) = expected {
            assert!(!model.skills.active);
            assert_eq!(model.active_view, view);
        } else {
            assert!(model.skills.active);
        }
    }

    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Up)),
        ControllerEffect::Redraw
    );
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Agents);

    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::End)),
        ControllerEffect::Redraw
    );
    assert!(model.skills.active);
    assert_eq!(model.focus, Focus::Navigation);

    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Home)),
        ControllerEffect::Redraw
    );
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Overview);
    assert_eq!(model.focus, Focus::Navigation);
}

#[test]
fn delayed_command_outcome_does_not_steal_a_newer_tab_or_reset_either_tabs_state() {
    let mut model = model();
    model.set_terminal_size(60, 18);
    model.set_focus(Focus::Command);
    model.inspector_open = true;
    model.command.ingest("/status");

    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Enter)),
        ControllerEffect::Submit(ApplicationCommand::ShowStatus)
    );
    assert!(model.command_in_flight);
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, plain_character('4')),
        ControllerEffect::Redraw
    );

    model.set_focus(Focus::Command);
    model.inspector_open = true;
    model.workspace_scroll = 7;
    model.command.ingest("help draft");
    let next_installation = InstallationId::from_uuid(Uuid::from_u128(30));
    let next_session = SessionId::from_uuid(Uuid::from_u128(31));

    assert_eq!(
        apply_outcome(&mut model, status_outcome(next_installation, next_session)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Help);
    assert_eq!(model.focus, Focus::Command);
    assert!(model.inspector_open);
    assert_eq!(model.workspace_scroll, 7);
    assert_eq!(model.command.text(), "help draft");
    assert_eq!(model.installation_id, next_installation);
    assert_eq!(model.session_id, next_session);

    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, plain_character('1')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Overview);
    assert_eq!(model.focus, Focus::Workspace);
    assert!(model.inspector_open);
    assert_eq!(model.workspace_scroll, 0);
    assert_eq!(model.command.text(), "");
}

#[test]
fn returning_to_the_origin_before_a_delayed_outcome_still_preserves_newer_input_state() {
    let mut model = model();
    model.set_focus(Focus::Command);
    model.command.ingest("/status");
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Enter)),
        ControllerEffect::Submit(ApplicationCommand::ShowStatus)
    );

    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, plain_character('4')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, plain_character('1')),
        ControllerEffect::Redraw
    );
    model.set_focus(Focus::Command);
    model.inspector_open = true;
    model.command.ingest("newer overview draft");

    assert_eq!(
        apply_outcome(
            &mut model,
            status_outcome(
                InstallationId::from_uuid(Uuid::from_u128(32)),
                SessionId::from_uuid(Uuid::from_u128(33)),
            )
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Overview);
    assert_eq!(model.focus, Focus::Command);
    assert!(model.inspector_open);
    assert_eq!(model.command.text(), "newer overview draft");
}

#[test]
fn delayed_skills_refresh_hydrates_in_background_without_changing_the_saved_pane() {
    let mut model = model();
    model.skills.active = true;
    model.skills.library_loaded = true;
    model.skills.pane = SkillsPane::History;
    model.skills.pending_active_skill = Some(SkillId::from_uuid(Uuid::from_u128(40)));
    model.set_focus(Focus::Command);
    model.command.ingest("/skills");
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Enter)),
        ControllerEffect::Submit(ApplicationCommand::ListSkills)
    );

    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, plain_character('2')),
        ControllerEffect::Redraw
    );
    model.set_focus(Focus::Command);
    model.command.ingest("setup draft");

    assert_eq!(
        apply_outcome(
            &mut model,
            CommandOutcome {
                command_id: CommandId::from_uuid(Uuid::from_u128(41)),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(42)),
                committed_events: Vec::new(),
                view: CommandView::Skills(SkillsView {
                    skills: Vec::new(),
                    total_count: 0,
                    returned_count: 0,
                    truncated: false,
                }),
                shutdown: ShutdownDisposition::Continue,
            }
        ),
        ControllerEffect::SubmitPreservingNavigation(ApplicationCommand::ShowSkill {
            selector: SkillId::from_uuid(Uuid::from_u128(40)).into(),
        })
    );
    assert_eq!(model.active_view, View::Setup);
    assert!(!model.skills.active);
    assert_eq!(model.skills.pane, SkillsPane::History);
    assert_eq!(model.focus, Focus::Command);
    assert_eq!(model.command.text(), "setup draft");
}

#[test]
fn sidebar_navigation_never_focuses_a_sidebar_hidden_by_the_target_layout() {
    let mut model = model();
    model.skills.library_loaded = true;
    model.set_terminal_size(80, 18);

    assert_eq!(
        handle_event(&mut model, plain_character('a')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.layout_mode, LayoutMode::Medium);
    model.set_focus(Focus::Navigation);
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Up)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Help);
    assert_eq!(model.layout_mode, LayoutMode::Narrow);
    assert_eq!(model.focus, Focus::Workspace);

    assert_eq!(
        handle_event(&mut model, plain_character('a')),
        ControllerEffect::Redraw
    );
    model.set_focus(Focus::Navigation);
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Home)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.active_view, View::Overview);
    assert_eq!(model.layout_mode, LayoutMode::Narrow);
    assert_eq!(model.focus, Focus::Workspace);
}

#[test]
fn restoring_a_tab_after_resize_normalizes_hidden_focus_and_uses_skills_geometry() {
    let mut model = model();
    model.skills.library_loaded = true;
    model.set_focus(Focus::Navigation);
    assert_eq!(
        handle_event(&mut model, plain_character('2')),
        ControllerEffect::Redraw
    );

    assert_eq!(
        handle_event(&mut model, TuiEvent::Resize(70, 20)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.layout_mode, LayoutMode::Narrow);
    assert_eq!(
        handle_event(&mut model, plain_character('1')),
        ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Workspace);

    for (width, expected_mode) in [(80, LayoutMode::Medium), (120, LayoutMode::Wide)] {
        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(width, 18)),
            ControllerEffect::Redraw
        );
        assert_eq!(
            handle_event(&mut model, plain_character('s')),
            ControllerEffect::Redraw
        );
        assert_eq!(model.layout_mode, expected_mode);
        assert_eq!(
            handle_event(&mut model, plain_character('1')),
            ControllerEffect::Redraw
        );
    }
}

#[test]
fn skills_inspector_owns_close_escape_and_focus_cycle_keys() {
    let mut model = model();
    model.skills.active = true;
    model.skills.library_loaded = true;
    model.set_terminal_size(80, 24);
    model.set_focus(Focus::Navigation);

    assert_eq!(
        handle_event(&mut model, plain_character('i')),
        ControllerEffect::Redraw
    );
    assert!(model.inspector_open);
    assert_eq!(model.focus, Focus::Inspector);
    assert_eq!(
        handle_event(&mut model, plain_character('i')),
        ControllerEffect::Redraw
    );
    assert!(!model.inspector_open);
    assert_eq!(model.focus, Focus::Workspace);

    model.set_focus(Focus::Navigation);
    assert_eq!(
        handle_event(&mut model, plain_character('i')),
        ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Tab)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Command);
    assert_eq!(
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Inspector);
    assert_eq!(
        handle_event(&mut model, plain_key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert!(!model.inspector_open);
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.skills.pane, SkillsPane::List);
}

#[test]
fn memory_non_text_layers_keep_exactly_six_global_destinations_and_restore_exact_state() {
    let targets = [
        ('1', Some(View::Overview)),
        ('2', Some(View::Setup)),
        ('3', Some(View::Audit)),
        ('4', Some(View::Help)),
        ('a', Some(View::Agents)),
        ('s', None),
    ];

    for (layer, initial) in authentic_memory_non_text_models() {
        for (shortcut, target) in targets {
            let mut model = initial.clone();
            let expected_memory = model.agents.memory.clone();

            assert!(
                matches!(
                    handle_event(&mut model, plain_character(shortcut)),
                    ControllerEffect::Redraw
                ),
                "layer={layer} shortcut={shortcut}"
            );
            if let Some(view) = target {
                assert_eq!(model.active_view, view, "layer={layer} shortcut={shortcut}");
                assert!(!model.skills.active, "layer={layer} shortcut={shortcut}");
            } else {
                assert!(model.skills.active, "layer={layer} shortcut={shortcut}");
                assert_eq!(
                    model.active_view,
                    View::Agents,
                    "layer={layer} shortcut={shortcut}"
                );
            }

            if shortcut != 'a' {
                assert!(
                    matches!(
                        handle_event(&mut model, plain_character('a')),
                        ControllerEffect::Redraw
                    ),
                    "layer={layer} shortcut={shortcut} return"
                );
            }
            assert_eq!(model.active_view, View::Agents, "layer={layer}");
            assert!(!model.skills.active, "layer={layer}");
            assert_eq!(model.agents.pane, AgentsPane::Memory, "layer={layer}");
            assert!(model.agents.memory == expected_memory, "layer={layer}");
        }
    }
}

#[test]
fn memory_non_text_layers_ignore_unassigned_and_modified_global_keys() {
    let modifiers = [
        KeyModifiers::SHIFT,
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::HYPER,
        KeyModifiers::META,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ];

    for (layer, initial) in authentic_memory_non_text_models() {
        for character in ['m', '7', 'q'] {
            let mut model = initial.clone();
            let before = model.clone();
            assert!(
                matches!(
                    handle_event(&mut model, plain_character(character)),
                    ControllerEffect::None
                ),
                "layer={layer} character={character}"
            );
            assert!(model == before, "layer={layer} character={character}");
        }

        for function_key in 1..=12 {
            let mut model = initial.clone();
            let before = model.clone();
            assert!(
                matches!(
                    handle_event(
                        &mut model,
                        TuiEvent::Key(KeyEvent::new(KeyCode::F(function_key), KeyModifiers::NONE,)),
                    ),
                    ControllerEffect::None
                ),
                "layer={layer} function_key=F{function_key}"
            );
            assert!(
                model == before,
                "layer={layer} function_key=F{function_key}"
            );
        }

        for modifier in modifiers {
            for character in ['1', '2', '3', '4', 'a', 's'] {
                let mut model = initial.clone();
                let before = model.clone();
                assert!(
                    matches!(
                        handle_event(&mut model, modified_character(character, modifier)),
                        ControllerEffect::None
                    ),
                    "layer={layer} modifier={modifier:?} character={character}"
                );
                assert!(
                    model == before,
                    "layer={layer} modifier={modifier:?} character={character}"
                );
            }
        }
    }
}

#[test]
fn memory_text_editor_stages_own_all_six_shortcut_characters() {
    let text = "1234as";

    for (stage, mut model) in authentic_memory_text_editor_models() {
        for character in text.chars() {
            assert!(
                matches!(
                    handle_event(&mut model, plain_character(character)),
                    ControllerEffect::Redraw
                ),
                "stage={stage} character={character}"
            );
        }
        assert_eq!(model.active_view, View::Agents, "stage={stage}");
        assert!(!model.skills.active, "stage={stage}");
        assert_eq!(model.agents.pane, AgentsPane::Memory, "stage={stage}");
        assert!(
            model.command.text() == text,
            "Memory text ownership changed at stage {stage}"
        );
    }

    for (stage, initial) in authentic_memory_text_editor_models() {
        for character in ['m', '7', 'q'] {
            let mut model = initial.clone();
            let _ = handle_event(&mut model, plain_character(character));
            assert_eq!(model.active_view, View::Agents, "stage={stage}");
            assert!(!model.skills.active, "stage={stage}");
            assert_eq!(model.agents.pane, AgentsPane::Memory, "stage={stage}");
        }

        for modifier in [
            KeyModifiers::SHIFT,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::HYPER,
            KeyModifiers::META,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            for character in ['1', '2', '3', '4', 'a', 's'] {
                let mut model = initial.clone();
                let _ = handle_event(&mut model, modified_character(character, modifier));
                assert_eq!(model.active_view, View::Agents, "stage={stage}");
                assert!(!model.skills.active, "stage={stage}");
                assert_eq!(model.agents.pane, AgentsPane::Memory, "stage={stage}");
            }
        }
    }
}

#[test]
fn rendered_memory_navigation_is_exactly_the_existing_six_ordered_labels() {
    let expected = [
        "1 Overview",
        "2 Setup",
        "3 Audit",
        "4 Help",
        "a Agents",
        "s Skills",
    ];

    for (layer, model) in authentic_memory_non_text_models() {
        assert!(
            rendered_memory_navigation_labels(&model) == expected,
            "layer={layer}"
        );
    }
}

#[test]
fn existing_memory_command_bar_read_does_not_create_a_seventh_global_destination() {
    let mut model = authentic_memory_non_text_models()
        .into_iter()
        .next()
        .expect("static fixture")
        .1;
    let expected_memory = model.agents.memory.clone();
    model.set_focus(Focus::Command);
    model.command.ingest("/memory list analyst");

    assert!(
        matches!(
            handle_event(&mut model, plain_key(KeyCode::Enter)),
            ControllerEffect::Submit(ApplicationCommand::ListMemoryEntries {
                selector: AgentProfileSelector::Name(name),
            }) if name == "analyst"
        ),
        "Memory command-bar read did not submit its exact safe command"
    );
    assert_eq!(model.active_view, View::Agents);
    assert!(!model.skills.active);
    assert_eq!(model.agents.pane, AgentsPane::Memory);
    assert!(model.agents.memory == expected_memory);
    assert_eq!(model.command.text(), "");
    assert!(model.command_in_flight);
}
