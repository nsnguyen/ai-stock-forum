use serde::{Deserialize, Deserializer, Serialize};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalResolution {
    status: ApprovalStatus,
    actor: Actor,
    resolved_at_millis: i64,
}

impl ApprovalResolution {
    pub fn new(
        status: ApprovalStatus,
        actor: Actor,
        resolved_at_millis: i64,
    ) -> Result<Self, ApprovalError> {
        if !status.is_terminal() {
            return Err(ApprovalError::ResolutionMustBeTerminal);
        }

        Ok(Self {
            status,
            actor,
            resolved_at_millis,
        })
    }

    pub fn status(&self) -> ApprovalStatus {
        self.status
    }

    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    pub fn resolved_at_millis(&self) -> i64 {
        self.resolved_at_millis
    }
}

#[derive(Deserialize)]
struct ApprovalResolutionWire {
    status: ApprovalStatus,
    actor: Actor,
    resolved_at_millis: i64,
}

impl TryFrom<ApprovalResolutionWire> for ApprovalResolution {
    type Error = ApprovalError;

    fn try_from(wire: ApprovalResolutionWire) -> Result<Self, Self::Error> {
        Self::new(wire.status, wire.actor, wire.resolved_at_millis)
    }
}

impl<'de> Deserialize<'de> for ApprovalResolution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        ApprovalResolutionWire::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApprovalRecord {
    approval_id: ApprovalId,
    action: ApprovalAction,
    object: ObjectRef,
    actor: Actor,
    status: ApprovalStatus,
    created_at_millis: i64,
    expires_at_millis: Option<i64>,
    resolution: Option<ApprovalResolution>,
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

    pub fn approval_id(&self) -> ApprovalId {
        self.approval_id
    }

    pub fn action(&self) -> ApprovalAction {
        self.action
    }

    pub fn object(&self) -> &ObjectRef {
        &self.object
    }

    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    pub fn status(&self) -> ApprovalStatus {
        self.status
    }

    pub fn created_at_millis(&self) -> i64 {
        self.created_at_millis
    }

    pub fn expires_at_millis(&self) -> Option<i64> {
        self.expires_at_millis
    }

    pub fn resolution(&self) -> Option<&ApprovalResolution> {
        self.resolution.as_ref()
    }
}

#[derive(Deserialize)]
struct ApprovalRecordWire {
    approval_id: ApprovalId,
    action: ApprovalAction,
    object: ObjectRef,
    actor: Actor,
    status: ApprovalStatus,
    created_at_millis: i64,
    expires_at_millis: Option<i64>,
    resolution: Option<ApprovalResolution>,
}

impl TryFrom<ApprovalRecordWire> for ApprovalRecord {
    type Error = ApprovalError;

    fn try_from(wire: ApprovalRecordWire) -> Result<Self, Self::Error> {
        validate_persisted_state(wire.status, wire.resolution.as_ref())?;
        validate_expiry(wire.created_at_millis, wire.expires_at_millis)?;

        Ok(Self {
            approval_id: wire.approval_id,
            action: wire.action,
            object: wire.object,
            actor: wire.actor,
            status: wire.status,
            created_at_millis: wire.created_at_millis,
            expires_at_millis: wire.expires_at_millis,
            resolution: wire.resolution,
        })
    }
}

impl<'de> Deserialize<'de> for ApprovalRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        ApprovalRecordWire::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
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

        validate_pending_creation(self.status, self.resolution.as_ref())?;
        validate_expiry(created_at_millis, self.expires_at_millis)?;

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

fn validate_pending_creation(
    status: ApprovalStatus,
    resolution: Option<&ApprovalResolution>,
) -> Result<(), ApprovalError> {
    if status != ApprovalStatus::Pending {
        return Err(ApprovalError::InitialStatusMustBePending);
    }
    if resolution.is_some() {
        return Err(ApprovalError::InitialResolutionNotAllowed);
    }

    Ok(())
}

fn validate_persisted_state(
    status: ApprovalStatus,
    resolution: Option<&ApprovalResolution>,
) -> Result<(), ApprovalError> {
    match (status, resolution) {
        (ApprovalStatus::Pending, None) => Ok(()),
        (ApprovalStatus::Pending, Some(_)) => Err(ApprovalError::PendingRecordHasResolution),
        (_, None) => Err(ApprovalError::TerminalRecordMissingResolution),
        (status, Some(resolution)) if status == resolution.status() => Ok(()),
        (_, Some(_)) => Err(ApprovalError::ResolutionStatusMismatch),
    }
}

fn validate_expiry(
    created_at_millis: i64,
    expires_at_millis: Option<i64>,
) -> Result<(), ApprovalError> {
    if expires_at_millis.is_some_and(|expiry| expiry <= created_at_millis) {
        return Err(ApprovalError::ExpiryMustFollowCreation);
    }

    Ok(())
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
    #[error("approval resolution status must be terminal")]
    ResolutionMustBeTerminal,
    #[error("pending approval records cannot carry a resolution")]
    PendingRecordHasResolution,
    #[error("terminal approval records require a matching resolution")]
    TerminalRecordMissingResolution,
    #[error("approval resolution status must match record status")]
    ResolutionStatusMismatch,
    #[error("approval expiry must be later than creation")]
    ExpiryMustFollowCreation,
}
