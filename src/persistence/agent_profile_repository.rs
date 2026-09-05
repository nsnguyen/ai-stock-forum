use rusqlite::{Connection, Error as SqliteError, ErrorCode, Transaction, params};

use crate::{
    agents::{AgentProfileVersion, AgentProfilesProjection, AgentReadiness},
    app::{ApplicationEvent, EventEnvelope},
    domain::canonical_json_bytes,
};

use super::PersistenceError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAgentProfileVersion {
    pub event_sequence: i64,
    pub profile: AgentProfileVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredRow {
    profile_id: String,
    profile_version_id: String,
    version: i64,
    normalized_name: String,
    content_digest: String,
    payload_json: Vec<u8>,
    source_event_sequence: i64,
    created_at_ms: i64,
}

pub fn insert_expected_version(
    transaction: &Transaction<'_>,
    event_sequence: i64,
    profile: &AgentProfileVersion,
) -> Result<(), PersistenceError> {
    let expected = expected_row(event_sequence, profile)?;
    let existing = load_rows_matching_logical_key(transaction, &expected)?;
    match existing.as_slice() {
        [] => insert_row(transaction, &expected),
        [stored] if stored == &expected => Ok(()),
        _ => Err(PersistenceError::AgentProfileHistoryMismatch),
    }
}

pub fn load_all_versions(
    connection: &Connection,
) -> Result<Vec<StoredAgentProfileVersion>, PersistenceError> {
    load_all_rows(connection)?
        .into_iter()
        .map(|stored| {
            let profile = serde_json::from_slice::<AgentProfileVersion>(&stored.payload_json)
                .map_err(|_| PersistenceError::InvalidAgentProfilePayload)?;
            if expected_row(stored.source_event_sequence, &profile)? != stored {
                return Err(PersistenceError::AgentProfileHistoryMismatch);
            }
            Ok(StoredAgentProfileVersion {
                event_sequence: stored.source_event_sequence,
                profile,
            })
        })
        .collect()
}

pub fn replace_active_profiles(
    transaction: &Transaction<'_>,
    projection: &AgentProfilesProjection,
) -> Result<(), PersistenceError> {
    transaction
        .execute("DELETE FROM active_agent_profiles", [])
        .map_err(|_| PersistenceError::ActiveAgentProfileRebuildFailed)?;
    for profile in projection.active_profiles() {
        transaction
            .execute(
                "INSERT INTO active_agent_profiles (
                    profile_id, profile_version_id, version, normalized_name, readiness
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    profile.profile_id().to_string(),
                    profile.profile_version_id().to_string(),
                    i64::try_from(profile.version().get())
                        .map_err(|_| PersistenceError::ActiveAgentProfileRebuildFailed)?,
                    profile.normalized_name().as_str(),
                    readiness(profile_readiness(&profile)),
                ],
            )
            .map_err(map_active_profile_insert_error)?;
    }
    Ok(())
}

pub(crate) fn reconcile_expected_versions(
    transaction: &Transaction<'_>,
    events: &[EventEnvelope],
) -> Result<(), super::RecoveryError> {
    let expected = events
        .iter()
        .filter_map(|event| match &event.event {
            ApplicationEvent::AgentProfileCreated { profile }
            | ApplicationEvent::AgentProfileVersionActivated { profile, .. } => {
                Some((event.sequence, profile))
            }
            _ => None,
        })
        .map(|(sequence, profile)| {
            let sequence = i64::try_from(sequence)
                .map_err(|_| super::RecoveryError::InvalidAgentProfilePayload)?;
            expected_row(sequence, profile).map_err(recovery_from_persistence)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let existing = load_all_rows(transaction).map_err(recovery_from_persistence)?;
    let mut matched = vec![false; existing.len()];
    let mut missing = Vec::new();

    for expected_row in expected {
        let candidates = existing
            .iter()
            .enumerate()
            .filter(|(_, stored)| same_logical_key(stored, &expected_row))
            .collect::<Vec<_>>();
        match candidates.as_slice() {
            [] => missing.push(expected_row),
            [(index, stored)] if *stored == &expected_row => matched[*index] = true,
            _ => return Err(super::RecoveryError::AgentProfileHistoryMismatch),
        }
    }

    if matched.iter().any(|matched| !matched) {
        return Err(super::RecoveryError::UnexpectedAgentProfileHistory);
    }
    for row in missing {
        insert_row(transaction, &row).map_err(recovery_from_persistence)?;
    }
    Ok(())
}

pub(crate) fn active_profiles_match(
    connection: &Connection,
    projection: &AgentProfilesProjection,
) -> Result<bool, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT profile_id, profile_version_id, version, normalized_name, readiness
             FROM active_agent_profiles ORDER BY normalized_name, profile_id",
        )
        .map_err(|_| PersistenceError::QueryFailed)?;
    let stored = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|_| PersistenceError::QueryFailed)?
        .map(|row| row.map_err(|_| PersistenceError::QueryFailed))
        .collect::<Result<Vec<_>, _>>()?;
    let expected = projection
        .active_profiles()
        .into_iter()
        .map(|profile| {
            Ok((
                profile.profile_id().to_string(),
                profile.profile_version_id().to_string(),
                i64::try_from(profile.version().get())
                    .map_err(|_| PersistenceError::InvalidAgentProfilePayload)?,
                profile.normalized_name().as_str().to_owned(),
                readiness(profile_readiness(&profile)).to_owned(),
            ))
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?;
    Ok(stored == expected)
}

fn expected_row(
    event_sequence: i64,
    profile: &AgentProfileVersion,
) -> Result<StoredRow, PersistenceError> {
    if event_sequence < 1 {
        return Err(PersistenceError::InvalidAgentProfilePayload);
    }
    Ok(StoredRow {
        profile_id: profile.profile_id().to_string(),
        profile_version_id: profile.profile_version_id().to_string(),
        version: i64::try_from(profile.version().get())
            .map_err(|_| PersistenceError::InvalidAgentProfilePayload)?,
        normalized_name: profile.normalized_name().as_str().to_owned(),
        content_digest: profile.content_digest().as_str().to_owned(),
        payload_json: canonical_json_bytes(profile)
            .map_err(|_| PersistenceError::InvalidAgentProfilePayload)?,
        source_event_sequence: event_sequence,
        created_at_ms: profile.created_at_ms(),
    })
}

fn load_rows_matching_logical_key(
    connection: &Connection,
    expected: &StoredRow,
) -> Result<Vec<StoredRow>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT profile_id, profile_version_id, version, normalized_name, content_digest,
                    payload_json, source_event_sequence, created_at_ms
             FROM agent_profile_versions
             WHERE (profile_id = ?1 AND version = ?2)
                OR profile_version_id = ?3
                OR source_event_sequence = ?4
             ORDER BY source_event_sequence",
        )
        .map_err(|_| PersistenceError::QueryFailed)?;
    statement
        .query_map(
            params![
                expected.profile_id,
                expected.version,
                expected.profile_version_id,
                expected.source_event_sequence,
            ],
            decode_row,
        )
        .map_err(|_| PersistenceError::QueryFailed)?
        .map(|row| row.map_err(|_| PersistenceError::QueryFailed))
        .collect()
}

