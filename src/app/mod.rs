//! Future app boundary. Behavior begins in Phase 2.
mod command;
mod event;
mod outcome;
mod service;

use thiserror::Error;

use crate::policy::{Capability, PolicyDecision};

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
    AuditTailView, CommandOutcome, CommandView, HelpView, InputRejectedView, SetupStatusView,
    ShutdownDisposition, ShutdownView, StatusView,
};
pub use service::{
    ApplicationService, AuthorizationDecision, CommandPolicy, CommandTransactionHook,
    NoopCommandTransactionHook,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AppError {
    #[error("persistence operation failed: {0}")]
    Persistence(#[from] crate::persistence::PersistenceError),
    #[error("event recovery failed: {0}")]
    Recovery(#[from] crate::persistence::RecoveryError),
    #[error("command capability is denied")]
    CapabilityDenied {
        capability: Capability,
        decision: PolicyDecision,
    },
    #[error("command requires approval")]
    ApprovalRequired { capability: Capability },
    #[error("command ID conflicts with a different request")]
    CommandConflict,
    #[error("application lifecycle is already finished")]
    LifecycleFinished,
}
