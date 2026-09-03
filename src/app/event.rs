use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Value, json};

use crate::{
    app::{AuditLimit, InputRejection},
    domain::{
        Actor, CausationId, CorrelationId, EventId, InstallationId, ObjectRef, SessionId,
        Sha256Digest, canonical_json_bytes, sha256,
    },
    persistence::RecoveryError,
};

pub const EVENT_SCHEMA_VERSION: u16 = 1;
const DIGEST_FORMAT_VERSION: u16 = 1;

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
    InstallationInitialized {
        installation_id: InstallationId,
    },
    ProcessSessionStarted {
        session_id: SessionId,
    },
    PreviousSessionInterrupted {
        session_id: SessionId,
    },
    HelpViewed,
    StatusViewed,
    SetupStatusViewed,
    AuditTailViewed {
        limit: AuditLimit,
    },
    CommandRejected {
        rejection: InputRejection,
    },
    ShutdownRequested,
    ProcessSessionEnded {
        session_id: SessionId,
        reason: ShutdownReason,
    },
    ProjectionRebuilt {
        through_sequence: u64,
    },
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelopeWire {
    pub sequence: u64,
    pub event_id: EventId,
    pub event_schema_version: u16,
    pub actor: Actor,
    pub occurred_at_ms: i64,
    pub correlation_id: CorrelationId,
    pub causation_id: Option<CausationId>,
    pub object: Option<ObjectRef>,
    pub event_type: String,
    pub payload_json: String,
    pub previous_event_digest: Option<Sha256Digest>,
    pub event_digest: Sha256Digest,
}

impl From<&EventEnvelope> for EventEnvelopeWire {
    fn from(envelope: &EventEnvelope) -> Self {
        Self {
            sequence: envelope.sequence,
            event_id: envelope.event_id,
            event_schema_version: envelope.event_schema_version,
            actor: envelope.actor.clone(),
            occurred_at_ms: envelope.occurred_at_ms,
            correlation_id: envelope.correlation_id,
            causation_id: envelope.causation_id,
            object: envelope.object.clone(),
            event_type: envelope.event.kind().to_owned(),
            payload_json: canonical_payload_json(&envelope.event)
                .expect("closed application events serialize canonically"),
            previous_event_digest: envelope.previous_event_digest.clone(),
            event_digest: envelope.event_digest.clone(),
        }
    }
}

impl TryFrom<EventEnvelopeWire> for EventEnvelope {
    type Error = RecoveryError;

    fn try_from(wire: EventEnvelopeWire) -> Result<Self, Self::Error> {
        if wire.sequence == 0 {
            return Err(RecoveryError::InvalidEventRecord);
        }
        if wire.event_schema_version != EVENT_SCHEMA_VERSION {
            return Err(RecoveryError::UnsupportedEventSchema);
        }
        if (wire.sequence == 1) != wire.previous_event_digest.is_none() {
            return Err(RecoveryError::InvalidPredecessorShape);
        }
        let event = decode_event(&wire.event_type, &wire.payload_json)?;
        if canonical_payload_json(&event).map_err(|_| RecoveryError::InvalidEventRecord)?
            != wire.payload_json
        {
            return Err(RecoveryError::InvalidEventRecord);
        }
        let pending = PendingEvent {
            event_id: wire.event_id,
            event_schema_version: wire.event_schema_version,
            actor: wire.actor,
            occurred_at_ms: wire.occurred_at_ms,
            correlation_id: wire.correlation_id,
            causation_id: wire.causation_id,
            object: wire.object,
            event,
        };
        let expected = digest_for(wire.sequence, &pending, wire.previous_event_digest.as_ref())
            .map_err(|_| RecoveryError::InvalidEventRecord)?;
        if expected != wire.event_digest {
            return Err(RecoveryError::EventDigestMismatch);
        }
        Ok(EventEnvelope {
            sequence: wire.sequence,
            event_id: pending.event_id,
            event_schema_version: pending.event_schema_version,
            actor: pending.actor,
            occurred_at_ms: pending.occurred_at_ms,
            correlation_id: pending.correlation_id,
            causation_id: pending.causation_id,
            object: pending.object,
            event: pending.event,
            previous_event_digest: wire.previous_event_digest,
            event_digest: wire.event_digest,
        })
    }
}

