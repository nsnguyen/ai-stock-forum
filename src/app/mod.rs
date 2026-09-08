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
    AgentProfileSelector, AgentSkillAssignmentOperation, ApplicationCommand, AuditLimit,
    AuditLimitError, CommandEnvelope, DEFAULT_AUDIT_LIMIT, InputRejection, InputRejectionCategory,
    MAX_AGENT_PROFILE_HISTORY_RESULTS, MAX_AGENT_PROFILE_LIST_RESULTS, MAX_AUDIT_LIMIT,
    MAX_INPUT_BYTES, MAX_SAFE_TOKEN_CHARS, MAX_SKILL_HISTORY_RESULTS, MAX_SKILL_LIST_RESULTS,
    SafeToken, SafeTokenError, SkillSelector,
};
pub use event::{
    ApplicationEvent, EVENT_SCHEMA_VERSION, EventEnvelope, EventEnvelopeWire,
    MemoryProposalStatusRef, PendingEvent, ShutdownReason, SkillEventSummary,
    SkillHistoryEventEntry,
};
pub(crate) use event::{actor_wire, envelope_from_pending};
pub use outcome::{
    AgentProfileCreatedView, AgentProfileHistoryEntry, AgentProfileHistoryView,
    AgentProfileSummary, AgentProfileVersionActivatedView, AgentProfileVersionView,
    AgentProfileView, AgentProfilesView, AgentSkillAssignmentPreview, AgentSkillMutationView,
    AuditTailView, CommandOutcome, CommandView, EpisodicSummariesView, EpisodicSummaryListItem,
    EpisodicSummaryView, HelpView, InputRejectedView, MemoryEntriesView, MemoryEntryHistorySummary,
    MemoryEntryHistoryView, MemoryEntryMutationView, MemoryEntrySummary, MemoryEntryVersionView,
    MemoryEntryView, MemoryProfileIdentityView, MemoryProposalCreatedView,
    MemoryProposalResolutionView, MemoryProposalSummary, MemoryProposalView, MemoryProposalsView,
    MemorySnapshotView, SetupStatusView, ShutdownDisposition, ShutdownView, SkillCreatedView,
    SkillHistoryEntry, SkillHistoryView, SkillSummary, SkillVersionActivatedView, SkillView,
    SkillsView, StatusView,
};
pub use service::{
    ApplicationService, ApplicationWorker, AuthorizationDecision, CommandPolicy,
    CommandTransactionHook, DatabaseReadiness, IndependentApplicationService, MemoryEditPreview,
    NoopCommandTransactionHook, PresentationSnapshot, ProcessGuardOwnership,
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
    #[error("skill was not found")]
    SkillNotFound,
    #[error("an active skill already uses that normalized name")]
    DuplicateSkillName,
    #[error("the active skill version is stale")]
    StaleSkillVersion,
    #[error("the skill review is unavailable")]
    SkillReviewUnavailable,
    #[error("the skill review does not match the mutation")]
    SkillReviewMismatch,
    #[error("the requested skill is already assigned")]
    SkillAlreadyAssigned,
    #[error("the requested skill is not assigned")]
    SkillNotAssigned,
    #[error("the agent already has the maximum number of skills")]
    AgentSkillLimitExceeded,
    #[error("the profile edit review is unavailable")]
    ProfileReviewUnavailable,
    #[error("the profile edit review does not match the activation")]
    ProfileReviewMismatch,
    #[error("the profile edit review digest does not match")]
    ReviewDigestMismatch,
    #[error("the immutable agent profile history does not match its event history")]
    AgentProfileHistoryMismatch,
    #[error("the selected binding reference is unavailable")]
    BindingReferenceUnavailable,
    #[error("application lifecycle is already finished")]
    LifecycleFinished,
    #[error("memory entry was not found")]
    MemoryEntryNotFound,
    #[error("memory proposal was not found")]
    MemoryProposalNotFound,
    #[error("episodic summary was not found")]
    EpisodicSummaryNotFound,
    #[error("memory command is not implemented")]
    MemoryCommandNotImplemented,
    #[error("wrong memory command dispatcher")]
    WrongMemoryCommandDispatcher,
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
            Self::DuplicateProfileName => "active_name_conflict",
            Self::StaleAgentProfileVersion => "stale_profile_version",
            Self::SkillNotFound => "skill_not_found",
            Self::DuplicateSkillName => "active_skill_name_conflict",
            Self::StaleSkillVersion => "stale_skill_version",
            Self::SkillReviewUnavailable => "skill_review_unavailable",
            Self::SkillReviewMismatch => "skill_review_mismatch",
            Self::SkillAlreadyAssigned => "skill_already_assigned",
            Self::SkillNotAssigned => "skill_not_assigned",
            Self::AgentSkillLimitExceeded => "agent_skill_limit_exceeded",
            Self::ProfileReviewUnavailable => "profile_review_unavailable",
            Self::ProfileReviewMismatch => "profile_review_mismatch",
            Self::ReviewDigestMismatch => "review_digest_mismatch",
            Self::AgentProfileHistoryMismatch => "agent_profile_history_mismatch",
            Self::BindingReferenceUnavailable => "binding_reference_unavailable",
            Self::LifecycleFinished => "lifecycle_finished",
            Self::MemoryEntryNotFound => "memory_entry_not_found",
            Self::MemoryProposalNotFound => "memory_proposal_not_found",
            Self::EpisodicSummaryNotFound => "episodic_summary_not_found",
            Self::MemoryCommandNotImplemented => "memory_command_not_implemented",
            Self::WrongMemoryCommandDispatcher => "wrong_memory_command_dispatcher",
        }
    }
}
