use std::{
    io::{self, BufRead, Write},
    panic::{AssertUnwindSafe, resume_unwind},
    sync::Arc,
    thread,
};

use crossbeam_channel::{Receiver, bounded, never, select_biased};
use thiserror::Error;

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{
    app::{
        ApplicationCommand, InputRejection, InputRejectionCategory, ShutdownDisposition,
        ShutdownReason,
    },
    panic_boundary::catch_sensitive_unwind,
    runtime::{ApplicationRuntime, RuntimeClient, RuntimeError},
};

#[cfg(windows)]
use super::windows::{
    CancelAttempt, CancelDecision, ReadErrorDisposition, ReadPhase, cancellation_decision,
    classify_read_error, may_begin_read,
};
use super::{
    BoundedLineReader, ParsedLine, RawLine, TextRenderer, parse_line, reader::LineAccumulator,
};

#[derive(Debug, Error)]
pub enum UiError {
    #[error("input could not be read")]
    Read,
    #[error("output could not be written")]
    Write,
    #[error("runtime operation failed")]
    Runtime(#[from] RuntimeError),
    #[error("interrupt handling could not be started")]
    InterruptHandler,
    #[error("input worker stopped unexpectedly")]
    ReaderThread,
    #[error("terminal input is unavailable on this platform")]
    LineSourceUnavailable,
    #[error("command host stopped unexpectedly")]
    Panicked,
}

pub trait LineSourceCancellation: Send + Sync + 'static {
    fn cancel(&self);

    fn failure(&self) -> Option<io::Error> {
        None
    }
}

pub enum LineSourceEvent {
    Line(RawLine),
    Eof,
    Cancelled,
}

pub trait CancellableLineSource: Send + 'static {
    fn cancellation(&self) -> Arc<dyn LineSourceCancellation>;
    fn next_line(&mut self) -> io::Result<LineSourceEvent>;
}

#[cfg(unix)]
struct UnixCancellation {
    writer: std::sync::Mutex<std::os::unix::net::UnixStream>,
}

#[cfg(unix)]
impl LineSourceCancellation for UnixCancellation {
    fn cancel(&self) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(&[1]);
        }
    }
}

#[cfg(unix)]
pub struct UnixLineSource {
    input_fd: std::os::fd::RawFd,
    cancel_reader: std::os::unix::net::UnixStream,
    cancellation: Arc<UnixCancellation>,
    accumulator: LineAccumulator,
    ready: std::collections::VecDeque<RawLine>,
    eof: bool,
}

#[cfg(unix)]
impl UnixLineSource {
    fn stdin() -> io::Result<Self> {
        Self::from_borrowed_fd(libc::STDIN_FILENO)
    }

    #[doc(hidden)]
    pub fn from_borrowed_fd(input_fd: std::os::fd::RawFd) -> io::Result<Self> {
        let (cancel_reader, cancel_writer) = std::os::unix::net::UnixStream::pair()?;
        cancel_writer.set_nonblocking(true)?;
        Ok(Self {
            input_fd,
            cancel_reader,
            cancellation: Arc::new(UnixCancellation {
                writer: std::sync::Mutex::new(cancel_writer),
            }),
            accumulator: LineAccumulator::new(),
            ready: std::collections::VecDeque::new(),
            eof: false,
        })
    }

    fn read_ready_stdin(&mut self) -> io::Result<Option<LineSourceEvent>> {
        let mut buffer = [0_u8; 8192];
        let amount = loop {
            let amount =
                unsafe { libc::read(self.input_fd, buffer.as_mut_ptr().cast(), buffer.len()) };
            if amount >= 0 {
                break amount as usize;
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(None);
            }
            return Err(error);
        };

        if amount == 0 {
            self.eof = true;
            return Ok(Some(match self.accumulator.finish_eof()? {
                Some(line) => LineSourceEvent::Line(line),
                None => LineSourceEvent::Eof,
            }));
        }
        self.ready
            .extend(self.accumulator.push_chunk(&buffer[..amount])?);
        Ok(self.ready.pop_front().map(LineSourceEvent::Line))
    }
}

