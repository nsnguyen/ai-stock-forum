//! Bounded command transport for the application worker.

use std::{
    io,
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{bounded, select_biased, unbounded, Receiver, Sender, TryRecvError, TrySendError};
use thiserror::Error;

use crate::app::{
    AppError, ApplicationCommand, ApplicationService, ApplicationWorker, CommandOutcome,
    ShutdownReason,
};

pub const MODULE_NAME: &str = "runtime";
pub const DEFAULT_QUEUE_CAPACITY: usize = 32;

pub trait CommandExecutor: Send + 'static {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError>;
    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError>;
}

impl CommandExecutor for ApplicationService {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        Self::execute_user(self, command)
    }

    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        Self::finish(self, reason)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeError {
    #[error("runtime queue capacity must be greater than zero")]
    InvalidCapacity,
    #[error("runtime command queue is full")]
    Backpressure,
    #[error("runtime is closed to new submissions")]
    Closed,
    #[error("application command failed: {0}")]
    Application(AppError),
    #[error("application worker could not be started")]
    WorkerStartup,
    #[error("application worker exited unexpectedly")]
    WorkerExited,
    #[error("application worker panicked")]
    WorkerPanicked,
    #[error("timed out waiting for application worker termination")]
    TerminationTimedOut,
}

pub trait RuntimeThreadSpawner: Send + Sync + 'static {
    fn spawn(&self, task: Box<dyn FnOnce() + Send + 'static>) -> io::Result<JoinHandle<()>>;
}

struct SystemThreadSpawner;

impl RuntimeThreadSpawner for SystemThreadSpawner {
    fn spawn(&self, task: Box<dyn FnOnce() + Send + 'static>) -> io::Result<JoinHandle<()>> {
        thread::Builder::new()
            .name("application-runtime".to_owned())
            .spawn(task)
    }
}

type CommandResult = Result<CommandOutcome, RuntimeError>;
type FinishResult = Result<(), RuntimeError>;

enum Request {
    Command {
        command: ApplicationCommand,
        response: Sender<CommandResult>,
    },
}

enum Control {
    Finish(ShutdownReason),
}

enum Admission {
    Open(Sender<Request>),
    Closing,
}

enum WorkerState {
    Running,
    Exited(FinishResult),
    Joined(FinishResult),
}

enum JoinState {
    Available(JoinHandle<()>),
    Joining,
    Joined,
}

struct SharedRuntime {
    admission: Mutex<Admission>,
    control: Sender<Control>,
    worker_state: Mutex<WorkerState>,
    worker_changed: Condvar,
}

impl SharedRuntime {
    fn close_admission(&self, reason: ShutdownReason) {
        let mut admission = match self.admission.lock() {
            Ok(admission) => admission,
            Err(_) => return self.publish_exited(Err(RuntimeError::WorkerPanicked)),
        };
        if matches!(*admission, Admission::Open(_)) {
            let _ = self.control.send(Control::Finish(reason));
            *admission = Admission::Closing;
        }
    }

    fn publish_exited(&self, result: FinishResult) {
        let Ok(mut state) = self.worker_state.lock() else {
            return;
        };
        if matches!(*state, WorkerState::Running) {
            *state = WorkerState::Exited(result);
            self.worker_changed.notify_all();
        }
    }

    fn publish_failure(&self, error: RuntimeError) {
        if let Ok(mut admission) = self.admission.lock() {
            if matches!(*admission, Admission::Open(_)) {
                *admission = Admission::Closing;
            }
        }
        self.publish_exited(Err(error));
    }

    fn publish_joined(&self, result: FinishResult) {
        let Ok(mut state) = self.worker_state.lock() else {
            return;
        };
        if !matches!(*state, WorkerState::Joined(_)) {
            *state = WorkerState::Joined(result);
            self.worker_changed.notify_all();
        }
    }

