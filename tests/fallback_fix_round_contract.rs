mod support;

use std::{
    collections::VecDeque,
    io::{self, Cursor, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use ai_stock_forum::{
    app::{
        AppError, ApplicationCommand, ApplicationEvent, ApplicationService, AuditTailView,
        AuthorizationDecision, CommandOutcome, CommandPolicy, CommandView, EventEnvelope,
        HelpView, InputRejection, InputRejectionCategory, ShutdownDisposition, ShutdownReason,
    },
    audit::AuditEntry,
    config::AppPaths,
    domain::{
        sha256, Actor, CorrelationId, EventId, IdGenerator, InstallationId, SessionId,
    },
    policy::Capability,
    runtime::{
        ApplicationRuntime, CommandExecutor, RuntimeError, RuntimeThreadSpawner,
    },
    ui::command::{
        BoundedLineReader, BufferedLineSource, FallbackHost, FallbackRunner, LineSource,
        LineSourceCancellation, LineSourceEvent, RawLine, TextRenderer, UiError,
    },
};
use crossbeam_channel::{bounded, never, Receiver, Sender};
use rusqlite::Connection;
use uuid::Uuid;

const TIMEOUT: Duration = Duration::from_secs(5);

fn receive<T>(receiver: &Receiver<T>) -> T {
    receiver
        .recv_timeout(TIMEOUT)
        .expect("timed out waiting for bounded test handshake")
}

fn raw_line(bytes: &[u8]) -> RawLine {
    BoundedLineReader::new(Cursor::new(bytes.to_vec()))
        .next_line()
        .unwrap()
        .unwrap()
}

#[test]
fn reader_only_strips_cr_immediately_before_the_actual_lf() {
    let mut physical = vec![b'x'; 4096];
    physical.extend_from_slice(b"\rX\n/help\n");
    let mut reader = BoundedLineReader::new(Cursor::new(physical));

    let line = reader.next_line().unwrap().unwrap();
    assert!(line.was_oversized());
    assert_eq!(line.full_byte_length(), 4098);
    assert_eq!(line.bytes().len(), 4097);
    assert_eq!(line.bytes().last(), Some(&b'\r'));
    assert_eq!(
        line.input_digest().as_str(),
        "8e3cfbc10aada275576c8a0fef719bd609a31630d173e1094d04ce81e0355e54"
    );
    assert_eq!(reader.next_line().unwrap().unwrap().bytes(), b"/help");
}

#[test]
fn reader_hashes_and_counts_the_complete_32_kib_logical_line_independently() {
    let mut physical = vec![0xff; 32 * 1024];
    physical.extend_from_slice(b"\r\n/status\n");
    let mut reader = BoundedLineReader::new(Cursor::new(physical));

    let line = reader.next_line().unwrap().unwrap();
    assert!(line.was_oversized());
    assert_eq!(line.full_byte_length(), 32 * 1024);
    assert_eq!(line.bytes().len(), 4097);
    assert_eq!(
        line.input_digest().as_str(),
        "2d864c0b789a43214eee8524d3182075125e5ca2cd527f3582ec87ffd94076bc"
    );
    assert_eq!(reader.next_line().unwrap().unwrap().bytes(), b"/status");
}

struct RecordingExecutor {
    command: Sender<ApplicationCommand>,
}

impl CommandExecutor for RecordingExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        self.command.send(command).unwrap();
        Ok(CommandOutcome {
            command_id: ai_stock_forum::domain::CommandId::from_uuid(Uuid::from_u128(1)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(2)),
            committed_events: Vec::new(),
            view: CommandView::Help(HelpView),
            shutdown: ShutdownDisposition::Continue,
        })
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

#[test]
fn runner_constructs_oversized_rejection_from_authoritative_metadata() {
    let (command_sender, command_receiver) = bounded(1);
    let runtime = ApplicationRuntime::spawn(
        RecordingExecutor {
            command: command_sender,
        },
        1,
    )
    .unwrap();
    let mut physical = vec![0xff; 32 * 1024];
    physical.push(b'\n');

    let reason = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(physical), Vec::new())
        .unwrap();
    let ApplicationCommand::RejectInput(rejection) = receive(&command_receiver) else {
        panic!("oversized input must bypass parsing and become a typed rejection");
    };
    assert_eq!(rejection.category, InputRejectionCategory::Oversized);
    assert_eq!(rejection.byte_length, 32 * 1024);
    assert_eq!(
        rejection.input_digest.as_str(),
        "2d864c0b789a43214eee8524d3182075125e5ca2cd527f3582ec87ffd94076bc"
    );
    assert!(rejection.safe_token.is_none());
    runtime.finish_and_join(reason).unwrap();
}

struct CancelSignal {
    sender: Sender<()>,
    calls: Arc<AtomicUsize>,
}

impl LineSourceCancellation for CancelSignal {
    fn cancel(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.sender.try_send(());
    }
}

struct CancellableSource {
    lines: VecDeque<RawLine>,
    cancellation: Receiver<()>,
    cancel_signal: Arc<CancelSignal>,
    exited: Sender<()>,
}

impl LineSource for CancellableSource {
    fn cancellation(&self) -> Arc<dyn LineSourceCancellation> {
        self.cancel_signal.clone()
    }

    fn next_line(&mut self) -> io::Result<LineSourceEvent> {
        if let Some(line) = self.lines.pop_front() {
            return Ok(LineSourceEvent::Line(line));
        }
        receive(&self.cancellation);
        self.exited.send(()).unwrap();
        Ok(LineSourceEvent::Cancelled)
    }
}

fn cancellable_source(
    lines: impl IntoIterator<Item = RawLine>,
) -> (CancellableSource, Arc<AtomicUsize>, Receiver<()>) {
    let (cancel_sender, cancel_receiver) = bounded(1);
    let (exited_sender, exited_receiver) = bounded(1);
    let calls = Arc::new(AtomicUsize::new(0));
    let cancel_signal = Arc::new(CancelSignal {
        sender: cancel_sender,
        calls: calls.clone(),
    });
    (
        CancellableSource {
            lines: lines.into_iter().collect(),
            cancellation: cancel_receiver,
            cancel_signal,
            exited: exited_sender,
        },
        calls,
        exited_receiver,
    )
}

struct FinishExecutor {
    finish: Sender<ShutdownReason>,
}

impl CommandExecutor for FinishExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        let (view, shutdown) = if command == ApplicationCommand::RequestShutdown {
            (
                CommandView::Shutdown(ai_stock_forum::app::ShutdownView {
                    disposition: ShutdownDisposition::Requested,
                }),
                ShutdownDisposition::Requested,
            )
        } else {
            (CommandView::Help(HelpView), ShutdownDisposition::Continue)
        };
        Ok(CommandOutcome {
            command_id: ai_stock_forum::domain::CommandId::from_uuid(Uuid::from_u128(3)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(4)),
            committed_events: Vec::new(),
            view,
            shutdown,
        })
    }

    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        self.finish.send(reason).unwrap();
        Ok(())
    }
}

