use crate::{
    app::{AuditLimit, EventEnvelope, InputRejection},
    audit::AuditEntry,
    domain::{CommandId, CorrelationId, InstallationId, SessionId},
    setup::SetupStatus,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutcome {
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub committed_events: Vec<EventEnvelope>,
    pub view: CommandView,
    pub shutdown: ShutdownDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpView;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusView {
    pub installation_id: InstallationId,
    pub session_id: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupStatusView {
    pub status: SetupStatus,
}

impl SetupStatusView {
    pub fn is_not_started(&self) -> bool {
        self.status == SetupStatus::NotStarted
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditTailView {
    pub limit: AuditLimit,
    pub entries: Vec<AuditEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRejectedView {
    pub rejection: InputRejection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownView {
    pub disposition: ShutdownDisposition,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownDisposition {
    Continue,
    Requested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CommandView {
    Help(HelpView),
    Status(StatusView),
    SetupStatus(SetupStatusView),
    AuditTail(AuditTailView),
    InputRejected(InputRejectedView),
    Shutdown(ShutdownView),
}