    fn terminal_error(&self) -> RuntimeError {
        let state = match self.worker_state.lock() {
            Ok(state) => state,
            Err(_) => return RuntimeError::WorkerPanicked,
        };
        match &*state {
            WorkerState::Exited(Err(error)) | WorkerState::Joined(Err(error)) => {
                error.clone()
            }
            WorkerState::Running | WorkerState::Exited(Ok(())) | WorkerState::Joined(Ok(())) => {
                RuntimeError::Closed
            }
        }
    }

    fn exited_result(&self) -> FinishResult {
        let state = match self.worker_state.lock() {
            Ok(state) => state,
            Err(_) => return Err(RuntimeError::WorkerPanicked),
        };
        match &*state {
            WorkerState::Exited(result) | WorkerState::Joined(result) => result.clone(),
            WorkerState::Running => Err(RuntimeError::WorkerExited),
        }
    }

    fn wait_for_joined(&self) -> FinishResult {
        let mut state = match self.worker_state.lock() {
            Ok(state) => state,
            Err(_) => return Err(RuntimeError::WorkerPanicked),
        };
        loop {
            if let WorkerState::Joined(result) = &*state {
                return result.clone();
            }
            state = match self.worker_changed.wait(state) {
                Ok(state) => state,
                Err(_) => return Err(RuntimeError::WorkerPanicked),
            };
        }
    }

    fn wait_for_joined_timeout(&self, timeout: Duration) -> FinishResult {
        let state = match self.worker_state.lock() {
            Ok(state) => state,
            Err(_) => return Err(RuntimeError::WorkerPanicked),
        };
        let (state, timed_out) = match self.worker_changed.wait_timeout_while(state, timeout, |state| {
            !matches!(*state, WorkerState::Joined(_))
        }) {
            Ok(result) => result,
            Err(_) => return Err(RuntimeError::WorkerPanicked),
        };
        if timed_out.timed_out() {
            Err(RuntimeError::TerminationTimedOut)
        } else if let WorkerState::Joined(result) = &*state {
            result.clone()
        } else {
            Err(RuntimeError::TerminationTimedOut)
        }
    }
}

#[derive(Clone)]
pub struct RuntimeClient {
    shared: Arc<SharedRuntime>,
}

impl RuntimeClient {
    pub fn submit(&self, command: ApplicationCommand) -> Result<CommandOutcome, RuntimeError> {
        self.try_submit(command)?.recv()
    }

    pub fn try_submit(&self, command: ApplicationCommand) -> Result<PendingOutcome, RuntimeError> {
        let (response_sender, response) = bounded(1);
        let attempt = {
            let admission = self
                .shared
                .admission
                .lock()
                .map_err(|_| RuntimeError::WorkerPanicked)?;
            match &*admission {
                Admission::Closing => Err(RuntimeError::Closed),
                Admission::Open(sender) => match sender.try_send(Request::Command {
                    command,
                    response: response_sender,
                }) {
                    Ok(()) => Ok(()),
                    Err(TrySendError::Full(_)) => Err(RuntimeError::Backpressure),
                    Err(TrySendError::Disconnected(_)) => Err(RuntimeError::WorkerExited),
                },
            }
        };
        match attempt {
            Ok(()) => Ok(PendingOutcome {
                response,
                shared: self.shared.clone(),
            }),
            Err(RuntimeError::Closed) | Err(RuntimeError::WorkerExited) => {
                Err(self.shared.terminal_error())
            }
            Err(error) => Err(error),
        }
    }

    pub fn wait_for_termination(&self, timeout: Duration) -> FinishResult {
        self.shared.wait_for_joined_timeout(timeout)
    }
}

pub struct PendingOutcome {
    response: Receiver<CommandResult>,
    shared: Arc<SharedRuntime>,
}

impl PendingOutcome {
    pub fn recv(self) -> Result<CommandOutcome, RuntimeError> {
        self.response
            .recv()
            .unwrap_or_else(|_| Err(self.shared.terminal_error()))
    }
}

pub struct ApplicationRuntime {
    shared: Arc<SharedRuntime>,
    join: Mutex<JoinState>,
}

