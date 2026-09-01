use std::str::FromStr;

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde::Serialize;
use thiserror::Error;

use crate::{
    app::{
        ApplicationEvent, AuditLimit, EventEnvelope, PendingEvent, EVENT_SCHEMA_VERSION,
    },
    domain::{
        canonical_json_bytes, sha256, Actor, CausationId, CorrelationId, EventId, ObjectRef,
        ObjectVersion, Sha256Digest,
    },
};

use super::PersistenceError;

const DIGEST_FORMAT_VERSION: u16 = 1;
const EVENT_COLUMNS: &str = "sequence, event_id, event_schema_version, event_type, actor_kind, actor_id, occurred_at_ms, correlation_id, causation_id, object_kind, object_id, object_version, object_digest, previous_event_digest, payload_json, event_digest";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RecoveryError {
    #[error("event sequence is not contiguous")]
    EventSequenceGap,
    #[error("event schema version is unsupported")]
    UnsupportedEventSchema,
    #[error("event record is invalid")]
    InvalidEventRecord,
    #[error("event previous digest does not match")]
    PreviousEventDigestMismatch,
    #[error("event digest does not match")]
    EventDigestMismatch,
    #[error("event stream query failed")]
    QueryFailed,
}

impl RecoveryError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::EventSequenceGap => "event_sequence_gap",
            Self::UnsupportedEventSchema => "unsupported_event_schema",
            Self::InvalidEventRecord => "invalid_event_record",
            Self::PreviousEventDigestMismatch => "previous_event_digest_mismatch",
            Self::EventDigestMismatch => "event_digest_mismatch",
            Self::QueryFailed => "event_query_failed",
        }
    }
}

pub struct EventRepository;

