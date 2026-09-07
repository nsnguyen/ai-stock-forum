mod support;

use std::{
    collections::VecDeque,
    io::{self, BufRead, Cursor, Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentRole},
    app::{
        AgentSkillAssignmentPreview, AppError, ApplicationCommand, ApplicationService,
        CommandOutcome, CommandView, HelpView, InputRejectionCategory, ShutdownDisposition,
        ShutdownReason, ShutdownView, SkillSelector,
    },
    config::AppPaths,
    domain::{
        AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, ObjectVersion, SkillId,
        SkillReviewToken, SkillVersionId, sha256,
    },
    policy::{Capability, PolicyDecision},
    runtime::{ApplicationRuntime, CommandExecutor, RuntimeClient, RuntimeError},
    skills::{SkillDraft, SkillEditPreview, SkillProvenance, SkillVersion, SkillVersionRef},
    ui::command::{
        BoundedLineReader, CancellableLineSource, FallbackHost, FallbackParsedLine, FallbackRunner,
        LineSourceCancellation, LineSourceEvent, UiError, parse_fallback_line,
    },
};
use crossbeam_channel::{Sender, bounded, never};
use tempfile::TempDir;
use uuid::Uuid;

fn direct_command(input: &[u8]) -> ApplicationCommand {
    match parse_fallback_line(input) {
        FallbackParsedLine::Command(command) => command,
        FallbackParsedLine::AgentWorkflow(_) => panic!("expected direct command"),
        FallbackParsedLine::SkillWorkflow(_) => panic!("expected direct command"),
        FallbackParsedLine::Ignored => panic!("expected command"),
    }
}

fn profile(name: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        "Fallback skill workflow profile.".to_owned(),
        AgentRole::Custom,
        "fallback review".to_owned(),
        vec!["fallback".to_owned()],
        "Deliberate.".to_owned(),
        "Keep exact skill references.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn skill(name: &str, instructions: &str) -> SkillDraft {
    SkillDraft::new(
        name.to_owned(),
        "Fallback skill workflow.".to_owned(),
        "Use while testing fallback review.".to_owned(),
        vec!["fallback".to_owned()],
        instructions.to_owned(),
        Vec::new(),
    )
    .unwrap()
}

fn create_profile(client: &RuntimeClient, name: &str) {
    let outcome = client
        .submit(ApplicationCommand::CreateAgentProfile {
            draft: profile(name),
            template_provenance: None,
        })
        .unwrap();
    assert!(matches!(outcome.view, CommandView::AgentProfileCreated(_)));
}

fn command_outcome(view: CommandView, shutdown: ShutdownDisposition) -> CommandOutcome {
    CommandOutcome {
        command_id: CommandId::from_uuid(Uuid::from_u128(901)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(902)),
        committed_events: Vec::new(),
        view,
        shutdown,
    }
}

fn exact_ref(seed: u128, version: u64) -> SkillVersionRef {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(seed)),
        SkillVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        1_700_000_000_000,
        SkillProvenance::User,
        skill("Probe Skill", &format!("Version {version}.")),
    )
    .unwrap()
    .reference()
}

#[derive(Default)]
struct ProbeState {
    registered: bool,
    cancel_calls: usize,
    mutation_attempts: usize,
    committed_mutations: usize,
    preview_candidates: Vec<SkillDraft>,
}

struct ReviewProbeExecutor {
    state: Arc<Mutex<ProbeState>>,
    commit_error: Option<AppError>,
    cancel_error: Option<AppError>,
    interrupt: Option<Sender<()>>,
}

