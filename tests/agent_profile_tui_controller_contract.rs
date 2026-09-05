use ai_stock_forum::{
    agents::{AgentProfileVersion, AgentReadiness, builtin_profile_templates},
    app::{
        AgentProfileSummary, AgentProfilesView, DatabaseReadiness, PresentationSnapshot,
        ProcessGuardOwnership, ShutdownReason,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, InstallationId, MemoryNamespaceId, SessionId,
    },
    setup::SetupStatus,
    ui::{
        profile_editor::{ProfileEditor, ProfileEditorMode},
        tui::{
        ControllerEffect, TuiEvent, handle_event,
            layout::view_geometry,
            model::{AgentsPane, AgentsViewState, ProfileConfirmation, TuiModel, View},
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

fn profile_summary(id: u128) -> AgentProfileSummary {
    let template = &builtin_profile_templates()[0];
    let profile = AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(id)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(id + 100)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(id + 200)),
        1_800_000_000_000,
        template.copy_to_draft().expect("template draft"),
        Some(template.provenance()),
    )
    .expect("profile");
    AgentProfileSummary {
        profile_id: profile.profile_id(),
        profile_version_id: profile.profile_version_id(),
        version: profile.version(),
        display_name: profile.display_name().to_owned(),
        role: profile.role(),
        primary_specialty: profile.primary_specialty().to_owned(),
        readiness: AgentReadiness::NotReady,
        content_digest: profile.content_digest().clone(),
    }
}

fn enter_line(model: &mut TuiModel, line: &str) -> ControllerEffect {
    for character in line.chars() {
        assert_eq!(handle_event(model, key(KeyCode::Char(character))), ControllerEffect::Redraw);
    }
    handle_event(model, key(KeyCode::Enter))
}

fn advance_create_editor_to_review(model: &mut TuiModel) {
    for control in [":next", ":next", ":next", ":next", ":next", ":next", ":next"] {
        assert_eq!(enter_line(model, control), ControllerEffect::Redraw);
    }
}

#[test]
fn a_opens_agents_but_remains_text_when_command_entry_owns_input() {
    let mut model = model();

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('a'))), ControllerEffect::LoadAgentProfiles);
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(model.agents.pane, AgentsPane::List);

    handle_event(&mut model, key(KeyCode::Char('/')));
    assert_eq!(handle_event(&mut model, key(KeyCode::Char('a'))), ControllerEffect::Redraw);
    assert_eq!(model.command.text(), "/a");
}

#[test]
fn existing_numeric_navigation_remains_stable() {
    let mut model = model();
    for (key_code, expected) in [
        (KeyCode::Char('1'), View::Overview),
        (KeyCode::Char('2'), View::Setup),
        (KeyCode::Char('3'), View::Audit),
        (KeyCode::Char('4'), View::Help),
    ] {
        assert_eq!(handle_event(&mut model, key(key_code)), ControllerEffect::Redraw);
        assert_eq!(model.active_view, expected);
    }
}

#[test]
fn agents_local_navigation_tracks_panes_selection_and_effects() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('a')));
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(10), profile_summary(11)],
    });

    assert_eq!(handle_event(&mut model, key(KeyCode::Down)), ControllerEffect::Redraw);
    assert_eq!(model.agents.selected_profile, 1);
    assert_eq!(model.agents.list_scroll, 1);

    assert_eq!(handle_event(&mut model, key(KeyCode::Enter)), ControllerEffect::LoadAgentProfile { selected_profile: 1 });
    assert_eq!(model.agents.pane, AgentsPane::Detail);

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('h'))), ControllerEffect::LoadAgentProfileHistory { selected_profile: 1 });
    assert_eq!(model.agents.pane, AgentsPane::History);
    assert_eq!(handle_event(&mut model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(model.agents.pane, AgentsPane::Detail);

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('c'))), ControllerEffect::StartProfileCreate { template_index: 0 });
    assert_eq!(model.agents.pane, AgentsPane::Detail);
    assert!(model.agents.editor.is_none());
}

#[test]
fn list_navigation_clamps_empty_one_last_and_refresh_shrink_states() {
    let mut empty = model();
    empty.active_view = View::Agents;
    assert_eq!(handle_event(&mut empty, key(KeyCode::Down)), ControllerEffect::Redraw);
    assert_eq!((empty.agents.selected_profile, empty.agents.list_scroll), (0, 0));

    empty.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(20)],
    });
    assert_eq!(handle_event(&mut empty, key(KeyCode::Down)), ControllerEffect::Redraw);
    assert_eq!((empty.agents.selected_profile, empty.agents.list_scroll), (0, 0));

    empty.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(20), profile_summary(21), profile_summary(22)],
    });
    for _ in 0..5 {
        handle_event(&mut empty, key(KeyCode::Down));
    }
    assert_eq!((empty.agents.selected_profile, empty.agents.list_scroll), (2, 2));

    empty.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(20)],
    });
    assert_eq!((empty.agents.selected_profile, empty.agents.list_scroll), (0, 0));
}

