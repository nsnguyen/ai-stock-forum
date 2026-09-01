use std::{collections::BTreeMap, str::FromStr};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::{
    app::{EventEnvelope, ShutdownReason},
    domain::Sha256Digest,
    recovery::{InstallationProjection, ProjectionState, SessionEndProjection, SessionProjection},
};

use super::{EventRepository, ImmediateTransaction, PersistenceError, RecoveryError};

pub struct ProjectionRepository;

impl ProjectionRepository {
    pub fn load(connection: &Connection) -> Result<ProjectionState, RecoveryError> {
        let installation = connection
            .query_row(
                "SELECT installation_id, created_event_id, created_at_ms FROM installation_projection WHERE singleton = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|_| RecoveryError::QueryFailed)?
            .map(|(installation_id, created_event_id, created_at_ms)| {
                Ok(InstallationProjection {
                    installation_id: parse_id(installation_id)?,
                    created_event_id: parse_id(created_event_id)?,
                    created_at_ms,
                })
            })
            .transpose()?;
        let mut statement = connection
            .prepare(
                "SELECT session_id, started_event_id, started_at_ms, ended_event_id, ended_at_ms, end_reason FROM process_session_projection ORDER BY session_id",
            )
            .map_err(|_| RecoveryError::QueryFailed)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(|_| RecoveryError::QueryFailed)?;
        let mut sessions = BTreeMap::new();
        for row in rows {
            let (session_id, started_event_id, started_at_ms, ended_event_id, ended_at_ms, reason) =
                row.map_err(|_| RecoveryError::QueryFailed)?;
            let session_id = parse_id(session_id)?;
            let ended = match (ended_event_id, ended_at_ms, reason) {
                (None, None, None) => None,
                (Some(event_id), Some(ended_at_ms), Some(reason)) => Some(SessionEndProjection {
                    ended_event_id: parse_id(event_id)?,
                    ended_at_ms,
                    reason: parse_shutdown_reason(&reason)?,
                }),
                _ => return Err(RecoveryError::InvalidEventRecord),
            };
            if sessions
                .insert(
                    session_id,
                    SessionProjection {
                        session_id,
                        started_event_id: parse_id(started_event_id)?,
                        started_at_ms,
                        ended,
                    },
                )
                .is_some()
            {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        let metadata = connection
            .query_row(
                "SELECT last_event_sequence, last_event_digest, projection_digest FROM projection_metadata WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| RecoveryError::QueryFailed)?;
        let (last_sequence, last_event_digest, persisted_digest) = match metadata {
            Some((sequence, digest, projection_digest)) => (
                u64::try_from(sequence).map_err(|_| RecoveryError::InvalidEventRecord)?,
                digest.map(|value| Sha256Digest::parse(&value)).transpose().map_err(|_| RecoveryError::InvalidEventRecord)?,
                Some(Sha256Digest::parse(&projection_digest).map_err(|_| RecoveryError::InvalidEventRecord)?),
            ),
            None => (0, None, None),
        };
        let state = ProjectionState {
            installation,
            sessions,
            last_sequence,
            last_event_digest,
            ..ProjectionState::default()
        };
        state.validate()?;
        if let Some(persisted_digest) = persisted_digest {
            if state.digest().map_err(|_| RecoveryError::InvalidEventRecord)? != persisted_digest {
                return Err(RecoveryError::InvalidEventRecord);
            }
        }
        Ok(state)
    }

    pub fn store(
        transaction: &ImmediateTransaction<'_>,
        state: &ProjectionState,
    ) -> Result<(), PersistenceError> {
        store_transaction(transaction.transaction(), state)
    }

    pub fn rebuild(
        connection: &mut Connection,
        events: &[EventEnvelope],
    ) -> Result<ProjectionState, RecoveryError> {
        EventRepository::verify(connection)?;
        if EventRepository::load_all(connection)? != events {
            return Err(RecoveryError::InvalidEventRecord);
        }
        let mut state = ProjectionState::default();
        for event in events {
            crate::recovery::reduce(&mut state, event)?;
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| RecoveryError::QueryFailed)?;
        transaction
            .execute("DELETE FROM projection_metadata", [])
            .map_err(|_| RecoveryError::QueryFailed)?;
        transaction
            .execute("DELETE FROM process_session_projection", [])
            .map_err(|_| RecoveryError::QueryFailed)?;
        transaction
            .execute("DELETE FROM installation_projection", [])
            .map_err(|_| RecoveryError::QueryFailed)?;
        store_transaction(&transaction, &state).map_err(recovery_from_persistence)?;
        transaction.commit().map_err(|_| RecoveryError::QueryFailed)?;
        Ok(state)
    }
}

fn store_transaction(
    transaction: &Transaction<'_>,
    state: &ProjectionState,
) -> Result<(), PersistenceError> {
    state.validate().map_err(|_| PersistenceError::InvalidEventRecord)?;
    if let Some(installation) = &state.installation {
        transaction
            .execute(
                "INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (1, ?1, ?2, ?3) ON CONFLICT(singleton) DO UPDATE SET installation_id = excluded.installation_id, created_event_id = excluded.created_event_id, created_at_ms = excluded.created_at_ms",
                params![
                    installation.installation_id.to_string(),
                    installation.created_event_id.to_string(),
                    installation.created_at_ms,
                ],
            )
            .map_err(|_| PersistenceError::QueryFailed)?;
    }
    for session in state.sessions.values() {
        let (ended_event_id, ended_at_ms, end_reason) = match &session.ended {
            Some(ended) => (
                Some(ended.ended_event_id.to_string()),
                Some(ended.ended_at_ms),
                Some(shutdown_reason(ended.reason)),
            ),
            None => (None, None, None),
        };
        transaction
            .execute(
                "INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms, ended_event_id, ended_at_ms, end_reason) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(session_id) DO UPDATE SET started_event_id = excluded.started_event_id, started_at_ms = excluded.started_at_ms, ended_event_id = excluded.ended_event_id, ended_at_ms = excluded.ended_at_ms, end_reason = excluded.end_reason",
                params![
                    session.session_id.to_string(),
                    session.started_event_id.to_string(),
                    session.started_at_ms,
                    ended_event_id,
                    ended_at_ms,
                    end_reason,
                ],
            )
            .map_err(|_| PersistenceError::QueryFailed)?;
    }
    let digest = state
        .digest()
        .map_err(|_| PersistenceError::InvalidEventRecord)?;
    transaction
        .execute(
            "INSERT INTO projection_metadata (singleton, last_event_sequence, last_event_digest, projection_digest) VALUES (1, ?1, ?2, ?3) ON CONFLICT(singleton) DO UPDATE SET last_event_sequence = excluded.last_event_sequence, last_event_digest = excluded.last_event_digest, projection_digest = excluded.projection_digest",
            params![
                i64::try_from(state.last_sequence).map_err(|_| PersistenceError::InvalidEventRecord)?,
                state.last_event_digest.as_ref().map(Sha256Digest::as_str),
                digest.as_str(),
            ],
        )
        .map_err(|_| PersistenceError::QueryFailed)?;
    Ok(())
}

fn parse_id<T: FromStr>(value: String) -> Result<T, RecoveryError> {
    value.parse().map_err(|_| RecoveryError::InvalidEventRecord)
}

fn parse_shutdown_reason(value: &str) -> Result<ShutdownReason, RecoveryError> {
    match value {
        "user_quit" => Ok(ShutdownReason::UserQuit),
        "input_closed" => Ok(ShutdownReason::InputClosed),
        "interrupted" => Ok(ShutdownReason::Interrupted),
        "application_error" => Ok(ShutdownReason::ApplicationError),
        _ => Err(RecoveryError::InvalidEventRecord),
    }
}

fn shutdown_reason(reason: ShutdownReason) -> &'static str {
    match reason {
        ShutdownReason::UserQuit => "user_quit",
        ShutdownReason::InputClosed => "input_closed",
        ShutdownReason::Interrupted => "interrupted",
        ShutdownReason::ApplicationError => "application_error",
    }
}

fn recovery_from_persistence(error: PersistenceError) -> RecoveryError {
    match error {
        PersistenceError::InvalidEventRecord => RecoveryError::InvalidEventRecord,
        _ => RecoveryError::QueryFailed,
    }
}
