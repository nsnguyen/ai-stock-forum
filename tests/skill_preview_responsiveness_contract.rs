use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use ai_stock_forum::{
    app::{
        AppError, ApplicationCommand, CommandOutcome, CommandView, PresentationSnapshot,
        ShutdownDisposition, ShutdownReason, SkillSummary, SkillView, SkillsView,
    },
    domain::{CommandId, CorrelationId, InstallationId, SessionId, SkillId, SkillVersionId},
    runtime::{ApplicationRuntime, CommandExecutor},
    setup::SetupStatus,
    skills::{SkillDraft, SkillProvenance, SkillVersion, SkillVersionRef},
    ui::tui::{
        EventSource, Screen, TuiError, TuiEvent,
        model::{SkillsPane, TuiModel, View},
        run_tui_with_screen,
        theme::Theme,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use uuid::Uuid;

const DEADLOCK_ESCAPE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug)]
enum FirstPreviewResponse {
    Exact,
    ApplicationError,
    MismatchedSameSkillVersion,
}

struct BlockingPreviewExecutor {
    skills: Vec<SkillVersion>,
    preview_started: Sender<SkillVersionRef>,
    preview_calls: Arc<Mutex<Vec<SkillVersionRef>>>,
    first_preview_completed: Arc<AtomicBool>,
    release_first_preview: Receiver<()>,
    block_preview_call: usize,
    first_preview_response: FirstPreviewResponse,
}

impl CommandExecutor for BlockingPreviewExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        match command {
            ApplicationCommand::ListSkills => Ok(outcome(CommandView::Skills(SkillsView {
                skills: self
                    .skills
                    .iter()
                    .map(|skill| SkillSummary {
                        skill_ref: skill.reference(),
                        display_name: skill.content().display_name.clone(),
                        provenance: skill.provenance().clone(),
                    })
                    .collect(),
                total_count: self.skills.len() as u32,
                returned_count: self.skills.len() as u32,
                truncated: false,
            }))),
            ApplicationCommand::ShowSkillVersion { selector, version } => {
                let skill = self
                    .skills
                    .iter()
                    .find(|skill| selector == skill.skill_id().into() && version == skill.version())
                    .expect("preview request names a listed synthetic skill");
                let reference = skill.reference();
                let call_number = {
                    let mut calls = self.preview_calls.lock().unwrap();
                    calls.push(reference.clone());
                    calls.len()
                };
                self.preview_started.send(reference).unwrap();
                if call_number == self.block_preview_call {
                    self.release_first_preview.recv().unwrap();
                    self.first_preview_completed.store(true, Ordering::Release);
                }
                match (call_number, self.first_preview_response) {
                    (1, FirstPreviewResponse::ApplicationError) => Err(AppError::LifecycleFinished),
                    (1, FirstPreviewResponse::MismatchedSameSkillVersion) => {
                        let mismatched = mismatched_next_version(skill);
                        Ok(outcome(CommandView::SkillVersion(skill_view(&mismatched))))
                    }
                    _ => Ok(outcome(CommandView::SkillVersion(skill_view(skill)))),
                }
            }
            _ => Err(AppError::LifecycleFinished),
        }
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

struct ChannelEvents {
    events: Receiver<TuiEvent>,
}

impl EventSource for ChannelEvents {
    fn next_event(&mut self, timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
        match self.events.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(TuiError::TerminalInput),
        }
    }
}

#[derive(Clone, Debug)]
struct Frame {
    selected_skill: usize,
    pane: SkillsPane,
    selected_create_source: usize,
    detail: Option<SkillVersionRef>,
    create_source_detail: Option<SkillVersionRef>,
    active_view: View,
    skills_active: bool,
    editor_name: Option<String>,
    after_first_preview_completed: bool,
}

struct ObservationScreen {
    frame_tx: Sender<Frame>,
    all_frames: Arc<Mutex<Vec<Frame>>>,
    first_preview_completed: Arc<AtomicBool>,
}

impl Screen for ObservationScreen {
    fn size(&self) -> Result<Rect, TuiError> {
        Ok(Rect::new(0, 0, 140, 40))
    }

