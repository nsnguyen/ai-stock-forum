mod support;

use std::{
    fs,
    io::{self, BufRead, Cursor, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use ai_stock_forum::{
    app::{
        AppError, ApplicationCommand, AuditTailView, CommandOutcome, CommandView, HelpView,
        InputRejectedView, InputRejection, InputRejectionCategory, SetupStatusView,
        ShutdownDisposition, ShutdownReason, ShutdownView, StatusView,
    },
    audit::AuditEntry,
    config::{AppPaths, StartupError},
    domain::{
        Actor, CommandId, ConfigurationVersionId, CorrelationId, InstallationId, SessionId,
        SetupDraftId,
    },
    persistence::{PersistenceError, RecoveryError},
    runtime::{ApplicationRuntime, CommandExecutor, RuntimeError},
    setup::SetupStatus,
    ui::command::{
        BoundedLineReader, CancellableLineSource, FallbackHost, FallbackRunner,
        LineSourceCancellation, LineSourceEvent, ParsedLine, TextRenderer, UiError, parse_line,
    },
};
use crossbeam_channel::{Receiver, Sender, bounded, never};
use tempfile::TempDir;
use uuid::Uuid;

struct ScriptedCancellation(std::sync::atomic::AtomicBool);

impl LineSourceCancellation for ScriptedCancellation {
    fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

struct ScriptedLineSource {
    events: std::collections::VecDeque<std::io::Result<LineSourceEvent>>,
    cancellation: std::sync::Arc<ScriptedCancellation>,
}

impl CancellableLineSource for ScriptedLineSource {
    fn cancellation(&self) -> std::sync::Arc<dyn LineSourceCancellation> {
        self.cancellation.clone()
    }

    fn next_line(&mut self) -> std::io::Result<LineSourceEvent> {
        self.events.pop_front().unwrap_or(Ok(LineSourceEvent::Eof))
    }
}

fn scripted_source<R: std::io::BufRead>(reader: R) -> ScriptedLineSource {
    let mut reader = BoundedLineReader::new(reader);
    let mut events = std::collections::VecDeque::new();
    loop {
        match reader.next_line() {
            Ok(Some(line)) => events.push_back(Ok(LineSourceEvent::Line(line))),
            Ok(None) => {
                events.push_back(Ok(LineSourceEvent::Eof));
                break;
            }
            Err(error) => {
                events.push_back(Err(error));
                break;
            }
        }
    }
    ScriptedLineSource {
        events,
        cancellation: std::sync::Arc::new(ScriptedCancellation(
            std::sync::atomic::AtomicBool::new(false),
        )),
    }
}

const TIMEOUT: Duration = Duration::from_secs(5);

fn receive<T>(receiver: &Receiver<T>) -> T {
    receiver
        .recv_timeout(TIMEOUT)
        .expect("timed out waiting for bounded test handshake")
}

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn outcome(view: CommandView, shutdown: ShutdownDisposition) -> CommandOutcome {
    CommandOutcome {
        command_id: CommandId::from_uuid(id(101)),
        correlation_id: CorrelationId::from_uuid(id(102)),
        committed_events: Vec::new(),
        view,
        shutdown,
    }
}

fn outcome_for(command: ApplicationCommand) -> CommandOutcome {
    match command {
        ApplicationCommand::RequestShutdown => outcome(
            CommandView::Shutdown(ShutdownView {
                disposition: ShutdownDisposition::Requested,
            }),
            ShutdownDisposition::Requested,
        ),
        _ => outcome(CommandView::Help(HelpView), ShutdownDisposition::Continue),
    }
}

fn render(view: CommandView) -> String {
    let mut bytes = Vec::new();
    TextRenderer::render_view(&view, &mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn oversized_line_is_one_rejection_and_next_line_survives() {
    let mut bytes = vec![b'x'; 32 * 1024];
    bytes.extend_from_slice(b"\n/help\n");
    let mut reader = BoundedLineReader::new(Cursor::new(bytes));

    let rejected = reader.next_line().unwrap().unwrap();
    assert!(rejected.was_oversized());
    assert_eq!(rejected.bytes().len(), 4097);
    assert_eq!(reader.next_line().unwrap().unwrap().bytes(), b"/help");
}

#[test]
fn oversized_invalid_utf8_is_rejected_by_size_before_decoding() {
    let mut bytes = vec![0xff; 4097];
    bytes.extend_from_slice(b"\n/status\n");
    let mut reader = BoundedLineReader::new(Cursor::new(bytes));
    let rejected = reader.next_line().unwrap().unwrap();

    let ParsedLine::Command(ApplicationCommand::RejectInput(rejection)) =
        parse_line(rejected.bytes())
    else {
        panic!("oversized input must become a typed rejection");
    };
    assert_eq!(rejection.category, InputRejectionCategory::Oversized);
    assert_eq!(reader.next_line().unwrap().unwrap().bytes(), b"/status");
}

#[test]
fn reader_accepts_eof_without_newline_and_normalizes_crlf() {
    let mut reader = BoundedLineReader::new(Cursor::new(b"/help\r\n/status".to_vec()));
    assert_eq!(reader.next_line().unwrap().unwrap().bytes(), b"/help");
    assert_eq!(reader.next_line().unwrap().unwrap().bytes(), b"/status");
    assert!(reader.next_line().unwrap().is_none());
}

struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("secret reader detail"))
    }
}

impl BufRead for FailingReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::Error::other("secret reader detail"))
    }

    fn consume(&mut self, _amount: usize) {}
}

