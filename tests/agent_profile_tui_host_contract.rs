mod support;

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use ai_stock_forum::{
    agents::{
        AgentProfileDraft, AgentProfileVersion, AgentReadiness, ProfileEditPreview,
        ProfileTemplate, builtin_profile_templates,
    },
    app::{
        AgentProfileCreatedView, AgentProfileSummary, AgentProfileView, AgentProfilesView,
        AppError, ApplicationCommand, ApplicationService, CommandEnvelope, CommandOutcome,
        CommandView, DatabaseReadiness, HelpView, PresentationSnapshot, ProcessGuardOwnership,
        ShutdownDisposition, ShutdownReason,
    },
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, MemoryNamespaceId,
        ProfileReviewToken, SessionId, sha256,
    },
    runtime::{ApplicationRuntime, CommandExecutor},
    setup::SetupStatus,
    ui::{
        profile_editor::{PreviewEditRequest, ProfileEditor, ProfileEditorEffect},
        tui::{
            ControllerEffect, EventSource, Screen, TuiError, TuiEvent, execute_agent_effect,
            layout::view_geometry,
            model::{AgentsPane, LayoutMode, TuiModel, View},
            run_tui_with_screen,
            theme::Theme,
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use tempfile::TempDir;
use uuid::Uuid;

fn empty_snapshot() -> PresentationSnapshot {
    PresentationSnapshot {
        installation_id: ai_stock_forum::domain::InstallationId::from_uuid(Uuid::from_u128(1)),
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

fn service() -> (TempDir, AppPaths, ApplicationService) {
    let temporary_directory = tempfile::tempdir().expect("tempdir");
    let paths = AppPaths::for_test(temporary_directory.path());
    let service = ApplicationService::bootstrap(
        &paths,
        Arc::new(support::TestClock::new()),
        Arc::new(support::TestIds::new()),
    )
    .expect("service");
    (temporary_directory, paths, service)
}

fn create_profile(
    service: &mut ApplicationService,
    template_index: usize,
) -> AgentProfileCreatedView {
    let template = &builtin_profile_templates()[template_index];
    let outcome = service
        .execute_user(ApplicationCommand::CreateAgentProfile {
            draft: template.copy_to_draft().expect("template draft"),
            template_provenance: Some(template.provenance()),
        })
        .expect("create profile");
    let CommandView::AgentProfileCreated(created) = outcome.view else {
        panic!("created view")
    };
    created
}

fn advance_edit_to_preview(editor: &mut ProfileEditor, display_name: &str) -> PreviewEditRequest {
    assert_eq!(editor.submit_line(":next"), ProfileEditorEffect::None);
    assert_eq!(editor.submit_line(display_name), ProfileEditorEffect::None);
    for _ in 0..6 {
        assert_eq!(editor.submit_line(":next"), ProfileEditorEffect::None);
    }
    let ProfileEditorEffect::PreviewEdit(request) = editor.submit_line(":review") else {
        panic!("preview request")
    };
    request
}

#[test]
fn explicit_create_leaves_command_type_and_opens_a_nav_template_picker() {
    use ai_stock_forum::ui::tui::{
        handle_event,
        model::{Focus, InputMode},
    };
    let (_temp, _paths, service) = service();
    let runtime = ApplicationRuntime::spawn(service, 2).unwrap();
    let client = runtime.client();
    let mut model = TuiModel::new(empty_snapshot(), false);
    model.select_view(View::Agents);
    model.set_focus(Focus::Command);
    execute_agent_effect(
        &client,
        &mut model,
        ControllerEffect::StartProfileCreateByTemplate {
            template_id: builtin_profile_templates()[0].id.clone(),
        },
    )
    .unwrap();
    assert_eq!(model.input_mode, InputMode::Nav);
    assert_eq!(model.focus, Focus::Workspace);
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
    );
    assert_eq!(
        model.agents.editor.as_ref().unwrap().draft().role,
        builtin_profile_templates()[1].role
    );
    assert!(model.command.text().is_empty());
    runtime
        .finish_and_join(ShutdownReason::Interrupted)
        .unwrap();
}

#[derive(Clone)]
struct TemplateReadRecorder {
    calls: Arc<Mutex<usize>>,
    fail: bool,
}

impl CommandExecutor for TemplateReadRecorder {
    fn execute_user(&mut self, _command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        Err(AppError::AgentProfileNotFound)
    }

    fn agent_profile_templates(&mut self) -> Result<Vec<ProfileTemplate>, AppError> {
        *self.calls.lock().unwrap() += 1;
        if self.fail {
            Err(AppError::LifecycleFinished)
        } else {
            Ok(builtin_profile_templates().to_vec())
        }
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

#[test]
fn start_create_uses_the_service_template_outcome_and_failure_preserves_model_state() {
    for fail in [false, true] {
        let calls = Arc::new(Mutex::new(0));
        let runtime = ApplicationRuntime::spawn(
            TemplateReadRecorder {
                calls: calls.clone(),
                fail,
            },
            4,
        )
        .expect("runtime");
        let mut model = TuiModel::new(empty_snapshot(), false);
        model.active_view = ai_stock_forum::ui::tui::model::View::Agents;

        let result = execute_agent_effect(
            &runtime.client(),
            &mut model,
            ControllerEffect::StartProfileCreate { template_index: 0 },
        );

        assert_eq!(*calls.lock().unwrap(), 1);
        if fail {
            assert!(result.is_err());
            assert_eq!(model.agents.pane, AgentsPane::List);
            assert!(model.agents.editor.is_none());
            assert!(model.message.is_none());
        } else {
            result.expect("typed template read");
            assert_eq!(model.agents.pane, AgentsPane::Editor);
            assert!(model.agents.editor.is_some());
        }
        runtime
            .finish_and_join(ShutdownReason::UserQuit)
            .expect("finish runtime");
    }
}

#[test]
fn presentation_snapshot_and_host_effects_cover_load_create_edit_preview_activate_history_and_cancel()
 {
    let (_temporary_directory, _paths, mut service) = service();
    let created = create_profile(&mut service, 0);
    let snapshot = service
        .presentation_snapshot(ai_stock_forum::app::AuditLimit::new(20).unwrap())
        .expect("typed presentation snapshot");

    assert_eq!(snapshot.agent_profiles.profiles.len(), 1);
    assert_eq!(
        snapshot
            .selected_agent_profile
            .as_ref()
            .map(|detail| detail.profile.profile_id()),
        Some(created.profile_id)
    );
    assert_eq!(
        snapshot
            .selected_agent_profile_history
            .as_ref()
            .map(|history| history.active_version_id),
        Some(created.profile_version_id)
    );

    let runtime = ApplicationRuntime::spawn_application(service, 32).expect("runtime");
    let client = runtime.client();
    let mut model = TuiModel::new(snapshot, false);

    for effect in [
        ControllerEffect::LoadAgentProfiles,
        ControllerEffect::LoadAgentProfile {
            selected_profile: 0,
        },
        ControllerEffect::LoadAgentProfileHistory {
            selected_profile: 0,
        },
        ControllerEffect::StartProfileEdit {
            selected_profile: 0,
        },
    ] {
        execute_agent_effect(&client, &mut model, effect).expect("host effect");
    }
    assert!(model.agents.editor.is_some());
    assert!(model.agents.history.is_some());

    let request = advance_edit_to_preview(
        model.agents.editor.as_mut().expect("edit editor"),
        "Updated Bull Analyst",
    );
    execute_agent_effect(
        &client,
        &mut model,
        ControllerEffect::RequestProfilePreview(request),
    )
    .expect("preview effect");
    assert!(
        model
            .agents
            .editor
            .as_ref()
            .and_then(ProfileEditor::review)
            .is_some()
    );

    let ProfileEditorEffect::Execute(activate) = model
        .agents
        .editor
        .as_mut()
        .expect("edit editor")
        .submit_line(":activate")
    else {
        panic!("activate command")
    };
    execute_agent_effect(
        &client,
        &mut model,
        ControllerEffect::ExecuteProfile(activate),
    )
    .expect("activation effect");
    assert_eq!(
        model
            .agents
            .detail
            .as_ref()
            .expect("refreshed detail")
            .profile
            .version()
            .get(),
        2
    );

    let create_template = &builtin_profile_templates()[1];
    let create_editor = ProfileEditor::for_create(create_template).expect("create editor");
    model.agents.editor = Some(create_editor);
    execute_agent_effect(
        &client,
        &mut model,
        ControllerEffect::StartProfileCreate { template_index: 1 },
    )
    .expect("start create effect");
    execute_agent_effect(
        &client,
        &mut model,
        ControllerEffect::ExecuteProfile(ApplicationCommand::CreateAgentProfile {
            draft: create_template.copy_to_draft().expect("create draft"),
            template_provenance: Some(create_template.provenance()),
        }),
    )
    .expect("create effect");
    assert_eq!(model.agents.profiles.profiles.len(), 2);

    execute_agent_effect(&client, &mut model, ControllerEffect::CancelProfileReview)
        .expect("cancel effect");
    runtime
        .finish_and_join(ShutdownReason::UserQuit)
        .expect("finish runtime");
}

#[derive(Clone)]
struct PreviewRecorder {
    previews: Arc<Mutex<Vec<(AgentProfileId, AgentProfileVersionId, AgentProfileDraft)>>>,
}

impl CommandExecutor for PreviewRecorder {
    fn execute_user(&mut self, _command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        Err(AppError::AgentProfileNotFound)
    }

    fn preview_agent_profile_edit(
        &mut self,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate: AgentProfileDraft,
    ) -> Result<ProfileEditPreview, AppError> {
        self.previews
            .lock()
            .unwrap()
            .push((profile_id, expected_active_version_id, candidate));
        Ok(ProfileEditPreview {
            profile_id,
            expected_active_version_id,
            diffs: Vec::new(),
            review_token: ProfileReviewToken::from_uuid(Uuid::from_u128(90)),
            review_digest: sha256(b"preview-round-trip"),
        })
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

#[test]
fn preview_request_generation_and_payload_round_trip_exactly() {
    let template = &builtin_profile_templates()[0];
    let profile_id = AgentProfileId::from_uuid(Uuid::from_u128(70));
    let version_id = AgentProfileVersionId::from_uuid(Uuid::from_u128(71));
    let editor = ProfileEditor::for_edit(
        profile_id,
        version_id,
        template.copy_to_draft().expect("edit draft"),
    );
    let previews = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        PreviewRecorder {
            previews: previews.clone(),
        },
        4,
    )
    .expect("runtime");
    let mut model = TuiModel::new(empty_snapshot(), false);
    model.agents.pane = AgentsPane::Editor;
    model.agents.editor = Some(editor);
    let request = advance_edit_to_preview(
        model.agents.editor.as_mut().expect("editor"),
        "Exact Candidate",
    );
    let expected = request.clone();

    execute_agent_effect(
        &runtime.client(),
        &mut model,
        ControllerEffect::RequestProfilePreview(request),
    )
    .expect("preview execution");

    assert_eq!(
        previews.lock().unwrap().as_slice(),
        [(
            expected.profile_id,
            expected.expected_active_version_id,
            expected.candidate,
        )]
    );
    assert!(
        model
            .agents
            .editor
            .as_ref()
            .and_then(ProfileEditor::review)
            .is_some(),
        "the exact generation was fed back to the editor"
    );
    runtime
        .finish_and_join(ShutdownReason::UserQuit)
        .expect("finish runtime");
}

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 1_000)),
        actor: Actor::Human,
        command,
    }
}

#[test]
fn stale_preview_refreshes_current_detail_and_never_rebases_or_activates() {
    let (_temporary_directory, _paths, mut service) = service();
    let created = create_profile(&mut service, 0);
    let mut concurrent = service
        .independent_profile_instance()
        .expect("independent profile service");
    let snapshot = service
        .presentation_snapshot(ai_stock_forum::app::AuditLimit::new(20).unwrap())
        .expect("snapshot");
    let runtime = ApplicationRuntime::spawn_application(service, 32).expect("runtime");
    let client = runtime.client();
    let mut model = TuiModel::new(snapshot, false);
    execute_agent_effect(
        &client,
        &mut model,
        ControllerEffect::StartProfileEdit {
            selected_profile: 0,
        },
    )
    .expect("start stale editor");

    let mut concurrent_candidate = builtin_profile_templates()[0]
        .copy_to_draft()
        .expect("concurrent draft");
    concurrent_candidate.display_name = "Concurrent Winner".to_owned();
    let preview = concurrent
        .preview_agent_profile_edit(
            created.profile_id,
            created.profile_version_id,
            concurrent_candidate.clone(),
        )
        .expect("concurrent preview");
    concurrent
        .execute(envelope(
            10_000,
            ApplicationCommand::ActivateAgentProfileVersion {
                profile_id: created.profile_id,
                expected_active_version_id: created.profile_version_id,
                candidate: concurrent_candidate,
                review_token: preview.review_token,
                review_digest: preview.review_digest,
            },
        ))
        .expect("concurrent activation");

    let stale_request = advance_edit_to_preview(
        model.agents.editor.as_mut().expect("stale editor"),
        "Losing Candidate",
    );
    execute_agent_effect(
        &client,
        &mut model,
        ControllerEffect::RequestProfilePreview(stale_request),
    )
    .expect("stale conflict is handled in the model");

    assert_eq!(model.agents.pane, AgentsPane::Detail);
    assert!(model.agents.editor.is_none());
    let detail = model.agents.detail.as_ref().expect("refreshed detail");
    assert_eq!(detail.profile.display_name(), "Concurrent Winner");
    assert_eq!(detail.profile.version().get(), 2);
    assert_eq!(
        model.message.as_ref().map(|message| message.text.as_str()),
        Some("Profile changed elsewhere. Detail refreshed; reopen Edit to continue.")
    );

    execute_agent_effect(
        &client,
        &mut model,
        ControllerEffect::LoadAgentProfileHistory {
            selected_profile: 0,
        },
    )
    .expect("history refresh");
    assert_eq!(model.agents.history.as_ref().unwrap().versions.len(), 2);
    runtime
        .finish_and_join(ShutdownReason::UserQuit)
        .expect("finish runtime");
}

struct OrderingExecutor {
    order: Arc<Mutex<Vec<&'static str>>>,
}

impl CommandExecutor for OrderingExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        let view = match command {
            ApplicationCommand::ListAgentProfiles => {
                CommandView::AgentProfiles(AgentProfilesView {
                    profiles: Vec::new(),
                    total_count: 0,
                    returned_count: 0,
                    truncated: false,
                })
            }
            _ => CommandView::Help(HelpView),
        };
        Ok(CommandOutcome {
            command_id: CommandId::from_uuid(Uuid::from_u128(200)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(201)),
            committed_events: Vec::new(),
            view,
            shutdown: ShutdownDisposition::Continue,
        })
    }

    fn agent_profile_templates(&mut self) -> Result<Vec<ProfileTemplate>, AppError> {
        Ok(builtin_profile_templates().to_vec())
    }

    fn cancel_agent_profile_edit(&mut self) -> Result<(), AppError> {
        self.order.lock().unwrap().push("cancel");
        Ok(())
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        self.order.lock().unwrap().push("finish");
        Ok(())
    }
}

