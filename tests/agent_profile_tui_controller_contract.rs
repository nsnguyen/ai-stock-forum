use ai_stock_forum::{
    agents::{
        AgentProfileVersion, AgentReadiness, AgentRole, ProfileEditPreview,
        builtin_profile_templates,
    },
    app::{
        AgentProfileHistoryEntry, AgentProfileHistoryView, AgentProfileSummary,
        AgentProfileVersionView, AgentProfileView, AgentProfilesView, ApplicationCommand,
        CommandOutcome, CommandView, DatabaseReadiness, PresentationSnapshot,
        ProcessGuardOwnership, ShutdownDisposition, ShutdownReason,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, InstallationId,
        MemoryNamespaceId, ObjectVersion, ProfileReviewToken, SessionId, sha256,
    },
    setup::SetupStatus,
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorMode},
        tui::{
            ControllerEffect, TuiEvent, apply_outcome, handle_event,
            layout::view_geometry,
            model::{AgentsPane, AgentsViewState, ProfileConfirmation, TuiModel, View},
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
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
            agent_profiles: ai_stock_forum::app::AgentProfilesView {
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

fn key_event(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new_with_kind(code, modifiers, kind))
}

fn alt_tab(number: char) -> TuiEvent {
    key_event(
        KeyCode::Char(number),
        KeyModifiers::ALT,
        KeyEventKind::Press,
    )
}

fn create_editor() -> ProfileEditor {
    ProfileEditor::for_create(&builtin_profile_templates()[0]).expect("builtin template is valid")
}

fn edit_editor() -> ProfileEditor {
    ProfileEditor::for_edit(
        AgentProfileId::from_uuid(Uuid::from_u128(3)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(4)),
        builtin_profile_templates()[0]
            .copy_to_draft()
            .expect("builtin template copies to a valid draft"),
    )
}

fn profile_version(id: u128) -> AgentProfileVersion {
    let template = &builtin_profile_templates()[0];
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(id)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(id + 100)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(id + 200)),
        1_800_000_000_000,
        template.copy_to_draft().expect("template draft"),
        Some(template.provenance()),
    )
    .expect("profile")
}

fn profile_summary(id: u128) -> AgentProfileSummary {
    let profile = profile_version(id);
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

fn outcome(view: CommandView) -> CommandOutcome {
    CommandOutcome {
        command_id: CommandId::from_uuid(Uuid::from_u128(800)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(801)),
        committed_events: Vec::new(),
        view,
        shutdown: ShutdownDisposition::Continue,
    }
}

fn submitted_command(model: &mut TuiModel, line: &str) -> ApplicationCommand {
    match enter_line(model, line) {
        ControllerEffect::Submit(command) => command,
        effect => panic!("expected command submission for {line:?}, received {effect:?}"),
    }
}

fn enter_line(model: &mut TuiModel, line: &str) -> ControllerEffect {
    for character in line.chars() {
        assert_eq!(
            handle_event(model, key(KeyCode::Char(character))),
            ControllerEffect::Redraw
        );
    }
    handle_event(model, key(KeyCode::Enter))
}

fn advance_create_editor_to_review(model: &mut TuiModel) {
    for control in [
        ":next", ":next", ":next", ":next", ":next", ":next", ":next",
    ] {
        assert_eq!(enter_line(model, control), ControllerEffect::Redraw);
    }
}

fn advance_editor_to_review_with_enter(model: &mut TuiModel) {
    for _ in 0..7 {
        assert_eq!(
            handle_event(
                model,
                key_event(KeyCode::Enter, KeyModifiers::SHIFT, KeyEventKind::Press),
            ),
            ControllerEffect::Redraw
        );
    }
    assert_eq!(
        model.agents.editor.as_ref().map(ProfileEditor::step),
        Some(ai_stock_forum::ui::profile_editor::ProfileEditorStep::Review)
    );
}

#[test]
fn alt_five_opens_agents_and_bare_a_remains_text_when_command_entry_owns_input() {
    let mut model = model();

    assert_eq!(
        handle_event(&mut model, alt_tab('5')),
        ControllerEffect::LoadAgentProfiles
    );
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(model.agents.pane, AgentsPane::List);

    handle_event(&mut model, key(KeyCode::Char('/')));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('a'))),
        ControllerEffect::Redraw
    );
    assert_eq!(model.command.text(), "/a");
}

