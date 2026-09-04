use std::ffi::OsString;

use ai_stock_forum::cli::{CliError, LaunchMode, select_launch_mode};

#[test]
fn interactive_input_and_output_select_the_tui() {
    assert_eq!(select_launch_mode(&[], true, true), Ok(LaunchMode::Tui));
}

#[test]
fn command_mode_flag_always_selects_the_line_host() {
    let args = [OsString::from("--command-mode")];
    assert_eq!(select_launch_mode(&args, true, true), Ok(LaunchMode::Command));
}

#[test]
fn either_redirected_stream_selects_the_line_host() {
    assert_eq!(select_launch_mode(&[], false, true), Ok(LaunchMode::Command));
    assert_eq!(select_launch_mode(&[], true, false), Ok(LaunchMode::Command));
    assert_eq!(select_launch_mode(&[], false, false), Ok(LaunchMode::Command));
}

#[test]
fn unknown_or_repeated_arguments_are_rejected() {
    assert_eq!(
        select_launch_mode(&[OsString::from("--unknown")], true, true),
        Err(CliError::InvalidArguments)
    );
    assert_eq!(
        select_launch_mode(
            &[OsString::from("--command-mode"), OsString::from("--command-mode")],
            true,
            true,
        ),
        Err(CliError::InvalidArguments)
    );
}
