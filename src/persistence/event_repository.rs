use std::str::FromStr;

use rusqlite::{Connection, Error as SqliteError, ErrorCode, OptionalExtension, Row, params};
use thiserror::Error;

use crate::{
    app::{
        AuditLimit, EVENT_SCHEMA_VERSION, EventEnvelope, EventEnvelopeWire, PendingEvent,
        envelope_from_pending,
    },
    domain::{CausationId, EventId, ObjectRef, ObjectVersion, Sha256Digest},
};

use super::{ImmediateTransaction, PersistenceError};

const EVENT_COLUMNS: &str = "sequence, event_id, event_schema_version, event_type, actor_kind, actor_id, occurred_at_ms, correlation_id, causation_id, object_kind, object_id, object_version, object_digest, previous_event_digest, payload_json, event_digest";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RecoveryError {
    #[error("event sequence is not contiguous")]
    EventSequenceGap,
    #[error("event sequence cannot advance beyond its maximum value")]
    EventSequenceOverflow,
    #[error("event schema version is unsupported")]
    UnsupportedEventSchema,
    #[error("event record is invalid")]
    InvalidEventRecord,
    #[error("event predecessor shape is invalid")]
    InvalidPredecessorShape,
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
            Self::EventSequenceOverflow => "event_sequence_overflow",
            Self::UnsupportedEventSchema => "unsupported_event_schema",
            Self::InvalidEventRecord => "invalid_event_record",
            Self::InvalidPredecessorShape => "invalid_predecessor_shape",
            Self::PreviousEventDigestMismatch => "previous_event_digest_mismatch",
            Self::EventDigestMismatch => "event_digest_mismatch",
            Self::QueryFailed => "event_query_failed",
        }
    }
}

pub struct EventRepository;

impl EventRepository {
    pub fn load_by_event_id(
        transaction: &ImmediateTransaction<'_>,
        event_id: EventId,
    ) -> Result<Option<EventEnvelope>, PersistenceError> {
        load_by_event_id(transaction, event_id)
    }

    /// Appends exactly one event. Event-ID replay is repository scope only; command and batch
    /// idempotency are deferred to the application-service task.
    pub fn append(
        transaction: &ImmediateTransaction<'_>,
        pending: PendingEvent,
    ) -> Result<EventEnvelope, PersistenceError> {
        if pending.event_schema_version != EVENT_SCHEMA_VERSION {
            return Err(PersistenceError::UnsupportedEventSchema);
        }
        if let Some(existing) = load_by_event_id(transaction, pending.event_id)? {
            return replay_or_conflict(existing, &pending);
        }

        let previous = transaction
            .transaction()
            .query_row(
                "SELECT sequence, event_digest FROM event_stream ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(map_sqlite)?
            .map(|(sequence, digest)| {
                Ok((
                    u64::try_from(sequence).map_err(|_| PersistenceError::InvalidEventRecord)?,
                    Sha256Digest::parse(&digest)
                        .map_err(|_| PersistenceError::InvalidEventRecord)?,
                ))
            })
            .transpose()?;
        let sequence = previous.as_ref().map_or(1, |(sequence, _)| sequence + 1);
        let envelope = envelope_from_pending(sequence, pending, previous.map(|(_, digest)| digest))
            .map_err(|_| PersistenceError::InvalidEventRecord)?;
        let wire = EventEnvelopeWire::from(&envelope);
        let object = wire.object.as_ref();
        let object_version = object
            .map(|object| i64::try_from(object.version.get()))
            .transpose()
            .map_err(|_| PersistenceError::InvalidEventRecord)?;
        let result = transaction.transaction().execute(
            "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, actor_id, occurred_at_ms, correlation_id, causation_id, object_kind, object_id, object_version, object_digest, previous_event_digest, payload_json, event_digest) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                i64::try_from(wire.sequence).map_err(|_| PersistenceError::InvalidEventRecord)?,
                wire.event_id.to_string(),
                i64::from(wire.event_schema_version),
                wire.event_type,
                actor_kind(&wire.actor),
                wire.occurred_at_ms,
                wire.correlation_id.to_string(),
                wire.causation_id.map(|id| id.to_string()),
                object.map(|object| object.kind.as_str()),
                object.map(|object| object.id.as_str()),
                object_version,
                object.map(|object| object.digest.as_str()),
                wire.previous_event_digest.as_ref().map(Sha256Digest::as_str),
                wire.payload_json,
                wire.event_digest.as_str(),
            ],
        );
        match result {
            Ok(_) => Ok(envelope),
            Err(error) if is_constraint(&error) => {
                match load_by_event_id(transaction, envelope.event_id)? {
                    Some(existing) => replay_or_conflict(existing, &pending_from(&envelope)),
                    None => Err(map_sqlite(error)),
                }
            }
            Err(error) => Err(map_sqlite(error)),
        }
    }

    pub fn load_all(connection: &Connection) -> Result<Vec<EventEnvelope>, RecoveryError> {
        load_query(
            connection,
            &format!("SELECT {EVENT_COLUMNS} FROM event_stream ORDER BY sequence"),
            [],
        )
    }

