use std::{collections::VecDeque, sync::{Arc, Mutex}, time::Duration};

use ai_stock_forum::{
    app::{
        AgentProfileView, AgentSkillAssignmentOperation, AgentSkillAssignmentPreview, AppError,
        ApplicationCommand, CommandOutcome, CommandView, PresentationSnapshot,
        ShutdownDisposition, ShutdownReason, ShutdownView, SkillsView,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, InstallationId,
        MemoryNamespaceId, SessionId, SkillId, SkillReviewToken, SkillVersionId, sha256,
    },
    runtime::{ApplicationRuntime, CommandExecutor},
    policy::{Capability, PolicyDecision},
    setup::SetupStatus,
    skills::{SkillDraft, SkillEditPreview},
    ui::{
        skill_editor::{SkillEditor, SkillEditorEffect, SkillPreviewRequest},
        tui::{
            ControllerEffect, EventSource, Screen, TuiError, TuiEvent,
            execute_skill_effect, handle_event, run_tui_with_screen,
            model::{AgentsPane, SkillsPane, TuiModel, View},
            theme::Theme,
        },
    },
};
use uuid::Uuid;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Create,
    Version(SkillId, SkillVersionId, SkillDraft),
    Assign,
    Upgrade,
    Unassign,
    Cancel,
    Mutation,
    Finish,
}

struct RouteRecorder {
    calls: Arc<Mutex<Vec<Call>>>,
    execute_error: Option<AppError>,
}

struct ScriptedEvents {
    events: VecDeque<Result<Option<TuiEvent>, TuiError>>,
}

impl EventSource for ScriptedEvents {
    fn next_event(&mut self, _timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
        let event = self.events.pop_front().unwrap_or(Err(TuiError::TerminalInput));
        if matches!(event, Ok(None)) {
            std::thread::sleep(Duration::from_millis(1));
        }
        event
    }
}

struct ContractScreen {
    fail_on_review: bool,
}

impl Screen for ContractScreen {
    fn size(&self) -> Result<ratatui::layout::Rect, TuiError> {
        Ok(ratatui::layout::Rect::new(0, 0, 100, 40))
    }

    fn draw(&mut self, model: &TuiModel, _theme: &Theme) -> Result<(), TuiError> {
        if self.fail_on_review && model.skills.review_registered {
            return Err(TuiError::TerminalOutput);
        }
        Ok(())
    }

    fn restore(&mut self) -> Result<(), TuiError> {
        Ok(())
    }
}

impl CommandExecutor for RouteRecorder {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        match command {
            ApplicationCommand::ListSkills => Ok(outcome(CommandView::Skills(SkillsView {
                skills: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
            }))),
            ApplicationCommand::RequestShutdown => Ok(CommandOutcome {
                command_id: CommandId::from_uuid(Uuid::from_u128(701)),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(702)),
                committed_events: Vec::new(),
                view: CommandView::Shutdown(ShutdownView {
                    disposition: ShutdownDisposition::Requested,
                }),
                shutdown: ShutdownDisposition::Requested,
            }),
            ApplicationCommand::CreateSkill { .. }
            | ApplicationCommand::ActivateSkillVersion { .. }
            | ApplicationCommand::AssignAgentSkill { .. }
            | ApplicationCommand::UpgradeAgentSkill { .. }
            | ApplicationCommand::UnassignAgentSkill { .. } => {
                self.calls.lock().unwrap().push(Call::Mutation);
                Err(self.execute_error.take().unwrap_or(AppError::LifecycleFinished))
            }
            _ => Err(self.execute_error.take().unwrap_or(AppError::LifecycleFinished)),
        }
    }

    fn preview_skill_creation(
        &mut self,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Create);
        Ok(SkillEditPreview {
            skill_id: SkillId::from_uuid(Uuid::from_u128(703)),
            expected_active_version_id: None,
            candidate_digest: sha256(candidate.display_name.as_bytes()),
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(704)),
            review_digest: sha256(b"create-review"),
        })
    }

    fn preview_skill_version(
        &mut self,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Version(
            skill_id,
            expected_active_version_id,
            candidate.clone(),
        ));
        Ok(SkillEditPreview {
            skill_id,
            expected_active_version_id: Some(expected_active_version_id),
            candidate_digest: sha256(b"candidate"),
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(90)),
            review_digest: sha256(b"review"),
        })
    }

    fn preview_agent_skill_assignment(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_profile_version_id: AgentProfileVersionId,
        _skill: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<ai_stock_forum::app::AgentSkillAssignmentPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Assign);
        Err(AppError::SkillReviewUnavailable)
    }

    fn preview_agent_skill_upgrade(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_profile_version_id: AgentProfileVersionId,
        _expected: ai_stock_forum::skills::SkillVersionRef,
        _replacement: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<ai_stock_forum::app::AgentSkillAssignmentPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Upgrade);
        Err(AppError::SkillReviewUnavailable)
    }

    fn preview_agent_skill_unassignment(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_profile_version_id: AgentProfileVersionId,
        expected: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<ai_stock_forum::app::AgentSkillAssignmentPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Unassign);
        Ok(AgentSkillAssignmentPreview {
            profile_id: _profile_id,
            expected_active_profile_version_id: _expected_active_profile_version_id,
            operation: AgentSkillAssignmentOperation::Unassign { expected },
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(91)),
            review_digest: sha256(b"unassign-review"),
        })
    }

    fn cancel_skill_review(&mut self) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(Call::Cancel);
        match self.execute_error.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(Call::Finish);
        Ok(())
    }
}