#[test]
fn option_alt_numeric_navigation_selects_each_non_skill_tab() {
    let mut model = model();
    for (number, expected) in [
        ('1', View::Overview),
        ('2', View::Setup),
        ('3', View::Audit),
        ('4', View::Help),
        ('5', View::Agents),
    ] {
        assert_eq!(
            handle_event(&mut model, alt_tab(number)),
            if number == '5' {
                ControllerEffect::LoadAgentProfiles
            } else {
                ControllerEffect::Redraw
            }
        );
        assert_eq!(model.active_view, expected);
    }
}

#[test]
fn agents_local_navigation_tracks_panes_selection_and_effects() {
    let mut model = model();
    handle_event(&mut model, alt_tab('5'));
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(10), profile_summary(11)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.selected_profile, 1);
    assert_eq!(model.agents.list_scroll, 1);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadAgentProfile {
            selected_profile: 1
        }
    );
    assert_eq!(model.agents.pane, AgentsPane::Detail);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('h'))),
        ControllerEffect::LoadAgentProfileHistory {
            selected_profile: 1
        }
    );
    assert_eq!(model.agents.pane, AgentsPane::History);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.pane, AgentsPane::Detail);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('c'))),
        ControllerEffect::StartProfileCreate { template_index: 0 }
    );
    assert_eq!(model.agents.pane, AgentsPane::Detail);
    assert!(model.agents.editor.is_none());
}

#[test]
fn list_navigation_clamps_empty_one_last_and_refresh_shrink_states() {
    let mut empty = model();
    empty.active_view = View::Agents;
    assert_eq!(
        handle_event(&mut empty, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (empty.agents.selected_profile, empty.agents.list_scroll),
        (0, 0)
    );

    empty.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(20)],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    });
    assert_eq!(
        handle_event(&mut empty, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (empty.agents.selected_profile, empty.agents.list_scroll),
        (0, 0)
    );

    empty.agents.replace_profiles(AgentProfilesView {
        profiles: vec![
            profile_summary(20),
            profile_summary(21),
            profile_summary(22),
        ],
        total_count: 3,
        returned_count: 3,
        truncated: false,
    });
    for _ in 0..5 {
        handle_event(&mut empty, key(KeyCode::Down));
    }
    assert_eq!(
        (empty.agents.selected_profile, empty.agents.list_scroll),
        (2, 2)
    );

    empty.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(20)],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    });
    assert_eq!(
        (empty.agents.selected_profile, empty.agents.list_scroll),
        (0, 0)
    );
}

#[test]
fn profile_refresh_preserves_the_selected_identity_when_order_changes() {
    let mut model = model();
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(30), profile_summary(31)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    model.agents.selected_profile = 1;
    model.agents.list_scroll = 1;
    let selected = model.agents.selected_summary().unwrap().profile_id;

    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(31), profile_summary(30)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });

    assert_eq!(
        model.agents.selected_summary().unwrap().profile_id,
        selected
    );
    assert_eq!(
        (model.agents.selected_profile, model.agents.list_scroll),
        (0, 0)
    );
}

fn assert_cached_geometry(model: &TuiModel, width: u16, height: u16, view: View) {
    let expected = view_geometry(Rect::new(0, 0, width, height), view, model.inspector_open);
    assert_eq!(model.active_view, view);
    assert_eq!(
        (model.terminal_width, model.terminal_height),
        (width, height)
    );
    assert_eq!(model.layout_mode, expected.cockpit.mode);
    assert_eq!(
        (model.workspace_body_width, model.workspace_body_height),
        (
            expected.workspace_body_width,
            expected.workspace_body_height
        )
    );
}

