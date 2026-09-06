use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use ai_stock_forum::{
    app::{
        AgentProfileSelector, AgentProfileSummary, AgentProfileView, AgentProfilesView,
        AgentSkillAssignmentOperation, AgentSkillAssignmentPreview, AppError, ApplicationCommand,
        CommandOutcome, CommandView, PresentationSnapshot, ShutdownDisposition, ShutdownReason,
        ShutdownView, SkillHistoryEntry, SkillHistoryView, SkillSummary, SkillView, SkillsView,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, InstallationId,
        MemoryNamespaceId, SessionId, SkillId, SkillReviewToken, SkillVersionId, sha256,
    },
    runtime::{ApplicationRuntime, CommandExecutor},
    policy::{Capability, PolicyDecision},
    setup::SetupStatus,
    skills::{SkillDraft, SkillEditPreview, SkillProvenance, SkillVersion},
    ui::{
        skill_editor::{SkillEditor, SkillEditorEffect, SkillPreviewRequest},
        tui::{
            ControllerEffect, EventSource, Screen, TuiError, TuiEvent,
            execute_skill_effect, handle_event, run_tui_with_screen,
            model::{
                AgentsPane, SkillConfirmation, SkillOperationOrigin, SkillsPane, TuiModel, View,
            },
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
    ShowAgent(AgentProfileId),
    Finish,
}

struct RouteRecorder {
    calls: Arc<Mutex<Vec<Call>>>,
    execute_error: Option<AppError>,
}

struct BlockingExecutor {
    calls: Arc<Mutex<Vec<Call>>>,
    entered: mpsc::SyncSender<()>,
    release: mpsc::Receiver<()>,
    block_next: bool,
}

impl CommandExecutor for BlockingExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        if self.block_next {
            self.block_next = false;
            self.entered.send(()).unwrap();
            self.release.recv().unwrap();
        }
        if matches!(
            command,
            ApplicationCommand::CreateSkill { .. }
                | ApplicationCommand::ActivateSkillVersion { .. }
                | ApplicationCommand::AssignAgentSkill { .. }
                | ApplicationCommand::UpgradeAgentSkill { .. }
                | ApplicationCommand::UnassignAgentSkill { .. }
        ) {
            self.calls.lock().unwrap().push(Call::Mutation);
        }
        Err(AppError::LifecycleFinished)
    }

    fn preview_skill_creation(
        &mut self,
        _candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        Err(AppError::SkillReviewUnavailable)
    }

    fn preview_skill_version(
        &mut self,
        _skill_id: SkillId,
        _expected_active_version_id: SkillVersionId,
        _candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        Err(AppError::SkillReviewUnavailable)
    }

    fn preview_agent_skill_assignment(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_profile_version_id: AgentProfileVersionId,
        _skill: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        Err(AppError::SkillReviewUnavailable)
    }

    fn preview_agent_skill_upgrade(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_profile_version_id: AgentProfileVersionId,
        _expected: ai_stock_forum::skills::SkillVersionRef,
        _replacement: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        Err(AppError::SkillReviewUnavailable)
    }

    fn preview_agent_skill_unassignment(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_profile_version_id: AgentProfileVersionId,
        _expected: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<AgentSkillAssignmentPreview, AppError> {
        Err(AppError::SkillReviewUnavailable)
    }

    fn cancel_skill_review(&mut self) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(Call::Cancel);
        Ok(())
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(Call::Finish);
        Ok(())
    }
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
            ApplicationCommand::ListSkills => {
                let replacement = replacement_skill();
                Ok(outcome(CommandView::Skills(SkillsView {
                skills: vec![SkillSummary {
                    skill_ref: replacement.reference(),
                    display_name: replacement.content().display_name.clone(),
                    provenance: replacement.provenance().clone(),
                }],
                total_count: 1,
                returned_count: 1,
                truncated: false,
            })))
            }
            ApplicationCommand::ShowSkill { .. } => Ok(outcome(CommandView::Skill(
                host_skill_view(&replacement_skill()),
            ))),
            ApplicationCommand::ListAgentProfiles => Ok(outcome(CommandView::AgentProfiles(
                AgentProfilesView {
                    profiles: vec![AgentProfileSummary {
                        profile_id: AgentProfileId::from_uuid(Uuid::from_u128(110)),
                        profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(111)),
                        version: ai_stock_forum::domain::ObjectVersion::new(1).unwrap(),
                        display_name: "Assigned Agent".to_owned(),
                        role: ai_stock_forum::agents::builtin_profile_templates()[0].role,
                        primary_specialty: "Research".to_owned(),
                        readiness: ai_stock_forum::agents::AgentReadiness::Unbound,
                        content_digest: sha256(b"assigned-agent"),
                    }],
                    total_count: 1,
                    returned_count: 1,
                    truncated: false,
                },
            ))),
            ApplicationCommand::ShowAgentProfile { selector } => {
                if let AgentProfileSelector::Id(profile_id) = selector {
                    self.calls.lock().unwrap().push(Call::ShowAgent(profile_id));
                }
                Ok(outcome(CommandView::AgentProfile(assigned_profile_view())))
            }
            ApplicationCommand::RequestShutdown => Ok(CommandOutcome {
                command_id: CommandId::from_uuid(Uuid::from_u128(701)),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(702)),
                committed_events: Vec::new(),
                view: CommandView::Shutdown(ShutdownView {
                    disposition: ShutdownDisposition::Requested,
                }),
                shutdown: ShutdownDisposition::Requested,
            }),
            ApplicationCommand::ShowSkillVersion { .. } => Ok(outcome(
                CommandView::SkillVersion(host_skill_view(&assigned_skill())),
            )),
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
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        skill: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<ai_stock_forum::app::AgentSkillAssignmentPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Assign);
        Ok(AgentSkillAssignmentPreview {
            profile_id,
            expected_active_profile_version_id,
            operation: AgentSkillAssignmentOperation::Assign { skill },
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(92)),
            review_digest: sha256(b"assign-review"),
        })
    }

    fn preview_agent_skill_upgrade(
        &mut self,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        expected: ai_stock_forum::skills::SkillVersionRef,
        replacement: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<ai_stock_forum::app::AgentSkillAssignmentPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Upgrade);
        Ok(AgentSkillAssignmentPreview {
            profile_id,
            expected_active_profile_version_id,
            operation: AgentSkillAssignmentOperation::Upgrade {
                expected,
                replacement,
            },
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(93)),
            review_digest: sha256(b"upgrade-review"),
        })
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