#[test]
fn host_cancels_and_joins_its_owned_line_source_before_returning() {
    let (finish_sender, finish_receiver) = bounded(1);
    let runtime = ApplicationRuntime::spawn(
        FinishExecutor {
            finish: finish_sender,
        },
        1,
    )
    .unwrap();
    let (source, cancel_calls, source_exited) =
        cancellable_source([raw_line(b"/quit\n")]);

    let reason = FallbackHost::new(runtime, false, false)
        .run(source, Vec::new(), never())
        .unwrap();

    assert_eq!(reason, ShutdownReason::UserQuit);
    assert_eq!(receive(&finish_receiver), ShutdownReason::UserQuit);
    receive(&source_exited);
    assert_eq!(cancel_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn disconnected_interrupt_channel_is_disabled_without_prompt_livelock() {
    let (finish_sender, finish_receiver) = bounded(1);
    let runtime = ApplicationRuntime::spawn(
        FinishExecutor {
            finish: finish_sender,
        },
        1,
    )
    .unwrap();
    let (_interrupt_sender, interrupts) = bounded::<()>(1);
    drop(_interrupt_sender);
    let source = BufferedLineSource::new(Cursor::new(Vec::<u8>::new()));
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = SharedWriter(output.clone());

    let (result_sender, result_receiver) = bounded(1);
    thread::spawn(move || {
        let result = FallbackHost::new(runtime, true, false).run(source, writer, interrupts);
        result_sender.send(result).unwrap();
    });
    let result = receive(&result_receiver);

    assert_eq!(result.unwrap(), ShutdownReason::InputClosed);
    assert_eq!(output.lock().unwrap().as_slice(), b"> ");
    assert_eq!(receive(&finish_receiver), ShutdownReason::InputClosed);
}

#[derive(Clone)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FailingSpawner;

impl RuntimeThreadSpawner for FailingSpawner {
    fn spawn(
        &self,
        _task: Box<dyn FnOnce() + Send + 'static>,
    ) -> io::Result<thread::JoinHandle<()>> {
        Err(io::Error::new(io::ErrorKind::Other, "injected spawn failure"))
    }
}

fn latest_end_reason(paths: &AppPaths) -> Option<String> {
    Connection::open(paths.database_path())
        .unwrap()
        .query_row(
            "SELECT end_reason FROM process_session_projection ORDER BY started_at_ms DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn assert_next_launch_has_no_interruption(paths: &AppPaths) {
    struct NextLaunchIds(AtomicUsize);

    impl IdGenerator for NextLaunchIds {
        fn next_uuid(&self) -> Uuid {
            Uuid::from_u128(1_000_000 + self.0.fetch_add(1, Ordering::SeqCst) as u128)
        }
    }

    let mut next = ApplicationService::bootstrap(
        paths,
        Arc::new(support::TestClock::new()),
        Arc::new(NextLaunchIds(AtomicUsize::new(0))),
    )
    .unwrap();
    assert!(!next.previous_session_interrupted());
    next.finish(ShutdownReason::InputClosed).unwrap();
}

#[test]
fn service_runtime_spawn_failure_finishes_before_returning() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let service = ApplicationService::bootstrap(
        &paths,
        Arc::new(support::TestClock::new()),
        Arc::new(support::TestIds::new()),
    )
    .unwrap();

    let result = ApplicationRuntime::spawn_application_with_thread_spawner(
        service,
        1,
        Arc::new(FailingSpawner),
    );
    assert!(matches!(result, Err(RuntimeError::WorkerStartup)));
    assert_eq!(latest_end_reason(&paths).as_deref(), Some("application_error"));
    assert_next_launch_has_no_interruption(&paths);
}

struct PanickingPolicy;

impl CommandPolicy for PanickingPolicy {
    fn authorize(&self, _capability: Capability) -> AuthorizationDecision {
        panic!("policy panic with secret detail")
    }
}

#[test]
fn command_panic_best_effort_finishes_real_service_before_worker_exit() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let service = ApplicationService::bootstrap_with_policy(
        &paths,
        Arc::new(support::TestClock::new()),
        Arc::new(support::TestIds::new()),
        Arc::new(PanickingPolicy),
    )
    .unwrap();
    let runtime = ApplicationRuntime::spawn_application(service, 1).unwrap();

    assert_eq!(
        runtime.client().submit(ApplicationCommand::ShowHelp),
        Err(RuntimeError::WorkerPanicked)
    );
    assert_eq!(
        runtime.finish_and_join(ShutdownReason::ApplicationError),
        Err(RuntimeError::WorkerPanicked)
    );
    assert_eq!(latest_end_reason(&paths).as_deref(), Some("application_error"));
    assert_next_launch_has_no_interruption(&paths);
}

fn envelope(sequence: u64, event: ApplicationEvent) -> EventEnvelope {
    EventEnvelope {
        sequence,
        event_id: EventId::from_uuid(Uuid::from_u128(100 + sequence as u128)),
        event_schema_version: ai_stock_forum::app::EVENT_SCHEMA_VERSION,
        actor: if sequence % 2 == 0 { Actor::Human } else { Actor::System },
        occurred_at_ms: 1_700_000_000_000 + sequence as i64,
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(200 + sequence as u128)),
        causation_id: None,
        object: None,
        event,
        previous_event_digest: (sequence > 1).then(|| sha256(b"previous")),
        event_digest: sha256(format!("event-{sequence}").as_bytes()),
    }
}