    pub fn tail(
        connection: &Connection,
        limit: AuditLimit,
    ) -> Result<Vec<EventEnvelope>, PersistenceError> {
        load_query(
            connection,
            &format!("SELECT {EVENT_COLUMNS} FROM (SELECT {EVENT_COLUMNS} FROM event_stream ORDER BY sequence DESC LIMIT ?1) ORDER BY sequence"),
            [i64::from(limit.get())],
        )
        .map_err(persistence_from_recovery)
    }

    pub fn tail_through(
        connection: &Connection,
        limit: AuditLimit,
        through_sequence: u64,
    ) -> Result<Vec<EventEnvelope>, PersistenceError> {
        let through_sequence =
            i64::try_from(through_sequence).map_err(|_| PersistenceError::InvalidEventRecord)?;
        load_query(
            connection,
            &format!("SELECT {EVENT_COLUMNS} FROM (SELECT {EVENT_COLUMNS} FROM event_stream WHERE sequence <= ?1 ORDER BY sequence DESC LIMIT ?2) ORDER BY sequence"),
            params![through_sequence, i64::from(limit.get())],
        )
        .map_err(persistence_from_recovery)
    }

    pub fn load_by_causation_id(
        transaction: &ImmediateTransaction<'_>,
        causation_id: CausationId,
    ) -> Result<Option<EventEnvelope>, PersistenceError> {
        let mut statement = transaction
            .transaction()
            .prepare(&format!(
                "SELECT {EVENT_COLUMNS} FROM event_stream WHERE causation_id = ?1 ORDER BY sequence"
            ))
            .map_err(map_sqlite)?;
        let mut rows = statement
            .query([causation_id.to_string()])
            .map_err(map_sqlite)?;
        let first = rows
            .next()
            .map_err(map_sqlite)?
            .map(decode_row)
            .transpose()
            .map_err(persistence_from_recovery)?;
        if rows.next().map_err(map_sqlite)?.is_some() {
            return Err(PersistenceError::InvalidEventRecord);
        }
        Ok(first)
    }

    pub fn verify(connection: &Connection) -> Result<(), RecoveryError> {
        let events = Self::load_all(connection)?;
        let mut previous_digest = None;
        for (index, event) in events.iter().enumerate() {
            if event.sequence != (index as u64) + 1 {
                return Err(RecoveryError::EventSequenceGap);
            }
            if event.previous_event_digest != previous_digest {
                return Err(RecoveryError::PreviousEventDigestMismatch);
            }
            previous_digest = Some(event.event_digest.clone());
        }
        Ok(())
    }
}

fn load_query<P: rusqlite::Params>(
    connection: &Connection,
    query: &str,
    parameters: P,
) -> Result<Vec<EventEnvelope>, RecoveryError> {
    let mut statement = connection
        .prepare(query)
        .map_err(|_| RecoveryError::QueryFailed)?;
    let mut rows = statement
        .query(parameters)
        .map_err(|_| RecoveryError::QueryFailed)?;
    let mut events = Vec::new();
    while let Some(row) = rows.next().map_err(|_| RecoveryError::QueryFailed)? {
        events.push(decode_row(row)?);
    }
    Ok(events)
}

fn load_by_event_id(
    transaction: &ImmediateTransaction<'_>,
    event_id: EventId,
) -> Result<Option<EventEnvelope>, PersistenceError> {
    let mut statement = transaction
        .transaction()
        .prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM event_stream WHERE event_id = ?1"
        ))
        .map_err(map_sqlite)?;
    let mut rows = statement
        .query([event_id.to_string()])
        .map_err(map_sqlite)?;
    match rows.next().map_err(map_sqlite)? {
        Some(row) => decode_row(row).map(Some).map_err(persistence_from_recovery),
        None => Ok(None),
    }
}

fn replay_or_conflict(
    existing: EventEnvelope,
    pending: &PendingEvent,
) -> Result<EventEnvelope, PersistenceError> {
    if existing.event_id == pending.event_id
        && existing.event_schema_version == pending.event_schema_version
        && existing.actor == pending.actor
        && existing.occurred_at_ms == pending.occurred_at_ms
        && existing.correlation_id == pending.correlation_id
        && existing.causation_id == pending.causation_id
        && existing.object == pending.object
        && existing.event == pending.event
    {
        Ok(existing)
    } else {
        Err(PersistenceError::IdempotencyConflict)
    }
}

fn pending_from(envelope: &EventEnvelope) -> PendingEvent {
    PendingEvent {
        event_id: envelope.event_id,
        event_schema_version: envelope.event_schema_version,
        actor: envelope.actor.clone(),
        occurred_at_ms: envelope.occurred_at_ms,
        correlation_id: envelope.correlation_id,
        causation_id: envelope.causation_id,
        object: envelope.object.clone(),
        event: envelope.event.clone(),
    }
}

