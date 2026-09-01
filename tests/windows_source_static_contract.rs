use std::{fs, path::PathBuf};

fn repository_file(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(root.join(path)).expect("contract source must exist")
}

#[test]
fn windows_source_cancels_active_synchronous_reads_with_an_owned_thread_handle() {
    let runner = repository_file("src/ui/command/runner.rs");

    for required in [
        "OpenThread",
        "THREAD_TERMINATE",
        "CancelSynchronousIo",
        "reader_thread",
    ] {
        assert!(
            runner.contains(required),
            "missing Windows cancellation API: {required}"
        );
    }
    assert!(runner.contains("impl Drop for WindowsCancellation"));
    assert!(runner.contains("CloseHandle(reader_thread)"));
}

#[test]
fn windows_cancel_before_pending_uses_a_persistent_acknowledged_retry_protocol() {
    let runner = repository_file("src/ui/command/runner.rs");
    let state = repository_file("src/ui/command/windows.rs");

    for phase in ["IdleWaiting", "AboutToRead", "ReadActive", "Exited"] {
        assert!(state.contains(phase), "missing read phase: {phase}");
    }
    assert!(runner.contains("phase_changed: std::sync::Condvar"));
    assert!(runner.contains("ERROR_NOT_FOUND"));
    assert!(runner.contains("mark_exited"));
    assert!(runner.contains("[self.cancellation.event, self.input]"));
    assert!(runner.contains("if !self.cancellation.begin_read()?"));
    assert!(runner.contains("self.cancellation.end_read()"));
}

#[test]
fn windows_read_errors_have_explicit_cancel_eof_and_error_dispositions() {
    let mapping = repository_file("src/ui/command/windows.rs");

    assert!(mapping.contains("ERROR_OPERATION_ABORTED"));
    assert!(mapping.contains("ERROR_BROKEN_PIPE"));
    assert!(mapping.contains("ERROR_HANDLE_EOF"));
    assert!(mapping.contains("ReadErrorDisposition::Cancelled"));
    assert!(mapping.contains("ReadErrorDisposition::Eof"));
    assert!(mapping.contains("ReadErrorDisposition::Error"));
}

#[test]
fn windows_bindings_are_target_scoped_with_the_required_api_features() {
    let cargo = repository_file("Cargo.toml");
    let command_module = repository_file("src/ui/command/mod.rs");

    assert!(cargo.contains("[target.\"cfg(windows)\".dependencies]"));
    assert!(cargo.contains("windows-sys"));
    for feature in [
        "Win32_Foundation",
        "Win32_Storage_FileSystem",
        "Win32_System_Console",
        "Win32_System_Threading",
    ] {
        assert!(
            cargo.contains(feature),
            "missing windows-sys feature: {feature}"
        );
    }
    assert!(command_module.contains("#[cfg(any(windows, test))]"));
    assert!(command_module.contains("mod windows;"));
}

#[test]
fn home_and_xdg_binary_state_tests_are_unix_only() {
    let contracts = repository_file("tests/fallback_contract.rs");
    for test_name in [
        "binary_smoke_quit_and_eof_exit_successfully",
        "binary_startup_failure_is_redacted_and_uses_failure_status",
        "binary_prints_previous_session_warning_once_then_finishes_cleanly",
    ] {
        let marker = format!("#[cfg(unix)]\n#[test]\nfn {test_name}");
        assert!(
            contracts.contains(&marker),
            "POSIX state isolation test is not Unix-gated: {test_name}"
        );
    }
}