    fn draw(&mut self, model: &TuiModel, _theme: &Theme) -> Result<(), TuiError> {
        let frame = Frame {
            selected_skill: model.skills.selected_skill,
            pane: model.skills.pane,
            selected_create_source: model.skills.selected_create_source,
            detail: model
                .skills
                .detail
                .as_ref()
                .map(|detail| detail.skill_ref.clone()),
            create_source_detail: model
                .skills
                .create_source_detail
                .as_ref()
                .map(|detail| detail.skill_ref.clone()),
            active_view: model.active_view,
            skills_active: model.skills.active,
            editor_name: model
                .skills
                .editor
                .as_ref()
                .map(|editor| editor.raw_display_name().to_owned()),
            after_first_preview_completed: self.first_preview_completed.load(Ordering::Acquire),
        };
        self.all_frames.lock().unwrap().push(frame.clone());
        let _ = self.frame_tx.send(frame);
        Ok(())
    }

    fn restore(&mut self) -> Result<(), TuiError> {
        Ok(())
    }
}

struct HostHarness {
    event_tx: Sender<TuiEvent>,
    preview_started_rx: Receiver<SkillVersionRef>,
    release_tx: Option<Sender<()>>,
    frame_rx: Receiver<Frame>,
    all_frames: Arc<Mutex<Vec<Frame>>>,
    preview_calls: Arc<Mutex<Vec<SkillVersionRef>>>,
    host: Option<JoinHandle<Result<(), TuiError>>>,
}

impl HostHarness {
    fn start(skills: Vec<SkillVersion>) -> Self {
        Self::start_with_response(skills, 1, FirstPreviewResponse::Exact)
    }

    fn start_blocking_preview_call(skills: Vec<SkillVersion>, block_preview_call: usize) -> Self {
        Self::start_with_response(skills, block_preview_call, FirstPreviewResponse::Exact)
    }