#[test]
fn reader_propagates_io_errors_without_returning_partial_input() {
    let error = BoundedLineReader::new(FailingReader)
        .next_line()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
}

#[test]
fn renderer_exhaustively_maps_help_status_setup_and_shutdown_views() {
    let help = render(CommandView::Help(HelpView));
    assert_eq!(help.matches("\n  /").count(), 5);
    assert!(help.contains("Available commands"));
    assert!(help.contains("/audit tail [limit: 1-100]"));
    assert!(!help.contains("broker"));
    assert!(!help.contains("network"));

    let status = render(CommandView::Status(StatusView {
        installation_id: InstallationId::from_uuid(id(201)),
        session_id: SessionId::from_uuid(id(202)),
    }));
    assert_eq!(status, "Installation: ready\nSession: active\n");
    assert!(!status.contains(&id(201).to_string()));
    assert!(!status.contains(&id(202).to_string()));

    let not_started = render(CommandView::SetupStatus(SetupStatusView {
        status: SetupStatus::NotStarted,
    }));
    assert!(not_started.contains("Setup: not started"));
    assert!(not_started.contains("Guided setup is not implemented"));

    let draft = render(CommandView::SetupStatus(SetupStatusView {
        status: SetupStatus::DraftSaved {
            draft_id: SetupDraftId::from_uuid(id(203)),
        },
    }));
    assert_eq!(draft, "Setup: draft saved\n");
    assert!(!draft.contains(&id(203).to_string()));

    let applied = render(CommandView::SetupStatus(SetupStatusView {
        status: SetupStatus::Applied {
            configuration_id: ConfigurationVersionId::from_uuid(id(204)),
        },
    }));
    assert_eq!(applied, "Setup: applied\n");
    assert!(!applied.contains(&id(204).to_string()));

    assert_eq!(
        render(CommandView::Shutdown(ShutdownView {
            disposition: ShutdownDisposition::Continue,
        })),
        "Shutdown was not requested.\n"
    );
    assert_eq!(
        render(CommandView::Shutdown(ShutdownView {
            disposition: ShutdownDisposition::Requested,
        })),
        "Shutting down.\n"
    );
}