impl CommandExecutor for ReviewProbeExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        match command {
            ApplicationCommand::CreateSkill { .. } => {
                let mut state = self.state.lock().unwrap();
                state.mutation_attempts += 1;
                if let Some(error) = self.commit_error.take() {
                    return Err(error);
                }
                state.registered = false;
                state.committed_mutations += 1;
                Ok(command_outcome(
                    CommandView::Help(HelpView),
                    ShutdownDisposition::Continue,
                ))
            }
            ApplicationCommand::RequestShutdown => Ok(command_outcome(
                CommandView::Shutdown(ShutdownView {
                    disposition: ShutdownDisposition::Requested,
                }),
                ShutdownDisposition::Requested,
            )),
            _ => Ok(command_outcome(
                CommandView::Help(HelpView),
                ShutdownDisposition::Continue,
            )),
        }
    }

    fn preview_skill_creation(
        &mut self,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        let mut state = self.state.lock().unwrap();
        state.registered = true;
        state.preview_candidates.push(candidate);
        drop(state);
        if let Some(interrupt) = self.interrupt.take() {
            interrupt.send(()).unwrap();
        }
        Ok(SkillEditPreview {
            skill_id: SkillId::from_uuid(Uuid::from_u128(910)),
            expected_active_version_id: None,
            candidate_digest: sha256(b"candidate"),
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(911)),
            review_digest: sha256(b"review"),
        })
    }

    fn cancel_skill_review(&mut self) -> Result<(), AppError> {
        let mut state = self.state.lock().unwrap();
        state.cancel_calls += 1;
        if let Some(error) = self.cancel_error.clone() {
            return Err(error);
        }
        state.registered = false;
        Ok(())
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

fn add_review_script(terminal: &[u8]) -> Vec<u8> {
    let mut script = b"/skill add\n:name Cleanup Skill\n:description Cleanup contract.\n:use-when Use for cleanup.\n:instructions Keep review ownership exact.\n:review\n".to_vec();
    script.extend_from_slice(terminal);
    script
}

fn probe_runtime(
    commit_error: Option<AppError>,
    cancel_error: Option<AppError>,
    interrupt: Option<Sender<()>>,
) -> (ApplicationRuntime, Arc<Mutex<ProbeState>>) {
    let state = Arc::new(Mutex::new(ProbeState::default()));
    let runtime = ApplicationRuntime::spawn(
        ReviewProbeExecutor {
            state: state.clone(),
            commit_error,
            cancel_error,
            interrupt,
        },
        8,
    )
    .unwrap();
    (runtime, state)
}

struct ErrorAfterScript {
    script: Cursor<Vec<u8>>,
}

impl Read for ErrorAfterScript {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.script.position() < self.script.get_ref().len() as u64 {
            self.script.read(buffer)
        } else {
            Err(io::Error::other("injected input failure"))
        }
    }
}

impl BufRead for ErrorAfterScript {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.script.position() < self.script.get_ref().len() as u64 {
            self.script.fill_buf()
        } else {
            Err(io::Error::other("injected input failure"))
        }
    }

    fn consume(&mut self, amount: usize) {
        self.script.consume(amount);
    }
}

struct FailAfterPreviewWriter {
    state: Arc<Mutex<ProbeState>>,
}

