mod support;

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use ai_stock_forum::{
    agents::{
        AgentProfileDraft, ProfileEditPreview, builtin_profile_templates,
    },
    app::{
        AgentProfileCreatedView, AgentProfilesView, AppError, ApplicationCommand,
        ApplicationService, CommandEnvelope, CommandOutcome, CommandView, DatabaseReadiness,
        HelpView, PresentationSnapshot, ProcessGuardOwnership, ShutdownDisposition, ShutdownReason,
    },
    config::AppPaths,
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId,
        ProfileReviewToken, SessionId, sha256,
    },
    runtime::{ApplicationRuntime, CommandExecutor},
    setup::SetupStatus,
    ui::{
        profile_editor::{PreviewEditRequest, ProfileEditor, ProfileEditorEffect},
        tui::{
            ControllerEffect, EventSource, Screen, TuiError, TuiEvent, execute_agent_effect,
            model::{AgentsPane, TuiModel},
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

fn create_profile(service: &mut ApplicationService, template_index: usize) -> AgentProfileCreatedView {
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
fn presentation_snapshot_and_host_effects_cover_load_create_edit_preview_activate_history_and_cancel() {
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
        ControllerEffect::LoadAgentProfile { selected_profile: 0 },
        ControllerEffect::LoadAgentProfileHistory { selected_profile: 0 },
        ControllerEffect::StartProfileEdit { selected_profile: 0 },
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

    execute_agent_effect(
        &client,
        &mut model,
        ControllerEffect::CancelProfileReview,
    )
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
        self.previews.lock().unwrap().push((
            profile_id,
            expected_active_version_id,
            candidate,
        ));
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
        ControllerEffect::StartProfileEdit { selected_profile: 0 },
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
        ControllerEffect::LoadAgentProfileHistory { selected_profile: 0 },
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
            ApplicationCommand::ListAgentProfiles => CommandView::AgentProfiles(AgentProfilesView {
                profiles: Vec::new(),
            }),
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

    fn cancel_agent_profile_edit(&mut self) {
        self.order.lock().unwrap().push("cancel");
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        self.order.lock().unwrap().push("finish");
        Ok(())
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
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)),
            TuiEvent::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            )),
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

    assert_eq!(order.lock().unwrap().as_slice(), ["cancel", "restore", "finish"]);
}