#[test]
fn renderer_bounds_and_escapes_audit_fields_without_rendering_event_json() {
    let text = render(CommandView::AuditTail(AuditTailView {
        limit: ai_stock_forum::app::AuditLimit::new(3).unwrap(),
        entries: vec![AuditEntry {
            sequence: 7,
            occurred_at_ms: 1_700_000_000_000,
            actor: Actor::Human,
            kind: "command\u{1b}[31m".repeat(40),
            correlation_id: CorrelationId::from_uuid(id(301)),
            summary: "credential=do-not-print\n".repeat(80),
        }],
    }));

    assert!(text.contains("Audit tail"));
    assert!(text.contains("#7"));
    assert!(!text.contains('\u{1b}'));
    assert_eq!(text.matches('\n').count(), 2);
    assert!(text.contains(&id(301).to_string()));
    assert!(!text.contains("payload_json"));
    assert!(text.len() <= 700);
}

#[test]
fn renderer_never_echoes_raw_rejected_input_or_its_digest() {
    let raw = b"/unknown credential=top-secret-password";
    let rejection = InputRejection::from_input(
        InputRejectionCategory::Unknown,
        Some(ai_stock_forum::app::SafeToken::new("/unknown").unwrap()),
        raw,
    );
    let digest = rejection.input_digest.to_string();
    let text = render(CommandView::InputRejected(InputRejectedView { rejection }));

    assert_eq!(text, "Input rejected: unknown command /unknown.\n");
    assert!(!text.contains("top-secret-password"));
    assert!(!text.contains(&digest));
}

#[test]
fn renderer_maps_startup_and_runtime_errors_to_single_safe_lines() {
    let startup_errors = [
        StartupError::StateDirectoryUnavailable,
        StartupError::EventStreamRecovery(RecoveryError::QueryFailed),
        StartupError::Persistence(PersistenceError::QueryFailed),
    ];
    for error in startup_errors {
        let mut bytes = Vec::new();
        TextRenderer::render_startup_error(error, &mut bytes).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.matches('\n').count(), 1);
        assert!(text.starts_with("Startup failed ["));
        assert!(!text.contains("sqlite"));
        assert!(!text.contains("/private/"));
    }

    let runtime_errors = [
        RuntimeError::Backpressure,
        RuntimeError::Closed,
        RuntimeError::Application(AppError::CapabilityDenied {
            capability: ai_stock_forum::policy::Capability::HelpRead,
            decision: ai_stock_forum::policy::PolicyDecision::DeniedByDefault,
        }),
        RuntimeError::WorkerStartup,
        RuntimeError::WorkerExited,
        RuntimeError::WorkerPanicked,
        RuntimeError::TerminationTimedOut,
    ];
    for error in runtime_errors {
        let mut bytes = Vec::new();
        TextRenderer::render_runtime_error(&error, &mut bytes).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.matches('\n').count(), 1);
        assert!(!text.contains("DeniedByDefault"));
        assert!(!text.contains("Persistence"));
    }
}

#[test]
fn scripted_fallback_session_renders_required_commands_and_quits() {
    let input = Cursor::new(b"/help\n/status\n/setup status\n/audit tail 3\n/quit\n".to_vec());
    let mut output = Vec::new();
    let runtime = support::runtime();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(input, &mut output)
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Available commands"));
    assert!(text.contains("Installation"));
    assert!(text.contains("Guided setup is not implemented"));
    assert!(text.contains("Audit tail"));
    assert!(text.contains("Shutting down"));
    runtime.finish_and_join(reason);
    assert_eq!(runtime.last_shutdown_reason().as_deref(), Some("user_quit"));
}

#[test]
fn eof_without_a_line_records_input_closed() {
    let runtime = support::runtime();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(Vec::<u8>::new()), Vec::new())
        .unwrap();
    runtime.finish_and_join(reason);
    assert_eq!(
        runtime.last_shutdown_reason().as_deref(),
        Some("input_closed")
    );
}

struct RecordingExecutor {
    finish: Sender<ShutdownReason>,
}

impl CommandExecutor for RecordingExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        Ok(outcome_for(command))
    }

    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        self.finish.send(reason).unwrap();
        Ok(())
    }
}

fn recording_runtime() -> (ApplicationRuntime, Receiver<ShutdownReason>) {
    let (finish_sender, finish_receiver) = bounded(1);
    let runtime = ApplicationRuntime::spawn(
        RecordingExecutor {
            finish: finish_sender,
        },
        4,
    )
    .unwrap();
    (runtime, finish_receiver)
}