fn draft() -> SkillDraft {
    SkillDraft::new(
        "Version Route".to_owned(),
        "Purpose".to_owned(),
        "Use when routing previews.".to_owned(),
        Vec::new(),
        "Keep transport typed.".to_owned(),
        Vec::new(),
    )
    .unwrap()
}

fn outcome(view: CommandView) -> CommandOutcome {
    CommandOutcome {
        command_id: CommandId::from_uuid(Uuid::from_u128(705)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(706)),
        committed_events: Vec::new(),
        view,
        shutdown: ShutdownDisposition::Continue,
    }
}

fn snapshot() -> PresentationSnapshot {
    PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            database_readiness: ai_stock_forum::app::DatabaseReadiness::Ready,
            process_guard_ownership: ai_stock_forum::app::ProcessGuardOwnership::Held,
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
        }
}

fn model() -> TuiModel {
    TuiModel::new(snapshot(), false)
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn push_text(events: &mut VecDeque<Result<Option<TuiEvent>, TuiError>>, value: &str) {
    events.extend(value.chars().map(|value| Ok(Some(key(KeyCode::Char(value))))));
    events.push_back(Ok(Some(key(KeyCode::Enter))));
}

fn review_events() -> VecDeque<Result<Option<TuiEvent>, TuiError>> {
    let mut events = VecDeque::from([
        Ok(Some(key(KeyCode::Char('s')))),
        Ok(Some(key(KeyCode::Char('c')))),
        Ok(Some(key(KeyCode::Enter))),
    ]);
    push_text(&mut events, "Host cleanup");
    push_text(&mut events, "Protect pending reviews");
    push_text(&mut events, "Use during host termination");
    push_text(&mut events, "");
    push_text(&mut events, "Cancel exactly once");
    push_text(&mut events, "");
    events.push_back(Ok(Some(key(KeyCode::Enter))));
    events
}

fn run_review_host(
    tail: impl IntoIterator<Item = Result<Option<TuiEvent>, TuiError>>,
    fail_on_review: bool,
) -> (bool, Vec<Call>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        RouteRecorder {
            calls: Arc::clone(&calls),
            execute_error: None,
        },
        4,
    )
    .unwrap();
    let mut events = review_events();
    events.extend(tail);
    let mut events = ScriptedEvents { events };
    let mut screen = ContractScreen { fail_on_review };
    let result = run_tui_with_screen(
        runtime,
        snapshot(),
        false,
        &mut screen,
        &mut events,
        &Theme::from_no_color(true),
    );
    let calls = calls.lock().unwrap().clone();
    (result.is_ok(), calls)
}