fn assigned_skill() -> SkillVersion {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(100)),
        SkillVersionId::from_uuid(Uuid::from_u128(101)),
        1,
        SkillProvenance::User,
        draft(),
    )
    .unwrap()
}

fn replacement_skill() -> SkillVersion {
    let current = assigned_skill();
    let mut candidate = draft();
    candidate.instructions = "Use the explicit upgraded version.".to_owned();
    SkillVersion::next_version(
        &current,
        SkillVersionId::from_uuid(Uuid::from_u128(102)),
        2,
        candidate,
    )
    .unwrap()
}

fn assigned_profile_view() -> AgentProfileView {
    let assigned = assigned_skill().reference();
    let template = &ai_stock_forum::agents::builtin_profile_templates()[0];
    let mut profile_draft = template.copy_to_draft().unwrap();
    profile_draft.skill_refs = vec![assigned];
    let profile = ai_stock_forum::agents::AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(110)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(111)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(112)),
        1,
        profile_draft,
        Some(template.provenance()),
    )
    .unwrap();
    AgentProfileView {
        readiness: profile.readiness(),
        profile,
    }
}

fn host_skill(seed: u128, version: i64, name: &str) -> SkillVersion {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(seed)),
        SkillVersionId::from_uuid(Uuid::from_u128(seed + version as u128)),
        version,
        SkillProvenance::User,
        SkillDraft::new(
            name.to_owned(),
            format!("Purpose for {name}"),
            "Use for host contracts.".to_owned(),
            Vec::new(),
            "Keep exact references.".to_owned(),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn host_skill_view(skill: &SkillVersion) -> SkillView {
    SkillView {
        skill_ref: skill.reference(),
        content: skill.content().clone(),
        created_at_ms: skill.created_at_ms(),
        provenance: skill.provenance().clone(),
        predecessor_version_id: skill.predecessor(),
    }
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
    let skill = assigned_skill();
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
        model.agents.detail.as_ref().map(|detail| detail.profile.profile_id()),
        Some(AgentProfileId::from_uuid(Uuid::from_u128(110)))
    );
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
fn agent_view_exact_version_route_replaces_stale_context_and_returns_to_agents() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        RouteRecorder {
            calls: Arc::clone(&calls),
            execute_error: None,
        },
        4,
    )
    .unwrap();
    let (mut model, target) = agent_panel_model();
    model.agents.selected_skill_action_index = 0;
    let stale = host_skill(900, 1, "Stale A");
    model.skills.detail = Some(host_skill_view(&stale));
    model.skills.version_detail = Some(host_skill_view(&stale));
    model.skills.history = Some(SkillHistoryView {
        skill_id: stale.skill_id(),
        active_version_id: stale.skill_version_id(),
        versions: vec![SkillHistoryEntry {
            skill_ref: stale.reference(),
            created_at_ms: stale.created_at_ms(),
            predecessor_version_id: stale.predecessor(),
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    });

    let effect = handle_event(&mut model, key(KeyCode::Enter));
    execute_skill_effect(&runtime.client(), &mut model, effect).unwrap();

    assert_eq!(model.skills.pane, SkillsPane::Detail);
    assert_eq!(model.skills.selected_skill_ref(), Some(&target));
    assert_eq!(
        model.skills.detail.as_ref().map(|detail| detail.skill_ref.clone()),
        Some(target.clone())
    );
    assert!(model.skills.history.is_none());
    handle_event(&mut model, key(KeyCode::Esc));
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Agents);
    assert!(model.agents.skill_panel_open);
    runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
}