#[derive(Clone, Default)]
struct SharedWriter(Arc<Mutex<Vec<u8>>>);

impl SharedWriter {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for SharedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "secret write detail",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "secret write detail",
        ))
    }
}

struct PanickingWriter;

impl Write for PanickingWriter {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        panic!("secret panic detail")
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn host_finishes_with_application_error_after_input_write_and_panic_failures() {
    let (runtime, finished) = recording_runtime();
    let result = FallbackHost::new(runtime, false, false).run(
        scripted_source(FailingReader),
        SharedWriter::default(),
        never(),
    );
    assert!(matches!(result, Err(UiError::Read)));
    assert_eq!(receive(&finished), ShutdownReason::ApplicationError);

    let (runtime, finished) = recording_runtime();
    let result = FallbackHost::new(runtime, false, false).run(
        scripted_source(Cursor::new(b"/help\n".to_vec())),
        FailingWriter,
        never(),
    );
    assert!(matches!(result, Err(UiError::Write)));
    assert_eq!(receive(&finished), ShutdownReason::ApplicationError);

    let (runtime, finished) = recording_runtime();
    let result = FallbackHost::new(runtime, false, false).run(
        scripted_source(Cursor::new(b"/help\n".to_vec())),
        PanickingWriter,
        never(),
    );
    assert!(matches!(result, Err(UiError::Panicked)));
    assert_eq!(receive(&finished), ShutdownReason::ApplicationError);
}

#[test]
fn caught_host_writer_panic_subprocess_redacts_payload_and_emits_one_safe_line() {
    const CHILD_ENV: &str = "AI_STOCK_FORUM_HOST_PANIC_CHILD";
    const SECRET: &str = "credential=host-writer-secret-payload";
    const SAFE_LINE: &str = "Command host stopped unexpectedly.\n";

    if std::env::var_os(CHILD_ENV).is_some() {
        struct SecretPanickingWriter;
        impl Write for SecretPanickingWriter {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                panic!("{SECRET}")
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let (runtime, _finished) = recording_runtime();
        let error = FallbackHost::new(runtime, false, false)
            .run(
                scripted_source(Cursor::new(b"/help\n".to_vec())),
                SecretPanickingWriter,
                never(),
            )
            .expect_err("writer panic must become a typed host error");
        TextRenderer::render_ui_error(&error, &mut io::stderr()).unwrap();
        std::process::exit(0);
    }

    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "caught_host_writer_panic_subprocess_redacts_payload_and_emits_one_safe_line",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr, SAFE_LINE);
    assert!(!stderr.contains(SECRET));
}

#[test]
fn host_finishes_quit_eof_and_interrupt_with_exact_reasons() {
    for (input, expected) in [
        (b"/quit\n".as_slice(), ShutdownReason::UserQuit),
        (b"".as_slice(), ShutdownReason::InputClosed),
    ] {
        let (runtime, finished) = recording_runtime();
        let reason = FallbackHost::new(runtime, false, false)
            .run(
                scripted_source(Cursor::new(input.to_vec())),
                SharedWriter::default(),
                never(),
            )
            .unwrap();
        assert_eq!(reason, expected);
        assert_eq!(receive(&finished), expected);
    }

    let (runtime, finished) = recording_runtime();
    let (interrupt_sender, interrupt_receiver) = bounded(1);
    interrupt_sender.send(()).unwrap();
    let reason = FallbackHost::new(runtime, false, false)
        .run(
            scripted_source(Cursor::new(b"/help\n".to_vec())),
            SharedWriter::default(),
            interrupt_receiver,
        )
        .unwrap();
    assert_eq!(reason, ShutdownReason::Interrupted);
    assert_eq!(receive(&finished), ShutdownReason::Interrupted);
}

struct GateExecutor {
    entered: Sender<()>,
    release: Receiver<()>,
    first: bool,
}

impl CommandExecutor for GateExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        if self.first {
            self.first = false;
            self.entered.send(()).unwrap();
            receive(&self.release);
        }
        Ok(outcome_for(command))
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

#[test]
fn runner_reports_backpressure_without_reordering_or_blocking_input() {
    let (entered_sender, entered_receiver) = bounded(1);
    let (release_sender, release_receiver) = bounded(1);
    let runtime = ApplicationRuntime::spawn(
        GateExecutor {
            entered: entered_sender,
            release: release_receiver,
            first: true,
        },
        1,
    )
    .unwrap();
    let first = runtime
        .client()
        .try_submit(ApplicationCommand::ShowHelp)
        .unwrap();
    receive(&entered_receiver);
    let second = runtime
        .client()
        .try_submit(ApplicationCommand::ShowStatus)
        .unwrap();

    let mut output = Vec::new();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(b"/help\n".to_vec()), &mut output)
        .unwrap();
    assert_eq!(reason, ShutdownReason::InputClosed);
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "Command queue is busy; try again.\n"
    );

    release_sender.send(()).unwrap();
    first.recv().unwrap();
    second.recv().unwrap();
    runtime.finish_and_join(reason).unwrap();
}

