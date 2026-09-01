mod support;

use std::{
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use ai_stock_forum::{
    app::{
        AppError, ApplicationCommand, ApplicationService, CommandOutcome, CommandView,
        ShutdownReason,
    },
    config::AppPaths,
    domain::IdGenerator,
    runtime::{ApplicationRuntime, CommandExecutor, RuntimeError, RuntimeThreadSpawner},
};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender};
use tempfile::TempDir;

const TIMEOUT: Duration = Duration::from_secs(5);

fn receive<T>(receiver: &Receiver<T>) -> T {
    receiver
        .recv_timeout(TIMEOUT)
        .expect("timed out waiting for test handshake")
}

fn application_service() -> (TempDir, AppPaths, ApplicationService) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(directory.path());
    let clock = Arc::new(support::TestClock::new());
    let ids = Arc::new(support::TestIds::new());
    let service = ApplicationService::bootstrap(&paths, clock, ids).unwrap();
    (directory, paths, service)
}

struct GateExecutor {
    first_entered: Sender<()>,
    first_release: Receiver<()>,
    seen: Arc<Mutex<Vec<ApplicationCommand>>>,
    finishes: Arc<AtomicUsize>,
}

impl CommandExecutor for GateExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        let first = {
            let mut seen = self.seen.lock().unwrap();
            seen.push(command);
            seen.len() == 1
        };
        if first {
            self.first_entered.send(()).unwrap();
            self.first_release.recv_timeout(TIMEOUT).unwrap();
        }
        Err(AppError::LifecycleFinished)
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        self.finishes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct PanicAfterGateExecutor {
    first_entered: Sender<()>,
    first_release: Receiver<()>,
}

impl CommandExecutor for PanicAfterGateExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        if command == ApplicationCommand::ShowHelp {
            self.first_entered.send(()).unwrap();
            self.first_release.recv_timeout(TIMEOUT).unwrap();
            Err(AppError::LifecycleFinished)
        } else {
            panic!("injected worker panic");
        }
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

struct FinishGateExecutor {
    finish_entered: Sender<()>,
    finish_release: Receiver<()>,
    finishes: Arc<AtomicUsize>,
}

impl CommandExecutor for FinishGateExecutor {
    fn execute_user(&mut self, _command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        Err(AppError::LifecycleFinished)
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        self.finishes.fetch_add(1, Ordering::SeqCst);
        self.finish_entered.send(()).unwrap();
        self.finish_release.recv_timeout(TIMEOUT).unwrap();
        Ok(())
    }
}

struct RecordingExecutor {
    seen: Arc<Mutex<Vec<ApplicationCommand>>>,
    finishes: Arc<AtomicUsize>,
}

impl CommandExecutor for RecordingExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        self.seen.lock().unwrap().push(command);
        Err(AppError::LifecycleFinished)
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        self.finishes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct FailingSpawner;

impl RuntimeThreadSpawner for FailingSpawner {
    fn spawn(&self, _task: Box<dyn FnOnce() + Send + 'static>) -> io::Result<thread::JoinHandle<()>> {
        Err(io::Error::other("injected thread creation failure"))
    }
}

#[test]
fn generic_runtime_spawn_executes_commands_and_preserves_response_correlation() {
    let (_directory, _paths, service) = application_service();
    let runtime = ApplicationRuntime::spawn_application(service, 32).unwrap();
    let client = runtime.client();

    let help = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    let status = client.try_submit(ApplicationCommand::ShowStatus).unwrap();
    let help = help.recv().unwrap();
    let status = status.recv().unwrap();

    assert!(matches!(help.view, CommandView::Help(_)));
    assert!(matches!(status.view, CommandView::Status(_)));
    assert_ne!(help.command_id, status.command_id);
    assert_ne!(help.correlation_id, status.correlation_id);
    runtime.finish_and_join(ShutdownReason::InputClosed).unwrap();
}

