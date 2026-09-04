use std::{panic::AssertUnwindSafe, process::ExitCode, sync::Arc};

mod panic_boundary;

use ai_stock_forum::{
    app::{ApplicationService, AuditLimit, DEFAULT_AUDIT_LIMIT, ShutdownReason},
    cli::{CliError, LaunchMode, detect_launch_mode},
    config::{AppPaths, StartupError},
    domain::{SystemClock, UuidGenerator},
    runtime::{ApplicationRuntime, DEFAULT_QUEUE_CAPACITY, RuntimeError},
    ui::{
        command::{StdioResources, TextRenderer, UiError, run_stdio},
        tui::{TuiError, run_tui},
    },
};

enum MainError {
    Cli(CliError),
    Startup(StartupError),
    Runtime(RuntimeError),
    Ui(UiError),
    Tui(TuiError),
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let stderr = std::io::stderr();
            let mut stderr = stderr.lock();
            let _ = match error {
                MainError::Cli(error) => TextRenderer::render_cli_error(&error, &mut stderr),
                MainError::Startup(error) => TextRenderer::render_startup_error(error, &mut stderr),
                MainError::Runtime(error) => {
                    TextRenderer::render_runtime_error(&error, &mut stderr)
                }
                MainError::Ui(error) => TextRenderer::render_ui_error(&error, &mut stderr),
                MainError::Tui(error) => TextRenderer::render_tui_error(&error, &mut stderr),
            };
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), MainError> {
    let mode = detect_launch_mode().map_err(MainError::Cli)?;
    let paths = AppPaths::discover().map_err(MainError::Startup)?;
    let service =
        ApplicationService::bootstrap(&paths, Arc::new(SystemClock), Arc::new(UuidGenerator))
            .map_err(MainError::Startup)?;
    let previous_session_interrupted = service.previous_session_interrupted();

    match mode {
        LaunchMode::Command => run_command_mode(service, previous_session_interrupted),
        LaunchMode::Tui => run_full_screen(service, previous_session_interrupted),
    }
}

fn run_command_mode(
    mut service: ApplicationService,
    previous_session_interrupted: bool,
) -> Result<(), MainError> {
    let stdio = match StdioResources::initialize() {
        Ok(stdio) => stdio,
        Err(error) => {
            return preserve_primary_after_cleanup(error, || {
                service.finish(ShutdownReason::ApplicationError)
            })
            .map_err(MainError::Ui);
        }
    };
    let runtime = ApplicationRuntime::spawn_application(service, DEFAULT_QUEUE_CAPACITY)
        .map_err(MainError::Runtime)?;
    run_stdio(runtime, previous_session_interrupted, stdio).map_err(MainError::Ui)?;
    Ok(())
}

fn run_full_screen(
    mut service: ApplicationService,
    previous_session_interrupted: bool,
) -> Result<(), MainError> {
    let default_audit_limit =
        AuditLimit::new(DEFAULT_AUDIT_LIMIT).expect("the default audit limit must remain valid");
    let snapshot = match service.presentation_snapshot(default_audit_limit) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return preserve_primary_after_cleanup(RuntimeError::Application(error), || {
                service.finish(ShutdownReason::ApplicationError)
            })
            .map_err(MainError::Runtime);
        }
    };
    let runtime = ApplicationRuntime::spawn_application(service, DEFAULT_QUEUE_CAPACITY)
        .map_err(MainError::Runtime)?;
    run_tui(runtime, snapshot, previous_session_interrupted).map_err(MainError::Tui)
}

fn preserve_primary_after_cleanup<E, F, R>(primary: E, cleanup: F) -> Result<(), E>
where
    F: FnOnce() -> R,
{
    let _ = panic_boundary::catch_sensitive_unwind(AssertUnwindSafe(cleanup));
    Err(primary)
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, process::Command};

    #[derive(Debug, PartialEq, Eq)]
    enum PrimaryError {
        Stdio,
        Snapshot,
    }

    const CLEANUP_PANIC_CHILD: &str = "AI_STOCK_FORUM_MAIN_CLEANUP_PANIC_CHILD";
    const SECRET: &str = "credential=pre-runtime-cleanup-panic";
    const SAFE_PRIMARY_LINE: &str = "Application command failed.\n";

    #[test]
    fn cleanup_error_is_ignored_while_the_primary_error_is_preserved_once() {
        let attempts = Cell::new(0);

        let result = super::preserve_primary_after_cleanup(PrimaryError::Stdio, || {
            attempts.set(attempts.get() + 1);
            Err::<(), _>("cleanup failed")
        });

        assert_eq!(result, Err(PrimaryError::Stdio));
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn cleanup_panic_is_redacted_while_the_primary_error_is_preserved_once() {
        if std::env::var_os(CLEANUP_PANIC_CHILD).is_some() {
            let attempts = Cell::new(0);
            let result = super::preserve_primary_after_cleanup(PrimaryError::Snapshot, || {
                attempts.set(attempts.get() + 1);
                panic!("{SECRET}");
            });
            assert_eq!(result, Err(PrimaryError::Snapshot));
            assert_eq!(attempts.get(), 1);
            eprint!("{SAFE_PRIMARY_LINE}");
            std::process::exit(0);
        }

        let output = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "tests::cleanup_panic_is_redacted_while_the_primary_error_is_preserved_once",
                "--nocapture",
            ])
            .env(CLEANUP_PANIC_CHILD, "1")
            .output()
            .expect("cleanup panic child should run");

        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
        assert_eq!(stderr, SAFE_PRIMARY_LINE);
        assert!(!stderr.contains(SECRET));
    }
}