impl EventRepository {
    pub fn append(
        transaction: &Transaction<'_>,
        pending: PendingEvent,
    ) -> Result<EventEnvelope, PersistenceError> {
        if pending.event_schema_version != EVENT_SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedEventSchema);
        }

        if let Some(existing) = load_by_event_id(transaction, pending.event_id)? {
            return if matches_pending(&existing, &pending) {
                Ok(existing)
            } else {
                Err(PersistenceError::IdempotencyConflict)
            };
        }

        let previous = transaction
            .query_row(
                "SELECT sequence, event_digest FROM event_stream ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(query_error)?
            .map(|(sequence, digest)| {
                let sequence = u64::try_from(sequence).map_err(|_| PersistenceError::InvalidEventRecord)?;
                let digest = Sha256Digest::parse(&digest).map_err(|_| PersistenceError::InvalidEventRecord)?;
                Ok((sequence, digest))
            })
            .transpose()?;
        let sequence = previous.as_ref().map_or(1, |(value, _)| value + 1);
        let previous_event_digest = previous.map(|(_, digest)| digest);
        let payload_json = canonical_json(&pending.event)?;
        let event_digest = digest_for(
            sequence,
            &pending,
            previous_event_digest.as_ref(),
            &payload_json,
        )?;
        let actor_kind = actor_kind(&pending.actor);
        let object = pending.object.as_ref();
        let object_version = object
            .map(|value| i64::try_from(value.version.get()))
            .transpose()
            .map_err(|_| PersistenceError::InvalidEventRecord)?;

        transaction
            .execute(
                "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, actor_id, occurred_at_ms, correlation_id, causation_id, object_kind, object_id, object_version, object_digest, previous_event_digest, payload_json, event_digest) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    i64::try_from(sequence).map_err(|_| PersistenceError::InvalidEventRecord)?,
                    pending.event_id.to_string(),
                    i64::from(pending.event_schema_version),
                    pending.event.kind(),
                    actor_kind,
                    pending.occurred_at_ms,
                    pending.correlation_id.to_string(),
                    pending.causation_id.map(|value| value.to_string()),
                    object.map(|value| value.kind.as_str()),
                    object.map(|value| value.id.as_str()),
                    object_version,
                    object.map(|value| value.digest.as_str()),
                    previous_event_digest.as_ref().map(Sha256Digest::as_str),
                    payload_json,
                    event_digest.as_str(),
                ],
            )
            .map_err(query_error)?;

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

    pub fn load_all(connection: &Connection) -> Result<Vec<EventEnvelope>, RecoveryError> {
        let mut statement = connection
            .prepare(&format!("SELECT {EVENT_COLUMNS} FROM event_stream ORDER BY sequence"))
            .map_err(|_| RecoveryError::QueryFailed)?;
        let mut rows = statement.query([]).map_err(|_| RecoveryError::QueryFailed)?;
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(|_| RecoveryError::QueryFailed)? {
            events.push(decode_row(row)?);
        }
        Ok(events)
    }

    pub fn tail(
        connection: &Connection,
        limit: AuditLimit,
    ) -> Result<Vec<EventEnvelope>, PersistenceError> {
        let mut statement = connection
            .prepare(&format!("SELECT {EVENT_COLUMNS} FROM (SELECT {EVENT_COLUMNS} FROM event_stream ORDER BY sequence DESC LIMIT ?1) ORDER BY sequence"))
            .map_err(query_error)?;
        let rows = statement
            .query_map([i64::from(limit.get())], |row| {
                decode_row(row).map_err(|_| rusqlite::Error::InvalidQuery)
            })
            .map_err(query_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(query_error)
    }

    pub fn verify(connection: &Connection) -> Result<(), RecoveryError> {
        let events = Self::load_all(connection)?;
        let mut previous_digest = None;
        for (index, event) in events.iter().enumerate() {
            if event.sequence != (index as u64) + 1 {
                return Err(RecoveryError::EventSequenceGap);
            }
            if event.event_schema_version != EVENT_SCHEMA_VERSION {
                return Err(RecoveryError::UnsupportedEventSchema);
            }
            if event.previous_event_digest != previous_digest {
                return Err(RecoveryError::PreviousEventDigestMismatch);
            }
            let payload_json = canonical_json(&event.event).map_err(|_| RecoveryError::InvalidEventRecord)?;
            let expected_digest = digest_for(
                event.sequence,
                &PendingEvent {
                    event_id: event.event_id,
                    event_schema_version: event.event_schema_version,
                    actor: event.actor.clone(),
                    occurred_at_ms: event.occurred_at_ms,
                    correlation_id: event.correlation_id,
                    causation_id: event.causation_id,
                    object: event.object.clone(),
                    event: event.event.clone(),
                },
                event.previous_event_digest.as_ref(),
                &payload_json,
            )
            .map_err(|_| RecoveryError::InvalidEventRecord)?;
            if event.event_digest != expected_digest {
                return Err(RecoveryError::EventDigestMismatch);
            }
            previous_digest = Some(event.event_digest.clone());
        }
        Ok(())
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
    payload_json: &'a str,
}

fn digest_for(
    sequence: u64,
    pending: &PendingEvent,
    previous_event_digest: Option<&Sha256Digest>,
    payload_json: &str,
) -> Result<Sha256Digest, PersistenceError> {
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
        payload_json,
    };
    canonical_json_bytes(&material)
        .map(|bytes| sha256(&bytes))
        .map_err(|_| PersistenceError::InvalidEventRecord)
}

fn canonical_json(event: &ApplicationEvent) -> Result<String, PersistenceError> {
    String::from_utf8(
        canonical_json_bytes(event).map_err(|_| PersistenceError::InvalidEventRecord)?,
    )
    .map_err(|_| PersistenceError::InvalidEventRecord)
}

fn load_by_event_id(
    transaction: &Transaction<'_>,
    event_id: EventId,
) -> Result<Option<EventEnvelope>, PersistenceError> {
    transaction
        .query_row(
            &format!("SELECT {EVENT_COLUMNS} FROM event_stream WHERE event_id = ?1"),
            [event_id.to_string()],
            |row| decode_row(row).map_err(|_| rusqlite::Error::InvalidQuery),
        )
        .optional()
        .map_err(query_error)
}

fn matches_pending(existing: &EventEnvelope, pending: &PendingEvent) -> bool {
    existing.event_id == pending.event_id
        && existing.event_schema_version == pending.event_schema_version
        && existing.actor == pending.actor
        && existing.occurred_at_ms == pending.occurred_at_ms
        && existing.correlation_id == pending.correlation_id
        && existing.causation_id == pending.causation_id
        && existing.object == pending.object
        && existing.event == pending.event
}