impl Write for FailAfterPreviewWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.state.lock().unwrap().registered {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "injected write failure",
            ))
        } else {
            Ok(bytes.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ProbeCancellation(AtomicBool);

impl LineSourceCancellation for ProbeCancellation {
    fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct ProbeLineSource {
    events: VecDeque<io::Result<LineSourceEvent>>,
    cancellation: Arc<ProbeCancellation>,
}

impl ProbeLineSource {
    fn ending_with(terminal: io::Result<LineSourceEvent>) -> Self {
        let mut reader = BoundedLineReader::new(Cursor::new(add_review_script(b"")));
        let mut events = VecDeque::new();
        while let Some(line) = reader.next_line().unwrap() {
            events.push_back(Ok(LineSourceEvent::Line(line)));
        }
        events.push_back(terminal);
        Self {
            events,
            cancellation: Arc::new(ProbeCancellation(AtomicBool::new(false))),
        }
    }
}

impl CancellableLineSource for ProbeLineSource {
    fn cancellation(&self) -> Arc<dyn LineSourceCancellation> {
        self.cancellation.clone()
    }

    fn next_line(&mut self) -> io::Result<LineSourceEvent> {
        self.events
            .pop_front()
            .unwrap_or(Ok(LineSourceEvent::Cancelled))
    }
}

#[test]
fn parses_read_commands_with_quoted_selectors_and_positive_exact_versions() {
    assert_eq!(
        direct_command(b"/skill list"),
        ApplicationCommand::ListSkills
    );
    assert_eq!(direct_command(b"/skills"), ApplicationCommand::ListSkills);
    assert_eq!(
        direct_command(b"/skill show \"Evidence Review\""),
        ApplicationCommand::ShowSkill {
            selector: SkillSelector::from_input("Evidence Review").unwrap(),
        }
    );
    assert_eq!(
        direct_command(b"/skill show \"Evidence Review\" 1"),
        ApplicationCommand::ShowSkillVersion {
            selector: SkillSelector::from_input("Evidence Review").unwrap(),
            version: ObjectVersion::new(1).unwrap(),
        }
    );
}

#[test]
fn malformed_and_ambiguous_skill_commands_are_typed_and_actionable() {
    for input in [
        b"/skill".as_slice(),
        b"/skill show",
        b"/skill show evidence 0",
        b"/skill show evidence nope",
        b"/skill assign evidence agent 0",
        b"/skill assign evidence",
        b"/skill unassign evidence agent 1",
        b"/skill unknown",
        b"/skill show \"unterminated",
    ] {
        let ApplicationCommand::RejectInput(rejection) = direct_command(input) else {
            panic!("expected typed rejection for {input:?}");
        };
        assert_eq!(rejection.category, InputRejectionCategory::Malformed);
    }

    let runtime = support::runtime();
    let mut output = Vec::new();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(b"/skill unknown\n/quit\n"), &mut output)
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Usage: /skill list | /skill add | /skill show"));
    assert_eq!(reason, ShutdownReason::UserQuit);
    runtime.finish_and_join(reason);
}

#[test]
fn add_opens_a_guided_typed_review_and_never_commits_before_confirmation() {
    let runtime = support::runtime();
    let client = runtime.client();
    let input = Cursor::new(
        b"/skill add\n:name Fallback Research\n:description Optional command workflow.\n:use-when Use during research.\n:tag add fallback\n:instructions Keep evidence exact.\n:review\nq\n/quit\n",
    );
    let mut output = Vec::new();
    let reason = FallbackRunner::new(client.clone(), false)
        .run(input, &mut output)
        .unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("Create skill editor"));
    assert!(text.contains("Skill creation review"));
    assert!(text.contains("version 1"));
    assert!(text.contains("Confirmation did not match; skill review retained."));
    assert_eq!(reason, ShutdownReason::UserQuit);
    assert!(matches!(
        client.submit(ApplicationCommand::ShowSkill {
            selector: SkillSelector::from_input("Fallback Research").unwrap(),
        }),
        Err(RuntimeError::Application(AppError::SkillNotFound))
    ));
    runtime.finish_and_join(reason);
}

#[test]
fn add_confirmation_uses_the_registered_review_and_creates_once() {
    let runtime = support::runtime();
    let client = runtime.client();
    let input = Cursor::new(
        b"/skill add\n:name Fallback Research\n:description Optional command workflow.\n:use-when Use during research.\n:tag add fallback\n:instructions Keep evidence exact.\n:review\ncreate\n/skill show \"Fallback Research\"\n/quit\n",
    );
    let mut output = Vec::new();
    let reason = FallbackRunner::new(client, false)
        .run(input, &mut output)
        .unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("Skill creation review"));
    assert!(text.contains("Skill created:"));
    assert!(text.contains("Display name: Fallback Research"));
    assert_eq!(reason, ShutdownReason::UserQuit);
    runtime.finish_and_join(reason);
}

#[test]
fn assignment_stages_the_exact_active_ref_and_bare_q_never_commits_or_quits() {
    let runtime = support::runtime();
    let client = runtime.client();
    create_profile(&client, "Fallback Agent");

    let mut output = Vec::new();
    let reason = FallbackRunner::new(client.clone(), false)
        .run(
            Cursor::new(b"/skill assign \"Evidence Review\" \"Fallback Agent\"\nq\n/quit\n"),
            &mut output,
        )
        .unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("Skill assignment review: assign"));
    assert!(text.contains("exact version 1"));
    assert!(text.contains("Confirmation did not match; skill review retained."));
    assert_eq!(reason, ShutdownReason::UserQuit);

    let outcome = client
        .submit(ApplicationCommand::ShowAgentProfile {
            selector: ai_stock_forum::app::AgentProfileSelector::from_input("Fallback Agent")
                .unwrap(),
        })
        .unwrap();
    let CommandView::AgentProfile(view) = outcome.view else {
        panic!("expected profile view")
    };
    assert!(view.profile.skill_refs().is_empty());
    runtime.finish_and_join(reason);
}

