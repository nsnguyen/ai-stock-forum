mod parser;
mod reader;
mod renderer;
mod runner;
#[cfg(any(windows, test))]
mod windows;

pub use parser::{ParsedLine, parse_line};
pub use reader::{BoundedLineReader, RawLine};
pub use renderer::TextRenderer;
#[cfg(unix)]
pub use runner::UnixLineSource;
pub use runner::{
    CancellableLineSource, FallbackHost, FallbackRunner, LineSourceCancellation, LineSourceEvent,
    StdioResources, UiError, run_stdio,
};
