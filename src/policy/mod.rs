mod approval;
mod capability;

pub const MODULE_NAME: &str = "policy";

pub use approval::{
    ApprovalAction, ApprovalError, ApprovalRecord, ApprovalRecordBuilder, ApprovalResolution,
    ApprovalStatus,
};
pub use capability::{Capability, Effect, PolicyDecision, PolicyRule, evaluate};
