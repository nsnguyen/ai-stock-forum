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
        profile_editor::ProfileEditor,
        tui::{
            ControllerEffect, TuiEvent, apply_outcome, handle_event,
            layout::view_geometry,
            model::{AgentsPane, AgentsViewState, Focus, ProfileConfirmation, TuiModel, View},
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

fn navigation_key(character: char) -> TuiEvent {
    key_event(
        KeyCode::Char(character),
        KeyModifiers::NONE,
        KeyEventKind::Press,
    )
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

#[test]
fn selecting_an_exact_cached_origin_retains_only_its_matching_detail() {
    let mut model = model();
    model.agents.profiles.profiles = vec![profile_summary(500), profile_summary(600)];
    model.agents.detail = Some(AgentProfileView {
        profile: profile_version(600),
        readiness: AgentReadiness::Unbound,
    });
    model.agents.select_profile_index(1);
    assert!(model.agents.matching_detail().is_some());
    model.agents.select_profile_index(0);
    assert!(model.agents.detail.is_none());
}

#[test]
fn passive_profile_selection_rejects_late_results_without_changing_focus() {
    let mut model = model();
    model.select_view(View::Agents);
    model.agents.profiles.profiles = vec![profile_summary(500), profile_summary(600)];
    model.set_focus(Focus::List);
    let old = model.agents.profile_target().unwrap();
    handle_event(&mut model, key(KeyCode::Char('s')));
    assert_eq!(model.agents.selected_profile, 1);
    assert!(!model.agents.install_profile_result(
        &old,
        CommandView::AgentProfile(AgentProfileView {
            profile: profile_version(500),
            readiness: AgentReadiness::Unbound,
        })
    ));
    assert!(model.agents.detail.is_none());
    assert_eq!(model.focus, Focus::List);
    let current = model.agents.profile_target().unwrap();
    assert!(model.agents.install_profile_result(
        &current,
        CommandView::AgentProfile(AgentProfileView {
            profile: profile_version(600),
            readiness: AgentReadiness::Unbound,
        })
    ));
    assert_eq!(model.agents.selected_profile, 1);
    assert_eq!(model.focus, Focus::List);
}

#[test]
fn passive_profile_rejects_same_ids_with_wrong_content_and_generation_reentry() {
    let mut model = model();
    model.agents.profiles.profiles = vec![profile_summary(500), profile_summary(600)];
    let old = model.agents.profile_target().unwrap();
    model.agents.select_profile_index(1);
    model.agents.select_profile_index(0);
    assert!(!model.agents.install_profile_result(
        &old,
        CommandView::AgentProfile(AgentProfileView {
            profile: profile_version(500),
            readiness: AgentReadiness::Unbound
        })
    ));
    let target = model.agents.profile_target().unwrap();
    model.agents.profiles.profiles[0].content_digest = sha256(b"different selected content");
    assert!(!model.agents.install_profile_result(
        &target,
        CommandView::AgentProfile(AgentProfileView {
            profile: profile_version(500),
            readiness: AgentReadiness::Unbound
        })
    ));
    assert!(model.agents.detail.is_none());
}

#[test]
fn history_list_selection_returns_a_new_agent_to_profile_but_retains_same_agent_history() {
    use ai_stock_forum::ui::tui::model::{AgentDetailAction, AgentProfileRead};
    for (width, height) in [(60, 18), (120, 30)] {
        let mut model = model();
        model.agents.profiles.profiles = vec![profile_summary(500), profile_summary(600)];
        handle_event(&mut model, TuiEvent::Resize(width, height));
        handle_event(&mut model, key(KeyCode::Char('3')));
        handle_event(&mut model, key(KeyCode::Tab));
        let first = model.agents.profile_target().unwrap();
        assert!(model.agents.install_profile_result(
            &first,
            CommandView::AgentProfile(AgentProfileView {
                profile: profile_version(500),
                readiness: AgentReadiness::Unbound,
            })
        ));
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Char('h'))),
            ControllerEffect::LoadSelectedAgentProfile {
                target: first.clone(),
                read: AgentProfileRead::History
            }
        );
        let profile = profile_version(500);
        assert!(model.agents.install_profile_result(
            &first,
            CommandView::AgentProfileHistory(AgentProfileHistoryView {
                profile_id: profile.profile_id(),
                active_version_id: profile.profile_version_id(),
                versions: vec![AgentProfileHistoryEntry {
                    profile_version_id: profile.profile_version_id(),
                    version: profile.version(),
                    supersedes: None,
                    created_at_ms: profile.created_at_ms(),
                    readiness: AgentReadiness::Unbound,
                    content_digest: profile.content_digest().clone(),
                }],
                total_count: 1,
                returned_count: 1,
                truncated: false,
            })
        ));
        let history = model.agents.history.clone();
        handle_event(
            &mut model,
            key_event(KeyCode::BackTab, KeyModifiers::SHIFT, KeyEventKind::Press),
        );
        handle_event(&mut model, key(KeyCode::Char('w')));
        handle_event(&mut model, key(KeyCode::Tab));
        assert_eq!(model.agents.pane, AgentsPane::History);
        assert_eq!(model.agents.history, history);
        handle_event(
            &mut model,
            key_event(KeyCode::BackTab, KeyModifiers::SHIFT, KeyEventKind::Press),
        );
        assert_eq!(model.focus, Focus::List);
        let load = handle_event(&mut model, key(KeyCode::Char('s')));
        let second = model.agents.profile_target().unwrap();
        assert_eq!(
            load,
            ControllerEffect::LoadSelectedAgentProfile {
                target: second.clone(),
                read: AgentProfileRead::Detail
            }
        );
        handle_event(&mut model, key(KeyCode::Tab));
        assert_eq!(model.focus, Focus::Workspace);
        assert_eq!(model.agents.pane, AgentsPane::Detail);
        assert_eq!(
            model.agents.selected_detail_action,
            AgentDetailAction::Profile
        );
        assert!(model.agents.history.is_none());
        assert!(model.agents.install_profile_result(
            &second,
            CommandView::AgentProfile(AgentProfileView {
                profile: profile_version(600),
                readiness: AgentReadiness::Unbound,
            })
        ));
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Char('h'))),
            ControllerEffect::LoadSelectedAgentProfile {
                target: second,
                read: AgentProfileRead::History
            }
        );
    }
}

