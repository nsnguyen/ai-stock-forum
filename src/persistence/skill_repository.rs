use std::{collections::BTreeMap, str::FromStr};

use rusqlite::{
    Connection, Error as SqliteError, ErrorCode, OptionalExtension, Transaction, params,
};

use crate::{
    domain::{ObjectVersion, SkillId, SkillVersionId, canonical_json_bytes, sha256},
    skills::{NormalizedSkillName, SkillDraft, SkillProvenance, SkillVersion, SkillVersionRef},
};

use super::PersistenceError;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredSkillRow {
    skill_id: String,
    skill_version_id: String,
    version: i64,
    predecessor_version_id: Option<String>,
    display_name: String,
    normalized_name: String,
    content_digest: String,
    content_json: Vec<u8>,
    provenance_json: Vec<u8>,
    created_at_ms: i64,
    record_digest: String,
    record_json: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveSkillRow {
    skill_id: String,
    skill_version_id: String,
    version: i64,
    normalized_name: String,
    content_digest: String,
    record_digest: String,
}

pub fn insert_skill_version(
    transaction: &Transaction<'_>,
    skill: &SkillVersion,
) -> Result<(), PersistenceError> {
    let expected = expected_row(skill)?;
    let existing = load_rows_matching_logical_key(transaction, &expected)?;
    match existing.as_slice() {
        [] => insert_row(transaction, &expected),
        [stored] if stored == &expected => Ok(()),
        _ => Err(PersistenceError::SkillVersionIntegrityMismatch),
    }
}

pub fn reconcile_skill_versions(
    transaction: &Transaction<'_>,
    expected: &[SkillVersion],
) -> Result<(), PersistenceError> {
    for skill in expected {
        insert_skill_version(transaction, skill)?;
    }
    Ok(())
}

pub fn load_all_skill_versions(
    connection: &Connection,
) -> Result<Vec<SkillVersion>, PersistenceError> {
    let mut statement = connection
        .prepare("SELECT DISTINCT skill_id FROM skill_versions ORDER BY skill_id")
        .map_err(|_| PersistenceError::QueryFailed)?;
    let skill_ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| PersistenceError::QueryFailed)?
        .map(|row| {
            row.map_err(|_| PersistenceError::QueryFailed)
                .and_then(parse_id)
        })
        .collect::<Result<Vec<SkillId>, _>>()?;
    drop(statement);

    let mut versions = Vec::new();
    for skill_id in skill_ids {
        versions.extend(load_validated_history(connection, skill_id)?);
    }
    Ok(versions)
}

pub fn load_skill_history(
    connection: &Connection,
    skill_id: SkillId,
) -> Result<Vec<SkillVersion>, PersistenceError> {
    load_validated_history(connection, skill_id)
}

pub fn load_skill_version(
    connection: &Connection,
    reference: &SkillVersionRef,
) -> Result<Option<SkillVersion>, PersistenceError> {
    let version = load_validated_history(connection, reference.skill_id())?
        .into_iter()
        .find(|version| version.skill_version_id() == reference.skill_version_id());
    match version {
        Some(version) if version.reference() == *reference => Ok(Some(version)),
        Some(_) => Err(PersistenceError::SkillVersionReferenceMismatch),
        None => Ok(None),
    }
}

pub fn validate_skill_version_ref(
    connection: &Connection,
    reference: &SkillVersionRef,
) -> Result<(), PersistenceError> {
    match load_skill_version(connection, reference)? {
        Some(_) => Ok(()),
        None => Err(PersistenceError::SkillVersionReferenceMismatch),
    }
}

