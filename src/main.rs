use std::{process::ExitCode, sync::Arc};

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
            let _ = service.finish(ShutdownReason::ApplicationError);
            return Err(MainError::Ui(error));
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
    let default_audit_limit = AuditLimit::new(DEFAULT_AUDIT_LIMIT)
        .expect("the default audit limit must remain valid");
    let snapshot = match service.presentation_snapshot(default_audit_limit) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = service.finish(ShutdownReason::ApplicationError);
            return Err(MainError::Runtime(RuntimeError::Application(error)));
        }
    };
    let runtime = ApplicationRuntime::spawn_application(service, DEFAULT_QUEUE_CAPACITY)
        .map_err(MainError::Runtime)?;
    run_tui(runtime, snapshot, previous_session_interrupted).map_err(MainError::Tui)
}