#[cfg(unix)]
impl CancellableLineSource for UnixLineSource {
    fn cancellation(&self) -> Arc<dyn LineSourceCancellation> {
        self.cancellation.clone()
    }

    fn next_line(&mut self) -> io::Result<LineSourceEvent> {
        use std::os::fd::AsRawFd;

        if let Some(line) = self.ready.pop_front() {
            return Ok(LineSourceEvent::Line(line));
        }
        if self.eof {
            return Ok(LineSourceEvent::Eof);
        }

        loop {
            let mut descriptors = [
                libc::pollfd {
                    fd: self.input_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.cancel_reader.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let result = unsafe {
                libc::poll(
                    descriptors.as_mut_ptr(),
                    descriptors.len() as libc::nfds_t,
                    -1,
                )
            };
            if result < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if descriptors[1].revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
                return Err(io::Error::other("input cancellation poll failed"));
            }
            if descriptors[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
                return Ok(LineSourceEvent::Cancelled);
            }
            if descriptors[0].revents & libc::POLLNVAL != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "terminal input descriptor is invalid",
                ));
            }
            if descriptors[0].revents & libc::POLLERR != 0 {
                return Err(io::Error::other("terminal input poll failed"));
            }
            if descriptors[0].revents & (libc::POLLIN | libc::POLLHUP) != 0
                && let Some(event) = self.read_ready_stdin()?
            {
                return Ok(event);
            }
        }
    }
}

#[cfg(windows)]
mod windows_api {
    use std::ffi::c_void;

    pub type Handle = isize;
    pub const INVALID_HANDLE_VALUE: Handle = -1;
    pub const STD_INPUT_HANDLE: u32 = u32::MAX - 9;
    pub const WAIT_OBJECT_0: u32 = 0;
    pub const WAIT_FAILED: u32 = u32::MAX;
    pub const INFINITE: u32 = u32::MAX;
    pub const THREAD_TERMINATE: u32 = windows_sys::Win32::System::Threading::THREAD_TERMINATE;
    pub const ERROR_NOT_FOUND: u32 = 1168;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetStdHandle(kind: u32) -> Handle;
        pub fn CreateEventW(
            attributes: *const c_void,
            manual_reset: i32,
            initial_state: i32,
            name: *const u16,
        ) -> Handle;
        pub fn SetEvent(event: Handle) -> i32;
        pub fn OpenThread(desired_access: u32, inherit_handle: i32, thread_id: u32) -> Handle;
        pub fn GetCurrentThreadId() -> u32;
        pub fn CancelSynchronousIo(thread: Handle) -> i32;
        pub fn CancelIoEx(file: Handle, overlapped: *mut c_void) -> i32;
        pub fn WaitForMultipleObjects(
            count: u32,
            handles: *const Handle,
            wait_all: i32,
            milliseconds: u32,
        ) -> u32;
        pub fn ReadFile(
            file: Handle,
            buffer: *mut c_void,
            bytes_to_read: u32,
            bytes_read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        pub fn CloseHandle(handle: Handle) -> i32;
    }
}

#[cfg(windows)]
struct WindowsCancellation {
    event: windows_api::Handle,
    input: windows_api::Handle,
    state: std::sync::Mutex<WindowsReadState>,
    phase_changed: std::sync::Condvar,
    cancelled: AtomicBool,
}

#[cfg(windows)]
struct WindowsReadState {
    reader_thread: Option<windows_api::Handle>,
    phase: ReadPhase,
    cancel_error: Option<u32>,
}

#[cfg(windows)]
impl WindowsCancellation {
    fn register_current_thread(&self) -> io::Result<()> {
        let mut state = self.state.lock().map_err(|_| {
            io::Error::new(
                io::ErrorKind::Other,
                "input cancellation state is unavailable",
            )
        })?;
        if state.reader_thread.is_none() {
            let handle = unsafe {
                windows_api::OpenThread(
                    windows_api::THREAD_TERMINATE,
                    0,
                    windows_api::GetCurrentThreadId(),
                )
            };
            if handle == 0 {
                return Err(io::Error::last_os_error());
            }
            state.reader_thread = Some(handle);
        }
        Ok(())
    }