#[test]
fn historical_assign_preview_builds_a_command_for_the_exact_opened_version() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        RouteRecorder {
            calls: Arc::clone(&calls),
            execute_error: None,
        },
        4,
    )
    .unwrap();
    let historical = host_skill(920, 1, "Historical");
    let mut model = model();
    model.skills.active = true;
    model.skills.replace_version_detail(host_skill_view(&historical));
    model.skills.pane = SkillsPane::AssignmentReview;
    model.skills.selected_agent_detail = Some(agent_panel_model().0.agents.detail.unwrap());
    model.skills.assignment = Some(ai_stock_forum::ui::tui::AssignmentKind::Add);

    let effect = handle_event(&mut model, key(KeyCode::Enter));
    execute_skill_effect(&runtime.client(), &mut model, effect).unwrap();

    let command = &model
        .skills
        .pending_confirmation
        .as_ref()
        .expect("assignment confirmation")
        .command;
    assert!(matches!(
        command,
        ApplicationCommand::AssignAgentSkill { skill, .. } if skill == &historical.reference()
    ));
    execute_skill_effect(
        &runtime.client(),
        &mut model,
        ControllerEffect::CancelSkillReview,
    )
    .unwrap();
    runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
}

#[test]
fn agent_origin_upgrade_preview_builds_an_explicit_exact_upgrade_command() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        RouteRecorder {
            calls: Arc::clone(&calls),
            execute_error: None,
        },
        4,
    )
    .unwrap();
    let (mut model, expected) = agent_panel_model();
    model.agents.profiles = AgentProfilesView {
        profiles: vec![
            AgentProfileSummary {
                profile_id: AgentProfileId::from_uuid(Uuid::from_u128(120)),
                profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(121)),
                version: ai_stock_forum::domain::ObjectVersion::new(1).unwrap(),
                display_name: "Wrong Agent".to_owned(),
                role: ai_stock_forum::agents::builtin_profile_templates()[0].role,
                primary_specialty: "Research".to_owned(),
                readiness: ai_stock_forum::agents::AgentReadiness::Unbound,
                content_digest: sha256(b"wrong-agent"),
            },
            AgentProfileSummary {
                profile_id: AgentProfileId::from_uuid(Uuid::from_u128(110)),
                profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(111)),
                version: ai_stock_forum::domain::ObjectVersion::new(1).unwrap(),
                display_name: "Origin Agent".to_owned(),
                role: ai_stock_forum::agents::builtin_profile_templates()[0].role,
                primary_specialty: "Research".to_owned(),
                readiness: ai_stock_forum::agents::AgentReadiness::Unbound,
                content_digest: sha256(b"origin-agent"),
            },
        ],
        total_count: 2,
        returned_count: 2,
        truncated: false,
    };
    let available = replacement_skill();
    model.skills.replace_skills(SkillsView {
        skills: vec![SkillSummary {
            skill_ref: available.reference(),
            display_name: available.content().display_name.clone(),
            provenance: available.provenance().clone(),
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    });
    model.skills.selected_agent = 0;
    handle_event(&mut model, key(KeyCode::Left));
    let replacement = available.reference();
    let effect = handle_event(&mut model, key(KeyCode::Enter));
    assert_eq!(
        effect,
        ControllerEffect::RequestSkillAssignmentPreview {
            profile_id: AgentProfileId::from_uuid(Uuid::from_u128(110)),
            expected_active_profile_version_id: AgentProfileVersionId::from_uuid(
                Uuid::from_u128(111),
            ),
            target: replacement.clone(),
            assignment: ai_stock_forum::ui::tui::AssignmentKind::Upgrade {
                expected: expected.clone(),
            },
        }
    );
    assert_eq!(
        model.skills.operation_origin,
        SkillOperationOrigin::AgentSkills {
            profile_id: AgentProfileId::from_uuid(Uuid::from_u128(110)),
        }
    );
    execute_skill_effect(&runtime.client(), &mut model, effect).unwrap();
    assert_eq!(model.skills.pane, SkillsPane::Confirmation);

    let command = &model
        .skills
        .pending_confirmation
        .as_ref()
        .expect("upgrade confirmation")
        .command;
    assert!(matches!(
        command,
        ApplicationCommand::UpgradeAgentSkill {
            profile_id,
            expected_active_profile_version_id,
            expected: command_expected,
            replacement: command_replacement,
            review_token,
            ..
        } if *profile_id == AgentProfileId::from_uuid(Uuid::from_u128(110))
            && *expected_active_profile_version_id
                == AgentProfileVersionId::from_uuid(Uuid::from_u128(111))
            && command_expected == &expected
            && command_replacement == &replacement
            && *review_token == SkillReviewToken::from_uuid(Uuid::from_u128(93))
    ));
    assert!(model.skills.review_registered);
    assert!(!calls.lock().unwrap().contains(&Call::ShowAgent(
        AgentProfileId::from_uuid(Uuid::from_u128(120)),
    )));

    let cancel = handle_event(&mut model, key(KeyCode::Esc));
    assert_eq!(cancel, ControllerEffect::CancelSkillReview);
    execute_skill_effect(&runtime.client(), &mut model, cancel).unwrap();
    assert!(!model.skills.review_registered);
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Agents);
    assert!(model.agents.skill_panel_open);
    assert_eq!(
        model.agents.detail.as_ref().map(|detail| detail.profile.profile_id()),
        Some(AgentProfileId::from_uuid(Uuid::from_u128(110)))
    );
    assert_eq!(
        calls.lock().unwrap().iter().filter(|call| **call == Call::Upgrade).count(),
        1
    );
    assert_eq!(
        calls.lock().unwrap().iter().filter(|call| **call == Call::Cancel).count(),
        1
    );
    assert!(!calls.lock().unwrap().iter().any(|call| matches!(call, Call::Mutation)));
    runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
}

