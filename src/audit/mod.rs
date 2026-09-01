use serde::Serialize;

use crate::{
    app::{ApplicationEvent, EventEnvelope},
    domain::{Actor, CorrelationId},
};

pub const MODULE_NAME: &str = "audit";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditEntry {
    pub sequence: u64,
    pub occurred_at_ms: i64,
    pub actor: Actor,
    pub kind: String,
    pub correlation_id: CorrelationId,
    pub summary: String,
}

impl AuditEntry {
    pub fn from_event(envelope: &EventEnvelope) -> Self {
        Self {
            sequence: envelope.sequence,
            occurred_at_ms: envelope.occurred_at_ms,
            actor: envelope.actor.clone(),
            kind: envelope.event.kind().to_owned(),
            correlation_id: envelope.correlation_id,
            summary: summary(&envelope.event),
        }
    }
}

fn summary(event: &ApplicationEvent) -> String {
    match event {
        ApplicationEvent::InstallationInitialized { installation_id } => {
            format!("installation initialized: {installation_id}")
        }
        ApplicationEvent::ProcessSessionStarted { session_id } => {
            format!("process session started: {session_id}")
        }
        ApplicationEvent::PreviousSessionInterrupted { session_id } => {
            format!("previous session interrupted: {session_id}")
        }
        ApplicationEvent::HelpViewed => "help viewed".to_owned(),
        ApplicationEvent::StatusViewed => "status viewed".to_owned(),
        ApplicationEvent::SetupStatusViewed => "setup status viewed".to_owned(),
        ApplicationEvent::AuditTailViewed { limit } => format!("audit tail viewed: {}", limit.get()),
        ApplicationEvent::CommandRejected { rejection } => format!(
            "command rejected: category={:?}, token={}, bytes={}",
            rejection.category,
            rejection.safe_token.as_deref().unwrap_or("none"),
            rejection.byte_length,
        ),
        ApplicationEvent::ShutdownRequested => "shutdown requested".to_owned(),
        ApplicationEvent::ProcessSessionEnded { session_id, reason } => {
            format!("process session ended: {session_id}, reason={reason:?}")
        }
        ApplicationEvent::ProjectionRebuilt { through_sequence } => {
            format!("projection rebuilt through sequence {through_sequence}")
        }
    }
}