#[test]
fn every_view_transition_recomputes_geometry_without_resize_and_preserves_agents_state() {
    for (width, height) in [(80, 18), (120, 18)] {
        let mut model = model();
        model.agents.selected_profile = 3;
        model.agents.selected_template = 1;
        model.agents.list_scroll = 3;
        model.agents.detail_scroll = 4;
        model.agents.history_scroll = 5;
        model.agents.editor = Some(create_editor());
        let expected_agents_state = model.agents.clone();

        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(width, height)),
            ControllerEffect::Redraw
        );
        assert_cached_geometry(&model, width, height, View::Overview);

        for (code, view) in [
            (KeyCode::Char('1'), View::Overview),
            (KeyCode::Char('2'), View::Setup),
            (KeyCode::Char('3'), View::Audit),
            (KeyCode::Char('4'), View::Help),
        ] {
            assert_eq!(
                handle_event(&mut model, alt_tab('5')),
                ControllerEffect::LoadAgentProfiles
            );
            assert_cached_geometry(&model, width, height, View::Agents);
            assert_eq!(model.agents, expected_agents_state);

            assert_eq!(
                handle_event(
                    &mut model,
                    key_event(code, KeyModifiers::ALT, KeyEventKind::Press),
                ),
                ControllerEffect::Redraw
            );
            assert_cached_geometry(&model, width, height, view);
            assert_eq!(model.agents, expected_agents_state);
        }

        model.select_view(View::Agents);
        assert_cached_geometry(&model, width, height, View::Agents);
        model.select_view(View::Overview);
        assert_cached_geometry(&model, width, height, View::Overview);
        assert_eq!(model.agents, expected_agents_state);
    }
}

#[test]
fn escape_respects_active_agents_layers_and_bare_q_never_quits() {
    let mut model = model();
    handle_event(&mut model, alt_tab('5'));
    handle_event(&mut model, key(KeyCode::Char('c')));
    assert!(
        model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('q'))),
        ControllerEffect::Redraw
    );
    assert_ne!(
        model.runtime_status,
        ai_stock_forum::ui::tui::model::RuntimeStatus::Stopping
    );
    handle_event(&mut model, key(KeyCode::Backspace));
    assert_eq!(enter_line(&mut model, ":next"), ControllerEffect::Redraw);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.pane, AgentsPane::Editor);
    assert_eq!(
        model.agents.editor.as_ref().unwrap().step().as_str(),
        "template"
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::CancelProfileReview
    );
    assert_eq!(model.agents.pane, AgentsPane::Detail);
    let before_q = model.clone();
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('q'))),
        ControllerEffect::None
    );
    assert_eq!(model, before_q);
    assert_eq!(
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL,)),
        ),
        ControllerEffect::RequestShutdown(ShutdownReason::Interrupted)
    );
}

#[test]
fn resize_preserves_agents_selection_scroll_and_editor_draft() {
    let mut model = model();
    handle_event(&mut model, alt_tab('5'));
    handle_event(&mut model, key(KeyCode::Char('c')));
    assert!(
        model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );
    for character in "Draft Analyst".chars() {
        handle_event(&mut model, key(KeyCode::Char(character)));
    }
    let selected_profile = model.agents.selected_profile;
    let selected_template = model.agents.selected_template;
    let list_scroll = model.agents.list_scroll;
    let draft = model.command.text().to_owned();

    for (width, height) in [(70, 24), (110, 32), (160, 44)] {
        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(width, height)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.active_view, View::Agents);
        assert_eq!(model.agents.pane, AgentsPane::Editor);
        assert_eq!(model.agents.selected_profile, selected_profile);
        assert_eq!(model.agents.selected_template, selected_template);
        assert_eq!(model.agents.list_scroll, list_scroll);
        assert_eq!(model.command.text(), draft);
    }
}