pub(crate) fn validate_skill_version_refs_batch(
    connection: &Connection,
    references: &[SkillVersionRef],
) -> Result<(), PersistenceError> {
    let skill_ids = references
        .iter()
        .map(|reference| reference.skill_id().to_string())
        .collect::<Vec<_>>();
    let ids_json = canonical_json_bytes(&skill_ids)
        .map_err(|_| PersistenceError::SkillVersionIntegrityMismatch)?;
    let mut statement = connection
        .prepare(
            "SELECT skill_id, skill_version_id, version, predecessor_version_id,
                    display_name, normalized_name, content_digest, content_json,
                    provenance_json, created_at_ms, record_digest, record_json
               FROM skill_versions
              WHERE skill_id IN (SELECT value FROM json_each(CAST(?1 AS TEXT)))
              ORDER BY skill_id, version",
        )
        .map_err(|_| PersistenceError::QueryFailed)?;
    let rows = statement
        .query_map([ids_json], decode_row)
        .map_err(|_| PersistenceError::QueryFailed)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| PersistenceError::QueryFailed)?;
    let mut grouped = BTreeMap::<String, Vec<StoredSkillRow>>::new();
    for row in rows {
        grouped.entry(row.skill_id.clone()).or_default().push(row);
    }
    let mut authenticated = Vec::new();
    for (skill_id, rows) in grouped {
        let skill_id = parse_id::<SkillId>(skill_id)?;
        let mut versions = Vec::with_capacity(rows.len());
        for row in rows {
            if sha256(&row.record_json).as_str() != row.record_digest {
                return Err(PersistenceError::SkillVersionIntegrityMismatch);
            }
            let row_skill_id = parse_id::<SkillId>(row.skill_id.clone())?;
            let row_version_id = parse_id::<SkillVersionId>(row.skill_version_id.clone())?;
            let content = serde_json::from_slice::<SkillDraft>(&row.content_json)
                .map_err(|_| PersistenceError::SkillVersionIntegrityMismatch)?;
            let provenance = serde_json::from_slice::<SkillProvenance>(&row.provenance_json)
                .map_err(|_| PersistenceError::SkillVersionIntegrityMismatch)?;
            let version = if let Some(previous) = versions.last() {
                SkillVersion::next_version(previous, row_version_id, row.created_at_ms, content)
            } else {
                SkillVersion::create(
                    row_skill_id,
                    row_version_id,
                    row.created_at_ms,
                    provenance,
                    content,
                )
            }
            .map_err(|_| PersistenceError::SkillVersionIntegrityMismatch)?;
            if version.skill_id() != skill_id || expected_row(&version)? != row {
                return Err(PersistenceError::SkillVersionIntegrityMismatch);
            }
            versions.push(version);
        }
        authenticated.extend(versions);
    }
    if references.iter().all(|reference| {
        authenticated
            .iter()
            .any(|skill| skill.reference() == *reference)
    }) {
        Ok(())
    } else {
        Err(PersistenceError::SkillVersionReferenceMismatch)
    }
}

