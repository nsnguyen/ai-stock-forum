//! Future app boundary. Behavior begins in Phase 2.
mod command;
mod event;
mod outcome;

pub const MODULE_NAME: &str = "app";

pub use command::{
    ApplicationCommand, AuditLimit, AuditLimitError, CommandEnvelope, InputRejection,
    InputRejectionCategory, DEFAULT_AUDIT_LIMIT, MAX_AUDIT_LIMIT, MAX_INPUT_BYTES,
};
pub use event::{
    ApplicationEvent, EventEnvelope, PendingEvent, ShutdownReason, EVENT_SCHEMA_VERSION,
};
pub use outcome::{
    AuditTailView, CommandView, HelpView, InputRejectedView, SetupStatusView, ShutdownDisposition,
    ShutdownView, StatusView,
};