    fn was_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn begin_read(&self) -> io::Result<bool> {
        {
            let mut state = self.state.lock().map_err(|_| {
                io::Error::new(io::ErrorKind::Other, "input read state is unavailable")
            })?;
            if !may_begin_read(self.was_cancelled()) {
                return Ok(false);
            }
            state.phase = ReadPhase::AboutToRead;
            self.phase_changed.notify_all();
        }

        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "input read state is unavailable"))?;
        if !may_begin_read(self.was_cancelled()) {
            state.phase = ReadPhase::IdleWaiting;
            self.phase_changed.notify_all();
            return Ok(false);
        }
        state.phase = ReadPhase::ReadActive;
        self.phase_changed.notify_all();
        drop(state);

        if !may_begin_read(self.was_cancelled()) {
            self.end_read();
            return Ok(false);
        }
        Ok(true)
    }

    fn end_read(&self) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if state.phase != ReadPhase::Exited {
            state.phase = ReadPhase::IdleWaiting;
            self.phase_changed.notify_all();
        }
    }

    fn mark_exited(&self) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if state.phase != ReadPhase::Exited {
            state.phase = ReadPhase::Exited;
            self.phase_changed.notify_all();
        }
    }

    fn cancellation_attempt(&self, reader_thread: windows_api::Handle) -> CancelAttempt {
        if unsafe { windows_api::CancelSynchronousIo(reader_thread) } != 0 {
            return CancelAttempt::Succeeded;
        }
        let error = io::Error::last_os_error()
            .raw_os_error()
            .map(|code| code as u32)
            .unwrap_or(0);
        if error == windows_api::ERROR_NOT_FOUND {
            CancelAttempt::NotFound
        } else {
            CancelAttempt::Failed(error)
        }
    }

    fn cancel_fallback(&self) -> CancelAttempt {
        if unsafe { windows_api::CancelIoEx(self.input, std::ptr::null_mut()) } != 0 {
            return CancelAttempt::Succeeded;
        }
        let error = io::Error::last_os_error()
            .raw_os_error()
            .map(|code| code as u32)
            .unwrap_or(0);
        if error == windows_api::ERROR_NOT_FOUND {
            CancelAttempt::NotFound
        } else {
            CancelAttempt::Failed(error)
        }
    }

    fn cancel_pending_read(&self) {
        loop {
            let (phase, reader_thread) = {
                let state = match self.state.lock() {
                    Ok(state) => state,
                    Err(poisoned) => poisoned.into_inner(),
                };
                match state.phase {
                    ReadPhase::IdleWaiting | ReadPhase::Exited => return,
                    ReadPhase::AboutToRead | ReadPhase::ReadActive => {
                        (state.phase, state.reader_thread)
                    }
                }
            };

            let attempt = match reader_thread {
                Some(reader_thread) => self.cancellation_attempt(reader_thread),
                None => CancelAttempt::Failed(6),
            };
            match cancellation_decision(phase, attempt) {
                CancelDecision::Complete => return,
                CancelDecision::Failed(error) => {
                    let mut state = match self.state.lock() {
                        Ok(state) => state,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    state.cancel_error.get_or_insert(error);
                    drop(state);
                    if matches!(self.cancel_fallback(), CancelAttempt::Succeeded) {
                        return;
                    }
                }
                CancelDecision::Retry => {}
            }

            let state = match self.state.lock() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            if matches!(state.phase, ReadPhase::IdleWaiting | ReadPhase::Exited) {
                return;
            }
            match self
                .phase_changed
                .wait_timeout(state, std::time::Duration::from_millis(1))
            {
                Ok((state, _)) => drop(state),
                Err(poisoned) => {
                    let (state, _) = poisoned.into_inner();
                    drop(state);
                }
            }
        }
    }
}

#[cfg(windows)]
impl LineSourceCancellation for WindowsCancellation {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        unsafe {
            windows_api::SetEvent(self.event);
        }
        self.cancel_pending_read();
    }

    fn failure(&self) -> Option<io::Error> {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state
            .cancel_error
            .map(|code| io::Error::from_raw_os_error(code as i32))
    }
}