#[test]
fn history_escape_closes_only_the_loaded_version_and_edit_keeps_active_target() {
    let mut model = model();
    model.select_view(View::Agents);
    model.agents.profiles.profiles = vec![profile_summary(500)];
    model.agents.detail = Some(AgentProfileView {
        profile: profile_version(500),
        readiness: AgentReadiness::Unbound,
    });
    model.agents.version_detail = Some(AgentProfileVersionView {
        profile: profile_version(500),
        readiness: AgentReadiness::Unbound,
        predecessor_diff: Vec::new(),
    });
    model.agents.pane = AgentsPane::History;
    model.set_focus(Focus::Workspace);
    let target = model.agents.profile_target().unwrap();
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('e'))),
        ControllerEffect::StartSelectedProfileEdit { target }
    );
    handle_event(&mut model, key(KeyCode::Esc));
    assert_eq!(model.agents.pane, AgentsPane::History);
    assert!(model.agents.version_detail.is_none());
}

#[test]
fn fresh_agents_key_route_opens_selected_memory_with_one_tab_and_preserves_return_focus() {
    for (width, height) in [(60, 18), (120, 30)] {
        let mut model = model();
        model.agents.profiles.profiles = vec![profile_summary(500), profile_summary(600)];
        handle_event(&mut model, TuiEvent::Resize(width, height));
        handle_event(&mut model, key(KeyCode::Char('3')));
        handle_event(&mut model, key(KeyCode::Char('s')));
        assert_eq!(model.agents.selected_profile, 1);
        let target = model.agents.profile_target().unwrap();
        model.agents.install_profile_result(
            &target,
            CommandView::AgentProfile(AgentProfileView {
                profile: profile_version(600),
                readiness: AgentReadiness::Unbound,
            }),
        );
        handle_event(&mut model, key(KeyCode::Tab));
        assert_eq!(model.focus, Focus::Workspace);
        assert_eq!(model.agents.pane, AgentsPane::Detail);
        handle_event(&mut model, key(KeyCode::Char('d')));
        handle_event(&mut model, key(KeyCode::Enter));
        assert_eq!(model.agents.pane, AgentsPane::Memory);
        handle_event(&mut model, key(KeyCode::Char('9')));
        handle_event(&mut model, key(KeyCode::Char('3')));
        assert_eq!(model.focus, Focus::Workspace);
        assert_eq!(model.agents.pane, AgentsPane::Memory);
        assert_eq!(model.agents.selected_profile, 1);
    }
}

