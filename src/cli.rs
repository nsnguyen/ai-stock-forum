use std::ffi::OsString;
use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Tui,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    #[error("invalid command-line arguments")]
    InvalidArguments,
}

pub fn select_launch_mode(
    args: &[OsString],
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
) -> Result<LaunchMode, CliError> {
    match args {
        [] if stdin_is_terminal && stdout_is_terminal => Ok(LaunchMode::Tui),
        [] => Ok(LaunchMode::Command),
        [arg] if arg == "--command-mode" => Ok(LaunchMode::Command),
        _ => Err(CliError::InvalidArguments),
    }
}

pub fn detect_launch_mode() -> Result<LaunchMode, CliError> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    select_launch_mode(&args, std::io::stdin().is_terminal(), std::io::stdout().is_terminal())
}
