//! Future app boundary. Behavior begins in Phase 2.
mod command;
mod event;
mod outcome;
mod service;

use thiserror::Error;

use crate::{
    domain::DomainError,
    policy::{Capability, PolicyDecision},
};

pub const MODULE_NAME: &str = "app";

pub use command::{
    ApplicationCommand, AuditLimit, AuditLimitError, CommandEnvelope, DEFAULT_AUDIT_LIMIT,
    InputRejection, InputRejectionCategory, MAX_AUDIT_LIMIT, MAX_INPUT_BYTES, MAX_SAFE_TOKEN_CHARS,
    SafeToken, SafeTokenError,
};
pub(crate) use event::envelope_from_pending;
pub use event::{
    ApplicationEvent, EVENT_SCHEMA_VERSION, EventEnvelope, EventEnvelopeWire, PendingEvent,
    ShutdownReason,
};
pub use outcome::{
    AgentProfileCreatedView, AgentProfileHistoryEntry, AgentProfileHistoryView,
    AgentProfileSummary, AgentProfileVersionActivatedView, AgentProfileView, AgentProfilesView,
    AuditTailView, CommandOutcome, CommandView, HelpView, InputRejectedView, SetupStatusView,
    ShutdownDisposition, ShutdownView, StatusView,
};
pub use service::{
    ApplicationService, ApplicationWorker, AuthorizationDecision, CommandPolicy,
    CommandTransactionHook, DatabaseReadiness, NoopCommandTransactionHook, PresentationSnapshot,
    ProcessGuardOwnership,
};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AppError {
    #[error("agent profile domain validation failed: {0}")]
    Domain(#[from] DomainError),
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
    #[error("agent profile was not found")]
    AgentProfileNotFound,
    #[error("an active agent profile already uses that normalized name")]
    DuplicateProfileName,
    #[error("the active agent profile version is stale")]
    StaleAgentProfileVersion,
    #[error("the profile edit review is unavailable")]
    ProfileReviewUnavailable,
    #[error("the profile edit review does not match the activation")]
    ProfileReviewMismatch,
    #[error("the profile edit review digest does not match")]
    ReviewDigestMismatch,
    #[error("application lifecycle is already finished")]
    LifecycleFinished,
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Domain(error) => error.code(),
            Self::Persistence(error) => error.code(),
            Self::Recovery(error) => error.code(),
            Self::CapabilityDenied { .. } => "capability_denied",
            Self::ApprovalRequired { .. } => "approval_required",
            Self::CommandConflict => "command_conflict",
            Self::AgentProfileNotFound => "unknown_profile",
            Self::DuplicateProfileName => "duplicate_profile_name",
            Self::StaleAgentProfileVersion => "stale_profile_version",
            Self::ProfileReviewUnavailable => "profile_review_unavailable",
            Self::ProfileReviewMismatch => "profile_review_mismatch",
            Self::ReviewDigestMismatch => "review_digest_mismatch",
            Self::LifecycleFinished => "lifecycle_finished",
        }
    }
}