#[test]
fn audit_renderer_includes_every_typed_summary_and_required_metadata_safely() {
    let installation = InstallationId::from_uuid(Uuid::from_u128(11));
    let session = SessionId::from_uuid(Uuid::from_u128(12));
    let raw_secret = b"/unknown credential=top-secret /private/db.sqlite";
    let rejection = InputRejection::from_input(
        InputRejectionCategory::Unknown,
        Some(ai_stock_forum::app::SafeToken::new("/unknown").unwrap()),
        raw_secret,
    );
    let raw_digest = rejection.input_digest.to_string();
    let events = vec![
        ApplicationEvent::InstallationInitialized { installation_id: installation },
        ApplicationEvent::ProcessSessionStarted { session_id: session },
        ApplicationEvent::PreviousSessionInterrupted { session_id: session },
        ApplicationEvent::HelpViewed,
        ApplicationEvent::StatusViewed,
        ApplicationEvent::SetupStatusViewed,
        ApplicationEvent::AuditTailViewed {
            limit: ai_stock_forum::app::AuditLimit::new(3).unwrap(),
        },
        ApplicationEvent::CommandRejected { rejection },
        ApplicationEvent::ShutdownRequested,
        ApplicationEvent::ProcessSessionEnded {
            session_id: session,
            reason: ShutdownReason::Interrupted,
        },
        ApplicationEvent::ProjectionRebuilt { through_sequence: 10 },
    ];
    let entries: Vec<_> = events
        .into_iter()
        .enumerate()
        .map(|(index, event)| AuditEntry::from_event(&envelope(index as u64 + 1, event)))
        .collect();
    let expected_summaries = [
        format!("installation initialized: {installation}"),
        format!("process session started: {session}"),
        format!("previous session interrupted: {session}"),
        "help viewed".to_owned(),
        "status viewed".to_owned(),
        "setup status viewed".to_owned(),
        "audit tail viewed: 3".to_owned(),
        format!("command rejected: category=unknown, token=/unknown, bytes={}", raw_secret.len()),
        "shutdown requested".to_owned(),
        format!("process session ended: {session}, reason=Interrupted"),
        "projection rebuilt through sequence 10".to_owned(),
    ];
    assert_eq!(
        entries.iter().map(|entry| entry.summary.clone()).collect::<Vec<_>>(),
        expected_summaries
    );

    let view = CommandView::AuditTail(AuditTailView {
        limit: ai_stock_forum::app::AuditLimit::new(20).unwrap(),
        entries: entries.clone(),
    });
    let mut output = Vec::new();
    TextRenderer::render_view(&view, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    for entry in &entries {
        assert!(text.contains(&format!("#{}", entry.sequence)));
        assert!(text.contains(&entry.occurred_at_ms.to_string()));
        assert!(text.contains(match entry.actor { Actor::Human => "human", Actor::System => "system" }));
        assert!(text.contains(&entry.kind));
        assert!(text.contains(&entry.correlation_id.to_string()));
        assert!(text.contains(&entry.summary));
    }
    assert!(!text.contains("top-secret"));
    assert!(!text.contains("/private/db.sqlite"));
    assert!(!text.contains(&raw_digest));
    assert!(!text.contains("payload_json"));
}

struct ErrorExecutor;

impl CommandExecutor for ErrorExecutor {
    fn execute_user(&mut self, _command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        Err(AppError::LifecycleFinished)
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

#[test]
fn runner_leaves_terminal_runtime_error_unrendered_for_the_composition_root() {
    let runtime = ApplicationRuntime::spawn(ErrorExecutor, 1).unwrap();
    let mut output = Vec::new();
    let result = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(b"/help\n".to_vec()), &mut output);

    assert!(matches!(
        result,
        Err(UiError::Runtime(RuntimeError::Application(
            AppError::LifecycleFinished
        )))
    ));
    assert!(output.is_empty());
    assert!(runtime
        .finish_and_join(ShutdownReason::ApplicationError)
        .is_ok());
}

#[cfg(unix)]
mod unix_binary {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Child, ChildStdin, Command, Output, Stdio},
        time::Instant,
    };

    use super::*;
    use wait_timeout::ChildExt;

    fn state_path(home: &Path, xdg_data: &Path) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            let _ = xdg_data;
            home.join("Library/Application Support/ai-stock-forum")
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = home;
            xdg_data.join("ai-stock-forum")
        }
    }

    fn spawn_binary(home: &Path, xdg_data: &Path) -> Child {
        Command::new(env!("CARGO_BIN_EXE_ai-stock-forum"))
            .env("HOME", home)
            .env("XDG_DATA_HOME", xdg_data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn wait_for_output(mut child: Child, stdin: Option<ChildStdin>) -> Output {
        let status = child.wait_timeout(TIMEOUT).unwrap();
        if status.is_none() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("binary did not exit within bounded timeout");
        }
        drop(stdin);
        child.wait_with_output().unwrap()
    }

    fn wait_for_open_session(path: &Path) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if path.exists()
                && Connection::open(path)
                    .and_then(|connection| {
                        connection.query_row(
                            "SELECT COUNT(*) FROM process_session_projection WHERE ended_event_id IS NULL",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                    })
                    .is_ok_and(|count| count == 1)
            {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for open process session");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn latest_reason(state: &Path) -> String {
        Connection::open(state.join("ai-stock-forum.sqlite3"))
            .unwrap()
            .query_row(
                "SELECT end_reason FROM process_session_projection ORDER BY started_at_ms DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn run_line(home: &Path, xdg: &Path, input: &[u8]) -> Output {
        let mut child = spawn_binary(home, xdg);
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(input).unwrap();
        drop(stdin);
        wait_for_output(child, None)
    }

    #[test]
    fn binary_quit_and_eof_persist_exact_shutdown_reasons() {
        for (input, expected) in [
            (b"/quit\n".as_slice(), "user_quit"),
            (b"".as_slice(), "input_closed"),
        ] {
            let temporary = tempfile::tempdir().unwrap();
            let home = temporary.path().join("home");
            let xdg = temporary.path().join("xdg");
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&xdg).unwrap();
            let output = run_line(&home, &xdg, input);
            assert!(output.status.success());
            assert_eq!(latest_reason(&state_path(&home, &xdg)), expected);
        }
    }

    #[test]
    fn binary_sigint_with_open_stdin_exits_cleanly_and_does_not_warn_next_launch() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().join("home");
        let xdg = temporary.path().join("xdg");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&xdg).unwrap();
        let state = state_path(&home, &xdg);
        let database = state.join("ai-stock-forum.sqlite3");
        let mut child = spawn_binary(&home, &xdg);
        let stdin = child.stdin.take().unwrap();
        wait_for_open_session(&database);

        assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
        let output = wait_for_output(child, Some(stdin));
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(stdout.matches("Interrupted. Shutting down.").count(), 1);
        assert!(output.stderr.is_empty());
        assert_eq!(latest_reason(&state), "interrupted");

        let next = run_line(&home, &xdg, b"/quit\n");
        assert!(next.status.success());
        assert!(!String::from_utf8(next.stdout)
            .unwrap()
            .contains("previous session ended unexpectedly"));
    }
}
