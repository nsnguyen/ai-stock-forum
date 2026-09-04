use std::{
    ffi::OsString,
    io::Write,
    process::{Command, Output, Stdio},
};

use ai_stock_forum::{
    cli::{CliError, LaunchMode, select_launch_mode},
    runtime::RuntimeError,
    ui::{command::TextRenderer, tui::TuiError},
};
use tempfile::TempDir;

struct Binary {
    command: Command,
    input: Option<Vec<u8>>,
    _state_directory: TempDir,
}

impl Binary {
    fn arg(mut self, argument: &str) -> Self {
        self.command.arg(argument);
        self
    }

    fn write_stdin(mut self, input: &str) -> Self {
        self.input = Some(input.as_bytes().to_vec());
        self
    }

    fn output(mut self) -> Output {
        let mut child = self
            .command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(input) = self.input {
            child.stdin.take().unwrap().write_all(&input).unwrap();
        } else {
            drop(child.stdin.take());
        }
        child.wait_with_output().unwrap()
    }
}

fn binary() -> Binary {
    let state_directory = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ai-stock-forum"));
    command
        .env("HOME", state_directory.path())
        .env("XDG_DATA_HOME", state_directory.path());
    Binary {
        command,
        input: None,
        _state_directory: state_directory,
    }
}

#[test]
fn interactive_input_and_output_select_the_tui() {
    assert_eq!(select_launch_mode(&[], true, true), Ok(LaunchMode::Tui));
}

#[test]
fn command_mode_flag_always_selects_the_line_host() {
    let args = [OsString::from("--command-mode")];
    assert_eq!(
        select_launch_mode(&args, true, true),
        Ok(LaunchMode::Command)
    );
}

#[test]
fn either_redirected_stream_selects_the_line_host() {
    assert_eq!(
        select_launch_mode(&[], false, true),
        Ok(LaunchMode::Command)
    );
    assert_eq!(
        select_launch_mode(&[], true, false),
        Ok(LaunchMode::Command)
    );
    assert_eq!(
        select_launch_mode(&[], false, false),
        Ok(LaunchMode::Command)
    );
}

#[test]
fn unknown_or_repeated_arguments_are_rejected() {
    assert_eq!(
        select_launch_mode(&[OsString::from("--unknown")], true, true),
        Err(CliError::InvalidArguments)
    );
    assert_eq!(
        select_launch_mode(
            &[
                OsString::from("--command-mode"),
                OsString::from("--command-mode")
            ],
            true,
            true,
        ),
        Err(CliError::InvalidArguments)
    );
}

#[test]
fn explicit_command_mode_accepts_the_existing_protocol() {
    let output = binary()
        .arg("--command-mode")
        .write_stdin("/status\n/quit\n")
        .output();

    assert!(output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Installation: ready\nSession: active\n")
    );
}

#[test]
fn redirected_stdio_automatically_preserves_command_mode() {
    let output = binary().write_stdin("/help\n/quit\n").output();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("/status"));
}

#[test]
fn invalid_arguments_emit_one_safe_line_and_fail() {
    let output = binary().arg("--unknown").output();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.lines().count(), 1);
    assert!(!stderr.contains("--unknown"));
}

#[test]
fn cli_errors_render_one_fixed_safe_line() {
    let mut output = Vec::new();

    TextRenderer::render_cli_error(&CliError::InvalidArguments, &mut output).unwrap();

    assert_eq!(output, b"Command-line arguments are invalid.\n");
}

#[test]
fn tui_errors_render_fixed_safe_categories() {
    let cases = [
        (
            TuiError::TerminalInitialization,
            "Terminal interface could not be started.\n",
        ),
        (
            TuiError::TerminalInput,
            "Terminal input could not be read.\n",
        ),
        (
            TuiError::TerminalOutput,
            "Terminal output could not be written.\n",
        ),
        (
            TuiError::InterruptHandler,
            "Interrupt handling could not be started.\n",
        ),
        (
            TuiError::Runtime(RuntimeError::InvalidCapacity),
            "Runtime configuration is invalid.\n",
        ),
        (
            TuiError::Panicked,
            "Terminal interface stopped unexpectedly.\n",
        ),
    ];

    for (error, expected) in cases {
        let mut output = Vec::new();
        TextRenderer::render_tui_error(&error, &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), expected);
    }
}