#[test]
fn capacity_one_accepts_the_second_queued_command_and_rejects_the_third() {
    let (entered_sender, entered) = bounded(1);
    let (release_sender, release) = bounded(1);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = ApplicationRuntime::spawn(
        GateExecutor {
            first_entered: entered_sender,
            first_release: release,
            seen: seen.clone(),
            finishes: finishes.clone(),
        },
        1,
    )
    .unwrap();
    let client = runtime.client();

    let first = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    receive(&entered);
    let second = client.try_submit(ApplicationCommand::ShowStatus).unwrap();
    assert!(matches!(
        client.try_submit(ApplicationCommand::ShowSetupStatus),
        Err(RuntimeError::Backpressure)
    ));

    release_sender.send(()).unwrap();
    assert_eq!(
        first.recv(),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    assert_eq!(
        second.recv(),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    runtime.finish_and_join(ShutdownReason::InputClosed).unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![ApplicationCommand::ShowHelp, ApplicationCommand::ShowStatus]
    );
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[test]
fn full_queue_worker_panic_and_shutdown_complete_without_deadlock() {
    let (entered_sender, entered) = bounded(1);
    let (release_sender, release) = bounded(1);
    let runtime = Arc::new(
        ApplicationRuntime::spawn(
            PanicAfterGateExecutor {
                first_entered: entered_sender,
                first_release: release,
            },
            1,
        )
        .unwrap(),
    );
    let client = runtime.client();
    let first = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    receive(&entered);
    let second = client.try_submit(ApplicationCommand::ShowStatus).unwrap();
    assert!(matches!(
        client.try_submit(ApplicationCommand::ShowSetupStatus),
        Err(RuntimeError::Backpressure)
    ));
    let (shutdown_sender, shutdown) = bounded(1);
    let shutdown_runtime = runtime.clone();
    thread::spawn(move || {
        shutdown_sender
            .send(shutdown_runtime.finish_and_join(ShutdownReason::ApplicationError))
            .unwrap();
    });

    release_sender.send(()).unwrap();
    assert_eq!(
        first.recv(),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    assert_eq!(second.recv(), Err(RuntimeError::WorkerPanicked));
    assert_eq!(receive(&shutdown), Err(RuntimeError::WorkerPanicked));
}

#[test]
fn post_failure_submissions_return_the_stored_terminal_error() {
    let (entered_sender, _entered) = unbounded();
    let (_release_sender, release) = unbounded();
    let runtime = ApplicationRuntime::spawn(
        PanicAfterGateExecutor {
            first_entered: entered_sender,
            first_release: release,
        },
        1,
    )
    .unwrap();
    let client = runtime.client();
    let pending = client.try_submit(ApplicationCommand::ShowStatus).unwrap();

    assert_eq!(pending.recv(), Err(RuntimeError::WorkerPanicked));
    assert!(matches!(
        client.try_submit(ApplicationCommand::ShowHelp),
        Err(RuntimeError::WorkerPanicked)
    ));
    assert_eq!(
        runtime.finish_and_join(ShutdownReason::ApplicationError),
        Err(RuntimeError::WorkerPanicked)
    );
}

#[test]
fn shutdown_waiters_do_not_finish_early_while_finish_is_blocked() {
    let (entered_sender, entered) = bounded(1);
    let (release_sender, release) = bounded(1);
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(
        ApplicationRuntime::spawn(
            FinishGateExecutor {
                finish_entered: entered_sender,
                finish_release: release,
                finishes: finishes.clone(),
            },
            1,
        )
        .unwrap(),
    );
    let (first_sender, first) = bounded(1);
    let first_runtime = runtime.clone();
    thread::spawn(move || {
        first_sender
            .send(first_runtime.finish_and_join(ShutdownReason::InputClosed))
            .unwrap();
    });
    receive(&entered);
    let (second_sender, second) = bounded(1);
    let second_runtime = runtime.clone();
    thread::spawn(move || {
        second_sender
            .send(second_runtime.finish_and_join(ShutdownReason::InputClosed))
            .unwrap();
    });

    runtime.wait_for_join_owner(TIMEOUT).unwrap();
    runtime.wait_for_join_waiter(TIMEOUT).unwrap();
    release_sender.send(()).unwrap();
    assert_eq!(receive(&first), Ok(()));
    assert_eq!(receive(&second), Ok(()));
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[test]
fn owner_drop_closes_surviving_clients_and_releases_the_application_guard() {
    let (directory, paths, service) = application_service();
    let runtime = ApplicationRuntime::spawn_application(service, 1).unwrap();
    let client = runtime.client();

    drop(runtime);
    client.wait_for_termination(TIMEOUT).unwrap();
    assert!(matches!(
        client.try_submit(ApplicationCommand::ShowHelp),
        Err(RuntimeError::Closed)
    ));
    let replacement_ids = Arc::new(support::TestIds::new());
    for _ in 0..32 {
        replacement_ids.next_uuid();
    }
    ApplicationService::bootstrap(
        &paths,
        Arc::new(support::TestClock::new()),
        replacement_ids,
    )
    .unwrap();
    drop(directory);
}

#[test]
fn submit_and_shutdown_race_accepts_at_most_one_fifo_command() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(
        ApplicationRuntime::spawn(
            RecordingExecutor {
                seen: seen.clone(),
                finishes: finishes.clone(),
            },
            1,
        )
        .unwrap(),
    );
    let client = runtime.client();
    let (go_sender, go) = bounded(2);
    let (submitted_sender, submitted) = bounded(1);
    let submit_client = client.clone();
    let submit_go = go.clone();
    thread::spawn(move || {
        receive(&submit_go);
        submitted_sender
            .send(submit_client.try_submit(ApplicationCommand::ShowHelp).map(|_| ()))
            .unwrap();
    });
    let (shutdown_sender, shutdown) = bounded(1);
    let shutdown_runtime = runtime.clone();
    let shutdown_go = go.clone();
    thread::spawn(move || {
        receive(&shutdown_go);
        shutdown_sender
            .send(shutdown_runtime.finish_and_join(ShutdownReason::InputClosed))
            .unwrap();
    });

    go_sender.send(()).unwrap();
    go_sender.send(()).unwrap();
    let admission = receive(&submitted);
    assert!(matches!(admission, Ok(()) | Err(RuntimeError::Closed)));
    assert_eq!(receive(&shutdown), Ok(()));
    let observed = seen.lock().unwrap().clone();
    assert!(observed.is_empty() || observed == vec![ApplicationCommand::ShowHelp]);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[test]
fn dropped_outcomes_do_not_drop_accepted_work() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = ApplicationRuntime::spawn(
        RecordingExecutor {
            seen: seen.clone(),
            finishes: finishes.clone(),
        },
        1,
    )
    .unwrap();

    drop(runtime.client().try_submit(ApplicationCommand::ShowHelp).unwrap());
    runtime.finish_and_join(ShutdownReason::InputClosed).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![ApplicationCommand::ShowHelp]);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[test]
fn thread_creation_failure_maps_to_worker_startup() {
    assert!(matches!(
        ApplicationRuntime::spawn_with_thread_spawner(
            RecordingExecutor {
                seen: Arc::new(Mutex::new(Vec::new())),
                finishes: Arc::new(AtomicUsize::new(0)),
            },
            1,
            Arc::new(FailingSpawner),
        ),
        Err(RuntimeError::WorkerStartup)
    ));
}

#[test]
fn submit_blocks_for_capacity_then_returns_its_own_typed_outcome() {
    let (entered_sender, entered) = bounded(1);
    let (release_sender, release) = bounded(1);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(
        ApplicationRuntime::spawn(
            GateExecutor {
                first_entered: entered_sender,
                first_release: release,
                seen: seen.clone(),
                finishes: finishes.clone(),
            },
            1,
        )
        .unwrap(),
    );
    let client = runtime.client();
    let first = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    receive(&entered);
    let second = client.try_submit(ApplicationCommand::ShowStatus).unwrap();
    let (started_sender, started) = bounded(1);
    let (submitted_sender, submitted) = bounded(1);
    let submit_client = client.clone();
    thread::spawn(move || {
        started_sender.send(()).unwrap();
        submitted_sender
            .send(submit_client.submit(ApplicationCommand::ShowSetupStatus))
            .unwrap();
    });
    receive(&started);
    runtime.wait_for_reservations(1, TIMEOUT).unwrap();

    release_sender.send(()).unwrap();
    assert_eq!(
        first.recv(),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    assert_eq!(
        second.recv(),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    assert_eq!(
        receive(&submitted),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    runtime.finish_and_join(ShutdownReason::InputClosed).unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            ApplicationCommand::ShowHelp,
            ApplicationCommand::ShowStatus,
            ApplicationCommand::ShowSetupStatus,
        ]
    );
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[test]
fn shutdown_drains_a_blocked_submit_reservation_before_finishing() {
    let (entered_sender, entered) = bounded(1);
    let (release_sender, release) = bounded(1);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(
        ApplicationRuntime::spawn(
            GateExecutor {
                first_entered: entered_sender,
                first_release: release,
                seen: seen.clone(),
                finishes: finishes.clone(),
            },
            1,
        )
        .unwrap(),
    );
    let client = runtime.client();
    let first = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    receive(&entered);
    let second = client.try_submit(ApplicationCommand::ShowStatus).unwrap();
    let (submitted_sender, submitted) = bounded(1);
    let submit_client = client.clone();
    thread::spawn(move || {
        submitted_sender
            .send(submit_client.submit(ApplicationCommand::ShowSetupStatus))
            .unwrap();
    });
    runtime.wait_for_reservations(1, TIMEOUT).unwrap();
    let (shutdown_sender, shutdown) = bounded(1);
    let shutdown_runtime = runtime.clone();
    thread::spawn(move || {
        shutdown_sender
            .send(shutdown_runtime.finish_and_join(ShutdownReason::InputClosed))
            .unwrap();
    });

    release_sender.send(()).unwrap();
    assert_eq!(
        first.recv(),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    assert_eq!(
        second.recv(),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    assert_eq!(
        receive(&submitted),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    assert_eq!(receive(&shutdown), Ok(()));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            ApplicationCommand::ShowHelp,
            ApplicationCommand::ShowStatus,
            ApplicationCommand::ShowSetupStatus,
        ]
    );
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[test]
fn a_blocked_submit_observes_the_stored_failure_when_the_worker_panics() {
    let (entered_sender, entered) = bounded(1);
    let (release_sender, release) = bounded(1);
    let runtime = Arc::new(
        ApplicationRuntime::spawn(
            PanicAfterGateExecutor {
                first_entered: entered_sender,
                first_release: release,
            },
            1,
        )
        .unwrap(),
    );
    let client = runtime.client();
    let first = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    receive(&entered);
    let panic = client.try_submit(ApplicationCommand::ShowStatus).unwrap();
    let (submitted_sender, submitted) = bounded(1);
    let submit_client = client.clone();
    thread::spawn(move || {
        submitted_sender
            .send(submit_client.submit(ApplicationCommand::ShowSetupStatus))
            .unwrap();
    });
    runtime.wait_for_reservations(1, TIMEOUT).unwrap();

    release_sender.send(()).unwrap();
    assert_eq!(
        first.recv(),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    assert_eq!(panic.recv(), Err(RuntimeError::WorkerPanicked));
    assert_eq!(receive(&submitted), Err(RuntimeError::WorkerPanicked));
    assert_eq!(
        runtime.finish_and_join(ShutdownReason::ApplicationError),
        Err(RuntimeError::WorkerPanicked)
    );
}

#[test]
fn forced_submit_shutdown_orders_complete_or_reject_without_lost_work() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = ApplicationRuntime::spawn(
        RecordingExecutor {
            seen: seen.clone(),
            finishes: finishes.clone(),
        },
        1,
    )
    .unwrap();
    let pending = runtime.client().try_submit(ApplicationCommand::ShowHelp).unwrap();
    runtime.finish_and_join(ShutdownReason::InputClosed).unwrap();
    assert_eq!(
        pending.recv(),
        Err(RuntimeError::Application(AppError::LifecycleFinished))
    );
    assert_eq!(*seen.lock().unwrap(), vec![ApplicationCommand::ShowHelp]);
    assert_eq!(finishes.load(Ordering::SeqCst), 1);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = ApplicationRuntime::spawn(
        RecordingExecutor {
            seen: seen.clone(),
            finishes: finishes.clone(),
        },
        1,
    )
    .unwrap();
    runtime.finish_and_join(ShutdownReason::InputClosed).unwrap();
    assert!(matches!(
        runtime.client().try_submit(ApplicationCommand::ShowHelp),
        Err(RuntimeError::Closed)
    ));
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}