impl ApplicationRuntime {
    pub fn spawn(executor: impl CommandExecutor, capacity: usize) -> Result<Self, RuntimeError> {
        Self::spawn_with_initializer(capacity, Arc::new(SystemThreadSpawner), move || {
            Ok(Box::new(executor))
        })
    }

    pub fn spawn_application(
        service: ApplicationService,
        capacity: usize,
    ) -> Result<Self, RuntimeError> {
        Self::spawn_with_initializer(capacity, Arc::new(SystemThreadSpawner), move || {
            let worker = service.worker().map_err(|_| RuntimeError::WorkerStartup)?;
            Ok(Box::new(ServiceWorker { service, worker }))
        })
    }

    #[doc(hidden)]
    pub fn spawn_with_thread_spawner(
        executor: impl CommandExecutor,
        capacity: usize,
        spawner: Arc<dyn RuntimeThreadSpawner>,
    ) -> Result<Self, RuntimeError> {
        Self::spawn_with_initializer(capacity, spawner, move || Ok(Box::new(executor)))
    }

    pub fn client(&self) -> RuntimeClient {
        RuntimeClient {
            shared: self.shared.clone(),
        }
    }

    pub fn finish_and_join(&self, reason: ShutdownReason) -> FinishResult {
        self.shared.close_admission(reason);
        self.join_worker()
    }

    fn spawn_with_initializer(
        capacity: usize,
        spawner: Arc<dyn RuntimeThreadSpawner>,
        initializer: impl FnOnce() -> Result<Box<dyn CommandExecutor>, RuntimeError> + Send + 'static,
    ) -> Result<Self, RuntimeError> {
        if capacity == 0 {
            return Err(RuntimeError::InvalidCapacity);
        }
        let (request_sender, request_receiver) = bounded(capacity);
        let (control_sender, control_receiver) = unbounded();
        let (initialized_sender, initialized_receiver) = bounded(1);
        let shared = Arc::new(SharedRuntime {
            admission: Mutex::new(Admission::Open(request_sender)),
            control: control_sender,
            worker_state: Mutex::new(WorkerState::Running),
            worker_changed: Condvar::new(),
        });
        let worker_shared = shared.clone();
        let task = Box::new(move || {
            worker_entry(
                request_receiver,
                control_receiver,
                initializer,
                initialized_sender,
                worker_shared,
            )
        });
        let worker = spawner.spawn(task).map_err(|_| RuntimeError::WorkerStartup)?;
        let runtime = Self {
            shared,
            join: Mutex::new(JoinState::Available(worker)),
        };
        match initialized_receiver.recv() {
            Ok(Ok(())) => Ok(runtime),
            Ok(Err(error)) => {
                let _ = runtime.join_worker();
                Err(error)
            }
            Err(_) => {
                let error = runtime.join_worker().err().unwrap_or(RuntimeError::WorkerExited);
                Err(error)
            }
        }
    }

    fn join_worker(&self) -> FinishResult {
        let worker = {
            let mut join = match self.join.lock() {
                Ok(join) => join,
                Err(_) => return Err(RuntimeError::WorkerPanicked),
            };
            match std::mem::replace(&mut *join, JoinState::Joining) {
                JoinState::Available(worker) => Some(worker),
                JoinState::Joining => None,
                JoinState::Joined => {
                    *join = JoinState::Joined;
                    None
                }
            }
        };
        if let Some(worker) = worker {
            let result = match worker.join() {
                Ok(()) => self.shared.exited_result(),
                Err(_) => Err(RuntimeError::WorkerPanicked),
            };
            self.shared.publish_joined(result.clone());
            if let Ok(mut join) = self.join.lock() {
                *join = JoinState::Joined;
            }
            result
        } else {
            self.shared.wait_for_joined()
        }
    }