#[test]
fn typed_backpressure_keeps_protected_confirmation_retryable_without_token_loss() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let runtime = ApplicationRuntime::spawn(
        BlockingExecutor {
            calls: Arc::clone(&calls),
            entered: entered_tx,
            release: release_rx,
            block_next: true,
        },
        1,
    )
    .unwrap();
    let client = runtime.client();
    let _running = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let _queued = client.try_submit(ApplicationCommand::ShowStatus).unwrap();

    let target = host_skill(960, 1, "Backpressure").reference();
    let command = ApplicationCommand::AssignAgentSkill {
        profile_id: AgentProfileId::from_uuid(Uuid::from_u128(970)),
        expected_active_profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(971)),
        skill: target,
        review_token: SkillReviewToken::from_uuid(Uuid::from_u128(972)),
        review_digest: sha256(b"retryable-review"),
    };
    let confirmation = SkillConfirmation {
        command: command.clone(),
        origin: SkillOperationOrigin::Skills(SkillsPane::AssignmentReview),
    };
    let mut model = model();
    model.skills.active = true;
    model.skills.pane = SkillsPane::Confirmation;
    model.skills.pending_confirmation = Some(confirmation.clone());
    model.skills.review_registered = true;

    let worker_client = client.clone();
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let result = execute_skill_effect(
            &worker_client,
            &mut model,
            ControllerEffect::ExecuteSkill(command),
        );
        result_tx.send((result, model)).unwrap();
    });

    let received = result_rx.recv_timeout(Duration::from_millis(100));
    if received.is_err() {
        release_tx.send(()).unwrap();
        let _ = result_rx.recv_timeout(Duration::from_secs(1));
        handle.join().unwrap();
        runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
        panic!("protected skill submission blocked instead of returning typed backpressure");
    }
    let (result, model) = received.unwrap();
    assert!(result.is_ok());
    assert_eq!(model.skills.pane, SkillsPane::Confirmation);
    assert_eq!(model.skills.pending_confirmation, Some(confirmation));
    assert!(model.skills.review_registered);
    assert!(!model.command_in_flight);
    assert!(!calls.lock().unwrap().iter().any(|call| matches!(call, Call::Mutation | Call::Cancel)));

    release_tx.send(()).unwrap();
    handle.join().unwrap();
    runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
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