#[cfg(windows)]
impl Drop for WindowsCancellation {
    fn drop(&mut self) {
        let state = match self.state.get_mut() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        unsafe {
            if let Some(reader_thread) = state.reader_thread.take() {
                windows_api::CloseHandle(reader_thread);
            }
            windows_api::CloseHandle(self.event);
        }
    }
}

#[cfg(windows)]
struct WindowsStdinLineSource {
    input: windows_api::Handle,
    cancellation: Arc<WindowsCancellation>,
    accumulator: LineAccumulator,
    ready: std::collections::VecDeque<RawLine>,
    eof: bool,
}

#[cfg(windows)]
impl WindowsStdinLineSource {
    fn stdin() -> io::Result<Self> {
        let input = unsafe { windows_api::GetStdHandle(windows_api::STD_INPUT_HANDLE) };
        if input == 0 || input == windows_api::INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let event = unsafe { windows_api::CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if event == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            input,
            cancellation: Arc::new(WindowsCancellation {
                event,
                input,
                state: std::sync::Mutex::new(WindowsReadState {
                    reader_thread: None,
                    phase: ReadPhase::IdleWaiting,
                    cancel_error: None,
                }),
                phase_changed: std::sync::Condvar::new(),
                cancelled: AtomicBool::new(false),
            }),
            accumulator: LineAccumulator::new(),
            ready: std::collections::VecDeque::new(),
            eof: false,
        })
    }

    fn read_ready_input(&mut self) -> io::Result<Option<LineSourceEvent>> {
        let mut buffer = [0_u8; 8192];
        let mut amount = 0_u32;
        let succeeded = unsafe {
            windows_api::ReadFile(
                self.input,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut amount,
                std::ptr::null_mut(),
            )
        };
        if succeeded == 0 {
            let error = io::Error::last_os_error();
            let error_code = error.raw_os_error().map(|code| code as u32).unwrap_or(0);
            return match classify_read_error(error_code, self.cancellation.was_cancelled()) {
                ReadErrorDisposition::Cancelled => Ok(Some(LineSourceEvent::Cancelled)),
                ReadErrorDisposition::Eof => self.finish_input(),
                ReadErrorDisposition::Error => Err(error),
            };
        }
        if amount == 0 {
            return self.finish_input();
        }
        self.ready
            .extend(self.accumulator.push_chunk(&buffer[..amount as usize])?);
        Ok(self.ready.pop_front().map(LineSourceEvent::Line))
    }

    fn finish_input(&mut self) -> io::Result<Option<LineSourceEvent>> {
        self.eof = true;
        Ok(Some(match self.accumulator.finish_eof()? {
            Some(line) => LineSourceEvent::Line(line),
            None => LineSourceEvent::Eof,
        }))
    }
}

#[cfg(windows)]
impl CancellableLineSource for WindowsStdinLineSource {
    fn cancellation(&self) -> Arc<dyn LineSourceCancellation> {
        self.cancellation.clone()
    }

    fn next_line(&mut self) -> io::Result<LineSourceEvent> {
        self.cancellation.register_current_thread()?;
        if let Some(line) = self.ready.pop_front() {
            return Ok(LineSourceEvent::Line(line));
        }
        if self.cancellation.was_cancelled() {
            return Ok(LineSourceEvent::Cancelled);
        }
        if self.eof {
            return Ok(LineSourceEvent::Eof);
        }

        loop {
            let handles = [self.cancellation.event, self.input];
            match unsafe {
                windows_api::WaitForMultipleObjects(
                    handles.len() as u32,
                    handles.as_ptr(),
                    0,
                    windows_api::INFINITE,
                )
            } {
                windows_api::WAIT_OBJECT_0 => {
                    return Ok(LineSourceEvent::Cancelled);
                }
                result if result == windows_api::WAIT_OBJECT_0 + 1 => {
                    if !self.cancellation.begin_read()? {
                        return Ok(LineSourceEvent::Cancelled);
                    }
                    let event = self.read_ready_input();
                    self.cancellation.end_read();
                    if let Some(event) = event? {
                        return Ok(event);
                    }
                }
                windows_api::WAIT_FAILED => return Err(io::Error::last_os_error()),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "terminal input wait failed",
                    ));
                }
            }
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsStdinLineSource {
    fn drop(&mut self) {
        self.cancellation.mark_exited();
    }
}

