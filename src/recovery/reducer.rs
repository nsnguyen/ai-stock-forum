use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    app::{ApplicationEvent, EventEnvelope, ShutdownReason, EVENT_SCHEMA_VERSION},
    domain::{canonical_json_bytes, sha256, EventId, InstallationId, SessionId, Sha256Digest},
    persistence::RecoveryError,
    setup::SetupStatus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionState {
    pub installation: Option<InstallationProjection>,
    pub sessions: BTreeMap<SessionId, SessionProjection>,
    pub setup_status: SetupStatus,
    pub last_sequence: u64,
    pub last_event_digest: Option<Sha256Digest>,
    pub previous_session_interrupted: bool,
}

impl Default for ProjectionState {
    fn default() -> Self {
        Self {
            installation: None,
            sessions: BTreeMap::new(),
            setup_status: SetupStatus::NotStarted,
            last_sequence: 0,
            last_event_digest: None,
            previous_session_interrupted: false,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionStateWire {
    installation: Option<InstallationProjection>,
    sessions: BTreeMap<SessionId, SessionProjection>,
    setup_status: SetupStatus,
    last_sequence: u64,
    last_event_digest: Option<Sha256Digest>,
    previous_session_interrupted: bool,
}

impl<'de> Deserialize<'de> for ProjectionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ProjectionStateWire::deserialize(deserializer)?;
        let state = Self {
            installation: wire.installation,
            sessions: wire.sessions,
            setup_status: wire.setup_status,
            last_sequence: wire.last_sequence,
            last_event_digest: wire.last_event_digest,
            previous_session_interrupted: wire.previous_session_interrupted,
        };
        state.validate().map_err(serde::de::Error::custom)?;
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationProjection {
    pub installation_id: InstallationId,
    pub created_event_id: EventId,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProjection {
    pub session_id: SessionId,
    pub started_event_id: EventId,
    pub started_at_ms: i64,
    pub ended: Option<SessionEndProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEndProjection {
    pub ended_event_id: EventId,
    pub ended_at_ms: i64,
    pub reason: ShutdownReason,
}

impl ProjectionState {
    pub fn digest(&self) -> Result<Sha256Digest, crate::domain::DomainError> {
        canonical_json_bytes(&PersistentProjectionState {
            installation: &self.installation,
            sessions: &self.sessions,
            setup_status: &self.setup_status,
            last_sequence: self.last_sequence,
            last_event_digest: &self.last_event_digest,
        })
        .map(|bytes| sha256(&bytes))
    }

    pub(crate) fn validate(&self) -> Result<(), RecoveryError> {
        if (self.last_sequence == 0) != self.last_event_digest.is_none() {
            return Err(RecoveryError::InvalidEventRecord);
        }
        if self.installation.is_none() && !self.sessions.is_empty() {
            return Err(RecoveryError::InvalidEventRecord);
        }
        if self
            .sessions
            .iter()
            .any(|(session_id, projection)| session_id != &projection.session_id)
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct PersistentProjectionState<'a> {
    installation: &'a Option<InstallationProjection>,
    sessions: &'a BTreeMap<SessionId, SessionProjection>,
    setup_status: &'a SetupStatus,
    last_sequence: u64,
    last_event_digest: &'a Option<Sha256Digest>,
}

pub fn reduce(state: &mut ProjectionState, event: &EventEnvelope) -> Result<(), RecoveryError> {
    if event.event_schema_version != EVENT_SCHEMA_VERSION {
        return Err(RecoveryError::UnsupportedEventSchema);
    }
    if event.sequence != state.last_sequence + 1 {
        return Err(RecoveryError::EventSequenceGap);
    }
    if event.previous_event_digest != state.last_event_digest {
        return Err(RecoveryError::PreviousEventDigestMismatch);
    }

    let mut next = state.clone();
    match &event.event {
        ApplicationEvent::InstallationInitialized { installation_id } => {
            if next.installation.is_some() {
                return Err(RecoveryError::InvalidEventRecord);
            }
            next.installation = Some(InstallationProjection {
                installation_id: *installation_id,
                created_event_id: event.event_id,
                created_at_ms: event.occurred_at_ms,
            });
        }
        ApplicationEvent::ProcessSessionStarted { session_id } => {
            if next.installation.is_none() || next.sessions.contains_key(session_id) {
                return Err(RecoveryError::InvalidEventRecord);
            }
            next.sessions.insert(
                *session_id,
                SessionProjection {
                    session_id: *session_id,
                    started_event_id: event.event_id,
                    started_at_ms: event.occurred_at_ms,
                    ended: None,
                },
            );
        }
        ApplicationEvent::PreviousSessionInterrupted { session_id } => {
            end_session(&mut next, *session_id, event, ShutdownReason::Interrupted)?;
            next.previous_session_interrupted = true;
        }
        ApplicationEvent::ProcessSessionEnded { session_id, reason } => {
            end_session(&mut next, *session_id, event, *reason)?;
        }
        ApplicationEvent::ProjectionRebuilt { through_sequence } => {
            if *through_sequence != event.sequence - 1 {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        ApplicationEvent::HelpViewed
        | ApplicationEvent::StatusViewed
        | ApplicationEvent::SetupStatusViewed
        | ApplicationEvent::AuditTailViewed { .. }
        | ApplicationEvent::CommandRejected { .. }
        | ApplicationEvent::ShutdownRequested => {}
    }
    next.last_sequence = event.sequence;
    next.last_event_digest = Some(event.event_digest.clone());
    next.validate()?;
    *state = next;
    Ok(())
}

fn end_session(
    state: &mut ProjectionState,
    session_id: SessionId,
    event: &EventEnvelope,
    reason: ShutdownReason,
) -> Result<(), RecoveryError> {
    let session = state
        .sessions
        .get_mut(&session_id)
        .ok_or(RecoveryError::InvalidEventRecord)?;
    if session.ended.is_some() {
        return Err(RecoveryError::InvalidEventRecord);
    }
    session.ended = Some(SessionEndProjection {
        ended_event_id: event.event_id,
        ended_at_ms: event.occurred_at_ms,
        reason,
    });
    Ok(())
}