fn agent_panel_model() -> (TuiModel, ai_stock_forum::skills::SkillVersionRef) {
    let skill = ai_stock_forum::skills::SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(100)),
        SkillVersionId::from_uuid(Uuid::from_u128(101)),
        1,
        ai_stock_forum::skills::SkillProvenance::User,
        draft(),
    )
    .unwrap();
    let assigned = skill.reference();
    let template = &ai_stock_forum::agents::builtin_profile_templates()[0];
    let mut profile_draft = template.copy_to_draft().unwrap();
    profile_draft.skill_refs = vec![assigned.clone()];
    let profile = ai_stock_forum::agents::AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(110)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(111)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(112)),
        1,
        profile_draft,
        Some(template.provenance()),
    )
    .unwrap();
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    model.agents.skill_panel_open = true;
    model.agents.selected_skill_action_index = 2;
    model.agents.detail = Some(AgentProfileView {
        readiness: profile.readiness(),
        profile,
    });
    (model, assigned)
}

#[test]
fn version_preview_round_trips_through_its_own_runtime_route_and_cleanup_is_once() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        RouteRecorder {
            calls: calls.clone(),
            execute_error: None,
        },
        4,
    )
    .unwrap();
    let skill_id = SkillId::from_uuid(Uuid::from_u128(70));
    let version_id = SkillVersionId::from_uuid(Uuid::from_u128(71));
    let mut editor = SkillEditor::for_version(skill_id, version_id, draft());
    editor.go_to_review().unwrap();
    let SkillEditorEffect::Preview(request @ SkillPreviewRequest::Version { .. }) =
        editor.submit_keyboard_line("")
    else {
        panic!("version request")
    };
    let expected_candidate = request.candidate().clone();
    let mut model = model();
    model.skills.active = true;
    model.skills.pane = SkillsPane::Editor;
    model.skills.editor = Some(editor);

    execute_skill_effect(
        &runtime.client(),
        &mut model,
        ControllerEffect::RequestSkillPreview(request),
    )
    .expect("version preview route");

    assert!(model.skills.review_registered);
    assert!(model.skills.editor.as_ref().unwrap().review().is_some());
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [Call::Version(skill_id, version_id, expected_candidate)]
    );

    execute_skill_effect(
        &runtime.client(),
        &mut model,
        ControllerEffect::CancelSkillReview,
    )
    .unwrap();
    execute_skill_effect(
        &runtime.client(),
        &mut model,
        ControllerEffect::CancelSkillReview,
    )
    .unwrap();
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == Call::Cancel)
            .count(),
        1
    );

    runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
}

#[test]
fn agent_origin_unassign_cancel_returns_to_the_agent_skill_panel() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        RouteRecorder {
            calls: calls.clone(),
            execute_error: None,
        },
        4,
    )
    .unwrap();
    let (mut model, _) = agent_panel_model();
    let preview_effect = handle_event(&mut model, key(KeyCode::Enter));

    execute_skill_effect(&runtime.client(), &mut model, preview_effect).unwrap();
    assert!(!model.skills.active);
    assert!(model.agents.skill_panel_open);
    assert_eq!(model.skills.pane, SkillsPane::Confirmation);

    let cancel_effect = handle_event(&mut model, key(KeyCode::Esc));
    assert_eq!(cancel_effect, ControllerEffect::CancelSkillReview);
    execute_skill_effect(&runtime.client(), &mut model, cancel_effect).unwrap();

    assert!(!model.skills.active);
    assert!(model.agents.skill_panel_open);
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(
        calls.lock().unwrap().iter().filter(|call| **call == Call::Cancel).count(),
        1
    );
    runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
}

#[test]
fn cancellation_failure_consumes_registration_before_cleanup_can_retry() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        RouteRecorder {
            calls: Arc::clone(&calls),
            execute_error: Some(AppError::LifecycleFinished),
        },
        4,
    )
    .unwrap();
    let client = runtime.client();
    let (mut model, _) = agent_panel_model();
    model.skills.review_registered = true;

    assert!(
        execute_skill_effect(
            &client,
            &mut model,
            ControllerEffect::CancelSkillReview,
        )
        .is_err()
    );
    assert!(!model.skills.review_registered);
    assert!(
        execute_skill_effect(
            &client,
            &mut model,
            ControllerEffect::CancelSkillReview,
        )
        .is_ok()
    );
    assert_eq!(
        calls.lock().unwrap().iter().filter(|call| **call == Call::Cancel).count(),
        1
    );
    runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
}