#[test]
fn tab_exposes_profile_choices_without_an_enter_prerequisite() {
    use ai_stock_forum::ui::tui::model::AgentDetailAction;
    let mut model = model();
    model.select_view(View::Agents);
    model.agents.profiles.profiles = vec![profile_summary(500)];
    model.set_focus(Focus::List);
    handle_event(&mut model, key(KeyCode::Tab));
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.agents.selected_profile, 0);
    assert_eq!(
        model.agents.selected_detail_action,
        AgentDetailAction::Profile
    );
    handle_event(&mut model, key(KeyCode::Char('d')));
    assert_eq!(
        model.agents.selected_detail_action,
        AgentDetailAction::Memory
    );
    handle_event(&mut model, key(KeyCode::Char('d')));
    assert_eq!(
        model.agents.selected_detail_action,
        AgentDetailAction::AssignedSkills
    );
    handle_event(&mut model, key(KeyCode::Char('d')));
    assert_eq!(
        model.agents.selected_detail_action,
        AgentDetailAction::History
    );
    handle_event(&mut model, key(KeyCode::Char('d')));
    assert_eq!(
        model.agents.selected_detail_action,
        AgentDetailAction::History
    );
}

#[test]
fn profile_type_keeps_literal_invalid_fields_on_escape_and_tab() {
    use ai_stock_forum::ui::{profile_editor::ProfileTuiField, tui::model::InputMode};
    let mut model = model();
    model.select_view(View::Agents);
    model.agents.editor = Some(create_editor());
    model.agents.pane = AgentsPane::Editor;
    model.set_focus(Focus::Workspace);
    handle_event(&mut model, key(KeyCode::Tab));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().tui_field(),
        ProfileTuiField::DisplayName
    );
    handle_event(&mut model, key(KeyCode::Enter));
    assert_eq!(model.input_mode, InputMode::Type);
    model.agents.field_input.clear();
    for character in "wasd123456789/n :back".chars() {
        handle_event(&mut model, key(KeyCode::Char(character)));
    }
    handle_event(&mut model, key(KeyCode::Esc));
    assert_eq!(model.input_mode, InputMode::Nav);
    assert_eq!(
        model
            .agents
            .editor
            .as_ref()
            .unwrap()
            .tui_field_text(ProfileTuiField::DisplayName),
        "wasd123456789/n :back"
    );
    assert!(model.command.text().is_empty());
    handle_event(&mut model, key(KeyCode::Enter));
    model.agents.field_input.clear();
    handle_event(&mut model, key(KeyCode::Char(' ')));
    handle_event(&mut model, key(KeyCode::Tab));
    let editor = model.agents.editor.as_ref().unwrap();
    assert_eq!(editor.tui_field_text(ProfileTuiField::DisplayName), " ");
    assert!(
        editor
            .tui_field_error(ProfileTuiField::DisplayName)
            .is_some()
    );
    assert_eq!(editor.tui_field(), ProfileTuiField::Role);
    handle_event(&mut model, key(KeyCode::Char('1')));
    assert_eq!(model.active_view, View::Overview);
    assert!(model.agents.editor.is_some());
}

#[test]
fn logical_list_focus_routes_agent_keys_to_the_visible_profile_list() {
    let mut model = model();
    model.select_view(View::Agents);
    model.agents.profiles = AgentProfilesView {
        profiles: vec![profile_summary(30_000), profile_summary(31_000)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    };
    model.agents.detail = Some(AgentProfileView {
        profile: profile_version(30_000),
        readiness: AgentReadiness::Unbound,
    });
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail_scroll = 7;
    model.set_focus(Focus::List);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('s'))),
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Detail
        }
    );
    assert_eq!(model.agents.selected_profile, 1);
    assert_eq!(
        model.agents.detail_scroll, 0,
        "a new object starts at its own body origin"
    );

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Detail
        }
    );
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.agents.pane, AgentsPane::Detail);
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
    use ai_stock_forum::ui::profile_editor::ProfileTuiField;
    for _ in 0..11 {
        if model.agents.editor.as_ref().unwrap().tui_field() == ProfileTuiField::Review {
            return;
        }
        handle_event(model, key(KeyCode::Tab));
    }
    panic!("Review must remain reachable");
}

