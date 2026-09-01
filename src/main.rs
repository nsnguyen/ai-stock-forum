use std::{process::ExitCode, sync::Arc};

use ai_stock_forum::{
    app::ApplicationService,
    config::{AppPaths, StartupError},
    domain::{SystemClock, UuidGenerator},
    runtime::{ApplicationRuntime, RuntimeError, DEFAULT_QUEUE_CAPACITY},
    ui::command::{run_stdio, StdioResources, TextRenderer, UiError},
};

enum MainError {
    Startup(StartupError),
    Runtime(RuntimeError),
    Ui(UiError),
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let stderr = std::io::stderr();
            let mut stderr = stderr.lock();
            let _ = match error {
                MainError::Startup(error) => TextRenderer::render_startup_error(error, &mut stderr),
                MainError::Runtime(error) => TextRenderer::render_runtime_error(&error, &mut stderr),
                MainError::Ui(error) => TextRenderer::render_ui_error(&error, &mut stderr),
            };
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), MainError> {
    let stdio = StdioResources::initialize().map_err(MainError::Ui)?;
    let paths = AppPaths::discover().map_err(MainError::Startup)?;
    let service = ApplicationService::bootstrap(
        &paths,
        Arc::new(SystemClock),
        Arc::new(UuidGenerator),
    )
    .map_err(MainError::Startup)?;
    let previous_session_interrupted = service.previous_session_interrupted();
    let runtime = ApplicationRuntime::spawn_application(service, DEFAULT_QUEUE_CAPACITY)
        .map_err(MainError::Runtime)?;
    run_stdio(runtime, previous_session_interrupted, stdio).map_err(MainError::Ui)?;
    Ok(())
}
