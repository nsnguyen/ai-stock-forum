mod controller;
mod error;
mod event;
mod host;
pub mod layout;
pub mod model;
pub mod render;
mod terminal;
pub mod theme;
pub mod views;

pub use controller::{ControllerEffect, apply_outcome, handle_event};
pub use error::TuiError;
pub use event::{CrosstermEventSource, EventSource, TuiEvent};
pub use host::run_tui;
pub use terminal::{CrosstermScreen, Screen};