pub struct StdioResources {
    interrupts: Receiver<()>,
    #[cfg(unix)]
    source: UnixLineSource,
    #[cfg(windows)]
    source: WindowsStdinLineSource,
}

impl StdioResources {
    pub fn initialize() -> Result<Self, UiError> {
        let interrupts = crate::ui::interrupt::receiver()
            .map_err(|_| UiError::InterruptHandler)?;
        #[cfg(unix)]
        {
            let source = UnixLineSource::stdin().map_err(|_| UiError::LineSourceUnavailable)?;
            Ok(Self { interrupts, source })
        }
        #[cfg(windows)]
        {
            let source =
                WindowsStdinLineSource::stdin().map_err(|_| UiError::LineSourceUnavailable)?;
            Ok(Self { interrupts, source })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = interrupts;
            Err(UiError::LineSourceUnavailable)
        }
    }
}

pub struct FallbackRunner {
    client: RuntimeClient,
    show_prompt: bool,
}

impl FallbackRunner {
    pub fn new(client: RuntimeClient, show_prompt: bool) -> Self {
        Self {
            client,
            show_prompt,
        }
    }

    pub fn run<R: BufRead, W: Write>(
        &self,
        reader: R,
        mut writer: W,
    ) -> Result<ShutdownReason, UiError> {
        let mut reader = BoundedLineReader::new(reader);
        loop {
            self.prompt(&mut writer)?;
            let line = reader.next_line().map_err(|_| UiError::Read)?;
            let Some(line) = line else {
                return Ok(ShutdownReason::InputClosed);
            };
            if let Some(reason) = self.process_line(line, &mut writer)? {
                return Ok(reason);
            }
        }
    }

    fn prompt<W: Write>(&self, writer: &mut W) -> Result<(), UiError> {
        if self.show_prompt {
            writer.write_all(b"> ").map_err(|_| UiError::Write)?;
            writer.flush().map_err(|_| UiError::Write)?;
        }
        Ok(())
    }

    fn process_line<W: Write>(
        &self,
        line: RawLine,
        writer: &mut W,
    ) -> Result<Option<ShutdownReason>, UiError> {
        let command = if line.was_oversized() {
            ApplicationCommand::RejectInput(InputRejection {
                category: InputRejectionCategory::Oversized,
                safe_token: None,
                byte_length: line.full_byte_length(),
                input_digest: line.input_digest().clone(),
            })
        } else {
            let ParsedLine::Command(command) = parse_line(line.bytes()) else {
                return Ok(None);
            };
            command
        };

        let pending = match self.client.try_submit(command) {
            Ok(pending) => pending,
            Err(error @ RuntimeError::Backpressure) => {
                TextRenderer::render_runtime_error(&error, writer).map_err(|_| UiError::Write)?;
                return Ok(None);
            }
            Err(error) => return Err(UiError::Runtime(error)),
        };
        let outcome = pending.recv().map_err(UiError::Runtime)?;
        TextRenderer::render_outcome(&outcome, writer).map_err(|_| UiError::Write)?;
        if outcome.shutdown == ShutdownDisposition::Requested {
            Ok(Some(ShutdownReason::UserQuit))
        } else {
            Ok(None)
        }
    }
}

pub struct FallbackHost {
    runtime: ApplicationRuntime,
    show_prompt: bool,
    previous_session_interrupted: bool,
}

impl FallbackHost {
    pub fn new(
        runtime: ApplicationRuntime,
        show_prompt: bool,
        previous_session_interrupted: bool,
    ) -> Self {
        Self {
            runtime,
            show_prompt,
            previous_session_interrupted,
        }
    }