struct PanicExecutor;

impl CommandExecutor for PanicExecutor {
    fn execute_user(&mut self, _command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        panic!("worker secret detail")
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

#[test]
fn host_redacts_worker_failure_and_attempts_shutdown_without_deadlock() {
    let runtime = ApplicationRuntime::spawn(PanicExecutor, 1).unwrap();
    let writer = SharedWriter::default();
    let result = FallbackHost::new(runtime, false, false).run(
        scripted_source(Cursor::new(b"/help\n".to_vec())),
        writer.clone(),
        never(),
    );
    assert!(matches!(
        result,
        Err(UiError::Runtime(RuntimeError::WorkerPanicked))
    ));
    assert!(writer.text().is_empty());
}

fn binary_output(home: &Path, xdg_data: &Path, input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ai-stock-forum"))
        .env("HOME", home)
        .env("XDG_DATA_HOME", xdg_data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

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

#[cfg(unix)]
#[test]
fn binary_smoke_quit_and_eof_exit_successfully() {
    for input in [b"/quit\n".as_slice(), b"".as_slice()] {
        let temporary_directory = TempDir::new().unwrap();
        let home = temporary_directory.path().join("home");
        let xdg = temporary_directory.path().join("xdg");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&xdg).unwrap();
        let output = binary_output(&home, &xdg, input);
        assert!(
            output.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("sqlite"));
        let next = binary_output(&home, &xdg, b"/quit\n");
        assert!(next.status.success());
        assert!(
            !String::from_utf8_lossy(&next.stdout).contains("previous session ended unexpectedly")
        );
    }
}

#[cfg(unix)]
#[test]
fn binary_startup_failure_is_redacted_and_uses_failure_status() {
    let temporary_directory = TempDir::new().unwrap();
    let blocked = temporary_directory.path().join("private-secret-state-root");
    fs::write(&blocked, b"not a directory").unwrap();
    let output = binary_output(&blocked, &blocked, b"");
    let stderr = String::from_utf8(output.stderr).unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stderr.matches('\n').count(), 1);
    assert!(stderr.starts_with("Startup failed ["));
    assert!(!stderr.contains("private-secret-state-root"));
    assert!(!stderr.contains("not a directory"));
}

#[cfg(unix)]
#[test]
fn binary_prints_previous_session_warning_once_then_finishes_cleanly() {
    let temporary_directory = TempDir::new().unwrap();
    let home = temporary_directory.path().join("home");
    let xdg = temporary_directory.path().join("xdg");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&xdg).unwrap();
    let paths = AppPaths::for_test(state_path(&home, &xdg));
    let abandoned = ai_stock_forum::app::ApplicationService::bootstrap(
        &paths,
        Arc::new(support::TestClock::new()),
        Arc::new(support::TestIds::new()),
    )
    .unwrap();
    drop(abandoned);

    let output = binary_output(&home, &xdg, b"/quit\n");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout
            .matches("Warning: the previous session ended unexpectedly.")
            .count(),
        1
    );
}