fn advance_editor_to_review_with_enter(model: &mut TuiModel) {
    advance_create_editor_to_review(model);
}

#[test]
fn three_opens_agents_and_remains_text_when_command_entry_owns_input() {
    let mut model = model();

    assert_eq!(
        handle_event(&mut model, navigation_key('3')),
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
fn option_alt_numeric_keys_do_not_navigate() {
    let mut model = model();
    for number in ['1', '2', '3', '4', '5', '6'] {
        let before = model.clone();
        assert_eq!(
            handle_event(&mut model, alt_tab(number)),
            ControllerEffect::None
        );
        assert_eq!(model, before);
    }
}

#[test]
fn agents_local_navigation_tracks_panes_selection_and_effects() {
    let mut model = model();
    handle_event(&mut model, navigation_key('3'));
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(10), profile_summary(11)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Detail
        }
    );
    assert_eq!(model.agents.selected_profile, 1);
    assert_eq!(model.agents.list_scroll, 0);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Detail
        }
    );
    assert_eq!(model.agents.pane, AgentsPane::Detail);
    model.agents.detail = Some(AgentProfileView {
        profile: profile_version(11),
        readiness: AgentReadiness::Unbound,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('h'))),
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::History
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
        ControllerEffect::LoadSelectedAgentProfile {
            target: empty.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Detail
        }
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
        (2, 0)
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
fn agent_list_scrolls_only_when_selection_leaves_the_visible_cards() {
    let mut model = model();
    model.select_view(View::Agents);
    model.set_terminal_size(60, 18);
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![
            profile_summary(23),
            profile_summary(24),
            profile_summary(25),
        ],
        total_count: 3,
        returned_count: 3,
        truncated: false,
    });

    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(
        (model.agents.selected_profile, model.agents.list_scroll),
        (1, 0)
    );

    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(
        (model.agents.selected_profile, model.agents.list_scroll),
        (2, 1)
    );

    handle_event(&mut model, key(KeyCode::Up));
    assert_eq!(
        (model.agents.selected_profile, model.agents.list_scroll),
        (1, 1)
    );

    handle_event(&mut model, key(KeyCode::Up));
    assert_eq!(
        (model.agents.selected_profile, model.agents.list_scroll),
        (0, 0)
    );
}

#[test]
fn agent_list_scroll_accounts_for_wrapped_cards_without_repinning_each_selection() {
    let mut model = model();
    model.select_view(View::Agents);
    model.set_terminal_size(80, 23);
    let mut wrapped = profile_summary(26);
    wrapped.display_name = "W".repeat(64);
    wrapped.primary_specialty = "S".repeat(64);
    let mut second = profile_summary(27);
    second.display_name = "Second".to_owned();
    second.primary_specialty = "short".to_owned();
    let mut third = profile_summary(28);
    third.display_name = "Third".to_owned();
    third.primary_specialty = "short".to_owned();
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![wrapped, second, third],
        total_count: 3,
        returned_count: 3,
        truncated: false,
    });

    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(
        (model.agents.selected_profile, model.agents.list_scroll),
        (1, 0),
        "the full-width compact list keeps both cards visible"
    );

    handle_event(&mut model, key(KeyCode::Down));
    assert_eq!(
        (model.agents.selected_profile, model.agents.list_scroll),
        (2, 0),
        "the viewport must stay put when the next short card already fits"
    );
}