#[test]
fn bare_q_never_requests_shutdown_across_agent_input_owners_or_too_small() {
    let mut editor_model = model();
    handle_event(&mut editor_model, alt_tab('5'));
    handle_event(&mut editor_model, key(KeyCode::Char('c')));
    assert!(
        editor_model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );
    assert_eq!(
        handle_event(&mut editor_model, key(KeyCode::Char('q'))),
        ControllerEffect::Redraw
    );
    assert_eq!(editor_model.command.text(), "q");

    let mut command_model = model();
    handle_event(&mut command_model, key(KeyCode::Char('/')));
    assert_eq!(
        handle_event(&mut command_model, key(KeyCode::Char('q'))),
        ControllerEffect::Redraw
    );
    assert_eq!(command_model.command.text(), "/q");

    let mut confirmation_model = model();
    confirmation_model.active_view = View::Agents;
    confirmation_model.agents.pane = AgentsPane::Confirmation;
    confirmation_model.agents.editor = Some(create_editor());
    confirmation_model.agents.pending_confirmation = Some(ProfileConfirmation {
        command: ai_stock_forum::app::ApplicationCommand::RequestShutdown,
    });
    let confirmation_before_q = confirmation_model.clone();
    assert_eq!(
        handle_event(&mut confirmation_model, key(KeyCode::Char('q'))),
        ControllerEffect::None
    );
    assert_eq!(confirmation_model, confirmation_before_q);

    let mut local_model = model();
    handle_event(&mut local_model, alt_tab('5'));
    let local_before_q = local_model.clone();
    assert_eq!(
        handle_event(&mut local_model, key(KeyCode::Char('q'))),
        ControllerEffect::None
    );
    assert_eq!(local_model, local_before_q);

    handle_event(&mut editor_model, TuiEvent::Resize(10, 5));
    assert_eq!(
        handle_event(&mut editor_model, key(KeyCode::Char('q'))),
        ControllerEffect::None
    );
    assert_eq!(editor_model.command.text(), "q");

    handle_event(&mut command_model, TuiEvent::Resize(10, 5));
    assert_eq!(
        handle_event(&mut command_model, key(KeyCode::Char('q'))),
        ControllerEffect::Redraw
    );
    assert_eq!(command_model.command.text(), "/qq");

    handle_event(&mut confirmation_model, TuiEvent::Resize(10, 5));
    let confirmation_before_small_q = confirmation_model.clone();
    assert_eq!(
        handle_event(&mut confirmation_model, key(KeyCode::Char('q'))),
        ControllerEffect::None
    );
    assert_eq!(confirmation_model, confirmation_before_small_q);
    assert!(confirmation_model.agents.pending_confirmation.is_some());

    handle_event(&mut local_model, TuiEvent::Resize(10, 5));
    assert_eq!(
        handle_event(&mut local_model, key(KeyCode::Char('q'))),
        ControllerEffect::None
    );
    assert_eq!(
        enter_line(&mut local_model, "/quit"),
        ControllerEffect::Submit(ai_stock_forum::app::ApplicationCommand::RequestShutdown)
    );
}

#[test]
fn agents_edit_detail_and_history_navigation_keep_independent_scroll_state() {
    let mut model = model();
    handle_event(&mut model, alt_tab('5'));
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(40), profile_summary(41)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    handle_event(&mut model, key(KeyCode::Down));

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('e'))),
        ControllerEffect::StartProfileEdit {
            selected_profile: 1
        }
    );
    assert_eq!(model.agents.pane, AgentsPane::Editor);

    model.agents.pane = AgentsPane::List;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadAgentProfile {
            selected_profile: 1
        }
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.detail_scroll, 1);
    assert_eq!(model.agents.history_scroll, 0);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('h'))),
        ControllerEffect::LoadAgentProfileHistory {
            selected_profile: 1
        }
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.detail_scroll, 1);
    assert_eq!(model.agents.history_scroll, 0);
}