#[test]
fn profile_refresh_preserves_the_selected_identity_when_order_changes() {
    let mut model = model();
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(30), profile_summary(31)],
    });
    model.agents.selected_profile = 1;
    model.agents.list_scroll = 1;
    let selected = model.agents.selected_summary().unwrap().profile_id;

    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(31), profile_summary(30)],
    });

    assert_eq!(model.agents.selected_summary().unwrap().profile_id, selected);
    assert_eq!((model.agents.selected_profile, model.agents.list_scroll), (0, 0));
}

fn assert_cached_geometry(model: &TuiModel, width: u16, height: u16, view: View) {
    let expected = view_geometry(Rect::new(0, 0, width, height), view, model.inspector_open);
    assert_eq!(model.active_view, view);
    assert_eq!((model.terminal_width, model.terminal_height), (width, height));
    assert_eq!(model.layout_mode, expected.cockpit.mode);
    assert_eq!(
        (model.workspace_body_width, model.workspace_body_height),
        (expected.workspace_body_width, expected.workspace_body_height)
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
                handle_event(&mut model, key(KeyCode::Char('a'))),
                ControllerEffect::LoadAgentProfiles
            );
            assert_cached_geometry(&model, width, height, View::Agents);
            assert_eq!(model.agents, expected_agents_state);

            assert_eq!(handle_event(&mut model, key(code)), ControllerEffect::Redraw);
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
fn escape_and_quit_respect_active_agents_layers() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('a')));
    handle_event(&mut model, key(KeyCode::Char('c')));
    assert!(
        model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('q'))), ControllerEffect::Redraw);
    assert_ne!(model.runtime_status, ai_stock_forum::ui::tui::model::RuntimeStatus::Stopping);
    assert_eq!(handle_event(&mut model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(model.agents.pane, AgentsPane::List);
    assert_eq!(handle_event(&mut model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(model.active_view, View::Overview);
    assert_eq!(handle_event(&mut model, key(KeyCode::Char('q'))), ControllerEffect::RequestShutdown(ShutdownReason::UserQuit));
}

#[test]
fn resize_preserves_agents_selection_scroll_and_editor_draft() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('a')));
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
        assert_eq!(handle_event(&mut model, TuiEvent::Resize(width, height)), ControllerEffect::Redraw);
        assert_eq!(model.active_view, View::Agents);
        assert_eq!(model.agents.pane, AgentsPane::Editor);
        assert_eq!(model.agents.selected_profile, selected_profile);
        assert_eq!(model.agents.selected_template, selected_template);
        assert_eq!(model.agents.list_scroll, list_scroll);
        assert_eq!(model.command.text(), draft);
    }
}