fn load_all_rows(connection: &Connection) -> Result<Vec<StoredRow>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT profile_id, profile_version_id, version, normalized_name, content_digest,
                    payload_json, source_event_sequence, created_at_ms
             FROM agent_profile_versions ORDER BY source_event_sequence",
        )
        .map_err(|_| PersistenceError::QueryFailed)?;
    statement
        .query_map([], decode_row)
        .map_err(|_| PersistenceError::QueryFailed)?
        .map(|row| row.map_err(|_| PersistenceError::QueryFailed))
        .collect()
}

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRow> {
    Ok(StoredRow {
        profile_id: row.get(0)?,
        profile_version_id: row.get(1)?,
        version: row.get(2)?,
        normalized_name: row.get(3)?,
        content_digest: row.get(4)?,
        payload_json: row.get(5)?,
        source_event_sequence: row.get(6)?,
        created_at_ms: row.get(7)?,
    })
}

fn insert_row(
    transaction: &Transaction<'_>,
    row: &StoredRow,
) -> Result<(), PersistenceError> {
    transaction
        .execute(
            "INSERT INTO agent_profile_versions (
                profile_id, profile_version_id, version, normalized_name, content_digest,
                payload_json, source_event_sequence, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                row.profile_id,
                row.profile_version_id,
                row.version,
                row.normalized_name,
                row.content_digest,
                row.payload_json,
                row.source_event_sequence,
                row.created_at_ms,
            ],
        )
        .map(|_| ())
        .map_err(map_insert_error)
}

fn same_logical_key(left: &StoredRow, right: &StoredRow) -> bool {
    (left.profile_id == right.profile_id && left.version == right.version)
        || left.profile_version_id == right.profile_version_id
        || left.source_event_sequence == right.source_event_sequence
}

fn readiness(readiness: AgentReadiness) -> &'static str {
    match readiness {
        AgentReadiness::Ready => "ready",
        AgentReadiness::NotReady => "not_ready",
    }
}

fn profile_readiness(profile: &AgentProfileVersion) -> AgentReadiness {
    match (
        &profile.bindings().model_provider,
        &profile.bindings().model_name,
    ) {
        (Some(_), Some(_)) => AgentReadiness::Ready,
        _ => AgentReadiness::NotReady,
    }
}

fn map_insert_error(error: SqliteError) -> PersistenceError {
    match error {
        SqliteError::SqliteFailure(error, _)
            if error.code == ErrorCode::ConstraintViolation =>
        {
            PersistenceError::AgentProfileHistoryMismatch
        }
        _ => PersistenceError::QueryFailed,
    }
}

fn map_active_profile_insert_error(error: SqliteError) -> PersistenceError {
    match error {
        SqliteError::SqliteFailure(error, Some(message))
            if error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                && message
                    == "UNIQUE constraint failed: active_agent_profiles.normalized_name" =>
        {
            PersistenceError::AgentProfileHistoryMismatch
        }
        _ => PersistenceError::ActiveAgentProfileRebuildFailed,
    }
}

fn recovery_from_persistence(error: PersistenceError) -> super::RecoveryError {
    match error {
        PersistenceError::AgentProfileHistoryMismatch => {
            super::RecoveryError::AgentProfileHistoryMismatch
        }
        PersistenceError::InvalidAgentProfilePayload => {
            super::RecoveryError::InvalidAgentProfilePayload
        }
        PersistenceError::ActiveAgentProfileRebuildFailed => {
            super::RecoveryError::ActiveAgentProfileRebuildFailed
        }
        _ => super::RecoveryError::QueryFailed,
    }
}