pub fn load_skill_version_by_id(
    connection: &Connection,
    skill_version_id: SkillVersionId,
) -> Result<Option<SkillVersion>, PersistenceError> {
    let skill_id = connection
        .query_row(
            "SELECT skill_id FROM skill_versions WHERE skill_version_id = ?1",
            [skill_version_id.to_string()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| PersistenceError::QueryFailed)?
        .map(parse_id)
        .transpose()?;
    let Some(skill_id) = skill_id else {
        return Ok(None);
    };
    Ok(load_validated_history(connection, skill_id)?
        .into_iter()
        .find(|version| version.skill_version_id() == skill_version_id))
}

pub fn load_active_skill(
    connection: &Connection,
    skill_id: SkillId,
) -> Result<Option<SkillVersion>, PersistenceError> {
    let active = load_active_row(connection, "skill_id", skill_id.to_string())?;
    validate_active_row(connection, active)
}

pub fn load_active_skill_by_name(
    connection: &Connection,
    normalized_name: &NormalizedSkillName,
) -> Result<Option<SkillVersion>, PersistenceError> {
    let active = load_active_row(connection, "normalized_name", normalized_name.as_str())?;
    validate_active_row(connection, active)
}

pub fn set_active_skill(
    transaction: &Transaction<'_>,
    skill: &SkillVersion,
) -> Result<(), PersistenceError> {
    let stored = load_skill_version_by_id(transaction, skill.skill_version_id())?
        .ok_or(PersistenceError::SkillVersionReferenceMismatch)?;
    if stored != *skill {
        return Err(PersistenceError::SkillVersionIntegrityMismatch);
    }
    let row = expected_active_row(skill)?;
    transaction
        .execute(
            "INSERT INTO active_skills (
                skill_id, skill_version_id, version, normalized_name,
                content_digest, record_digest
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(skill_id) DO UPDATE SET
                skill_version_id = excluded.skill_version_id,
                version = excluded.version,
                normalized_name = excluded.normalized_name,
                content_digest = excluded.content_digest,
                record_digest = excluded.record_digest",
            params![
                row.skill_id,
                row.skill_version_id,
                row.version,
                row.normalized_name,
                row.content_digest,
                row.record_digest,
            ],
        )
        .map(|_| ())
        .map_err(map_active_insert_error)
}

pub fn replace_active_skills(
    transaction: &Transaction<'_>,
    active: &[SkillVersion],
) -> Result<(), PersistenceError> {
    transaction
        .execute("DELETE FROM active_skills", [])
        .map_err(|_| PersistenceError::ActiveSkillRebuildFailed)?;
    for skill in active {
        set_active_skill(transaction, skill).map_err(|error| match error {
            PersistenceError::DuplicateSkillName => error,
            _ => PersistenceError::ActiveSkillRebuildFailed,
        })?;
    }
    Ok(())
}

fn expected_row(skill: &SkillVersion) -> Result<StoredSkillRow, PersistenceError> {
    if skill.recompute_content_digest().map_err(invalid_skill)? != *skill.content_digest() {
        return Err(PersistenceError::SkillVersionIntegrityMismatch);
    }
    let record_json = canonical_json_bytes(skill).map_err(invalid_skill)?;
    Ok(StoredSkillRow {
        skill_id: skill.skill_id().to_string(),
        skill_version_id: skill.skill_version_id().to_string(),
        version: i64::try_from(skill.version().get()).map_err(invalid_skill)?,
        predecessor_version_id: skill.predecessor().map(|id| id.to_string()),
        display_name: skill.content().display_name.clone(),
        normalized_name: skill
            .normalized_name()
            .map_err(invalid_skill)?
            .as_str()
            .to_owned(),
        content_digest: skill.content_digest().as_str().to_owned(),
        content_json: canonical_json_bytes(skill.content()).map_err(invalid_skill)?,
        provenance_json: canonical_json_bytes(skill.provenance()).map_err(invalid_skill)?,
        created_at_ms: skill.created_at_ms(),
        record_digest: sha256(&record_json).as_str().to_owned(),
        record_json,
    })
}

fn expected_active_row(skill: &SkillVersion) -> Result<ActiveSkillRow, PersistenceError> {
    let row = expected_row(skill)?;
    Ok(ActiveSkillRow {
        skill_id: row.skill_id,
        skill_version_id: row.skill_version_id,
        version: row.version,
        normalized_name: row.normalized_name,
        content_digest: row.content_digest,
        record_digest: row.record_digest,
    })
}

fn load_validated_history(
    connection: &Connection,
    skill_id: SkillId,
) -> Result<Vec<SkillVersion>, PersistenceError> {
    let rows = load_rows_for_skill(connection, skill_id)?;
    let mut versions = Vec::with_capacity(rows.len());
    for row in rows {
        if sha256(&row.record_json).as_str() != row.record_digest {
            return Err(PersistenceError::SkillVersionIntegrityMismatch);
        }
        let row_skill_id = parse_id::<SkillId>(row.skill_id.clone())?;
        let row_version_id = parse_id::<SkillVersionId>(row.skill_version_id.clone())?;
        let content = serde_json::from_slice::<SkillDraft>(&row.content_json)
            .map_err(|_| PersistenceError::SkillVersionIntegrityMismatch)?;
        let provenance = serde_json::from_slice::<SkillProvenance>(&row.provenance_json)
            .map_err(|_| PersistenceError::SkillVersionIntegrityMismatch)?;
        let version = if versions.is_empty() {
            SkillVersion::create(
                row_skill_id,
                row_version_id,
                row.created_at_ms,
                provenance,
                content,
            )
        } else {
            SkillVersion::next_version(
                versions.last().expect("history is non-empty"),
                row_version_id,
                row.created_at_ms,
                content,
            )
        }
        .map_err(|_| PersistenceError::SkillVersionIntegrityMismatch)?;
        if version.skill_id() != skill_id
            || expected_row(&version)? != row
            || ObjectVersion::new(u64::try_from(row.version).map_err(invalid_skill)?)
                .map_err(invalid_skill)?
                != version.version()
        {
            return Err(PersistenceError::SkillVersionIntegrityMismatch);
        }
        versions.push(version);
    }
    Ok(versions)
}

fn load_rows_matching_logical_key(
    connection: &Connection,
    expected: &StoredSkillRow,
) -> Result<Vec<StoredSkillRow>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT skill_id, skill_version_id, version, predecessor_version_id,
                    display_name, normalized_name, content_digest, content_json,
                    provenance_json, created_at_ms, record_digest, record_json
             FROM skill_versions
             WHERE (skill_id = ?1 AND version = ?2) OR skill_version_id = ?3
             ORDER BY skill_id, version",
        )
        .map_err(|_| PersistenceError::QueryFailed)?;
    statement
        .query_map(
            params![
                expected.skill_id,
                expected.version,
                expected.skill_version_id
            ],
            decode_row,
        )
        .map_err(|_| PersistenceError::QueryFailed)?
        .map(|row| row.map_err(|_| PersistenceError::QueryFailed))
        .collect()
}