    fn start_with_response(
        skills: Vec<SkillVersion>,
        block_preview_call: usize,
        first_preview_response: FirstPreviewResponse,
    ) -> Self {
        let (preview_started_tx, preview_started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let (frame_tx, frame_rx) = mpsc::channel();
        let preview_calls = Arc::new(Mutex::new(Vec::new()));
        let all_frames = Arc::new(Mutex::new(Vec::new()));
        let first_preview_completed = Arc::new(AtomicBool::new(false));
        let runtime = ApplicationRuntime::spawn(
            BlockingPreviewExecutor {
                skills,
                preview_started: preview_started_tx,
                preview_calls: Arc::clone(&preview_calls),
                first_preview_completed: Arc::clone(&first_preview_completed),
                release_first_preview: release_rx,
                block_preview_call,
                first_preview_response,
            },
            4,
        )
        .unwrap();
        let screen_frames = Arc::clone(&all_frames);
        let host = thread::spawn(move || {
            let mut screen = ObservationScreen {
                frame_tx,
                all_frames: screen_frames,
                first_preview_completed,
            };
            let mut events = ChannelEvents { events: event_rx };
            run_tui_with_screen(
                runtime,
                snapshot(),
                false,
                &mut screen,
                &mut events,
                &Theme::from_no_color(true),
            )
        });
        Self {
            event_tx,
            preview_started_rx,
            release_tx: Some(release_tx),
            frame_rx,
            all_frames,
            preview_calls,
            host: Some(host),
        }
    }

    fn open_skills_and_wait_for_blocked_preview(&self) -> SkillVersionRef {
        self.send_key(KeyCode::Char('4'));
        self.preview_started_rx
            .recv_timeout(DEADLOCK_ESCAPE)
            .expect("the initial exact-version preview starts")
    }

    fn send_key(&self, code: KeyCode) {
        self.event_tx.send(key(code)).unwrap();
    }

    fn wait_for_frame(&self, mut predicate: impl FnMut(&Frame) -> bool) -> Option<Frame> {
        let deadline = Instant::now() + DEADLOCK_ESCAPE;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match self.frame_rx.recv_timeout(remaining) {
                Ok(frame) if predicate(&frame) => return Some(frame),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    fn release_first_preview(&mut self) {
        if let Some(release_tx) = self.release_tx.take() {
            let _ = release_tx.send(());
        }
    }

    fn preview_calls(&self) -> Vec<SkillVersionRef> {
        self.preview_calls.lock().unwrap().clone()
    }

    fn frames(&self) -> Vec<Frame> {
        self.all_frames.lock().unwrap().clone()
    }

    fn finish(mut self) -> Result<(), TuiError> {
        self.release_first_preview();
        let _ = self.event_tx.send(TuiEvent::Interrupt);
        self.host
            .take()
            .expect("host thread")
            .join()
            .expect("TUI host thread did not panic")
    }
}

impl Drop for HostHarness {
    fn drop(&mut self) {
        self.release_first_preview();
        let _ = self.event_tx.send(TuiEvent::Interrupt);
        if let Some(host) = self.host.take() {
            let _ = host.join();
        }
    }
}

#[test]
fn skill_list_navigation_redraws_for_s_and_w_while_the_previous_preview_is_still_blocked() {
    // Break caught: routing LoadSkillPreview through the synchronous skill-effect path freezes
    // event delivery, so none of these later highlighted rows can render before release.
    let host = HostHarness::start(synthetic_skills());
    let first = host.open_skills_and_wait_for_blocked_preview();

    host.send_key(KeyCode::Char('s'));
    let down_once = host.wait_for_frame(|frame| frame.selected_skill == 1);
    host.send_key(KeyCode::Char('s'));
    let down_twice = host.wait_for_frame(|frame| frame.selected_skill == 2);
    host.send_key(KeyCode::Char('w'));
    let back_up = host.wait_for_frame(|frame| frame.selected_skill == 1);
    let calls_before_release = host.preview_calls();

    let result = host.finish();

    assert!(
        down_once.is_some(),
        "S must highlight and redraw the second skill"
    );
    assert!(
        down_twice.is_some(),
        "a second S must keep navigation responsive"
    );
    assert!(
        back_up.is_some(),
        "W must highlight and redraw the prior skill"
    );
    assert_eq!(calls_before_release, [first]);
    assert!(result.is_ok());
}

#[test]
fn blocked_preview_coalesces_navigation_to_only_the_latest_selected_skill() {
    // Break caught: queueing every selection preview issues obsolete Beta work after Alpha,
    // delaying Gamma and allowing intermediate results to flicker into the visible detail.
    let skills = synthetic_skills();
    let expected_alpha = skills[0].reference();
    let expected_gamma = skills[2].reference();
    let mut host = HostHarness::start(skills);
    assert_eq!(
        host.open_skills_and_wait_for_blocked_preview(),
        expected_alpha
    );

    host.send_key(KeyCode::Char('s'));
    assert!(
        host.wait_for_frame(|frame| frame.selected_skill == 1)
            .is_some()
    );
    host.send_key(KeyCode::Char('s'));
    assert!(
        host.wait_for_frame(|frame| frame.selected_skill == 2)
            .is_some()
    );
    host.release_first_preview();

    let next_request = host
        .preview_started_rx
        .recv_timeout(DEADLOCK_ESCAPE)
        .expect("the coalesced latest preview starts after release");
    let gamma_frame = host.wait_for_frame(|frame| {
        frame.after_first_preview_completed
            && frame.selected_skill == 2
            && frame.detail.as_ref() == Some(&expected_gamma)
    });
    let calls = host.preview_calls();
    let frames = host.frames();
    let result = host.finish();

    assert_eq!(next_request, expected_gamma);
    assert_eq!(calls, [expected_alpha.clone(), expected_gamma.clone()]);
    assert!(
        gamma_frame.is_some(),
        "the latest exact preview must render"
    );
    assert!(
        !frames.iter().any(|frame| {
            frame.after_first_preview_completed
                && frame.selected_skill == 2
                && frame.detail.as_ref() == Some(&expected_alpha)
        }),
        "the stale Alpha response must never replace Gamma's selected detail: {frames:?}"
    );
    assert!(result.is_ok());
}

#[test]
fn failed_or_mismatched_preview_keeps_the_ui_alive_for_a_later_selection() {
    // Breaks caught: treating a passive ApplicationError as fatal exits the TUI, while weakening
    // exact-ref validation to skill ID accepts the wrong version and digest for the current row.
    for first_response in [
        FirstPreviewResponse::ApplicationError,
        FirstPreviewResponse::MismatchedSameSkillVersion,
    ] {
        let skills = synthetic_skills();
        let expected_alpha = skills[0].reference();
        let expected_beta = skills[1].reference();
        let mismatched_alpha = mismatched_next_version(&skills[0]).reference();
        let mut host = HostHarness::start_with_response(skills, 1, first_response);
        assert_eq!(
            host.open_skills_and_wait_for_blocked_preview(),
            expected_alpha
        );
        host.release_first_preview();
        assert!(
            host.wait_for_frame(|frame| frame.after_first_preview_completed)
                .is_some()
        );
        host.send_key(KeyCode::Char('s'));
        assert!(
            host.wait_for_frame(|frame| frame.selected_skill == 1)
                .is_some()
        );

        let next_request = host
            .preview_started_rx
            .recv_timeout(DEADLOCK_ESCAPE)
            .expect("the latest preview starts after the stale failure");
        let beta_frame = host.wait_for_frame(|frame| {
            frame.after_first_preview_completed
                && frame.selected_skill == 1
                && frame.detail.as_ref() == Some(&expected_beta)
        });
        let calls = host.preview_calls();
        let frames = host.frames();
        let result = host.finish();

        assert_eq!(next_request, expected_beta);
        assert_eq!(calls, [expected_alpha, expected_beta]);
        assert!(
            beta_frame.is_some(),
            "latest detail must load after {first_response:?}"
        );
        assert!(
            !frames.iter().any(|frame| {
                frame.after_first_preview_completed
                    && frame.detail.as_ref() == Some(&mismatched_alpha)
            }),
            "same-skill wrong-version detail must never render after {first_response:?}: {frames:?}"
        );
        assert!(result.is_ok(), "the TUI must survive {first_response:?}");
    }
}

#[test]
fn stale_preview_completion_does_not_switch_back_from_the_current_tab() {
    // Break caught: applying a late SkillVersion outcome as a normal command outcome reopens
    // Skills after the user has already moved back to Home.
    let mut host = HostHarness::start(synthetic_skills());
    host.open_skills_and_wait_for_blocked_preview();
    host.send_key(KeyCode::Char('1'));
    assert!(
        host.wait_for_frame(|frame| !frame.skills_active && frame.active_view == View::Overview)
            .is_some()
    );

    host.release_first_preview();
    let after_completion = host.wait_for_frame(|frame| frame.after_first_preview_completed);
    let result = host.finish();

    let after_completion = after_completion.expect("the completed preview triggers a redraw");
    assert!(!after_completion.skills_active);
    assert_eq!(after_completion.active_view, View::Overview);
    assert!(result.is_ok());
}

#[test]
fn returning_to_skills_retries_a_preview_that_completed_while_the_tab_was_inactive() {
    // Break caught: treating an already-loaded library as redraw-only leaves the selected row
    // permanently without detail after its first preview was correctly ignored off-tab.
    let skills = synthetic_skills();
    let expected_alpha = skills[0].reference();
    let mut host = HostHarness::start(skills);
    assert_eq!(
        host.open_skills_and_wait_for_blocked_preview(),
        expected_alpha
    );
    host.send_key(KeyCode::Char('1'));
    assert!(
        host.wait_for_frame(|frame| !frame.skills_active && frame.active_view == View::Overview)
            .is_some()
    );
    host.release_first_preview();
    assert!(
        host.wait_for_frame(|frame| frame.after_first_preview_completed && !frame.skills_active)
            .is_some()
    );

    host.send_key(KeyCode::Char('4'));
    let retried = host.preview_started_rx.recv_timeout(DEADLOCK_ESCAPE).ok();
    let retried_frame = host.wait_for_frame(|frame| {
        frame.skills_active && frame.detail.as_ref() == Some(&expected_alpha)
    });
    let result = host.finish();

    assert_eq!(retried, Some(expected_alpha));
    assert!(
        retried_frame.is_some(),
        "the selected exact preview must render after re-entry"
    );
    assert!(result.is_ok());
}

#[test]
fn returning_from_create_source_retries_the_discarded_list_preview() {
    // Break caught: only watching tab activation misses pane-local re-entry, leaving the selected
    // list row blank after its preview completed while Create Source was open.
    let skills = synthetic_skills();
    let expected_alpha = skills[0].reference();
    let mut host = HostHarness::start(skills);
    assert_eq!(
        host.open_skills_and_wait_for_blocked_preview(),
        expected_alpha
    );
    host.send_key(KeyCode::Char('n'));
    assert!(
        host.wait_for_frame(|frame| frame.pane == SkillsPane::CreateSource)
            .is_some()
    );
    host.release_first_preview();
    assert!(
        host.wait_for_frame(|frame| {
            frame.after_first_preview_completed && frame.pane == SkillsPane::CreateSource
        })
        .is_some()
    );

    host.send_key(KeyCode::Esc);
    assert!(
        host.wait_for_frame(|frame| frame.skills_active && frame.pane == SkillsPane::List)
            .is_some()
    );
    let retried = host.preview_started_rx.recv_timeout(DEADLOCK_ESCAPE).ok();
    let retried_frame = host.wait_for_frame(|frame| {
        frame.skills_active
            && frame.pane == SkillsPane::List
            && frame.detail.as_ref() == Some(&expected_alpha)
    });
    let result = host.finish();

    assert_eq!(retried, Some(expected_alpha));
    assert!(
        retried_frame.is_some(),
        "the selected list preview must render after pane re-entry"
    );
    assert!(result.is_ok());
}

#[test]
fn returning_to_the_starter_picker_retries_its_ignored_exact_preview() {
    // Break caught: re-entry logic that only understands the main skill list leaves a selected
    // starter blank after its response completed while Skills was inactive.
    let skills = synthetic_skills();
    let expected_alpha = skills[0].reference();
    let mut host = HostHarness::start_blocking_preview_call(skills, 2);
    host.send_key(KeyCode::Char('4'));
    assert_eq!(
        host.preview_started_rx
            .recv_timeout(DEADLOCK_ESCAPE)
            .unwrap(),
        expected_alpha
    );
    assert!(
        host.wait_for_frame(|frame| frame.detail.as_ref() == Some(&expected_alpha))
            .is_some()
    );
    host.send_key(KeyCode::Char('c'));
    assert!(
        host.wait_for_frame(|frame| frame.pane == SkillsPane::CreateSource)
            .is_some()
    );
    host.send_key(KeyCode::Char('s'));
    assert!(
        host.wait_for_frame(|frame| {
            frame.pane == SkillsPane::CreateSource && frame.selected_create_source == 1
        })
        .is_some()
    );
    assert_eq!(
        host.preview_started_rx
            .recv_timeout(DEADLOCK_ESCAPE)
            .unwrap(),
        expected_alpha
    );

    host.send_key(KeyCode::Char('1'));
    assert!(
        host.wait_for_frame(|frame| !frame.skills_active && frame.active_view == View::Overview)
            .is_some()
    );
    host.release_first_preview();
    assert!(
        host.wait_for_frame(|frame| frame.after_first_preview_completed && !frame.skills_active)
            .is_some()
    );
    host.send_key(KeyCode::Char('4'));

    let retried = host.preview_started_rx.recv_timeout(DEADLOCK_ESCAPE).ok();
    let retried_frame = host.wait_for_frame(|frame| {
        frame.skills_active
            && frame.pane == SkillsPane::CreateSource
            && frame.selected_create_source == 1
            && frame.create_source_detail.as_ref() == Some(&expected_alpha)
    });
    let result = host.finish();

    assert_eq!(retried, Some(expected_alpha));
    assert!(
        retried_frame.is_some(),
        "the exact starter preview must render after re-entry"
    );
    assert!(result.is_ok());
}

#[test]
fn stale_preview_completion_does_not_replace_an_editor_opened_while_it_was_pending() {
    // Break caught: restoring the Skills snapshot captured before the request discards the new
    // blank editor when the old preview response arrives.
    let mut host = HostHarness::start(synthetic_skills());
    host.open_skills_and_wait_for_blocked_preview();
    host.send_key(KeyCode::Char('c'));
    assert!(
        host.wait_for_frame(|frame| frame.pane == SkillsPane::CreateSource)
            .is_some()
    );
    host.send_key(KeyCode::Enter);
    assert!(
        host.wait_for_frame(|frame| {
            frame.pane == SkillsPane::Editor && frame.editor_name.as_deref() == Some("")
        })
        .is_some()
    );

    host.release_first_preview();
    let after_completion = host.wait_for_frame(|frame| frame.after_first_preview_completed);
    let result = host.finish();

    let after_completion = after_completion.expect("the completed preview triggers a redraw");
    assert_eq!(after_completion.pane, SkillsPane::Editor);
    assert_eq!(after_completion.editor_name.as_deref(), Some(""));
    assert!(result.is_ok());
}

fn synthetic_skills() -> Vec<SkillVersion> {
    vec![
        synthetic_skill(100, "Alpha"),
        synthetic_skill(200, "Beta"),
        synthetic_skill(300, "Gamma"),
    ]
}

fn synthetic_skill(seed: u128, name: &str) -> SkillVersion {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(seed)),
        SkillVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        1,
        SkillProvenance::User,
        SkillDraft::new(
            name.to_owned(),
            format!("Purpose for {name}"),
            "Use for deterministic responsiveness contracts.".to_owned(),
            Vec::new(),
            format!("Render {name} without blocking navigation."),
            Vec::new(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn mismatched_next_version(skill: &SkillVersion) -> SkillVersion {
    let mut candidate = skill.content().clone();
    candidate.instructions = "This wrong version and digest must not be installed.".to_owned();
    SkillVersion::next_version(
        skill,
        SkillVersionId::from_uuid(Uuid::from_u128(9_999)),
        2,
        candidate,
    )
    .unwrap()
}

fn skill_view(skill: &SkillVersion) -> SkillView {
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
        command_id: CommandId::from_uuid(Uuid::from_u128(900)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(901)),
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

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