#[test]
fn agent_origin_terminal_review_failures_restore_the_agent_panel_without_orphans() {
    for error in [
        AppError::StaleAgentProfileVersion,
        AppError::CapabilityDenied {
            capability: Capability::AgentSkillUnassign,
            decision: PolicyDecision::Denied,
        },
        AppError::SkillReviewMismatch,
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let runtime = ApplicationRuntime::spawn(
            RouteRecorder {
                calls: Arc::clone(&calls),
                execute_error: Some(error),
            },
            4,
        )
        .unwrap();
        let (mut model, _) = agent_panel_model();
        let preview_effect = handle_event(&mut model, key(KeyCode::Enter));
        execute_skill_effect(&runtime.client(), &mut model, preview_effect).unwrap();
        let command = model
            .skills
            .pending_confirmation
            .as_ref()
            .expect("protected confirmation")
            .command
            .clone();

        execute_skill_effect(
            &runtime.client(),
            &mut model,
            ControllerEffect::ExecuteSkill(command),
        )
        .unwrap();

        assert!(!model.skills.active);
        assert!(model.agents.skill_panel_open);
        assert_eq!(model.active_view, View::Agents);
        assert!(!model.skills.review_registered);
        assert!(model.skills.pending_confirmation.is_none());
        assert_eq!(
            calls.lock().unwrap().iter().filter(|call| **call == Call::Cancel).count(),
            1
        );
        runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
    }
}

#[test]
fn registered_review_is_cancelled_on_actual_host_interruption() {
    let (succeeded, calls) = run_review_host([Ok(Some(TuiEvent::Interrupt))], false);

    assert!(succeeded, "host calls: {calls:?}");
    assert_eq!(calls.iter().filter(|call| **call == Call::Create).count(), 1);
    assert_eq!(calls.iter().filter(|call| **call == Call::Cancel).count(), 1);
    assert_eq!(calls.iter().filter(|call| **call == Call::Finish).count(), 1);
    assert!(!calls.iter().any(|call| matches!(call, Call::Mutation)));
}

#[test]
fn registered_review_is_cancelled_on_quit_only_host_exit() {
    let mut tail = Vec::new();
    tail.push(Ok(Some(key(KeyCode::Char('/')))));
    tail.extend("quit".chars().map(|value| Ok(Some(key(KeyCode::Char(value))))));
    tail.push(Ok(Some(key(KeyCode::Enter))));
    tail.extend((0..4).map(|_| Ok(None)));
    let (succeeded, calls) = run_review_host(tail, false);

    assert!(succeeded, "host calls: {calls:?}");
    assert_eq!(calls.iter().filter(|call| **call == Call::Cancel).count(), 1);
    assert_eq!(calls.iter().filter(|call| **call == Call::Finish).count(), 1);
    assert!(!calls.iter().any(|call| matches!(call, Call::Mutation)));
}

#[test]
fn registered_review_is_cancelled_on_terminal_input_eof_or_io_failure() {
    let (succeeded, calls) = run_review_host([Err(TuiError::TerminalInput)], false);

    assert!(!succeeded);
    assert_eq!(calls.iter().filter(|call| **call == Call::Cancel).count(), 1);
    assert_eq!(calls.iter().filter(|call| **call == Call::Finish).count(), 1);
    assert!(!calls.iter().any(|call| matches!(call, Call::Mutation)));
}

#[test]
fn registered_review_is_cancelled_on_terminal_output_failure() {
    let (succeeded, calls) = run_review_host(std::iter::empty(), true);

    assert!(!succeeded);
    assert_eq!(calls.iter().filter(|call| **call == Call::Cancel).count(), 1);
    assert_eq!(calls.iter().filter(|call| **call == Call::Finish).count(), 1);
    assert!(!calls.iter().any(|call| matches!(call, Call::Mutation)));
}