#[test]
fn assign_and_unassign_commit_only_through_their_registered_reviews() {
    let runtime = support::runtime();
    let client = runtime.client();
    create_profile(&client, "Fallback Agent");

    let mut output = Vec::new();
    let reason = FallbackRunner::new(client.clone(), false)
        .run(
            Cursor::new(
                b"/skill assign \"Evidence Review\" \"Fallback Agent\" 1\nassign\n/skill unassign \"Evidence Review\" \"Fallback Agent\"\nunassign\n/quit\n",
            ),
            &mut output,
        )
        .unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("Skill assignment review: assign"));
    assert!(text.contains("Agent skill assigned:"));
    assert!(text.contains("Skill assignment review: unassign"));
    assert!(text.contains("Agent skill unassigned:"));
    assert_eq!(reason, ShutdownReason::UserQuit);

    let outcome = client
        .submit(ApplicationCommand::ShowAgentProfile {
            selector: ai_stock_forum::app::AgentProfileSelector::from_input("Fallback Agent")
                .unwrap(),
        })
        .unwrap();
    let CommandView::AgentProfile(view) = outcome.view else {
        panic!("expected profile view")
    };
    assert!(view.profile.skill_refs().is_empty());
    runtime.finish_and_join(reason);
}

#[test]
fn explicit_newer_version_routes_to_upgrade_without_changing_the_requested_ref() {
    let temporary_directory = TempDir::new().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut service = ApplicationService::bootstrap(
        &paths,
        Arc::new(support::TestClock::new()),
        Arc::new(support::TestIds::new()),
    )
    .unwrap();

    let first = skill("Versioned Fallback", "Version one.");
    let create_preview = service.preview_skill_creation(first.clone()).unwrap();
    service
        .execute_user(ApplicationCommand::CreateSkill {
            skill_id: create_preview.skill_id,
            candidate: first,
            review_token: create_preview.review_token,
            review_digest: create_preview.review_digest,
        })
        .unwrap();
    let first_ref = match service
        .execute_user(ApplicationCommand::ShowSkillVersion {
            selector: SkillSelector::from(create_preview.skill_id),
            version: ObjectVersion::new(1).unwrap(),
        })
        .unwrap()
        .view
    {
        CommandView::SkillVersion(view) => view.skill_ref,
        _ => panic!("expected version one"),
    };

    let created_profile = service
        .execute_user(ApplicationCommand::CreateAgentProfile {
            draft: profile("Upgrade Agent"),
            template_provenance: None,
        })
        .unwrap();
    let CommandView::AgentProfileCreated(created_profile) = created_profile.view else {
        panic!("expected created profile")
    };
    let assign_preview = service
        .preview_agent_skill_assignment(
            created_profile.profile_id,
            created_profile.profile_version_id,
            first_ref.clone(),
        )
        .unwrap();
    let assigned = service
        .execute_user(ApplicationCommand::AssignAgentSkill {
            profile_id: assign_preview.profile_id,
            expected_active_profile_version_id: assign_preview.expected_active_profile_version_id,
            skill: first_ref.clone(),
            review_token: assign_preview.review_token,
            review_digest: assign_preview.review_digest,
        })
        .unwrap();
    let CommandView::AgentSkillAssigned(assigned) = assigned.view else {
        panic!("expected assigned profile")
    };

    let second = skill("Versioned Fallback", "Version two.");
    let version_preview = service
        .preview_skill_version(
            create_preview.skill_id,
            first_ref.skill_version_id(),
            second.clone(),
        )
        .unwrap();
    service
        .execute_user(ApplicationCommand::ActivateSkillVersion {
            skill_id: version_preview.skill_id,
            expected_active_version_id: first_ref.skill_version_id(),
            candidate: second,
            review_token: version_preview.review_token,
            review_digest: version_preview.review_digest,
        })
        .unwrap();

    let runtime = ApplicationRuntime::spawn_application(service, 32).unwrap();
    let client = runtime.client();
    let mut output = Vec::new();
    let reason = FallbackRunner::new(client.clone(), false)
        .run(
            Cursor::new(
                b"/skill assign \"Versioned Fallback\" \"Upgrade Agent\" 2\nupgrade\n/quit\n",
            ),
            &mut output,
        )
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Skill assignment review: upgrade"));
    assert!(text.contains("exact version 2"));
    assert!(text.contains("Agent skill upgraded:"));

    let outcome = client
        .submit(ApplicationCommand::ShowAgentProfile {
            selector: ai_stock_forum::app::AgentProfileSelector::from_input("Upgrade Agent")
                .unwrap(),
        })
        .unwrap();
    let CommandView::AgentProfile(view) = outcome.view else {
        panic!("expected profile view")
    };
    assert_eq!(view.profile.skill_refs().len(), 1);
    assert_eq!(view.profile.skill_refs()[0].version().get(), 2);
    assert_ne!(view.profile.skill_refs()[0], first_ref);
    assert_eq!(view.profile.supersedes(), Some(assigned.profile_version_id));
    runtime.finish_and_join(reason).unwrap();
}