#[test]
fn profile_refresh_preserves_a_visible_agent_list_offset() {
    let mut model = model();
    let profiles = vec![profile_summary(29), profile_summary(30)];
    model.agents.replace_profiles(AgentProfilesView {
        profiles: profiles.clone(),
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    model.agents.selected_profile = 1;
    model.agents.list_scroll = 0;

    model.agents.replace_profiles(AgentProfilesView {
        profiles,
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });

    assert_eq!(
        (model.agents.selected_profile, model.agents.list_scroll),
        (1, 0)
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
            (KeyCode::Char('7'), View::Setup),
            (KeyCode::Char('8'), View::Audit),
            (KeyCode::Char('9'), View::Help),
        ] {
            assert_eq!(
                handle_event(&mut model, navigation_key('3')),
                ControllerEffect::LoadAgentProfiles
            );
            assert_cached_geometry(&model, width, height, View::Agents);
            assert_eq!(model.agents, expected_agents_state);

            assert_eq!(
                handle_event(
                    &mut model,
                    key_event(code, KeyModifiers::NONE, KeyEventKind::Press),
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
    model.select_view(View::Agents);
    model
        .agents
        .start_profile_create(0, builtin_profile_templates());
    let draft = model.agents.editor.clone();
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('q'))),
        ControllerEffect::None
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.pane, AgentsPane::Detail);
    assert_eq!(model.agents.editor, draft);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('q'))),
        ControllerEffect::None
    );
    assert_eq!(
        handle_event(
            &mut model,
            key_event(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
                KeyEventKind::Press
            )
        ),
        ControllerEffect::RequestShutdown(ShutdownReason::Interrupted)
    );
}

#[test]
fn resize_preserves_agents_selection_scroll_and_editor_draft() {
    let mut model = model();
    handle_event(&mut model, navigation_key('3'));
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
    handle_event(&mut editor_model, navigation_key('3'));
    handle_event(&mut editor_model, key(KeyCode::Char('c')));
    assert!(
        editor_model
            .agents
            .start_profile_create(0, builtin_profile_templates())
    );
    handle_event(&mut editor_model, key(KeyCode::Tab));
    handle_event(&mut editor_model, key(KeyCode::Enter));
    editor_model.agents.field_input.clear();
    assert_eq!(
        handle_event(&mut editor_model, key(KeyCode::Char('q'))),
        ControllerEffect::Redraw
    );
    assert_eq!(editor_model.agents.field_input.text(), "q");

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
    handle_event(&mut local_model, navigation_key('3'));
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
    assert_eq!(editor_model.agents.field_input.text(), "q");

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
    model.set_terminal_size(60, 18);
    handle_event(&mut model, navigation_key('3'));
    model.agents.replace_profiles(AgentProfilesView {
        profiles: vec![profile_summary(40), profile_summary(41)],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    });
    handle_event(&mut model, key(KeyCode::Down));
    model.agents.detail = Some(AgentProfileView {
        profile: profile_version(41),
        readiness: AgentReadiness::Unbound,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('e'))),
        ControllerEffect::StartSelectedProfileEdit {
            target: model.agents.profile_target().unwrap()
        }
    );
    assert_eq!(model.focus, Focus::Workspace);

    model.agents.pane = AgentsPane::List;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Detail
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
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::History
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
    advance_create_editor_to_review(&mut preview_model);
    match handle_event(&mut preview_model, key(KeyCode::Enter)) {
        ControllerEffect::RequestProfilePreview(request) => {
            assert_eq!(request.generation, 1);
            assert_eq!(
                request.profile_id,
                AgentProfileId::from_uuid(Uuid::from_u128(3))
            );
        }
        effect => panic!("expected preview, got {effect:?}"),
    }
    handle_event(&mut preview_model, key(KeyCode::Tab));
    assert_eq!(
        handle_event(&mut preview_model, key(KeyCode::Enter)),
        ControllerEffect::CancelProfileReview
    );
    assert!(preview_model.agents.editor.is_none());
    let mut confirmation_model = model();
    confirmation_model.active_view = View::Agents;
    confirmation_model.agents.pane = AgentsPane::Editor;
    confirmation_model.agents.editor = Some(create_editor());
    advance_create_editor_to_review(&mut confirmation_model);
    handle_event(&mut confirmation_model, key(KeyCode::Enter));
    assert_eq!(confirmation_model.agents.pane, AgentsPane::Confirmation);
    handle_event(&mut confirmation_model, key(KeyCode::Esc));
    assert_eq!(confirmation_model.agents.pane, AgentsPane::Editor);
    assert!(confirmation_model.agents.editor.is_some());
}