struct StaleCleanupExecutor {
    profile: AgentProfileVersion,
    commands: Arc<Mutex<Vec<&'static str>>>,
    fail_refresh: bool,
}

impl CommandExecutor for StaleCleanupExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        let label = match command {
            ApplicationCommand::ListAgentProfiles => "list",
            ApplicationCommand::ShowAgentProfile { .. } => "detail",
            _ => "other",
        };
        self.commands.lock().unwrap().push(label);
        if self.fail_refresh {
            return Err(AppError::AgentProfileNotFound);
        }
        let readiness = AgentReadiness::Unbound;
        let view = match command {
            ApplicationCommand::ListAgentProfiles => {
                CommandView::AgentProfiles(AgentProfilesView {
                    profiles: vec![AgentProfileSummary {
                        profile_id: self.profile.profile_id(),
                        profile_version_id: self.profile.profile_version_id(),
                        version: self.profile.version(),
                        display_name: self.profile.display_name().to_owned(),
                        role: self.profile.role(),
                        primary_specialty: self.profile.primary_specialty().to_owned(),
                        readiness,
                        content_digest: self.profile.content_digest().clone(),
                    }],
                    total_count: 1,
                    returned_count: 1,
                    truncated: false,
                })
            }
            ApplicationCommand::ShowAgentProfile { .. } => {
                CommandView::AgentProfile(AgentProfileView {
                    profile: self.profile.clone(),
                    readiness,
                })
            }
            _ => return Err(AppError::AgentProfileNotFound),
        };
        Ok(CommandOutcome {
            command_id: CommandId::from_uuid(Uuid::from_u128(40_000)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(40_001)),
            committed_events: Vec::new(),
            view,
            shutdown: ShutdownDisposition::Continue,
        })
    }

    fn preview_agent_profile_edit(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_version_id: AgentProfileVersionId,
        _candidate: AgentProfileDraft,
    ) -> Result<ProfileEditPreview, AppError> {
        Err(AppError::StaleAgentProfileVersion)
    }

    fn cancel_agent_profile_edit(&mut self) -> Result<(), AppError> {
        Err(AppError::LifecycleFinished)
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

fn stale_cleanup_profile() -> AgentProfileVersion {
    let template = &builtin_profile_templates()[0];
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(30_000)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(30_001)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(30_002)),
        1_800_000_000_000,
        template.copy_to_draft().expect("draft"),
        Some(template.provenance()),
    )
    .expect("profile")
}

