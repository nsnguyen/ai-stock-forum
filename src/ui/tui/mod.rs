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
pub use event::{CrosstermEventSource, EventSource, SkillKey, TuiEvent};
pub use host::{execute_agent_effect, execute_skill_effect, run_tui, run_tui_with_screen};
pub use model::{
    AgentSkillAction, AgentsPane, AgentsViewState, AssignmentKind, ProfileConfirmation,
    SkillConfirmation, SkillDetailAction, SkillOperationOrigin, SkillWorkspaceOrigin, SkillsPane,
    SkillsViewState,
};
pub use terminal::{CrosstermScreen, Screen};