#[test]
fn keyboard_create_path_cycles_complete_templates_and_uses_enter_only() {
    use ai_stock_forum::ui::profile_editor::ProfileTuiField;
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(create_editor());
    handle_event(&mut model, key(KeyCode::Char('s')));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft(),
        &builtin_profile_templates()[1].copy_to_draft().unwrap()
    );
    handle_event(&mut model, key(KeyCode::Char('w')));
    handle_event(&mut model, key(KeyCode::Char('w')));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft().role,
        AgentRole::Bull
    );
    handle_event(&mut model, key(KeyCode::Char('s')));
    handle_event(&mut model, key(KeyCode::Tab));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().tui_field(),
        ProfileTuiField::DisplayName
    );
    handle_event(&mut model, key(KeyCode::Enter));
    model.agents.field_input.clear();
    enter_line(&mut model, "Keyboard Bear");
    advance_create_editor_to_review(&mut model);
    handle_event(&mut model, key(KeyCode::Enter));
    assert_eq!(model.agents.pane, AgentsPane::Confirmation);
    let Some(ProfileConfirmation {
        command:
            ApplicationCommand::CreateAgentProfile {
                draft,
                template_provenance,
            },
    }) = model.agents.pending_confirmation.as_ref()
    else {
        panic!("create confirmation");
    };
    assert_eq!(draft.display_name, "Keyboard Bear");
    assert_eq!(draft.role, AgentRole::Bear);
    assert_eq!(
        template_provenance.as_ref().unwrap().template_id.as_str(),
        "builtin.bear"
    );
    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::ExecuteProfile(ApplicationCommand::CreateAgentProfile { .. })
    ));
}

#[test]
fn edit_template_step_ignores_arrows_but_keeps_enter_and_role_alias() {
    use ai_stock_forum::ui::profile_editor::ProfileTuiField;
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(edit_editor());
    let draft = model.agents.editor.as_ref().unwrap().draft().clone();
    handle_event(&mut model, key(KeyCode::Down));
    handle_event(&mut model, key(KeyCode::Up));
    assert_eq!(model.agents.editor.as_ref().unwrap().draft(), &draft);
    handle_event(&mut model, key(KeyCode::Tab));
    handle_event(&mut model, key(KeyCode::Tab));
    assert_eq!(
        model.agents.editor.as_ref().unwrap().tui_field(),
        ProfileTuiField::Role
    );
    handle_event(&mut model, key(KeyCode::Char('d')));
    let editor = model.agents.editor.as_ref().unwrap();
    assert_eq!(editor.draft().role, AgentRole::Bear);
    assert_eq!(editor.draft().display_name, "Bull Researcher");
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
    let mut row = profile_summary(700);
    row.profile_version_id = active_version_id;
    model.agents.profiles.profiles = vec![row];
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
        ControllerEffect::LoadSelectedAgentProfile {
            target: model.agents.profile_target().unwrap(),
            read: ai_stock_forum::ui::tui::model::AgentProfileRead::Version(
                ObjectVersion::new(1).unwrap()
            )
        }
    );
    let old_version_target = model.agents.profile_target().unwrap();
    handle_event(&mut model, key(KeyCode::Up));
    assert!(
        !model.agents.install_profile_result(
            &old_version_target,
            CommandView::AgentProfileVersion(AgentProfileVersionView {
                profile: profile_version(700),
                readiness: AgentReadiness::Unbound,
                predecessor_diff: Vec::new(),
            })
        ),
        "moving history selection must reject the previous version response"
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
    use ai_stock_forum::ui::{profile_editor::ProfileTuiField, tui::model::InputMode};
    for modifiers in enter_modifier_variants() {
        let mut model = model();
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Editor;
        model.agents.editor = Some(create_editor());
        handle_event(&mut model, key(KeyCode::Tab));
        handle_event(&mut model, key(KeyCode::Enter));
        model.agents.field_input.clear();
        for character in ":next".chars() {
            handle_event(&mut model, key(KeyCode::Char(character)));
        }
        handle_event(
            &mut model,
            key_event(KeyCode::Enter, modifiers, KeyEventKind::Press),
        );
        assert_eq!(model.input_mode, InputMode::Nav);
        assert_eq!(
            model
                .agents
                .editor
                .as_ref()
                .unwrap()
                .tui_field_text(ProfileTuiField::DisplayName),
            ":next"
        );
        assert!(model.command.text().is_empty());
    }
}

#[test]
fn every_modified_enter_is_inert_during_profile_confirmation() {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(create_editor());
    advance_create_editor_to_review(&mut model);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
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
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
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