fn decode_row(row: &Row<'_>) -> Result<EventEnvelope, RecoveryError> {
    let sequence = integer_to_u64(row.get::<_, i64>(0).map_err(|_| RecoveryError::QueryFailed)?)?;
    let event_id = parse_id(row.get::<_, String>(1).map_err(|_| RecoveryError::QueryFailed)?)?;
    let event_schema_version = u16::try_from(row.get::<_, i64>(2).map_err(|_| RecoveryError::QueryFailed)?)
        .map_err(|_| RecoveryError::InvalidEventRecord)?;
    let event_type = row.get::<_, String>(3).map_err(|_| RecoveryError::QueryFailed)?;
    let actor = parse_actor(&row.get::<_, String>(4).map_err(|_| RecoveryError::QueryFailed)?)?;
    if row.get::<_, Option<String>>(5).map_err(|_| RecoveryError::QueryFailed)?.is_some() {
        return Err(RecoveryError::InvalidEventRecord);
    }
    let occurred_at_ms = row.get(6).map_err(|_| RecoveryError::QueryFailed)?;
    let correlation_id = parse_id(row.get::<_, String>(7).map_err(|_| RecoveryError::QueryFailed)?)?;
    let causation_id = row
        .get::<_, Option<String>>(8)
        .map_err(|_| RecoveryError::QueryFailed)?
        .map(parse_id)
        .transpose()?;
    let object = decode_object(row)?;
    let previous_event_digest = row
        .get::<_, Option<String>>(13)
        .map_err(|_| RecoveryError::QueryFailed)?
        .map(|value| Sha256Digest::parse(&value).map_err(|_| RecoveryError::InvalidEventRecord))
        .transpose()?;
    let payload_json = row.get::<_, String>(14).map_err(|_| RecoveryError::QueryFailed)?;
    let event = serde_json::from_str::<ApplicationEvent>(&payload_json)
        .map_err(|_| RecoveryError::InvalidEventRecord)?;
    if event.kind() != event_type {
        return Err(RecoveryError::InvalidEventRecord);
    }
    if canonical_json(&event).map_err(|_| RecoveryError::InvalidEventRecord)? != payload_json {
        return Err(RecoveryError::InvalidEventRecord);
    }
    let event_digest = Sha256Digest::parse(
        &row.get::<_, String>(15).map_err(|_| RecoveryError::QueryFailed)?,
    )
    .map_err(|_| RecoveryError::InvalidEventRecord)?;
    Ok(EventEnvelope {
        sequence,
        event_id,
        event_schema_version,
        actor,
        occurred_at_ms,
        correlation_id,
        causation_id,
        object,
        event,
        previous_event_digest,
        event_digest,
    })
}

fn decode_object(row: &Row<'_>) -> Result<Option<ObjectRef>, RecoveryError> {
    let kind = row.get::<_, Option<String>>(9).map_err(|_| RecoveryError::QueryFailed)?;
    let id = row.get::<_, Option<String>>(10).map_err(|_| RecoveryError::QueryFailed)?;
    let version = row.get::<_, Option<i64>>(11).map_err(|_| RecoveryError::QueryFailed)?;
    let digest = row.get::<_, Option<String>>(12).map_err(|_| RecoveryError::QueryFailed)?;
    match (kind, id, version, digest) {
        (None, None, None, None) => Ok(None),
        (Some(kind), Some(id), Some(version), Some(digest)) => ObjectRef::new(
            kind,
            id,
            ObjectVersion::new(u64::try_from(version).map_err(|_| RecoveryError::InvalidEventRecord)?)
                .map_err(|_| RecoveryError::InvalidEventRecord)?,
            Sha256Digest::parse(&digest).map_err(|_| RecoveryError::InvalidEventRecord)?,
        )
        .map(Some)
        .map_err(|_| RecoveryError::InvalidEventRecord),
        _ => Err(RecoveryError::InvalidEventRecord),
    }
}

fn parse_id<T: FromStr>(value: String) -> Result<T, RecoveryError> {
    value.parse().map_err(|_| RecoveryError::InvalidEventRecord)
}

fn parse_actor(value: &str) -> Result<Actor, RecoveryError> {
    match value {
        "human" => Ok(Actor::Human),
        "system" => Ok(Actor::System),
        _ => Err(RecoveryError::InvalidEventRecord),
    }
}

fn actor_kind(actor: &Actor) -> &'static str {
    match actor {
        Actor::Human => "human",
        Actor::System => "system",
    }
}

fn integer_to_u64(value: i64) -> Result<u64, RecoveryError> {
    u64::try_from(value).map_err(|_| RecoveryError::InvalidEventRecord)
}

fn query_error(_: rusqlite::Error) -> PersistenceError {
    PersistenceError::QueryFailed
}