impl Serialize for EventEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        EventEnvelopeWire::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EventEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        EventEnvelopeWire::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

pub(crate) fn envelope_from_pending(
    sequence: u64,
    pending: PendingEvent,
    previous_event_digest: Option<Sha256Digest>,
) -> Result<EventEnvelope, ()> {
    let event_digest = digest_for(sequence, &pending, previous_event_digest.as_ref())?;
    Ok(EventEnvelope {
        sequence,
        event_id: pending.event_id,
        event_schema_version: pending.event_schema_version,
        actor: pending.actor,
        occurred_at_ms: pending.occurred_at_ms,
        correlation_id: pending.correlation_id,
        causation_id: pending.causation_id,
        object: pending.object,
        event: pending.event,
        previous_event_digest,
        event_digest,
    })
}

pub(crate) fn canonical_payload_json(event: &ApplicationEvent) -> Result<String, ()> {
    let tagged = serde_json::to_value(event).map_err(|_| ())?;
    let payload = tagged
        .as_object()
        .and_then(|object| object.get("data"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    String::from_utf8(canonical_json_bytes(&payload).map_err(|_| ())?).map_err(|_| ())
}

fn decode_event(event_type: &str, payload_json: &str) -> Result<ApplicationEvent, RecoveryError> {
    let payload = serde_json::from_str::<Value>(payload_json)
        .map_err(|_| RecoveryError::InvalidEventRecord)?;
    let event = match event_type {
        "help_viewed" => unit_event(payload, ApplicationEvent::HelpViewed),
        "status_viewed" => unit_event(payload, ApplicationEvent::StatusViewed),
        "setup_status_viewed" => unit_event(payload, ApplicationEvent::SetupStatusViewed),
        "shutdown_requested" => unit_event(payload, ApplicationEvent::ShutdownRequested),
        _ => serde_json::from_value(json!({"type": event_type, "data": payload}))
            .map_err(|_| RecoveryError::InvalidEventRecord),
    }?;
    if event.kind() == event_type {
        Ok(event)
    } else {
        Err(RecoveryError::InvalidEventRecord)
    }
}

fn unit_event(payload: Value, event: ApplicationEvent) -> Result<ApplicationEvent, RecoveryError> {
    if payload == json!({}) {
        Ok(event)
    } else {
        Err(RecoveryError::InvalidEventRecord)
    }
}

#[derive(Serialize)]
struct DigestMaterial<'a> {
    digest_format_version: u16,
    sequence: u64,
    event_id: &'a EventId,
    event_schema_version: u16,
    event_type: &'a str,
    actor_kind: &'a str,
    actor_id: Option<&'a str>,
    occurred_at_ms: i64,
    correlation_id: &'a CorrelationId,
    causation_id: Option<&'a CausationId>,
    object: Option<&'a ObjectRef>,
    previous_event_digest: Option<&'a Sha256Digest>,
    payload_json: String,
}

fn digest_for(
    sequence: u64,
    pending: &PendingEvent,
    previous_event_digest: Option<&Sha256Digest>,
) -> Result<Sha256Digest, ()> {
    let material = DigestMaterial {
        digest_format_version: DIGEST_FORMAT_VERSION,
        sequence,
        event_id: &pending.event_id,
        event_schema_version: pending.event_schema_version,
        event_type: pending.event.kind(),
        actor_kind: actor_kind(&pending.actor),
        actor_id: None,
        occurred_at_ms: pending.occurred_at_ms,
        correlation_id: &pending.correlation_id,
        causation_id: pending.causation_id.as_ref(),
        object: pending.object.as_ref(),
        previous_event_digest,
        payload_json: canonical_payload_json(&pending.event)?,
    };
    canonical_json_bytes(&material)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| ())
}

fn actor_kind(actor: &Actor) -> &'static str {
    match actor {
        Actor::Human => "human",
        Actor::System => "system",
    }
}