#[test]
fn explicit_cancel_and_eof_clear_registered_reviews_once_without_mutation() {
    for terminal in [b":cancel\n/quit\n".as_slice(), b""] {
        let (runtime, state) = probe_runtime(None, None, None);
        let reason = FallbackRunner::new(runtime.client(), false)
            .run(Cursor::new(add_review_script(terminal)), Vec::new())
            .unwrap();
        let state = state.lock().unwrap();
        assert!(!state.registered);
        assert_eq!(state.cancel_calls, 1);
        assert_eq!(state.mutation_attempts, 0);
        assert_eq!(state.committed_mutations, 0);
        drop(state);
        runtime.finish_and_join(reason).unwrap();
    }
}

#[test]
fn input_and_write_failures_after_preview_cancel_once_and_preserve_primary_error() {
    let (runtime, state) = probe_runtime(None, None, None);
    let result = FallbackRunner::new(runtime.client(), false).run(
        ErrorAfterScript {
            script: Cursor::new(add_review_script(b"")),
        },
        Vec::new(),
    );
    assert!(matches!(result, Err(UiError::Read)));
    let snapshot = state.lock().unwrap();
    assert!(!snapshot.registered);
    assert_eq!(snapshot.cancel_calls, 1);
    assert_eq!(snapshot.mutation_attempts, 0);
    drop(snapshot);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();

    let (runtime, state) = probe_runtime(None, None, None);
    let result = FallbackRunner::new(runtime.client(), false).run(
        Cursor::new(add_review_script(b"")),
        FailAfterPreviewWriter {
            state: state.clone(),
        },
    );
    assert!(matches!(result, Err(UiError::Write)));
    let snapshot = state.lock().unwrap();
    assert!(!snapshot.registered);
    assert_eq!(snapshot.cancel_calls, 1);
    assert_eq!(snapshot.mutation_attempts, 0);
    drop(snapshot);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();

    let (runtime, state) = probe_runtime(None, Some(AppError::LifecycleFinished), None);
    let result = FallbackRunner::new(runtime.client(), false).run(
        Cursor::new(add_review_script(b"")),
        FailAfterPreviewWriter {
            state: state.clone(),
        },
    );
    assert!(matches!(result, Err(UiError::Write)));
    assert_eq!(state.lock().unwrap().cancel_calls, 1);
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn host_interrupt_cancellation_and_reader_error_do_not_orphan_reviews() {
    let (interrupt_sender, interrupt_receiver) = bounded(1);
    let (runtime, state) = probe_runtime(None, None, Some(interrupt_sender));
    let reason = FallbackHost::new(runtime, false, false)
        .run(
            ProbeLineSource::ending_with(Ok(LineSourceEvent::Cancelled)),
            Vec::new(),
            interrupt_receiver,
        )
        .unwrap();
    assert_eq!(reason, ShutdownReason::Interrupted);
    let snapshot = state.lock().unwrap();
    assert!(!snapshot.registered);
    assert_eq!(snapshot.cancel_calls, 1);
    assert_eq!(snapshot.mutation_attempts, 0);
    drop(snapshot);

    for terminal in [
        Ok(LineSourceEvent::Cancelled),
        Err(io::Error::other("injected source failure")),
    ] {
        let (runtime, state) = probe_runtime(None, None, None);
        let result = FallbackHost::new(runtime, false, false).run(
            ProbeLineSource::ending_with(terminal),
            Vec::new(),
            never(),
        );
        assert!(matches!(result, Err(UiError::ReaderThread | UiError::Read)));
        let snapshot = state.lock().unwrap();
        assert!(!snapshot.registered);
        assert_eq!(snapshot.cancel_calls, 1);
        assert_eq!(snapshot.mutation_attempts, 0);
    }
}

#[test]
fn application_failures_cancel_review_and_restore_the_creation_editor() {
    let failures = [
        AppError::StaleSkillVersion,
        AppError::CapabilityDenied {
            capability: Capability::SkillCreate,
            decision: PolicyDecision::DeniedByDefault,
        },
        AppError::SkillReviewUnavailable,
        AppError::SkillReviewMismatch,
    ];
    for failure in failures {
        let (runtime, state) = probe_runtime(Some(failure), None, None);
        let mut output = Vec::new();
        let reason = FallbackRunner::new(runtime.client(), false)
            .run(
                Cursor::new(add_review_script(b"create\n:cancel\n/quit\n")),
                &mut output,
            )
            .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(!text.contains("Start a fresh /skill command to request a new review."));
        assert_eq!(text.matches("Create skill editor").count(), 6);
        let snapshot = state.lock().unwrap();
        assert!(!snapshot.registered);
        assert_eq!(snapshot.cancel_calls, 2);
        assert_eq!(snapshot.mutation_attempts, 1);
        assert_eq!(snapshot.committed_mutations, 0);
        drop(snapshot);
        runtime.finish_and_join(reason).unwrap();
    }
}

#[test]
fn recoverable_creation_failure_cancels_review_and_restores_the_exact_draft() {
    let (runtime, state) = probe_runtime(Some(AppError::StaleSkillVersion), None, None);
    let mut output = Vec::new();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(
            Cursor::new(add_review_script(b"create\n:review\ncreate\n/quit\n")),
            &mut output,
        )
        .unwrap();

    let expected = SkillDraft::new(
        "Cleanup Skill".to_owned(),
        "Cleanup contract.".to_owned(),
        "Use for cleanup.".to_owned(),
        Vec::new(),
        "Keep review ownership exact.".to_owned(),
        Vec::new(),
    )
    .unwrap();
    let snapshot = state.lock().unwrap();
    assert!(!snapshot.registered);
    assert_eq!(snapshot.cancel_calls, 1);
    assert_eq!(snapshot.mutation_attempts, 2);
    assert_eq!(snapshot.committed_mutations, 1);
    assert_eq!(
        snapshot.preview_candidates,
        vec![expected.clone(), expected]
    );
    drop(snapshot);
    runtime.finish_and_join(reason).unwrap();
}

#[test]
fn runtime_skill_requests_are_typed_and_cannot_cross_wire_replies() {
    struct RouteExecutor {
        calls: Sender<&'static str>,
    }

    impl CommandExecutor for RouteExecutor {
        fn execute_user(
            &mut self,
            _command: ApplicationCommand,
        ) -> Result<CommandOutcome, AppError> {
            Ok(command_outcome(
                CommandView::Help(HelpView),
                ShutdownDisposition::Continue,
            ))
        }

        fn preview_skill_creation(
            &mut self,
            _candidate: SkillDraft,
        ) -> Result<SkillEditPreview, AppError> {
            self.calls.send("create").unwrap();
            Err(AppError::DuplicateSkillName)
        }

        fn preview_agent_skill_assignment(
            &mut self,
            _profile_id: AgentProfileId,
            _expected_active_profile_version_id: AgentProfileVersionId,
            _skill: SkillVersionRef,
        ) -> Result<AgentSkillAssignmentPreview, AppError> {
            self.calls.send("assign").unwrap();
            Err(AppError::SkillAlreadyAssigned)
        }

        fn preview_agent_skill_upgrade(
            &mut self,
            _profile_id: AgentProfileId,
            _expected_active_profile_version_id: AgentProfileVersionId,
            _expected: SkillVersionRef,
            _replacement: SkillVersionRef,
        ) -> Result<AgentSkillAssignmentPreview, AppError> {
            self.calls.send("upgrade").unwrap();
            Err(AppError::SkillNotAssigned)
        }

        fn preview_agent_skill_unassignment(
            &mut self,
            _profile_id: AgentProfileId,
            _expected_active_profile_version_id: AgentProfileVersionId,
            _expected: SkillVersionRef,
        ) -> Result<AgentSkillAssignmentPreview, AppError> {
            self.calls.send("unassign").unwrap();
            Err(AppError::AgentSkillLimitExceeded)
        }

        fn cancel_skill_review(&mut self) -> Result<(), AppError> {
            self.calls.send("cancel").unwrap();
            Err(AppError::CommandConflict)
        }

        fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
            Ok(())
        }
    }

    let (calls_sender, calls_receiver) = bounded(5);
    let runtime = ApplicationRuntime::spawn(
        RouteExecutor {
            calls: calls_sender,
        },
        8,
    )
    .unwrap();
    let client = runtime.client();
    let profile_id = AgentProfileId::from_uuid(Uuid::from_u128(930));
    let profile_version_id = AgentProfileVersionId::from_uuid(Uuid::from_u128(931));
    let first = exact_ref(940, 1);
    let second = exact_ref(950, 2);

    assert!(matches!(
        client.preview_skill_creation(skill("Route Skill", "Create.")),
        Err(RuntimeError::Application(AppError::DuplicateSkillName))
    ));
    assert!(matches!(
        client.preview_agent_skill_assignment(profile_id, profile_version_id, first.clone()),
        Err(RuntimeError::Application(AppError::SkillAlreadyAssigned))
    ));
    assert!(matches!(
        client.preview_agent_skill_upgrade(
            profile_id,
            profile_version_id,
            first.clone(),
            second.clone(),
        ),
        Err(RuntimeError::Application(AppError::SkillNotAssigned))
    ));
    assert!(matches!(
        client.preview_agent_skill_unassignment(profile_id, profile_version_id, first),
        Err(RuntimeError::Application(AppError::AgentSkillLimitExceeded))
    ));
    assert!(matches!(
        client.cancel_skill_review(),
        Err(RuntimeError::Application(AppError::CommandConflict))
    ));
    assert_eq!(
        calls_receiver.iter().take(5).collect::<Vec<_>>(),
        vec!["create", "assign", "upgrade", "unassign", "cancel"]
    );
    runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .unwrap();
}

#[test]
fn runtime_cancel_invalidates_the_real_registered_review_without_mutation() {
    let runtime = support::runtime();
    let client = runtime.client();
    let candidate = skill("Cancelled Runtime Skill", "Never commit.");
    let preview = client.preview_skill_creation(candidate.clone()).unwrap();
    client.cancel_skill_review().unwrap();
    assert!(matches!(
        client.submit(ApplicationCommand::CreateSkill {
            skill_id: preview.skill_id,
            candidate,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        }),
        Err(RuntimeError::Application(AppError::SkillReviewUnavailable))
    ));
    assert!(matches!(
        client.submit(ApplicationCommand::ShowSkill {
            selector: SkillSelector::from(preview.skill_id),
        }),
        Err(RuntimeError::Application(AppError::SkillNotFound))
    ));
    runtime.finish_and_join(ShutdownReason::ApplicationError);
}