#[test]
fn stale_cleanup_preserves_cancel_failure_when_refresh_succeeds_or_fails() {
    for fail_refresh in [false, true] {
        let profile = stale_cleanup_profile();
        let commands = Arc::new(Mutex::new(Vec::new()));
        let runtime = ApplicationRuntime::spawn(
            StaleCleanupExecutor {
                profile: profile.clone(),
                commands: commands.clone(),
                fail_refresh,
            },
            8,
        )
        .expect("runtime");
        let mut model = TuiModel::new(empty_snapshot(), false);
        model.active_view = ai_stock_forum::ui::tui::model::View::Agents;
        model.agents.pane = AgentsPane::Editor;
        model.agents.editor = Some(ProfileEditor::for_edit(
            profile.profile_id(),
            profile.profile_version_id(),
            builtin_profile_templates()[0]
                .copy_to_draft()
                .expect("draft"),
        ));
        let request = PreviewEditRequest {
            generation: 1,
            profile_id: profile.profile_id(),
            expected_active_version_id: profile.profile_version_id(),
            candidate: builtin_profile_templates()[0]
                .copy_to_draft()
                .expect("candidate"),
        };

        execute_agent_effect(
            &runtime.client(),
            &mut model,
            ControllerEffect::RequestProfilePreview(request),
        )
        .expect("stale cleanup is represented in safe local state");

        assert_eq!(commands.lock().unwrap().as_slice(), ["list", "detail"]);
        assert_eq!(model.agents.pane, AgentsPane::Detail);
        assert!(model.agents.editor.is_none());
        let message = &model.message.as_ref().expect("safe diagnostic").text;
        if fail_refresh {
            assert_eq!(
                message,
                "Profile changed elsewhere. Review cleanup and profile refresh both failed."
            );
        } else {
            assert_eq!(
                message,
                "Profile changed elsewhere. Detail refreshed, but review cleanup could not be confirmed."
            );
        }
        runtime
            .finish_and_join(ShutdownReason::UserQuit)
            .expect("finish runtime");
    }
}