#[test]
fn editor_preview_cancellation_and_confirmation_are_typed_controller_effects() {
    let mut preview_model = model();
    preview_model.active_view = View::Agents;
    preview_model.agents.pane = AgentsPane::Editor;
    preview_model.agents.editor = Some(edit_editor());
    for control in [
        ":next", ":next", ":next", ":next", ":next", ":next", ":next",
    ] {
        assert_eq!(
            enter_line(&mut preview_model, control),
            ControllerEffect::Redraw
        );
    }
    match enter_line(&mut preview_model, ":review") {
        ControllerEffect::RequestProfilePreview(request) => {
            assert_eq!(request.generation, 1);
            assert_eq!(
                request.profile_id,
                AgentProfileId::from_uuid(Uuid::from_u128(3))
            );
        }
        effect => panic!("expected preview request, received {effect:?}"),
    }

    let mut cancel_model = model();
    cancel_model.active_view = View::Agents;
    cancel_model.agents.pane = AgentsPane::Editor;
    cancel_model.agents.editor = Some(create_editor());
    assert_eq!(
        enter_line(&mut cancel_model, ":cancel"),
        ControllerEffect::CancelProfileReview
    );
    assert_eq!(cancel_model.agents.pane, AgentsPane::Detail);
    assert!(cancel_model.agents.editor.is_none());

    let mut confirmation_model = model();
    confirmation_model.active_view = View::Agents;
    confirmation_model.agents.pane = AgentsPane::Editor;
    confirmation_model.agents.editor = Some(create_editor());
    advance_create_editor_to_review(&mut confirmation_model);
    assert_eq!(
        enter_line(&mut confirmation_model, ":create"),
        ControllerEffect::Redraw
    );
    assert_eq!(confirmation_model.agents.pane, AgentsPane::Confirmation);
    assert!(matches!(
        handle_event(&mut confirmation_model, key(KeyCode::Enter)),
        ControllerEffect::ExecuteProfile(
            ai_stock_forum::app::ApplicationCommand::CreateAgentProfile { .. }
        )
    ));

    confirmation_model.agents.editor = Some(edit_editor());
    confirmation_model.agents.pane = AgentsPane::Confirmation;
    confirmation_model.agents.pending_confirmation = Some(ProfileConfirmation {
        command: ai_stock_forum::app::ApplicationCommand::RequestShutdown,
    });
    assert_eq!(
        handle_event(&mut confirmation_model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(confirmation_model.agents.pane, AgentsPane::Editor);
    assert!(matches!(
        confirmation_model
            .agents
            .editor
            .as_ref()
            .map(ProfileEditor::mode),
        Some(ProfileEditorMode::Edit { .. })
    ));
}

#[test]
fn keyboard_create_path_cycles_complete_templates_and_uses_enter_only() {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(create_editor());

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft(),
        &builtin_profile_templates()[1].copy_to_draft().unwrap()
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Up)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft().role,
        builtin_profile_templates()[0].role
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Up)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft().role,
        builtin_profile_templates()[4].role
    );
    handle_event(&mut model, key(KeyCode::Down));
    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft().role,
        builtin_profile_templates()[1].role
    );

    assert_eq!(
        handle_event(
            &mut model,
            key_event(KeyCode::Enter, KeyModifiers::SHIFT, KeyEventKind::Press),
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.editor.as_ref().unwrap().current_field_label(),
        "Display name"
    );

    for character in "Keyboard Bear".chars() {
        handle_event(&mut model, key(KeyCode::Char(character)));
    }
    assert_eq!(
        handle_event(
            &mut model,
            key_event(KeyCode::Enter, KeyModifiers::CONTROL, KeyEventKind::Press),
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft().display_name,
        "Keyboard Bear"
    );

    for modifiers in [
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::HYPER,
        KeyModifiers::META,
        KeyModifiers::SHIFT | KeyModifiers::CONTROL,
    ] {
        assert_eq!(
            handle_event(
                &mut model,
                key_event(KeyCode::Enter, modifiers, KeyEventKind::Press),
            ),
            ControllerEffect::Redraw
        );
    }
    assert_eq!(
        model.agents.editor.as_ref().unwrap().step(),
        ai_stock_forum::ui::profile_editor::ProfileEditorStep::Review
    );

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.pane, AgentsPane::Confirmation);
    let Some(ProfileConfirmation {
        command:
            ApplicationCommand::CreateAgentProfile {
                draft,
                template_provenance,
            },
    }) = model.agents.pending_confirmation.as_ref()
    else {
        panic!("expected create confirmation")
    };
    assert_eq!(draft.display_name, "Keyboard Bear");
    assert_eq!(draft.role, builtin_profile_templates()[1].role);
    assert_eq!(
        template_provenance
            .as_ref()
            .map(|provenance| provenance.template_id.as_str()),
        Some("builtin.bear")
    );

    assert!(matches!(
        handle_event(
            &mut model,
            key_event(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Press),
        ),
        ControllerEffect::ExecuteProfile(ApplicationCommand::CreateAgentProfile { .. })
    ));
}

