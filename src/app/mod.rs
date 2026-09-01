//! Future app boundary. Behavior begins in Phase 2.
mod command;
mod event;
mod outcome;

pub const MODULE_NAME: &str = "app";

pub use command::{
    ApplicationCommand, AuditLimit, AuditLimitError, CommandEnvelope, InputRejection,
    InputRejectionCategory, SafeToken, SafeTokenError, DEFAULT_AUDIT_LIMIT, MAX_AUDIT_LIMIT,
    MAX_INPUT_BYTES, MAX_SAFE_TOKEN_CHARS,
};
pub use event::{
    ApplicationEvent, EventEnvelope, EventEnvelopeWire, PendingEvent, ShutdownReason,
    EVENT_SCHEMA_VERSION,
};
pub(crate) use event::envelope_from_pending;
pub use outcome::{
    AuditTailView, CommandView, HelpView, InputRejectedView, SetupStatusView, ShutdownDisposition,
    ShutdownView, StatusView,
};
