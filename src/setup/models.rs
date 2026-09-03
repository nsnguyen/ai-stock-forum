use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{ConfigurationVersionId, EventId, ObjectVersion, SetupDraftId, Sha256Digest};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupDraftState {
    Drafting,
    Reviewed,
    Applied,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupPath {
    QuickStart,
    Customize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum SetupStatus {
    #[default]
    NotStarted,
    DraftSaved {
        draft_id: SetupDraftId,
    },
    Applied {
        configuration_id: ConfigurationVersionId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SetupDraftWire")]
pub struct SetupDraft {
    pub draft_id: SetupDraftId,
    pub schema_version: u32,
    pub state: SetupDraftState,
    pub path: SetupPath,
    pub current_review_digest: Option<Sha256Digest>,
    pub payload: Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupDraftWire {
    draft_id: SetupDraftId,
    schema_version: u32,
    state: SetupDraftState,
    path: SetupPath,
    current_review_digest: Option<Sha256Digest>,
    payload: Value,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl SetupDraft {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        draft_id: SetupDraftId,
        schema_version: u32,
        state: SetupDraftState,
        path: SetupPath,
        current_review_digest: Option<Sha256Digest>,
        payload: Value,
        created_at_ms: i64,
        updated_at_ms: i64,
    ) -> Result<Self, SetupModelError> {
        if schema_version == 0 {
            return Err(SetupModelError::InvalidSchemaVersion);
        }
        if updated_at_ms < created_at_ms {
            return Err(SetupModelError::InvalidTimestampOrder);
        }
        Ok(Self {
            draft_id,
            schema_version,
            state,
            path,
            current_review_digest,
            payload,
            created_at_ms,
            updated_at_ms,
        })
    }
}

impl TryFrom<SetupDraftWire> for SetupDraft {
    type Error = SetupModelError;

    fn try_from(value: SetupDraftWire) -> Result<Self, Self::Error> {
        Self::new(
            value.draft_id,
            value.schema_version,
            value.state,
            value.path,
            value.current_review_digest,
            value.payload,
            value.created_at_ms,
            value.updated_at_ms,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "InstallationConfigurationVersionWire")]
pub struct InstallationConfigurationVersion {
    pub configuration_id: ConfigurationVersionId,
    pub version: ObjectVersion,
    pub source_draft_id: SetupDraftId,
    pub review_digest: Sha256Digest,
    pub object_digest: Sha256Digest,
    pub payload: Value,
    pub created_event_id: EventId,
    pub created_at_ms: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallationConfigurationVersionWire {
    configuration_id: ConfigurationVersionId,
    version: ObjectVersion,
    source_draft_id: SetupDraftId,
    review_digest: Sha256Digest,
    object_digest: Sha256Digest,
    payload: Value,
    created_event_id: EventId,
    created_at_ms: i64,
}

impl InstallationConfigurationVersion {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        configuration_id: ConfigurationVersionId,
        version: ObjectVersion,
        source_draft_id: SetupDraftId,
        review_digest: Sha256Digest,
        object_digest: Sha256Digest,
        payload: Value,
        created_event_id: EventId,
        created_at_ms: i64,
    ) -> Self {
        Self {
            configuration_id,
            version,
            source_draft_id,
            review_digest,
            object_digest,
            payload,
            created_event_id,
            created_at_ms,
        }
    }
}

impl TryFrom<InstallationConfigurationVersionWire> for InstallationConfigurationVersion {
    type Error = SetupModelError;

    fn try_from(value: InstallationConfigurationVersionWire) -> Result<Self, Self::Error> {
        Ok(Self::new(
            value.configuration_id,
            value.version,
            value.source_draft_id,
            value.review_digest,
            value.object_digest,
            value.payload,
            value.created_event_id,
            value.created_at_ms,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStepStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SetupStepOutcomeWire")]
pub struct SetupStepOutcome {
    pub draft_id: SetupDraftId,
    pub step_key: String,
    pub attempt: u32,
    pub status: SetupStepStatus,
    pub safe_code: Option<String>,
    pub occurred_at_ms: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupStepOutcomeWire {
    draft_id: SetupDraftId,
    step_key: String,
    attempt: u32,
    status: SetupStepStatus,
    safe_code: Option<String>,
    occurred_at_ms: i64,
}

impl SetupStepOutcome {
    pub fn new(
        draft_id: SetupDraftId,
        step_key: String,
        attempt: u32,
        status: SetupStepStatus,
        safe_code: Option<String>,
        occurred_at_ms: i64,
    ) -> Result<Self, SetupModelError> {
        if step_key.trim().is_empty() || step_key.chars().any(char::is_control) {
            return Err(SetupModelError::InvalidStepKey);
        }
        if attempt == 0 {
            return Err(SetupModelError::InvalidAttempt);
        }
        if safe_code
            .as_deref()
            .is_some_and(|code| code.is_empty() || code.chars().any(char::is_control))
        {
            return Err(SetupModelError::InvalidSafeCode);
        }
        Ok(Self {
            draft_id,
            step_key,
            attempt,
            status,
            safe_code,
            occurred_at_ms,
        })
    }
}

impl TryFrom<SetupStepOutcomeWire> for SetupStepOutcome {
    type Error = SetupModelError;

    fn try_from(value: SetupStepOutcomeWire) -> Result<Self, Self::Error> {
        Self::new(
            value.draft_id,
            value.step_key,
            value.attempt,
            value.status,
            value.safe_code,
            value.occurred_at_ms,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReadinessStatus {
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CapabilityReadinessWire")]
pub struct CapabilityReadiness {
    pub configuration_id: ConfigurationVersionId,
    pub capability: String,
    pub status: CapabilityReadinessStatus,
    pub reason_code: Option<String>,
    pub checked_at_ms: i64,
    pub projection_digest: Sha256Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityReadinessWire {
    configuration_id: ConfigurationVersionId,
    capability: String,
    status: CapabilityReadinessStatus,
    reason_code: Option<String>,
    checked_at_ms: i64,
    projection_digest: Sha256Digest,
}

impl CapabilityReadiness {
    pub fn new(
        configuration_id: ConfigurationVersionId,
        capability: String,
        status: CapabilityReadinessStatus,
        reason_code: Option<String>,
        checked_at_ms: i64,
        projection_digest: Sha256Digest,
    ) -> Result<Self, SetupModelError> {
        if capability.trim().is_empty() || capability.chars().any(char::is_control) {
            return Err(SetupModelError::InvalidCapability);
        }
        if reason_code
            .as_deref()
            .is_some_and(|code| code.is_empty() || code.chars().any(char::is_control))
        {
            return Err(SetupModelError::InvalidSafeCode);
        }
        Ok(Self {
            configuration_id,
            capability,
            status,
            reason_code,
            checked_at_ms,
            projection_digest,
        })
    }
}

impl TryFrom<CapabilityReadinessWire> for CapabilityReadiness {
    type Error = SetupModelError;

    fn try_from(value: CapabilityReadinessWire) -> Result<Self, Self::Error> {
        Self::new(
            value.configuration_id,
            value.capability,
            value.status,
            value.reason_code,
            value.checked_at_ms,
            value.projection_digest,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupModelError {
    InvalidSchemaVersion,
    InvalidTimestampOrder,
    InvalidStepKey,
    InvalidAttempt,
    InvalidSafeCode,
    InvalidCapability,
}

impl std::fmt::Display for SetupModelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid setup model")
    }
}

impl std::error::Error for SetupModelError {}
