use ai_stock_forum::{
    agents::builtin_profile_templates,
    app::{
        AgentProfilesView, ApplicationCommand, CommandOutcome, CommandView, DatabaseReadiness,
        PresentationSnapshot, ProcessGuardOwnership, ShutdownDisposition, SkillsView, StatusView,
    },
    domain::{CommandId, CorrelationId, InstallationId, SessionId, SkillId},
    setup::SetupStatus,
    ui::tui::{
        ControllerEffect, TuiEvent, apply_outcome, handle_event,
        model::{
            AgentsPane, Focus, LayoutMode, ProfileConfirmation, SkillConfirmation,
            SkillOperationOrigin, SkillsPane, TuiModel, View,
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
fn bare_five_six_and_function_keys_never_switch_tabs() {
    for character in ['5', '6'] {
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