struct OneInterrupt {
    events: VecDeque<TuiEvent>,
}

impl EventSource for OneInterrupt {
    fn next_event(&mut self, _timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
        Ok(self.events.pop_front())
    }
}

struct OrderingScreen {
    order: Arc<Mutex<Vec<&'static str>>>,
}

impl Screen for OrderingScreen {
    fn size(&self) -> Result<Rect, TuiError> {
        Ok(Rect::new(0, 0, 120, 30))
    }

    fn draw(&mut self, _model: &TuiModel, _theme: &Theme) -> Result<(), TuiError> {
        Ok(())
    }

    fn restore(&mut self) -> Result<(), TuiError> {
        self.order.lock().unwrap().push("restore");
        Ok(())
    }
}

#[test]
fn shutdown_cancels_service_review_before_terminal_restoration() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        OrderingExecutor {
            order: order.clone(),
        },
        4,
    )
    .expect("runtime");
    let mut screen = OrderingScreen {
        order: order.clone(),
    };
    let mut events = OneInterrupt {
        events: VecDeque::from([
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE)),
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)),
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        ]),
    };

    run_tui_with_screen(
        runtime,
        empty_snapshot(),
        false,
        &mut screen,
        &mut events,
        &Theme::from_no_color(true),
    )
    .expect("clean shutdown");

    assert_eq!(
        order.lock().unwrap().as_slice(),
        ["cancel", "restore", "finish"]
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObservedGeometry {
    view: View,
    mode: LayoutMode,
    terminal_width: u16,
    terminal_height: u16,
    body_width: u16,
    body_height: u16,
}

struct GeometryScreen {
    area: Rect,
    draws: Arc<Mutex<Vec<ObservedGeometry>>>,
}

impl Screen for GeometryScreen {
    fn size(&self) -> Result<Rect, TuiError> {
        Ok(self.area)
    }

    fn draw(&mut self, model: &TuiModel, _theme: &Theme) -> Result<(), TuiError> {
        self.draws.lock().unwrap().push(ObservedGeometry {
            view: model.active_view,
            mode: model.layout_mode,
            terminal_width: model.terminal_width,
            terminal_height: model.terminal_height,
            body_width: model.workspace_body_width,
            body_height: model.workspace_body_height,
        });
        Ok(())
    }

    fn restore(&mut self) -> Result<(), TuiError> {
        Ok(())
    }
}

fn observed(area: Rect, view: View) -> ObservedGeometry {
    let geometry = view_geometry(area, view, false);
    ObservedGeometry {
        view,
        mode: geometry.cockpit.mode,
        terminal_width: area.width,
        terminal_height: area.height,
        body_width: geometry.workspace_body_width,
        body_height: geometry.workspace_body_height,
    }
}

#[test]
fn host_initializes_and_round_trips_view_geometry_without_resize() {
    for area in [Rect::new(0, 0, 80, 18), Rect::new(0, 0, 120, 18)] {
        let order = Arc::new(Mutex::new(Vec::new()));
        let runtime = ApplicationRuntime::spawn(
            OrderingExecutor {
                order: order.clone(),
            },
            4,
        )
        .expect("runtime");
        let draws = Arc::new(Mutex::new(Vec::new()));
        let mut screen = GeometryScreen {
            area,
            draws: draws.clone(),
        };
        let mut events = OneInterrupt {
            events: VecDeque::from([
                TuiEvent::Key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE)),
                TuiEvent::Key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
                TuiEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            ]),
        };

        run_tui_with_screen(
            runtime,
            empty_snapshot(),
            false,
            &mut screen,
            &mut events,
            &Theme::from_no_color(true),
        )
        .expect("geometry round trip");

        assert_eq!(
            draws.lock().unwrap().as_slice(),
            [
                observed(area, View::Overview),
                observed(area, View::Agents),
                observed(area, View::Overview),
            ]
        );
    }
}