    pub fn run<S, W>(
        self,
        mut source: S,
        mut writer: W,
        interrupts: Receiver<()>,
    ) -> Result<ShutdownReason, UiError>
    where
        S: CancellableLineSource,
        W: Write,
    {
        let cancellation = source.cancellation();
        let (line_sender, line_receiver) = bounded(1);
        let source_thread = match thread::Builder::new()
            .name("fallback-input".to_owned())
            .spawn(move || {
                let result = catch_sensitive_unwind(AssertUnwindSafe(|| {
                    loop {
                        let event = source.next_line();
                        let terminal = !matches!(event, Ok(LineSourceEvent::Line(_)));
                        if line_sender.send(event).is_err() || terminal {
                            return;
                        }
                    }
                }));
                if let Err(payload) = result {
                    resume_unwind(payload);
                }
            }) {
            Ok(thread) => thread,
            Err(_) => {
                let _ = self
                    .runtime
                    .finish_and_join(ShutdownReason::ApplicationError);
                return Err(UiError::ReaderThread);
            }
        };

        let runner = FallbackRunner::new(self.runtime.client(), self.show_prompt);
        let body = catch_sensitive_unwind(AssertUnwindSafe(|| {
            run_host_loop(
                &runner,
                line_receiver,
                &mut writer,
                interrupts,
                self.previous_session_interrupted,
            )
        }));
        cancellation.cancel();
        let source_joined = source_thread.join().is_ok();
        let cancellation_failed = cancellation.failure().is_some();

        let (reason, body_error) = match body {
            Ok(Ok(reason)) if source_joined && !cancellation_failed => (reason, None),
            Ok(Ok(_)) if !source_joined => (
                ShutdownReason::ApplicationError,
                Some(UiError::ReaderThread),
            ),
            Ok(Ok(_)) => (ShutdownReason::ApplicationError, Some(UiError::Read)),
            Ok(Err(error)) => (ShutdownReason::ApplicationError, Some(error)),
            Err(_) => (ShutdownReason::ApplicationError, Some(UiError::Panicked)),
        };
        let finish = self.runtime.finish_and_join(reason);
        if let Some(error) = body_error {
            return Err(error);
        }
        finish.map_err(UiError::Runtime)?;
        Ok(reason)
    }
}

fn run_host_loop<W: Write>(
    runner: &FallbackRunner,
    line_receiver: Receiver<io::Result<LineSourceEvent>>,
    writer: &mut W,
    mut interrupts: Receiver<()>,
    previous_session_interrupted: bool,
) -> Result<ShutdownReason, UiError> {
    if previous_session_interrupted {
        TextRenderer::render_previous_session_warning(writer).map_err(|_| UiError::Write)?;
    }

    loop {
        runner.prompt(writer)?;
        loop {
            select_biased! {
                recv(interrupts) -> signal => match signal {
                    Ok(()) => {
                        TextRenderer::render_shutdown_reason(ShutdownReason::Interrupted, writer)
                            .map_err(|_| UiError::Write)?;
                        return Ok(ShutdownReason::Interrupted);
                    }
                    Err(_) => {
                        interrupts = never();
                        continue;
                    }
                },
                recv(line_receiver) -> line => match line {
                    Ok(Ok(LineSourceEvent::Line(line))) => {
                        if let Some(reason) = runner.process_line(line, writer)? {
                            return Ok(reason);
                        }
                        break;
                    }
                    Ok(Ok(LineSourceEvent::Eof)) => return Ok(ShutdownReason::InputClosed),
                    Ok(Ok(LineSourceEvent::Cancelled)) => return Err(UiError::ReaderThread),
                    Ok(Err(_)) => return Err(UiError::Read),
                    Err(_) => return Err(UiError::ReaderThread),
                }
            }
        }
    }
}

pub fn run_stdio(
    runtime: ApplicationRuntime,
    previous_session_interrupted: bool,
    resources: StdioResources,
) -> Result<ShutdownReason, UiError> {
    #[cfg(any(unix, windows))]
    {
        let stdout = io::stdout();
        FallbackHost::new(runtime, true, previous_session_interrupted).run(
            resources.source,
            stdout.lock(),
            resources.interrupts,
        )
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = resources;
        let _ = runtime.finish_and_join(ShutdownReason::ApplicationError);
        Err(UiError::LineSourceUnavailable)
    }
}