#[test]
fn edit_template_step_ignores_arrows_but_keeps_enter_and_role_alias() {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(edit_editor());
    let original = model.clone();

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::None
    );
    assert_eq!(model, original);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Up)),
        ControllerEffect::None
    );
    assert_eq!(model, original);

    assert_eq!(
        enter_line(&mut model, ":role bear"),
        ControllerEffect::Redraw
    );
    let editor = model.agents.editor.as_ref().unwrap();
    assert_eq!(editor.draft().role, AgentRole::Bear);
    assert_eq!(editor.draft().display_name, "Bull Researcher");

    assert_eq!(
        handle_event(
            &mut model,
            key_event(KeyCode::Enter, KeyModifiers::SHIFT, KeyEventKind::Press),
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.editor.as_ref().unwrap().current_field_label(),
        "Display name"
    );
}

#[test]
fn edit_review_enter_requests_preview_before_opening_activation_confirmation() {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(edit_editor());
    advance_editor_to_review_with_enter(&mut model);

    let request = match handle_event(&mut model, key(KeyCode::Enter)) {
        ControllerEffect::RequestProfilePreview(request) => request,
        effect => panic!("expected preview request, received {effect:?}"),
    };
    assert_eq!(model.agents.pane, AgentsPane::Editor);
    assert!(model.agents.pending_confirmation.is_none());

    let review_digest = sha256(b"keyboard-review");
    model.agents.editor.as_mut().unwrap().apply_preview(
        request.generation,
        ProfileEditPreview {
            profile_id: request.profile_id,
            expected_active_version_id: request.expected_active_version_id,
            diffs: Vec::new(),
            review_token: ProfileReviewToken::from_uuid(Uuid::from_u128(9)),
            review_digest: review_digest.clone(),
        },
    );

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.pane, AgentsPane::Confirmation);
    assert!(matches!(
        model
            .agents
            .pending_confirmation
            .as_ref()
            .map(|confirmation| &confirmation.command),
        Some(ApplicationCommand::ActivateAgentProfileVersion {
            review_digest: digest,
            ..
        }) if digest == &review_digest
    ));
}

#[test]
fn repeated_enter_cannot_open_or_execute_a_durable_confirmation() {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(create_editor());
    advance_editor_to_review_with_enter(&mut model);

    let repeat = key_event(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat);
    assert_eq!(
        handle_event(&mut model, repeat.clone()),
        ControllerEffect::None
    );
    assert_eq!(model.agents.pane, AgentsPane::Editor);
    assert!(model.agents.pending_confirmation.is_none());

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.pane, AgentsPane::Confirmation);
    assert_eq!(handle_event(&mut model, repeat), ControllerEffect::None);
    assert_eq!(model.agents.pane, AgentsPane::Confirmation);

    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::ExecuteProfile(ApplicationCommand::CreateAgentProfile { .. })
    ));
}

#[test]
fn command_bar_dispatches_canonical_profile_workflows() {
    let mut create = model();
    assert!(matches!(
        enter_line(&mut create, "/agent create bull"),
        ControllerEffect::StartProfileCreateByTemplate { template_id }
            if template_id.as_str() == "builtin.bull"
    ));

    let mut edit = model();
    assert!(matches!(
        enter_line(&mut edit, "/agent edit \"Bull Researcher\""),
        ControllerEffect::StartProfileEditBySelector { selector }
            if selector.display_name() == Some("Bull Researcher")
    ));
}