fn load_rows_for_skill(
    connection: &Connection,
    skill_id: SkillId,
) -> Result<Vec<StoredSkillRow>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT skill_id, skill_version_id, version, predecessor_version_id,
                    display_name, normalized_name, content_digest, content_json,
                    provenance_json, created_at_ms, record_digest, record_json
             FROM skill_versions WHERE skill_id = ?1 ORDER BY version",
        )
        .map_err(|_| PersistenceError::QueryFailed)?;
    statement
        .query_map([skill_id.to_string()], decode_row)
        .map_err(|_| PersistenceError::QueryFailed)?
        .map(|row| row.map_err(|_| PersistenceError::QueryFailed))
        .collect()
}

fn load_active_row(
    connection: &Connection,
    column: &str,
    value: impl rusqlite::ToSql,
) -> Result<Option<ActiveSkillRow>, PersistenceError> {
    debug_assert!(matches!(column, "skill_id" | "normalized_name"));
    connection
        .query_row(
            &format!(
                "SELECT skill_id, skill_version_id, version, normalized_name,
                        content_digest, record_digest
                 FROM active_skills WHERE {column} = ?1"
            ),
            [value],
            |row| {
                Ok(ActiveSkillRow {
                    skill_id: row.get(0)?,
                    skill_version_id: row.get(1)?,
                    version: row.get(2)?,
                    normalized_name: row.get(3)?,
                    content_digest: row.get(4)?,
                    record_digest: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|_| PersistenceError::QueryFailed)
}

fn validate_active_row(
    connection: &Connection,
    active: Option<ActiveSkillRow>,
) -> Result<Option<SkillVersion>, PersistenceError> {
    let Some(active) = active else {
        return Ok(None);
    };
    let version_id = parse_id::<SkillVersionId>(active.skill_version_id.clone())?;
    let version = load_skill_version_by_id(connection, version_id)?
        .ok_or(PersistenceError::SkillVersionIntegrityMismatch)?;
    if expected_active_row(&version)? != active {
        return Err(PersistenceError::SkillVersionIntegrityMismatch);
    }
    Ok(Some(version))
}

fn insert_row(transaction: &Transaction<'_>, row: &StoredSkillRow) -> Result<(), PersistenceError> {
    transaction
        .execute(
            "INSERT INTO skill_versions (
                skill_id, skill_version_id, version, predecessor_version_id,
                display_name, normalized_name, content_digest, content_json,
                provenance_json, created_at_ms, record_digest, record_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                row.skill_id,
                row.skill_version_id,
                row.version,
                row.predecessor_version_id,
                row.display_name,
                row.normalized_name,
                row.content_digest,
                row.content_json,
                row.provenance_json,
                row.created_at_ms,
                row.record_digest,
                row.record_json,
            ],
        )
        .map(|_| ())
        .map_err(map_skill_insert_error)
}

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSkillRow> {
    Ok(StoredSkillRow {
        skill_id: row.get(0)?,
        skill_version_id: row.get(1)?,
        version: row.get(2)?,
        predecessor_version_id: row.get(3)?,
        display_name: row.get(4)?,
        normalized_name: row.get(5)?,
        content_digest: row.get(6)?,
        content_json: row.get(7)?,
        provenance_json: row.get(8)?,
        created_at_ms: row.get(9)?,
        record_digest: row.get(10)?,
        record_json: row.get(11)?,
    })
}

fn parse_id<T: FromStr>(value: String) -> Result<T, PersistenceError> {
    value
        .parse()
        .map_err(|_| PersistenceError::SkillVersionIntegrityMismatch)
}

fn invalid_skill<T>(_: T) -> PersistenceError {
    PersistenceError::SkillVersionIntegrityMismatch
}

fn map_skill_insert_error(error: SqliteError) -> PersistenceError {
    match error {
        SqliteError::SqliteFailure(sqlite, _) if sqlite.code == ErrorCode::ConstraintViolation => {
            PersistenceError::SkillVersionIntegrityMismatch
        }
        _ => PersistenceError::QueryFailed,
    }
}

fn map_active_insert_error(error: SqliteError) -> PersistenceError {
    match error {
        SqliteError::SqliteFailure(sqlite, Some(message))
            if sqlite.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                && message == "UNIQUE constraint failed: active_skills.normalized_name" =>
        {
            PersistenceError::DuplicateSkillName
        }
        SqliteError::SqliteFailure(sqlite, _) if sqlite.code == ErrorCode::ConstraintViolation => {
            PersistenceError::ActiveSkillRebuildFailed
        }
        _ => PersistenceError::QueryFailed,
    }
}