    fn reap_on_drop(&self) {
        let worker = {
            let mut join = match self.join.lock() {
                Ok(join) => join,
                Err(_) => return,
            };
            match std::mem::replace(&mut *join, JoinState::Joining) {
                JoinState::Available(worker) => Some(worker),
                JoinState::Joining | JoinState::Joined => None,
            }
        };
        let Some(worker) = worker else {
            return;
        };
        let shared = self.shared.clone();
        let worker_slot = Arc::new(Mutex::new(Some(worker)));
        let reaper_slot = worker_slot.clone();
        let reap = move || {
            let result = match reaper_slot.lock().ok().and_then(|mut slot| slot.take()) {
                Some(worker) => match worker.join() {
                    Ok(()) => shared.exited_result(),
                    Err(_) => Err(RuntimeError::WorkerPanicked),
                },
                None => Err(RuntimeError::WorkerExited),
            };
            shared.publish_joined(result);
        };
        if thread::Builder::new()
            .name("application-runtime-reaper".to_owned())
            .spawn(reap)
            .is_err()
        {
            let result = match worker_slot.lock().ok().and_then(|mut slot| slot.take()) {
                Some(worker) => match worker.join() {
                    Ok(()) => self.shared.exited_result(),
                    Err(_) => Err(RuntimeError::WorkerPanicked),
                },
                None => Err(RuntimeError::WorkerExited),
            };
            self.shared.publish_joined(result);
        }
    }
}

impl Drop for ApplicationRuntime {
    fn drop(&mut self) {
        self.shared.close_admission(ShutdownReason::ApplicationError);
        self.reap_on_drop();
    }
}

struct ServiceWorker {
    service: ApplicationService,
    worker: ApplicationWorker,
}

impl CommandExecutor for ServiceWorker {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        self.worker.execute_user(command)
    }

    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        self.service.finish(reason)
    }
}

fn worker_entry(
    requests: Receiver<Request>,
    control: Receiver<Control>,
    initializer: impl FnOnce() -> Result<Box<dyn CommandExecutor>, RuntimeError>,
    initialized: Sender<FinishResult>,
    shared: Arc<SharedRuntime>,
) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut executor = match initializer() {
            Ok(executor) => executor,
            Err(error) => {
                let _ = initialized.send(Err(error.clone()));
                shared.publish_failure(error);
                return;
            }
        };
        let _ = initialized.send(Ok(()));
        worker_loop(&mut *executor, requests, control, &shared);
    }));
    if let Err(payload) = result {
        shared.publish_failure(RuntimeError::WorkerPanicked);
        let _ = initialized.send(Err(RuntimeError::WorkerPanicked));
        resume_unwind(payload);
    }
}

fn worker_loop(
    executor: &mut dyn CommandExecutor,
    requests: Receiver<Request>,
    control: Receiver<Control>,
    shared: &SharedRuntime,
) {
    loop {
        select_biased! {
            recv(control) -> control => match control {
                Ok(Control::Finish(reason)) => {
                    drain_requests(executor, &requests, shared);
                    let result = executor.finish(reason).map_err(RuntimeError::Application);
                    shared.publish_exited(result);
                    return;
                }
                Err(_) => {
                    shared.publish_exited(Err(RuntimeError::WorkerExited));
                    return;
                }
            },
            recv(requests) -> request => match request {
                Ok(request) => execute_request(executor, request, shared),
                Err(_) => {
                    shared.publish_exited(Err(RuntimeError::WorkerExited));
                    return;
                }
            },
        }
    }
}

fn drain_requests(
    executor: &mut dyn CommandExecutor,
    requests: &Receiver<Request>,
    shared: &SharedRuntime,
) {
    loop {
        match requests.try_recv() {
            Ok(request) => execute_request(executor, request, shared),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return,
        }
    }
}

fn execute_request(executor: &mut dyn CommandExecutor, request: Request, shared: &SharedRuntime) {
    let Request::Command { command, response } = request;
    match catch_unwind(AssertUnwindSafe(|| executor.execute_user(command))) {
        Ok(result) => {
            let _ = response.send(result.map_err(RuntimeError::Application));
        }
        Err(payload) => {
            shared.publish_failure(RuntimeError::WorkerPanicked);
            let _ = response.send(Err(RuntimeError::WorkerPanicked));
            resume_unwind(payload);
        }
    }
}
