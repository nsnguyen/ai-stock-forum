use std::{
    io::{self, BufRead, Write},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    thread,
};

use crossbeam_channel::{bounded, never, select_biased, Receiver};
use thiserror::Error;

use crate::{
    app::{
        ApplicationCommand, InputRejection, InputRejectionCategory, ShutdownDisposition,
        ShutdownReason,
    },
    runtime::{ApplicationRuntime, RuntimeClient, RuntimeError},
};

use super::{
    parse_line,
    reader::LineAccumulator,
    BoundedLineReader, ParsedLine, RawLine, TextRenderer,
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
}

pub enum LineSourceEvent {
    Line(RawLine),
    Eof,
    Cancelled,
}

pub trait LineSource: Send + 'static {
    fn cancellation(&self) -> Arc<dyn LineSourceCancellation>;
    fn next_line(&mut self) -> io::Result<LineSourceEvent>;
}

struct AtomicCancellation {
    cancelled: AtomicBool,
}

impl LineSourceCancellation for AtomicCancellation {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

pub struct BufferedLineSource<R> {
    reader: BoundedLineReader<R>,
    cancellation: Arc<AtomicCancellation>,
}

impl<R: BufRead> BufferedLineSource<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: BoundedLineReader::new(reader),
            cancellation: Arc::new(AtomicCancellation {
                cancelled: AtomicBool::new(false),
            }),
        }
    }
}

impl<R> LineSource for BufferedLineSource<R>
where
    R: BufRead + Send + 'static,
{
    fn cancellation(&self) -> Arc<dyn LineSourceCancellation> {
        self.cancellation.clone()
    }

    fn next_line(&mut self) -> io::Result<LineSourceEvent> {
        if self.cancellation.cancelled.load(Ordering::SeqCst) {
            return Ok(LineSourceEvent::Cancelled);
        }
        match self.reader.next_line()? {
            Some(line) => Ok(LineSourceEvent::Line(line)),
            None => Ok(LineSourceEvent::Eof),
        }
    }
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
struct UnixStdinLineSource {
    cancel_reader: std::os::unix::net::UnixStream,
    cancellation: Arc<UnixCancellation>,
    accumulator: LineAccumulator,
    ready: std::collections::VecDeque<RawLine>,
    eof: bool,
}

#[cfg(unix)]
impl UnixStdinLineSource {
    fn new() -> io::Result<Self> {
        let (cancel_reader, cancel_writer) = std::os::unix::net::UnixStream::pair()?;
        cancel_writer.set_nonblocking(true)?;
        Ok(Self {
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
            let amount = unsafe {
                libc::read(
                    libc::STDIN_FILENO,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
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
impl LineSource for UnixStdinLineSource {
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
                    fd: libc::STDIN_FILENO,
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
            if descriptors[1].revents != 0 {
                return Ok(LineSourceEvent::Cancelled);
            }
            if descriptors[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                if let Some(event) = self.read_ready_stdin()? {
                    return Ok(event);
                }
            }
        }
    }
}

pub struct StdioResources {
    interrupts: Receiver<()>,
    #[cfg(unix)]
    source: UnixStdinLineSource,
}

impl StdioResources {
    pub fn initialize() -> Result<Self, UiError> {
        let interrupts = interrupt_receiver()?;
        #[cfg(unix)]
        {
            let source = UnixStdinLineSource::new().map_err(|_| UiError::LineSourceUnavailable)?;
            Ok(Self { interrupts, source })
        }
        #[cfg(not(unix))]
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
                TextRenderer::render_runtime_error(&error, writer)
                    .map_err(|_| UiError::Write)?;
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
        S: LineSource,
        W: Write,
    {
        let cancellation = source.cancellation();
        let (line_sender, line_receiver) = bounded(1);
        let source_thread = match thread::Builder::new()
            .name("fallback-input".to_owned())
            .spawn(move || loop {
                let event = source.next_line();
                let terminal = !matches!(event, Ok(LineSourceEvent::Line(_)));
                if line_sender.send(event).is_err() || terminal {
                    return;
                }
            })
        {
            Ok(thread) => thread,
            Err(_) => {
                let _ = self
                    .runtime
                    .finish_and_join(ShutdownReason::ApplicationError);
                return Err(UiError::ReaderThread);
            }
        };

        let runner = FallbackRunner::new(self.runtime.client(), self.show_prompt);
        let body = catch_unwind(AssertUnwindSafe(|| {
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

        let (reason, body_error) = match body {
            Ok(Ok(reason)) if source_joined => (reason, None),
            Ok(Ok(_)) => (ShutdownReason::ApplicationError, Some(UiError::ReaderThread)),
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

struct InterruptBus {
    receiver: Receiver<()>,
}

static INTERRUPT_BUS: OnceLock<Result<InterruptBus, ()>> = OnceLock::new();

fn interrupt_receiver() -> Result<Receiver<()>, UiError> {
    let bus = INTERRUPT_BUS.get_or_init(|| {
        let (sender, receiver) = bounded(1);
        let handler_sender = sender.clone();
        ctrlc::set_handler(move || {
            let _ = handler_sender.try_send(());
        })
        .map_err(|_| ())?;
        Ok(InterruptBus { receiver })
    });
    let receiver = bus
        .as_ref()
        .map_err(|_| UiError::InterruptHandler)?
        .receiver
        .clone();
    while receiver.try_recv().is_ok() {}
    Ok(receiver)
}

pub fn run_stdio(
    runtime: ApplicationRuntime,
    previous_session_interrupted: bool,
    resources: StdioResources,
) -> Result<ShutdownReason, UiError> {
    #[cfg(unix)]
    {
        let stdout = io::stdout();
        FallbackHost::new(runtime, true, previous_session_interrupted).run(
            resources.source,
            stdout.lock(),
            resources.interrupts,
        )
    }
    #[cfg(not(unix))]
    {
        let _ = resources;
        let _ = runtime.finish_and_join(ShutdownReason::ApplicationError);
        Err(UiError::LineSourceUnavailable)
    }
}
