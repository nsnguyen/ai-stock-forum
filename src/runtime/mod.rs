//! Bounded command transport for the application worker.

use std::{
    io,
    panic::{AssertUnwindSafe, resume_unwind},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, select_biased, unbounded};
use thiserror::Error;

use crate::app::{
    AppError, ApplicationCommand, ApplicationService, ApplicationWorker, CommandOutcome,
    ShutdownReason,
};
use crate::panic_boundary::catch_sensitive_unwind;

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

struct State {
    admission: Admission,
    reservations: usize,
    worker: WorkerState,
    join_owner: bool,
    join_waiters: usize,
}

enum JoinState {
    Available(JoinHandle<()>),
    Joining,
    Joined,
}

struct SharedRuntime {
    state: Mutex<State>,
    changed: Condvar,
    control: Sender<Control>,
}

impl SharedRuntime {
    fn terminal_error(state: &State) -> RuntimeError {
        match &state.worker {
            WorkerState::Exited(Err(error)) | WorkerState::Joined(Err(error)) => error.clone(),
            WorkerState::Running | WorkerState::Exited(Ok(())) | WorkerState::Joined(Ok(())) => {
                RuntimeError::Closed
            }
        }
    }

    fn close_admission(&self, reason: ShutdownReason) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        if matches!(state.admission, Admission::Open(_)) {
            let _ = self.control.send(Control::Finish(reason));
            state.admission = Admission::Closing;
            self.changed.notify_all();
        }
    }

    fn reserve(&self) -> Result<Sender<Request>, RuntimeError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RuntimeError::WorkerPanicked)?;
        if !matches!(state.worker, WorkerState::Running) {
            return Err(Self::terminal_error(&state));
        }
        let sender = match &state.admission {
            Admission::Closing => return Err(RuntimeError::Closed),
            Admission::Open(sender) => sender.clone(),
        };
        state.reservations += 1;
        self.changed.notify_all();
        Ok(sender)
    }

    fn resolve_reservation(&self, accepted: bool) -> Result<(), RuntimeError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| RuntimeError::WorkerPanicked)?;
        state.reservations = state.reservations.saturating_sub(1);
        self.changed.notify_all();
        if accepted {
            Ok(())
        } else {
            Err(Self::terminal_error(&state))
        }
    }

    fn publish_exited(&self, result: FinishResult) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if matches!(state.worker, WorkerState::Running) {
            state.admission = Admission::Closing;
            state.worker = WorkerState::Exited(result);
            self.changed.notify_all();
        }
    }

    fn exited_result(&self) -> FinishResult {
        match self.state.lock() {
            Ok(state) => match &state.worker {
                WorkerState::Exited(result) | WorkerState::Joined(result) => result.clone(),
                WorkerState::Running => Err(RuntimeError::WorkerExited),
            },
            Err(_) => Err(RuntimeError::WorkerPanicked),
        }
    }

    fn publish_joined(&self, result: FinishResult) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if !matches!(state.worker, WorkerState::Joined(_)) {
            state.worker = WorkerState::Joined(result);
            self.changed.notify_all();
        }
    }

    fn wait_for_joined(&self, waiter: bool) -> FinishResult {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return Err(RuntimeError::WorkerPanicked),
        };
        if waiter {
            state.join_waiters += 1;
            self.changed.notify_all();
        }
        loop {
            if let WorkerState::Joined(result) = &state.worker {
                let result = result.clone();
                if waiter {
                    state.join_waiters -= 1;
                    self.changed.notify_all();
                }
                return result;
            }
            state = match self.changed.wait(state) {
                Ok(state) => state,
                Err(_) => return Err(RuntimeError::WorkerPanicked),
            };
        }
    }

    fn wait_for_joined_timeout(&self, timeout: Duration) -> FinishResult {
        self.wait_timeout(timeout, |state| {
            matches!(state.worker, WorkerState::Joined(_))
        })
    }

    fn wait_for_reservations(&self, count: usize, timeout: Duration) -> FinishResult {
        self.wait_timeout(timeout, |state| state.reservations >= count)
    }

    fn wait_for_join_owner(&self, timeout: Duration) -> FinishResult {
        self.wait_timeout(timeout, |state| state.join_owner)
    }

    fn wait_for_join_waiter(&self, timeout: Duration) -> FinishResult {
        self.wait_timeout(timeout, |state| state.join_waiters > 0)
    }

    fn wait_timeout(&self, timeout: Duration, ready: impl Fn(&State) -> bool) -> FinishResult {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return Err(RuntimeError::WorkerPanicked),
        };
        let (state, result) = match self
            .changed
            .wait_timeout_while(state, timeout, |state| !ready(state))
        {
            Ok(result) => result,
            Err(_) => return Err(RuntimeError::WorkerPanicked),
        };
        if result.timed_out() || !ready(&state) {
            Err(RuntimeError::TerminationTimedOut)
        } else {
            Ok(())
        }
    }

    fn drain_reservations(&self, executor: &mut dyn CommandExecutor, requests: &Receiver<Request>) {
        loop {
            while let Ok(request) = requests.try_recv() {
                execute_request(executor, request, self);
            }
            let state = match self.state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            if state.reservations == 0 {
                drop(state);
                while let Ok(request) = requests.try_recv() {
                    execute_request(executor, request, self);
                }
                return;
            }
            drop(self.changed.wait(state));
        }
    }
}

