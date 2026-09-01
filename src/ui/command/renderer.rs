use std::io::{self, Write};

use crate::{
    app::{
        AppError, CommandOutcome, CommandView, InputRejectionCategory, ShutdownDisposition,
        ShutdownReason,
    },
    config::StartupError,
    domain::Actor,
    runtime::RuntimeError,
    setup::SetupStatus,
};

pub struct TextRenderer;

impl TextRenderer {
    pub fn render_outcome<W: Write>(outcome: &CommandOutcome, writer: &mut W) -> io::Result<()> {
        Self::render_view(&outcome.view, writer)
    }

    pub fn render_view<W: Write>(view: &CommandView, writer: &mut W) -> io::Result<()> {
        match view {
            CommandView::Help(_) => writer.write_all(
                b"Available commands:\n  /help\n  /status\n  /setup status\n  /audit tail [limit: 1-100]\n  /quit\n",
            ),
            CommandView::Status(_) => {
                writer.write_all(b"Installation: ready\nSession: active\n")
            }
            CommandView::SetupStatus(view) => match &view.status {
                SetupStatus::NotStarted => writer.write_all(
                    b"Setup: not started\nGuided setup is not implemented in Phase 0.\n",
                ),
                SetupStatus::DraftSaved { .. } => writer.write_all(b"Setup: draft saved\n"),
                SetupStatus::Applied { .. } => writer.write_all(b"Setup: applied\n"),
            },
            CommandView::AuditTail(view) => {
                writeln!(writer, "Audit tail (limit {}):", view.limit.get())?;
                if view.entries.is_empty() {
                    return writer.write_all(b"  No audit entries.\n");
                }
                for entry in &view.entries {
                    let actor = match entry.actor {
                        Actor::Human => "human",
                        Actor::System => "system",
                    };
                    let kind = escaped_bounded(&entry.kind, 64);
                    let summary = escaped_bounded(&entry.summary, 256);
                    writeln!(
                        writer,
                        "  #{} {} {} {} {} {}",
                        entry.sequence,
                        entry.occurred_at_ms,
                        actor,
                        kind,
                        entry.correlation_id,
                        summary,
                    )?;
                }
                Ok(())
            }
            CommandView::InputRejected(view) => match view.rejection.category {
                InputRejectionCategory::InvalidEncoding => {
                    writer.write_all(b"Input rejected: invalid encoding.\n")
                }
                InputRejectionCategory::Oversized => {
                    writer.write_all(b"Input rejected: input exceeds 4096 bytes.\n")
                }
                InputRejectionCategory::Malformed => {
                    writer.write_all(b"Input rejected: malformed command.\n")
                }
                InputRejectionCategory::Unknown => {
                    if let Some(token) = &view.rejection.safe_token {
                        writeln!(
                            writer,
                            "Input rejected: unknown command {}.",
                            escaped_bounded(token.as_str(), 64)
                        )
                    } else {
                        writer.write_all(b"Input rejected: unknown command.\n")
                    }
                }
            },
            CommandView::Shutdown(view) => match view.disposition {
                ShutdownDisposition::Continue => {
                    writer.write_all(b"Shutdown was not requested.\n")
                }
                ShutdownDisposition::Requested => writer.write_all(b"Shutting down.\n"),
            },
        }
    }

    pub fn render_startup_error<W: Write>(
        error: StartupError,
        writer: &mut W,
    ) -> io::Result<()> {
        writeln!(writer, "Startup failed [{}].", error.code())
    }

    pub fn render_runtime_error<W: Write>(
        error: &RuntimeError,
        writer: &mut W,
    ) -> io::Result<()> {
        let message = match error {
            RuntimeError::InvalidCapacity => "Runtime configuration is invalid.",
            RuntimeError::Backpressure => "Command queue is busy; try again.",
            RuntimeError::Closed => "Application is shutting down.",
            RuntimeError::Application(error) => app_error_message(error),
            RuntimeError::WorkerStartup => "Application worker could not start.",
            RuntimeError::WorkerExited | RuntimeError::WorkerPanicked => {
                "Application worker stopped unexpectedly."
            }
            RuntimeError::TerminationTimedOut => "Application worker did not stop in time.",
        };
        writeln!(writer, "{message}")
    }

    pub fn render_shutdown_reason<W: Write>(
        reason: ShutdownReason,
        writer: &mut W,
    ) -> io::Result<()> {
        let message = match reason {
            ShutdownReason::UserQuit => "Shutting down.",
            ShutdownReason::InputClosed => "Input closed. Shutting down.",
            ShutdownReason::Interrupted => "Interrupted. Shutting down.",
            ShutdownReason::ApplicationError => "Stopping after an application error.",
        };
        writeln!(writer, "{message}")
    }

    pub fn render_previous_session_warning<W: Write>(writer: &mut W) -> io::Result<()> {
        writer.write_all(b"Warning: the previous session ended unexpectedly.\n")
    }

    pub fn render_ui_error<W: Write>(
        error: &super::UiError,
        writer: &mut W,
    ) -> io::Result<()> {
        match error {
            super::UiError::Read => writer.write_all(b"Input could not be read.\n"),
            super::UiError::Write => writer.write_all(b"Output could not be written.\n"),
            super::UiError::Runtime(error) => Self::render_runtime_error(error, writer),
            super::UiError::InterruptHandler => {
                writer.write_all(b"Interrupt handling could not be started.\n")
            }
            super::UiError::ReaderThread => {
                writer.write_all(b"Input worker stopped unexpectedly.\n")
            }
            super::UiError::LineSourceUnavailable => {
                writer.write_all(b"Terminal input is unavailable on this platform.\n")
            }
            super::UiError::Panicked => {
                writer.write_all(b"Command host stopped unexpectedly.\n")
            }
        }
    }
}

fn app_error_message(error: &AppError) -> &'static str {
    match error {
        AppError::Persistence(_) | AppError::Recovery(_) => "Application command failed.",
        AppError::CapabilityDenied { .. } | AppError::ApprovalRequired { .. } => {
            "Command is unavailable."
        }
        AppError::CommandConflict => "Command could not be completed.",
        AppError::LifecycleFinished => "Application is shutting down.",
    }
}

fn escaped_bounded(value: &str, maximum_scalars: usize) -> String {
    let mut escaped = String::new();
    let mut scalars = 0;
    for character in value.chars() {
        let fragment = character.escape_default().to_string();
        let fragment_scalars = fragment.chars().count();
        if scalars + fragment_scalars > maximum_scalars {
            escaped.push_str("...");
            break;
        }
        escaped.push_str(&fragment);
        scalars += fragment_scalars;
    }
    escaped
}