fn decode_row(row: &Row<'_>) -> Result<EventEnvelope, RecoveryError> {
    let sequence = u64::try_from(
        row.get::<_, i64>(0)
            .map_err(|_| RecoveryError::QueryFailed)?,
    )
    .map_err(|_| RecoveryError::InvalidEventRecord)?;
    let event_schema_version = u16::try_from(
        row.get::<_, i64>(2)
            .map_err(|_| RecoveryError::QueryFailed)?,
    )
    .map_err(|_| RecoveryError::InvalidEventRecord)?;
    if event_schema_version != EVENT_SCHEMA_VERSION {
        return Err(RecoveryError::UnsupportedEventSchema);
    }
    let actor = parse_actor(
        &row.get::<_, String>(4)
            .map_err(|_| RecoveryError::QueryFailed)?,
    )?;
    if row
        .get::<_, Option<String>>(5)
        .map_err(|_| RecoveryError::QueryFailed)?
        .is_some()
    {
        return Err(RecoveryError::InvalidEventRecord);
    }
    let wire = EventEnvelopeWire {
        sequence,
        event_id: parse_id(
            row.get::<_, String>(1)
                .map_err(|_| RecoveryError::QueryFailed)?,
        )?,
        event_schema_version,
        actor,
        occurred_at_ms: row.get(6).map_err(|_| RecoveryError::QueryFailed)?,
        correlation_id: parse_id(
            row.get::<_, String>(7)
                .map_err(|_| RecoveryError::QueryFailed)?,
        )?,
        causation_id: row
            .get::<_, Option<String>>(8)
            .map_err(|_| RecoveryError::QueryFailed)?
            .map(parse_id)
            .transpose()?,
        object: decode_object(row)?,
        event_type: row.get(3).map_err(|_| RecoveryError::QueryFailed)?,
        payload_json: row.get(14).map_err(|_| RecoveryError::QueryFailed)?,
        previous_event_digest: row
            .get::<_, Option<String>>(13)
            .map_err(|_| RecoveryError::QueryFailed)?
            .map(|digest| {
                Sha256Digest::parse(&digest).map_err(|_| RecoveryError::InvalidEventRecord)
            })
            .transpose()?,
        event_digest: Sha256Digest::parse(
            &row.get::<_, String>(15)
                .map_err(|_| RecoveryError::QueryFailed)?,
        )
        .map_err(|_| RecoveryError::InvalidEventRecord)?,
    };
    wire.try_into()
}

fn decode_object(row: &Row<'_>) -> Result<Option<ObjectRef>, RecoveryError> {
    let kind = row
        .get::<_, Option<String>>(9)
        .map_err(|_| RecoveryError::QueryFailed)?;
    let id = row
        .get::<_, Option<String>>(10)
        .map_err(|_| RecoveryError::QueryFailed)?;
    let version = row
        .get::<_, Option<i64>>(11)
        .map_err(|_| RecoveryError::QueryFailed)?;
    let digest = row
        .get::<_, Option<String>>(12)
        .map_err(|_| RecoveryError::QueryFailed)?;
    match (kind, id, version, digest) {
        (None, None, None, None) => Ok(None),
        (Some(kind), Some(id), Some(version), Some(digest)) => ObjectRef::new(
            kind,
            id,
            ObjectVersion::new(
                u64::try_from(version).map_err(|_| RecoveryError::InvalidEventRecord)?,
            )
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

fn parse_actor(value: &str) -> Result<crate::domain::Actor, RecoveryError> {
    match value {
        "human" => Ok(crate::domain::Actor::Human),
        "system" => Ok(crate::domain::Actor::System),
        _ => Err(RecoveryError::InvalidEventRecord),
    }
}

fn actor_kind(actor: &crate::domain::Actor) -> &'static str {
    match actor {
        crate::domain::Actor::Human => "human",
        crate::domain::Actor::System => "system",
    }
}

fn is_constraint(error: &SqliteError) -> bool {
    matches!(error, SqliteError::SqliteFailure(error, _) if error.code == ErrorCode::ConstraintViolation)
}

fn map_sqlite(error: SqliteError) -> PersistenceError {
    match error {
        SqliteError::SqliteFailure(error, _)
            if matches!(
                error.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            ) =>
        {
            PersistenceError::Contention
        }
        SqliteError::SqliteFailure(error, Some(message))
            if error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER
                && message == "event_stream is append-only" =>
        {
            PersistenceError::ImmutableEventStream
        }
        _ => PersistenceError::QueryFailed,
    }
}

fn persistence_from_recovery(error: RecoveryError) -> PersistenceError {
    match error {
        RecoveryError::UnsupportedEventSchema => PersistenceError::UnsupportedEventSchema,
        RecoveryError::InvalidEventRecord | RecoveryError::InvalidPredecessorShape => {
            PersistenceError::InvalidEventRecord
        }
        RecoveryError::PreviousEventDigestMismatch => PersistenceError::PreviousEventDigestMismatch,
        RecoveryError::EventDigestMismatch => PersistenceError::EventDigestMismatch,
        RecoveryError::EventSequenceGap
        | RecoveryError::EventSequenceOverflow
        | RecoveryError::QueryFailed => PersistenceError::QueryFailed,
    }
}
