use std::{
    io::{self, BufRead, BufReader, Write},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::OnceLock,
    thread,
};

use crossbeam_channel::{bounded, select_biased, Receiver};
use thiserror::Error;

use crate::{
    app::{ShutdownDisposition, ShutdownReason},
    runtime::{ApplicationRuntime, RuntimeClient, RuntimeError},
};

use super::{parse_line, BoundedLineReader, ParsedLine, RawLine, TextRenderer};

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
    #[error("command host stopped unexpectedly")]
    Panicked,
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
        let ParsedLine::Command(command) = parse_line(line.bytes()) else {
            return Ok(None);
        };
        let pending = match self.client.try_submit(command) {
            Ok(pending) => pending,
            Err(error @ RuntimeError::Backpressure) => {
                TextRenderer::render_runtime_error(&error, writer)
                    .map_err(|_| UiError::Write)?;
                return Ok(None);
            }
            Err(error) => {
                TextRenderer::render_runtime_error(&error, writer)
                    .map_err(|_| UiError::Write)?;
                return Err(UiError::Runtime(error));
            }
        };
        let outcome = match pending.recv() {
            Ok(outcome) => outcome,
            Err(error) => {
                TextRenderer::render_runtime_error(&error, writer)
                    .map_err(|_| UiError::Write)?;
                return Err(UiError::Runtime(error));
            }
        };
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

    pub fn run<R, W>(
        self,
        reader: R,
        mut writer: W,
        interrupts: Receiver<()>,
    ) -> Result<ShutdownReason, UiError>
    where
        R: BufRead + Send + 'static,
        W: Write,
    {
        let runner = FallbackRunner::new(self.runtime.client(), self.show_prompt);
        let previous_session_interrupted = self.previous_session_interrupted;
        let body = catch_unwind(AssertUnwindSafe(|| {
            run_host_loop(
                &runner,
                reader,
                &mut writer,
                interrupts,
                previous_session_interrupted,
            )
        }));

        let (reason, body_error) = match body {
            Ok(Ok(reason)) => (reason, None),
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

fn run_host_loop<R, W>(
    runner: &FallbackRunner,
    reader: R,
    writer: &mut W,
    interrupts: Receiver<()>,
    previous_session_interrupted: bool,
) -> Result<ShutdownReason, UiError>
where
    R: BufRead + Send + 'static,
    W: Write,
{
    if previous_session_interrupted {
        TextRenderer::render_previous_session_warning(writer).map_err(|_| UiError::Write)?;
    }

    let (line_sender, line_receiver) = bounded(1);
    thread::Builder::new()
        .name("fallback-input".to_owned())
        .spawn(move || {
            let mut reader = BoundedLineReader::new(reader);
            loop {
                let line = reader.next_line();
                let finished = !matches!(line, Ok(Some(_)));
                if line_sender.send(line).is_err() || finished {
                    return;
                }
            }
        })
        .map_err(|_| UiError::ReaderThread)?;

    loop {
        runner.prompt(writer)?;
        select_biased! {
            recv(interrupts) -> signal => {
                if signal.is_ok() {
                    TextRenderer::render_shutdown_reason(ShutdownReason::Interrupted, writer)
                        .map_err(|_| UiError::Write)?;
                    return Ok(ShutdownReason::Interrupted);
                }
            }
            recv(line_receiver) -> line => match line {
                Ok(Ok(Some(line))) => {
                    if let Some(reason) = runner.process_line(line, writer)? {
                        return Ok(reason);
                    }
                }
                Ok(Ok(None)) => return Ok(ShutdownReason::InputClosed),
                Ok(Err(_)) => return Err(UiError::Read),
                Err(_) => return Err(UiError::ReaderThread),
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
) -> Result<ShutdownReason, UiError> {
    let interrupts = interrupt_receiver()?;
    let reader = BufReader::new(io::stdin());
    let stdout = io::stdout();
    FallbackHost::new(runtime, true, previous_session_interrupted).run(
        reader,
        stdout.lock(),
        interrupts,
    )
}
