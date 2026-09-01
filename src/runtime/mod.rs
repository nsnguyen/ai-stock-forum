//! Bounded command transport for the application worker.

use std::{
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
};

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use thiserror::Error;

use crate::app::{
    AppError, ApplicationCommand, ApplicationService, ApplicationWorker, CommandOutcome,
    ShutdownReason,
};

pub const MODULE_NAME: &str = "runtime";
pub const DEFAULT_QUEUE_CAPACITY: usize = 32;

/// The serial command boundary executed by the runtime's dedicated worker thread.
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
    #[error("application worker disconnected unexpectedly")]
    WorkerDisconnected,
    #[error("application worker panicked")]
    WorkerPanicked,
}

type CommandResult = Result<CommandOutcome, RuntimeError>;
type FinishResult = Result<(), RuntimeError>;

enum Request {
    Command {
        command: ApplicationCommand,
        response: Sender<CommandResult>,
    },
    Finish {
        reason: ShutdownReason,
        response: Sender<FinishResult>,
    },
}

enum RuntimeState {
    Running,
    Closing,
    Finished(FinishResult),
}

struct RuntimeInner {
    state: Mutex<RuntimeState>,
    finished: Condvar,
    requests: Mutex<Option<Sender<Request>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl RuntimeInner {
    fn terminal_error(&self) -> RuntimeError {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return RuntimeError::WorkerPanicked,
        };
        match &*state {
            RuntimeState::Finished(Err(error)) => error.clone(),
            RuntimeState::Finished(Ok(())) | RuntimeState::Closing | RuntimeState::Running => {
                RuntimeError::WorkerDisconnected
            }
        }
    }

    fn finish_with(&self, result: FinishResult) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if !matches!(*state, RuntimeState::Finished(_)) {
            *state = RuntimeState::Finished(result);
            self.finished.notify_all();
        }
    }

    fn fail(&self, error: RuntimeError) {
        self.finish_with(Err(error));
    }
}

/// A handle for enqueueing user commands without exposing service execution to callers.
#[derive(Clone)]
pub struct RuntimeClient {
    inner: Arc<RuntimeInner>,
}

impl RuntimeClient {
    pub fn submit(&self, command: ApplicationCommand) -> Result<CommandOutcome, RuntimeError> {
        self.enqueue(command, false)?.recv()
    }

    pub fn try_submit(&self, command: ApplicationCommand) -> Result<PendingOutcome, RuntimeError> {
        self.enqueue(command, true)
    }

    fn enqueue(
        &self,
        command: ApplicationCommand,
        nonblocking: bool,
    ) -> Result<PendingOutcome, RuntimeError> {
        let (response_sender, response) = bounded(1);
        let state = self.inner.state.lock().map_err(|_| RuntimeError::WorkerPanicked)?;
        if !matches!(*state, RuntimeState::Running) {
            return Err(RuntimeError::Closed);
        }
        let requests = self
            .inner
            .requests
            .lock()
            .map_err(|_| RuntimeError::WorkerPanicked)?;
        let sender = requests.as_ref().ok_or(RuntimeError::Closed)?;
        let request = Request::Command {
            command,
            response: response_sender,
        };
        let sent = if nonblocking {
            match sender.try_send(request) {
                Ok(()) => Ok(()),
                Err(TrySendError::Full(_)) => Err(RuntimeError::Backpressure),
                Err(TrySendError::Disconnected(_)) => Err(RuntimeError::WorkerDisconnected),
            }
        } else {
            sender.send(request).map_err(|_| RuntimeError::WorkerDisconnected)
        };
        drop(requests);
        drop(state);
        sent?;
        Ok(PendingOutcome {
            response,
            inner: self.inner.clone(),
        })
    }
}

/// The one-command response path returned after a command is accepted into the queue.
pub struct PendingOutcome {
    response: Receiver<CommandResult>,
    inner: Arc<RuntimeInner>,
}

impl PendingOutcome {
    pub fn recv(self) -> Result<CommandOutcome, RuntimeError> {
        match self.response.recv() {
            Ok(result) => result,
            Err(_) => Err(self.inner.terminal_error()),
        }
    }
}

/// Owns the bounded request port and the dedicated application worker thread.
#[derive(Clone)]
pub struct ApplicationRuntime {
    inner: Arc<RuntimeInner>,
}

impl ApplicationRuntime {
    /// Starts one [`ApplicationWorker`] on a dedicated thread and retains its service owner for
    /// the single terminal lifecycle write.
    pub fn spawn(service: ApplicationService, capacity: usize) -> Result<Self, RuntimeError> {
        Self::spawn_with_initializer(capacity, move || {
            let worker = service.worker().map_err(|_| RuntimeError::WorkerStartup)?;
            Ok(Box::new(ServiceWorker { service, worker }))
        })
    }

    /// Starts a bounded runtime around an alternate executor for deterministic headless tests.
    pub fn spawn_executor(
        executor: impl CommandExecutor,
        capacity: usize,
    ) -> Result<Self, RuntimeError> {
        Self::spawn_with_initializer(capacity, move || Ok(Box::new(executor)))
    }

    pub fn client(&self) -> RuntimeClient {
        RuntimeClient {
            inner: self.inner.clone(),
        }
    }

