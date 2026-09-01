//! Future app boundary. Behavior begins in Phase 2.
mod command;
mod outcome;

pub const MODULE_NAME: &str = "app";

pub use command::{
    ApplicationCommand, AuditLimit, AuditLimitError, CommandEnvelope, InputRejection,
    InputRejectionCategory, DEFAULT_AUDIT_LIMIT, MAX_AUDIT_LIMIT, MAX_INPUT_BYTES,
};
pub use outcome::{
    AuditTailView, CommandView, HelpView, InputRejectedView, SetupStatusView, ShutdownDisposition,
    ShutdownView, StatusView,
};