#[test]
fn command_bar_applies_every_profile_read_form_to_typed_agents_state() {
    let profile = profile_version(700);
    let profile_id = profile.profile_id();
    let summary = profile_summary(700);
    let profiles = AgentProfilesView {
        profiles: vec![summary],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };
    let detail = AgentProfileView {
        profile: profile.clone(),
        readiness: AgentReadiness::Unbound,
    };
    let history = AgentProfileHistoryView {
        profile_id,
        active_version_id: profile.profile_version_id(),
        versions: vec![AgentProfileHistoryEntry {
            profile_version_id: profile.profile_version_id(),
            version: profile.version(),
            supersedes: profile.supersedes(),
            created_at_ms: profile.created_at_ms(),
            readiness: AgentReadiness::Unbound,
            content_digest: profile.content_digest().clone(),
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };
    let version = AgentProfileVersionView {
        profile: profile.clone(),
        readiness: AgentReadiness::Unbound,
        predecessor_diff: Vec::new(),
    };

    let mut listed = model();
    assert_eq!(
        submitted_command(&mut listed, "/agent list"),
        ApplicationCommand::ListAgentProfiles
    );
    assert_eq!(
        apply_outcome(
            &mut listed,
            outcome(CommandView::AgentProfiles(profiles.clone()))
        ),
        ControllerEffect::Redraw
    );
    assert_eq!(listed.active_view, View::Agents);
    assert_eq!(listed.agents.pane, AgentsPane::List);
    assert_eq!(listed.agents.profiles, profiles);

    for selector in ["\"Bull Researcher\"".to_owned(), profile_id.to_string()] {
        let mut shown = model();
        assert!(matches!(
            submitted_command(&mut shown, &format!("/agent show {selector}")),
            ApplicationCommand::ShowAgentProfile { .. }
        ));
        apply_outcome(
            &mut shown,
            outcome(CommandView::AgentProfile(detail.clone())),
        );
        assert_eq!(shown.active_view, View::Agents);
        assert_eq!(shown.agents.pane, AgentsPane::Detail);
        assert_eq!(shown.agents.detail.as_ref(), Some(&detail));
    }

    for selector in ["\"Bull Researcher\"".to_owned(), profile_id.to_string()] {
        let mut shown = model();
        assert!(matches!(
            submitted_command(&mut shown, &format!("/agent history {selector}")),
            ApplicationCommand::ShowAgentProfileHistory { .. }
        ));
        apply_outcome(
            &mut shown,
            outcome(CommandView::AgentProfileHistory(history.clone())),
        );
        assert_eq!(shown.active_view, View::Agents);
        assert_eq!(shown.agents.pane, AgentsPane::History);
        assert_eq!(shown.agents.history.as_ref(), Some(&history));
    }

    for selector in ["\"Bull Researcher\"".to_owned(), profile_id.to_string()] {
        let mut shown = model();
        assert!(matches!(
            submitted_command(&mut shown, &format!("/agent history {selector} 1")),
            ApplicationCommand::ShowAgentProfileVersion { version, .. } if version.get() == 1
        ));
        apply_outcome(
            &mut shown,
            outcome(CommandView::AgentProfileVersion(version.clone())),
        );
        assert_eq!(shown.active_view, View::Agents);
        assert_eq!(shown.agents.pane, AgentsPane::History);
        assert_eq!(shown.agents.version_detail.as_ref(), Some(&version));
    }
}

#[test]
fn history_navigation_selects_and_loads_exact_immutable_versions() {
    let mut model = model();
    let profile_id = AgentProfileId::from_uuid(Uuid::from_u128(700));
    let active_version_id = AgentProfileVersionId::from_uuid(Uuid::from_u128(702));
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::History;
    model.agents.replace_history(AgentProfileHistoryView {
        profile_id,
        active_version_id,
        versions: vec![
            AgentProfileHistoryEntry {
                profile_version_id: active_version_id,
                version: ObjectVersion::new(2).unwrap(),
                supersedes: Some(AgentProfileVersionId::from_uuid(Uuid::from_u128(701))),
                created_at_ms: 2,
                readiness: AgentReadiness::Unbound,
                content_digest: sha256(b"v2"),
            },
            AgentProfileHistoryEntry {
                profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(701)),
                version: ObjectVersion::new(1).unwrap(),
                supersedes: None,
                created_at_ms: 1,
                readiness: AgentReadiness::Unbound,
                content_digest: sha256(b"v1"),
            },
        ],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.selected_history_version, 1);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadAgentProfileVersion {
            profile_id,
            version: ObjectVersion::new(1).unwrap(),
        }
    );
}

#[test]
fn resize_preserves_every_agents_state_field_across_all_layout_modes() {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.selected_profile = 3;
    model.agents.selected_template = 1;
    model.agents.list_scroll = 4;
    model.agents.detail_scroll = 5;
    model.agents.history_scroll = 6;
    let mut editor = create_editor();
    editor.submit_line(":next");
    editor.submit_line("Resize Draft");
    model.agents.editor = Some(editor);
    model.agents.pane = AgentsPane::Confirmation;
    model.agents.pending_confirmation = Some(ProfileConfirmation {
        command: ai_stock_forum::app::ApplicationCommand::RequestShutdown,
    });
    let expected = model.agents.clone();

    for (width, height) in [(70, 24), (110, 32), (160, 44), (10, 5)] {
        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(width, height)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.active_view, View::Agents);
        assert_eq!(model.agents, expected);
    }
}

