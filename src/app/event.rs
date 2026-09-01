use serde::{Deserialize, Serialize};

use crate::{
    app::{AuditLimit, InputRejection},
    domain::{
        Actor, CausationId, CorrelationId, EventId, InstallationId, ObjectRef, SessionId,
        Sha256Digest,
    },
};

pub const EVENT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownReason {
    UserQuit,
    InputClosed,
    Interrupted,
    ApplicationError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ApplicationEvent {
    InstallationInitialized { installation_id: InstallationId },
    ProcessSessionStarted { session_id: SessionId },
    PreviousSessionInterrupted { session_id: SessionId },
    HelpViewed,
    StatusViewed,
    SetupStatusViewed,
    AuditTailViewed { limit: AuditLimit },
    CommandRejected { rejection: InputRejection },
    ShutdownRequested,
    ProcessSessionEnded { session_id: SessionId, reason: ShutdownReason },
    ProjectionRebuilt { through_sequence: u64 },
}

impl ApplicationEvent {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::InstallationInitialized { .. } => "installation_initialized",
            Self::ProcessSessionStarted { .. } => "process_session_started",
            Self::PreviousSessionInterrupted { .. } => "previous_session_interrupted",
            Self::HelpViewed => "help_viewed",
            Self::StatusViewed => "status_viewed",
            Self::SetupStatusViewed => "setup_status_viewed",
            Self::AuditTailViewed { .. } => "audit_tail_viewed",
            Self::CommandRejected { .. } => "command_rejected",
            Self::ShutdownRequested => "shutdown_requested",
            Self::ProcessSessionEnded { .. } => "process_session_ended",
            Self::ProjectionRebuilt { .. } => "projection_rebuilt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEvent {
    pub event_id: EventId,
    pub event_schema_version: u16,
    pub actor: Actor,
    pub occurred_at_ms: i64,
    pub correlation_id: CorrelationId,
    pub causation_id: Option<CausationId>,
    pub object: Option<ObjectRef>,
    pub event: ApplicationEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub sequence: u64,
    pub event_id: EventId,
    pub event_schema_version: u16,
    pub actor: Actor,
    pub occurred_at_ms: i64,
    pub correlation_id: CorrelationId,
    pub causation_id: Option<CausationId>,
    pub object: Option<ObjectRef>,
    pub event: ApplicationEvent,
    pub previous_event_digest: Option<Sha256Digest>,
    pub event_digest: Sha256Digest,
}
