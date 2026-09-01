use crate::{app::{AuditLimit, InputRejection}, audit::AuditEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupStatusView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditTailView {
    pub limit: AuditLimit,
    pub entries: Vec<AuditEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputRejectedView {
    pub rejection: InputRejection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownView {
    pub disposition: ShutdownDisposition,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ShutdownDisposition {
    Continue,
    ShutdownRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandView {
    Help(HelpView),
    Status(StatusView),
    SetupStatus(SetupStatusView),
    AuditTail(AuditTailView),
    InputRejected(InputRejectedView),
    Shutdown(ShutdownView),
}