#[test]
fn agents_state_equality_detects_different_editor_drafts() {
    let mut left = AgentsViewState::default();
    let mut editor = create_editor();
    editor.submit_line(":next");
    editor.submit_line("Left Draft");
    left.editor = Some(editor);
    let mut right = left.clone();
    right
        .editor
        .as_mut()
        .expect("editor exists")
        .submit_line("Right Draft");

    assert_ne!(left, right);
}

fn enter_modifier_variants() -> [KeyModifiers; 8] {
    [
        KeyModifiers::NONE,
        KeyModifiers::SHIFT,
        KeyModifiers::CONTROL,
        KeyModifiers::ALT,
        KeyModifiers::SUPER,
        KeyModifiers::HYPER,
        KeyModifiers::META,
        KeyModifiers::SHIFT
            | KeyModifiers::CONTROL
            | KeyModifiers::ALT
            | KeyModifiers::SUPER
            | KeyModifiers::HYPER
            | KeyModifiers::META,
    ]
}

#[test]
fn every_enter_modifier_submits_template_controls_and_clears_stale_validation() {
    for modifiers in enter_modifier_variants() {
        let mut model = model();
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Editor;
        model.agents.editor = Some(create_editor());

        assert_eq!(
            enter_line(&mut model, "text is unavailable here"),
            ControllerEffect::Redraw
        );
        assert_eq!(
            model
                .agents
                .editor
                .as_ref()
                .and_then(ProfileEditor::local_message)
                .map(|message| message.code()),
            Some("editor_field_unavailable")
        );

        for character in ":role bull".chars() {
            assert_eq!(
                handle_event(&mut model, key(KeyCode::Char(character))),
                ControllerEffect::Redraw
            );
        }
        assert_eq!(
            handle_event(
                &mut model,
                TuiEvent::Key(KeyEvent::new(KeyCode::Enter, modifiers)),
            ),
            ControllerEffect::Redraw
        );
        assert!(model.command.text().is_empty());
        assert!(
            model
                .agents
                .editor
                .as_ref()
                .and_then(ProfileEditor::local_message)
                .is_none()
        );

        for character in ":next".chars() {
            assert_eq!(
                handle_event(&mut model, key(KeyCode::Char(character))),
                ControllerEffect::Redraw
            );
        }
        assert_eq!(
            handle_event(
                &mut model,
                TuiEvent::Key(KeyEvent::new(KeyCode::Enter, modifiers)),
            ),
            ControllerEffect::Redraw
        );
        assert!(model.command.text().is_empty());
        assert_eq!(
            model.agents.editor.as_ref().unwrap().step().as_str(),
            "identity"
        );
    }
}

#[test]
fn every_modified_enter_is_inert_during_profile_confirmation() {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(create_editor());
    advance_create_editor_to_review(&mut model);
    assert_eq!(enter_line(&mut model, ":create"), ControllerEffect::Redraw);
    assert_eq!(model.agents.pane, AgentsPane::Confirmation);
    for modifiers in enter_modifier_variants()
        .into_iter()
        .filter(|modifiers| !modifiers.is_empty())
    {
        let mut candidate = model.clone();
        assert_eq!(
            handle_event(
                &mut candidate,
                key_event(KeyCode::Enter, modifiers, KeyEventKind::Press),
            ),
            ControllerEffect::None
        );
        assert_eq!(candidate, model);
    }
}

#[test]
fn confirmation_ignores_character_and_editing_keys_without_hidden_input() {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(create_editor());
    advance_create_editor_to_review(&mut model);
    assert_eq!(enter_line(&mut model, ":create"), ControllerEffect::Redraw);
    assert_eq!(model.agents.pane, AgentsPane::Confirmation);
    let expected = model.clone();

    for code in [
        KeyCode::Char('x'),
        KeyCode::Backspace,
        KeyCode::Delete,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Up,
        KeyCode::Down,
    ] {
        assert_eq!(handle_event(&mut model, key(code)), ControllerEffect::None);
        assert_eq!(model, expected);
    }
}