    /// Rejects new submissions, drains prior accepted work in FIFO order, finishes once, and
    /// joins the worker. Concurrent callers observe the same terminal result.
    pub fn finish_and_join(&self, reason: ShutdownReason) -> FinishResult {
        let response = {
            let mut state = match self.inner.state.lock() {
                Ok(state) => state,
                Err(_) => return Err(RuntimeError::WorkerPanicked),
            };
            match &*state {
                RuntimeState::Finished(result) => return result.clone(),
                RuntimeState::Closing => None,
                RuntimeState::Running => {
                    *state = RuntimeState::Closing;
                    let mut requests = match self.inner.requests.lock() {
                        Ok(requests) => requests,
                        Err(_) => {
                            *state = RuntimeState::Finished(Err(RuntimeError::WorkerPanicked));
                            self.inner.finished.notify_all();
                            return Err(RuntimeError::WorkerPanicked);
                        }
                    };
                    let Some(sender) = requests.take() else {
                        *state = RuntimeState::Finished(Err(RuntimeError::WorkerDisconnected));
                        self.inner.finished.notify_all();
                        return Err(RuntimeError::WorkerDisconnected);
                    };
                    let (response_sender, response) = bounded(1);
                    if sender
                        .send(Request::Finish {
                            reason,
                            response: response_sender,
                        })
                        .is_err()
                    {
                        *state = RuntimeState::Finished(Err(RuntimeError::WorkerDisconnected));
                        self.inner.finished.notify_all();
                        return Err(RuntimeError::WorkerDisconnected);
                    }
                    Some(response)
                }
            }
        };

        if let Some(response) = response {
            let result = response.recv().unwrap_or_else(|_| Err(self.inner.terminal_error()));
            let result = match self.join_worker() {
                Ok(()) => result,
                Err(error) => Err(error),
            };
            self.inner.finish_with(result.clone());
            result
        } else {
            self.wait_for_finish()
        }
    }

    fn spawn_with_initializer(
        capacity: usize,
        initializer: impl FnOnce() -> Result<Box<dyn CommandExecutor>, RuntimeError> + Send + 'static,
    ) -> Result<Self, RuntimeError> {
        if capacity == 0 {
            return Err(RuntimeError::InvalidCapacity);
        }
        let (request_sender, request_receiver) = bounded(capacity);
        let (initialized_sender, initialized_receiver) = bounded(1);
        let inner = Arc::new(RuntimeInner {
            state: Mutex::new(RuntimeState::Running),
            finished: Condvar::new(),
            requests: Mutex::new(Some(request_sender)),
            worker: Mutex::new(None),
        });
        let worker_inner = inner.clone();
        let worker = thread::spawn(move || {
            let loop_inner = worker_inner.clone();
            let result = catch_unwind(AssertUnwindSafe(|| {
                worker_loop(request_receiver, initializer, initialized_sender, loop_inner)
            }));
            if let Err(payload) = result {
                worker_inner.fail(RuntimeError::WorkerPanicked);
                resume_unwind(payload);
            }
        });
        *inner.worker.lock().map_err(|_| RuntimeError::WorkerPanicked)? = Some(worker);

        match initialized_receiver.recv() {
            Ok(Ok(())) => Ok(Self { inner }),
            Ok(Err(error)) => {
                let runtime = Self { inner };
                let _ = runtime.join_worker();
                Err(error)
            }
            Err(_) => {
                let runtime = Self { inner };
                Err(runtime.join_worker().err().unwrap_or(RuntimeError::WorkerDisconnected))
            }
        }
    }

    fn join_worker(&self) -> FinishResult {
        let worker = self
            .inner
            .worker
            .lock()
            .map_err(|_| RuntimeError::WorkerPanicked)?
            .take();
        match worker {
            Some(worker) => worker.join().map_err(|_| RuntimeError::WorkerPanicked),
            None => Ok(()),
        }
    }

    fn wait_for_finish(&self) -> FinishResult {
        let mut state = self.inner.state.lock().map_err(|_| RuntimeError::WorkerPanicked)?;
        loop {
            match &*state {
                RuntimeState::Finished(result) => return result.clone(),
                RuntimeState::Running | RuntimeState::Closing => {
                    state = self
                        .inner
                        .finished
                        .wait(state)
                        .map_err(|_| RuntimeError::WorkerPanicked)?;
                }
            }
        }
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

fn worker_loop(
    requests: Receiver<Request>,
    initializer: impl FnOnce() -> Result<Box<dyn CommandExecutor>, RuntimeError>,
    initialized: Sender<Result<(), RuntimeError>>,
    inner: Arc<RuntimeInner>,
) {
    let mut executor = match initializer() {
        Ok(executor) => executor,
        Err(error) => {
            let _ = initialized.send(Err(error));
            return;
        }
    };
    let _ = initialized.send(Ok(()));
    while let Ok(request) = requests.recv() {
        match request {
            Request::Command { command, response } => {
                match catch_unwind(AssertUnwindSafe(|| executor.execute_user(command))) {
                    Ok(result) => {
                        let _ = response.send(result.map_err(RuntimeError::Application));
                    }
                    Err(payload) => {
                        inner.fail(RuntimeError::WorkerPanicked);
                        let _ = response.send(Err(RuntimeError::WorkerPanicked));
                        resume_unwind(payload);
                    }
                }
            }
            Request::Finish { reason, response } => {
                let result = executor.finish(reason).map_err(RuntimeError::Application);
                let _ = response.send(result);
                return;
            }
        }
    }
}