#[test]
fn too_small_routes_text_and_escape_to_the_current_agents_or_command_owner() {
    let mut editor_model = model();
    handle_event(&mut editor_model, key(KeyCode::Char('a')));
    handle_event(&mut editor_model, key(KeyCode::Char('c')));
    assert!(
        editor_model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );
    handle_event(&mut editor_model, TuiEvent::Resize(10, 5));
    assert_eq!(handle_event(&mut editor_model, key(KeyCode::Char('a'))), ControllerEffect::Redraw);
    assert_eq!(editor_model.command.text(), "a");
    assert_eq!(handle_event(&mut editor_model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(editor_model.agents.pane, AgentsPane::List);

    let mut command_model = model();
    handle_event(&mut command_model, key(KeyCode::Char('/')));
    handle_event(&mut command_model, TuiEvent::Resize(10, 5));
    assert_eq!(handle_event(&mut command_model, key(KeyCode::Char('a'))), ControllerEffect::Redraw);
    assert_eq!(command_model.command.text(), "/a");
    assert_eq!(handle_event(&mut command_model, key(KeyCode::Esc)), ControllerEffect::Redraw);

    let mut confirmation_model = model();
    confirmation_model.active_view = View::Agents;
    confirmation_model.agents.pane = AgentsPane::Confirmation;
    confirmation_model.agents.editor = Some(create_editor());
    confirmation_model.agents.pending_confirmation = Some(ProfileConfirmation {
        command: ai_stock_forum::app::ApplicationCommand::RequestShutdown,
    });
    handle_event(&mut confirmation_model, TuiEvent::Resize(10, 5));
    assert_eq!(handle_event(&mut confirmation_model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(confirmation_model.agents.pane, AgentsPane::Editor);
    assert!(confirmation_model.agents.pending_confirmation.is_none());

    let mut local_model = model();
    handle_event(&mut local_model, key(KeyCode::Char('a')));
    handle_event(&mut local_model, TuiEvent::Resize(10, 5));
    assert_eq!(handle_event(&mut local_model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(local_model.active_view, View::Overview);
    assert_eq!(handle_event(&mut local_model, key(KeyCode::Char('q'))), ControllerEffect::RequestShutdown(ShutdownReason::UserQuit));
}

#[test]
fn agents_edit_detail_and_history_navigation_keep_independent_scroll_state() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('a')));
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(40), profile_summary(41)],
    });
    handle_event(&mut model, key(KeyCode::Down));

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('e'))), ControllerEffect::StartProfileEdit { selected_profile: 1 });
    assert_eq!(model.agents.pane, AgentsPane::Editor);

    model.agents.pane = AgentsPane::List;
    assert_eq!(handle_event(&mut model, key(KeyCode::Enter)), ControllerEffect::LoadAgentProfile { selected_profile: 1 });
    assert_eq!(handle_event(&mut model, key(KeyCode::Down)), ControllerEffect::Redraw);
    assert_eq!(model.agents.detail_scroll, 1);
    assert_eq!(model.agents.history_scroll, 0);

    assert_eq!(handle_event(&mut model, key(KeyCode::Char('h'))), ControllerEffect::LoadAgentProfileHistory { selected_profile: 1 });
    assert_eq!(handle_event(&mut model, key(KeyCode::Down)), ControllerEffect::Redraw);
    assert_eq!(model.agents.detail_scroll, 1);
    assert_eq!(model.agents.history_scroll, 1);
}

#[test]
fn editor_preview_cancellation_and_confirmation_are_typed_controller_effects() {
    let mut preview_model = model();
    preview_model.active_view = View::Agents;
    preview_model.agents.pane = AgentsPane::Editor;
    preview_model.agents.editor = Some(edit_editor());
    for control in [":next", ":next", ":next", ":next", ":next", ":next", ":next"] {
        assert_eq!(enter_line(&mut preview_model, control), ControllerEffect::Redraw);
    }
    match enter_line(&mut preview_model, ":review") {
        ControllerEffect::RequestProfilePreview(request) => {
            assert_eq!(request.generation, 1);
            assert_eq!(request.profile_id, AgentProfileId::from_uuid(Uuid::from_u128(3)));
        }
        effect => panic!("expected preview request, received {effect:?}"),
    }

    let mut cancel_model = model();
    cancel_model.active_view = View::Agents;
    cancel_model.agents.pane = AgentsPane::Editor;
    cancel_model.agents.editor = Some(create_editor());
    assert_eq!(enter_line(&mut cancel_model, ":cancel"), ControllerEffect::CancelProfileReview);
    assert_eq!(cancel_model.agents.pane, AgentsPane::Detail);
    assert!(cancel_model.agents.editor.is_none());

    let mut confirmation_model = model();
    confirmation_model.active_view = View::Agents;
    confirmation_model.agents.pane = AgentsPane::Editor;
    confirmation_model.agents.editor = Some(create_editor());
    advance_create_editor_to_review(&mut confirmation_model);
    assert_eq!(enter_line(&mut confirmation_model, ":activate"), ControllerEffect::Redraw);
    assert_eq!(confirmation_model.agents.pane, AgentsPane::Confirmation);
    assert!(matches!(
        handle_event(&mut confirmation_model, key(KeyCode::Enter)),
        ControllerEffect::ExecuteProfile(ai_stock_forum::app::ApplicationCommand::CreateAgentProfile { .. })
    ));

    confirmation_model.agents.editor = Some(edit_editor());
    confirmation_model.agents.pane = AgentsPane::Confirmation;
    confirmation_model.agents.pending_confirmation = Some(ProfileConfirmation {
        command: ai_stock_forum::app::ApplicationCommand::RequestShutdown,
    });
    assert_eq!(handle_event(&mut confirmation_model, key(KeyCode::Esc)), ControllerEffect::Redraw);
    assert_eq!(confirmation_model.agents.pane, AgentsPane::Editor);
    assert!(matches!(
        confirmation_model.agents.editor.as_ref().map(ProfileEditor::mode),
        Some(ProfileEditorMode::Edit { .. })
    ));
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
        assert_eq!(handle_event(&mut model, TuiEvent::Resize(width, height)), ControllerEffect::Redraw);
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
