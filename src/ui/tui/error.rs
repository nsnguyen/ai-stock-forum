use thiserror::Error;

use crate::runtime::RuntimeError;

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("terminal initialization failed")]
    TerminalInitialization,
    #[error("terminal input failed")]
    TerminalInput,
    #[error("terminal output failed")]
    TerminalOutput,
    #[error("interrupt handler unavailable")]
    InterruptHandler,
    #[error("application runtime failed")]
    Runtime(#[from] RuntimeError),
    #[error("terminal interface stopped unexpectedly")]
    Panicked,
}
