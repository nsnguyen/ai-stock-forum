use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::{Actor, ApprovalId, ObjectRef};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalAction {
    DiscussionRun,
    McpUse,
    EngineeringJobRun,
    GitMerge,
    GitPush,
    FinanceRecommendation,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
}

impl ApprovalStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalResolution {
    pub status: ApprovalStatus,
    pub actor: Actor,
    pub resolved_at_millis: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub approval_id: ApprovalId,
    pub action: ApprovalAction,
    pub object: ObjectRef,
    pub actor: Actor,
    pub status: ApprovalStatus,
    pub created_at_millis: i64,
    pub expires_at_millis: Option<i64>,
    pub resolution: Option<ApprovalResolution>,
}

impl ApprovalRecord {
    pub fn builder(action: ApprovalAction) -> ApprovalRecordBuilder {
        ApprovalRecordBuilder {
            action,
            approval_id: None,
            object: None,
            actor: None,
            created_at_millis: None,
            expires_at_millis: None,
            status: ApprovalStatus::Pending,
            resolution: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApprovalRecordBuilder {
    action: ApprovalAction,
    approval_id: Option<ApprovalId>,
    object: Option<ObjectRef>,
    actor: Option<Actor>,
    created_at_millis: Option<i64>,
    expires_at_millis: Option<i64>,
    status: ApprovalStatus,
    resolution: Option<ApprovalResolution>,
}

impl ApprovalRecordBuilder {
    pub fn approval_id(mut self, approval_id: ApprovalId) -> Self {
        self.approval_id = Some(approval_id);
        self
    }

    pub fn object(mut self, object: ObjectRef) -> Self {
        self.object = Some(object);
        self
    }

    pub fn actor(mut self, actor: Actor) -> Self {
        self.actor = Some(actor);
        self
    }

    pub fn created_at_millis(mut self, created_at_millis: i64) -> Self {
        self.created_at_millis = Some(created_at_millis);
        self
    }

    pub fn expires_at_millis(mut self, expires_at_millis: i64) -> Self {
        self.expires_at_millis = Some(expires_at_millis);
        self
    }

    pub fn status(mut self, status: ApprovalStatus) -> Self {
        self.status = status;
        self
    }

    pub fn resolution(mut self, resolution: ApprovalResolution) -> Self {
        self.resolution = Some(resolution);
        self
    }

    pub fn build(self) -> Result<ApprovalRecord, ApprovalError> {
        let approval_id = self.approval_id.ok_or(ApprovalError::MissingApprovalId)?;
        let object = self.object.ok_or(ApprovalError::MissingObject)?;
        let actor = self.actor.ok_or(ApprovalError::MissingActor)?;
        let created_at_millis = self
            .created_at_millis
            .ok_or(ApprovalError::MissingCreationTimestamp)?;

        if self.status != ApprovalStatus::Pending {
            return Err(ApprovalError::InitialStatusMustBePending);
        }
        if self.resolution.is_some() {
            return Err(ApprovalError::InitialResolutionNotAllowed);
        }
        if self
            .expires_at_millis
            .is_some_and(|expiry| expiry <= created_at_millis)
        {
            return Err(ApprovalError::ExpiryMustFollowCreation);
        }

        Ok(ApprovalRecord {
            approval_id,
            action: self.action,
            object,
            actor,
            status: self.status,
            created_at_millis,
            expires_at_millis: self.expires_at_millis,
            resolution: self.resolution,
        })
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Error)]
pub enum ApprovalError {
    #[error("approval record requires an approval ID")]
    MissingApprovalId,
    #[error("approval record requires an exact object reference")]
    MissingObject,
    #[error("approval record requires an actor")]
    MissingActor,
    #[error("approval record requires a creation timestamp")]
    MissingCreationTimestamp,
    #[error("approval records must be created pending")]
    InitialStatusMustBePending,
    #[error("approval records cannot be created with a resolution")]
    InitialResolutionNotAllowed,
    #[error("approval expiry must be later than creation")]
    ExpiryMustFollowCreation,
}
