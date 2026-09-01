mod support;

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier, Mutex,
    },
    thread,
};

use ai_stock_forum::{
    app::{AppError, ApplicationCommand, ApplicationService, CommandOutcome, CommandView, ShutdownReason},
    config::AppPaths,
    runtime::{ApplicationRuntime, CommandExecutor, RuntimeError},
};
use tempfile::TempDir;

fn application_service() -> (TempDir, ApplicationService) {
    let directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(directory.path());
    let clock = Arc::new(support::TestClock::new());
    let ids = Arc::new(support::TestIds::new());
    let service = ApplicationService::bootstrap(&paths, clock, ids).unwrap();
    (directory, service)
}

struct BlockingExecutor {
    entered_first: Arc<Barrier>,
    release_first: Arc<Barrier>,
    seen: Arc<Mutex<Vec<ApplicationCommand>>>,
    finishes: Arc<AtomicUsize>,
}

impl CommandExecutor for BlockingExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        let is_first = {
            let mut seen = self.seen.lock().unwrap();
            seen.push(command);
            seen.len() == 1
        };
        if is_first {
            self.entered_first.wait();
            self.release_first.wait();
        }
        Err(AppError::LifecycleFinished)
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        self.finishes.fetch_add(1, Ordering::SeqCst);
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

struct PanicExecutor;

impl CommandExecutor for PanicExecutor {
    fn execute_user(&mut self, _command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        panic!("injected worker panic");
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

#[test]
fn application_commands_execute_on_the_worker_and_keep_response_correlation() {
    let (_directory, service) = application_service();
    let runtime = ApplicationRuntime::spawn(service, 32).unwrap();
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
fn capacity_one_reports_backpressure_after_the_second_command_is_queued() {
    let entered_first = Arc::new(Barrier::new(2));
    let release_first = Arc::new(Barrier::new(2));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = ApplicationRuntime::spawn_executor(
        BlockingExecutor {
            entered_first: entered_first.clone(),
            release_first: release_first.clone(),
            seen: seen.clone(),
            finishes: finishes.clone(),
        },
        1,
    )
    .unwrap();
    let client = runtime.client();

    let first = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    entered_first.wait();
    let second = client.try_submit(ApplicationCommand::ShowStatus).unwrap();
    assert!(matches!(
        client.try_submit(ApplicationCommand::ShowSetupStatus),
        Err(RuntimeError::Backpressure)
    ));

    release_first.wait();
    assert_eq!(first.recv(), Err(RuntimeError::Application(AppError::LifecycleFinished)));
    assert_eq!(second.recv(), Err(RuntimeError::Application(AppError::LifecycleFinished)));
    runtime.finish_and_join(ShutdownReason::InputClosed).unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        vec![ApplicationCommand::ShowHelp, ApplicationCommand::ShowStatus]
    );
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[test]
fn shutdown_drains_accepted_work_then_rejects_new_submissions() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = ApplicationRuntime::spawn_executor(
        RecordingExecutor {
            seen: seen.clone(),
            finishes: finishes.clone(),
        },
        2,
    )
    .unwrap();
    let client = runtime.client();

    let first = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    let second = client.try_submit(ApplicationCommand::ShowStatus).unwrap();
    runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();

    assert_eq!(first.recv(), Err(RuntimeError::Application(AppError::LifecycleFinished)));
    assert_eq!(second.recv(), Err(RuntimeError::Application(AppError::LifecycleFinished)));
    assert!(matches!(
        client.try_submit(ApplicationCommand::ShowSetupStatus),
        Err(RuntimeError::Closed)
    ));
    assert_eq!(
        *seen.lock().unwrap(),
        vec![ApplicationCommand::ShowHelp, ApplicationCommand::ShowStatus]
    );
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_a_caller_response_does_not_drop_accepted_work() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = ApplicationRuntime::spawn_executor(
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
fn a_worker_panic_is_reported_as_a_typed_failure() {
    let runtime = ApplicationRuntime::spawn_executor(PanicExecutor, 1).unwrap();
    let pending = runtime.client().try_submit(ApplicationCommand::ShowHelp).unwrap();

    assert_eq!(pending.recv(), Err(RuntimeError::WorkerPanicked));
    assert_eq!(
        runtime.finish_and_join(ShutdownReason::ApplicationError),
        Err(RuntimeError::WorkerPanicked)
    );
}

#[test]
fn concurrent_shutdown_calls_finish_the_executor_once() {
    let finishes = Arc::new(AtomicUsize::new(0));
    let runtime = Arc::new(
        ApplicationRuntime::spawn_executor(
            RecordingExecutor {
                seen: Arc::new(Mutex::new(Vec::new())),
                finishes: finishes.clone(),
            },
            1,
        )
        .unwrap(),
    );
    let start = Arc::new(Barrier::new(3));
    let left_runtime = runtime.clone();
    let left_start = start.clone();
    let left = thread::spawn(move || {
        left_start.wait();
        left_runtime.finish_and_join(ShutdownReason::InputClosed)
    });
    let right_runtime = runtime.clone();
    let right_start = start.clone();
    let right = thread::spawn(move || {
        right_start.wait();
        right_runtime.finish_and_join(ShutdownReason::InputClosed)
    });

    start.wait();
    assert_eq!(left.join().unwrap(), Ok(()));
    assert_eq!(right.join().unwrap(), Ok(()));
    assert_eq!(finishes.load(Ordering::SeqCst), 1);
}