#[derive(Clone)]
pub struct RuntimeClient {
    shared: Arc<SharedRuntime>,
}

impl RuntimeClient {
    pub fn submit(&self, command: ApplicationCommand) -> Result<CommandOutcome, RuntimeError> {
        let sender = self.shared.reserve()?;
        let (response_sender, response) = bounded(1);
        let accepted = sender
            .send(Request::Command {
                command,
                response: response_sender,
            })
            .is_ok();
        drop(sender);
        self.shared.resolve_reservation(accepted)?;
        PendingOutcome {
            response,
            shared: self.shared.clone(),
        }
        .recv()
    }

    pub fn try_submit(&self, command: ApplicationCommand) -> Result<PendingOutcome, RuntimeError> {
        let (response_sender, response) = bounded(1);
        let result = {
            let state = self
                .shared
                .state
                .lock()
                .map_err(|_| RuntimeError::WorkerPanicked)?;
            if !matches!(state.worker, WorkerState::Running) {
                Err(SharedRuntime::terminal_error(&state))
            } else {
                match &state.admission {
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
            }
        };
        match result {
            Ok(()) => Ok(PendingOutcome {
                response,
                shared: self.shared.clone(),
            }),
            Err(RuntimeError::WorkerExited) => {
                let state = self
                    .shared
                    .state
                    .lock()
                    .map_err(|_| RuntimeError::WorkerPanicked)?;
                Err(SharedRuntime::terminal_error(&state))
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
        self.response.recv().unwrap_or_else(|_| {
            self.shared
                .state
                .lock()
                .map(|state| Err(SharedRuntime::terminal_error(&state)))
                .unwrap_or(Err(RuntimeError::WorkerPanicked))
        })
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
        Self::spawn_application_with_thread_spawner(
            service,
            capacity,
            Arc::new(SystemThreadSpawner),
        )
    }

    #[doc(hidden)]
    pub fn spawn_application_with_thread_spawner(
        mut service: ApplicationService,
        capacity: usize,
        spawner: Arc<dyn RuntimeThreadSpawner>,
    ) -> Result<Self, RuntimeError> {
        let worker = match service.worker() {
            Ok(worker) => worker,
            Err(_) => {
                let _ = catch_sensitive_unwind(AssertUnwindSafe(|| {
                    service.finish(ShutdownReason::ApplicationError)
                }));
                return Err(RuntimeError::WorkerStartup);
            }
        };
        let executor = ServiceWorker {
            service,
            worker,
            finished: false,
        };
        Self::spawn_with_initializer(capacity, spawner, move || Ok(Box::new(executor)))
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

    #[doc(hidden)]
    pub fn wait_for_reservations(&self, count: usize, timeout: Duration) -> FinishResult {
        self.shared.wait_for_reservations(count, timeout)
    }

    #[doc(hidden)]
    pub fn wait_for_join_owner(&self, timeout: Duration) -> FinishResult {
        self.shared.wait_for_join_owner(timeout)
    }

    #[doc(hidden)]
    pub fn wait_for_join_waiter(&self, timeout: Duration) -> FinishResult {
        self.shared.wait_for_join_waiter(timeout)
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
            state: Mutex::new(State {
                admission: Admission::Open(request_sender),
                reservations: 0,
                worker: WorkerState::Running,
                join_owner: false,
                join_waiters: 0,
            }),
            changed: Condvar::new(),
            control: control_sender,
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
        let worker = spawner
            .spawn(task)
            .map_err(|_| RuntimeError::WorkerStartup)?;
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
            Err(_) => Err(runtime
                .join_worker()
                .err()
                .unwrap_or(RuntimeError::WorkerExited)),
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
            if let Ok(mut state) = self.shared.state.lock() {
                state.join_owner = true;
                self.shared.changed.notify_all();
            }
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
            self.shared.wait_for_joined(true)
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
        let Some(worker) = worker else { return };
        let shared = self.shared.clone();
        let slot = Arc::new(Mutex::new(Some(worker)));
        let reaper_slot = slot.clone();
        let reaper = move || {
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
            .spawn(reaper)
            .is_err()
        {
            let result = match slot.lock().ok().and_then(|mut slot| slot.take()) {
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
        self.shared
            .close_admission(ShutdownReason::ApplicationError);
        self.reap_on_drop();
    }
}

struct ServiceWorker {
    service: ApplicationService,
    worker: ApplicationWorker,
    finished: bool,
}

impl CommandExecutor for ServiceWorker {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        self.worker.execute_user(command)
    }
    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError> {
        let result = self.service.finish(reason);
        if result.is_ok() {
            self.finished = true;
        }
        result
    }
}

impl Drop for ServiceWorker {
    fn drop(&mut self) {
        if !self.finished {
            let _ = catch_sensitive_unwind(AssertUnwindSafe(|| {
                self.service.finish(ShutdownReason::ApplicationError)
            }));
        }
    }
}

fn worker_entry(
    requests: Receiver<Request>,
    control: Receiver<Control>,
    initializer: impl FnOnce() -> Result<Box<dyn CommandExecutor>, RuntimeError>,
    initialized: Sender<FinishResult>,
    shared: Arc<SharedRuntime>,
) {
    let result = catch_sensitive_unwind(AssertUnwindSafe(|| {
        let mut executor = match initializer() {
            Ok(executor) => executor,
            Err(error) => {
                let _ = initialized.send(Err(error.clone()));
                shared.publish_exited(Err(error));
                return;
            }
        };
        let _ = initialized.send(Ok(()));
        worker_loop(&mut *executor, requests, control, &shared);
    }));
    if let Err(payload) = result {
        shared.publish_exited(Err(RuntimeError::WorkerPanicked));
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
                    shared.drain_reservations(executor, &requests);
                    match catch_sensitive_unwind(AssertUnwindSafe(|| executor.finish(reason))) {
                        Ok(result) => {
                            shared.publish_exited(result.map_err(RuntimeError::Application));
                            return;
                        }
                        Err(payload) => {
                            shared.publish_exited(Err(RuntimeError::WorkerPanicked));
                            resume_unwind(payload);
                        }
                    }
                }
                Err(_) => { shared.publish_exited(Err(RuntimeError::WorkerExited)); return; }
            },
            recv(requests) -> request => match request {
                Ok(request) => execute_request(executor, request, shared),
                Err(_) => { shared.publish_exited(Err(RuntimeError::WorkerExited)); return; }
            },
        }
    }
}

fn execute_request(executor: &mut dyn CommandExecutor, request: Request, shared: &SharedRuntime) {
    let Request::Command { command, response } = request;
    match catch_sensitive_unwind(AssertUnwindSafe(|| executor.execute_user(command))) {
        Ok(result) => {
            let _ = response.send(result.map_err(RuntimeError::Application));
        }
        Err(payload) => {
            let _ = catch_sensitive_unwind(AssertUnwindSafe(|| {
                executor.finish(ShutdownReason::ApplicationError)
            }));
            shared.publish_exited(Err(RuntimeError::WorkerPanicked));
            let _ = response.send(Err(RuntimeError::WorkerPanicked));
            resume_unwind(payload);
        }
    }
}
