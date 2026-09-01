use std::str::FromStr;

use rusqlite::{params, Error as SqliteError, ErrorCode, OptionalExtension};

use crate::domain::{CommandId, EventId, Sha256Digest};

use super::{ImmediateTransaction, PersistenceError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandReceiptRecord {
    pub command_id: CommandId,
    pub command_fingerprint: Sha256Digest,
    pub request_json: String,
    pub capability: String,
    pub policy_decision: String,
    pub outcome_json: String,
    pub event_ids: Vec<EventId>,
}

pub struct CommandReceiptRepository;

impl CommandReceiptRepository {
    pub fn load(
        transaction: &ImmediateTransaction<'_>,
        command_id: CommandId,
    ) -> Result<Option<CommandReceiptRecord>, PersistenceError> {
        let row = transaction.transaction().query_row(
            "SELECT command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json FROM command_receipts WHERE command_id = ?1",
            [command_id.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?)),
        ).optional().map_err(map_sqlite)?;
        let Some((stored_id, fingerprint, request_json, capability, policy_decision, outcome_json)) = row else { return Ok(None); };
        let stored_id = CommandId::from_str(&stored_id).map_err(|_| PersistenceError::InvalidEventRecord)?;
        if stored_id != command_id { return Err(PersistenceError::InvalidEventRecord); }
        let command_fingerprint = Sha256Digest::parse(&fingerprint).map_err(|_| PersistenceError::InvalidEventRecord)?;
        let mut statement = transaction.transaction().prepare(
            "SELECT event_ordinal, event_id FROM command_event_refs WHERE command_id = ?1 ORDER BY event_ordinal",
        ).map_err(map_sqlite)?;
        let rows = statement.query_map([command_id.to_string()], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))).map_err(map_sqlite)?;
        let mut event_ids = Vec::new();
        for (expected, row) in rows.enumerate() {
            let (ordinal, event_id) = row.map_err(map_sqlite)?;
            if usize::try_from(ordinal).ok() != Some(expected) { return Err(PersistenceError::InvalidEventRecord); }
            event_ids.push(EventId::from_str(&event_id).map_err(|_| PersistenceError::InvalidEventRecord)?);
        }
        Ok(Some(CommandReceiptRecord { command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json, event_ids }))
    }

    pub fn insert(transaction: &ImmediateTransaction<'_>, receipt: &CommandReceiptRecord) -> Result<(), PersistenceError> {
        transaction.transaction().execute(
            "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![receipt.command_id.to_string(), receipt.command_fingerprint.as_str(), receipt.request_json, receipt.capability, receipt.policy_decision, receipt.outcome_json],
        ).map_err(map_sqlite)?;
        for (ordinal, event_id) in receipt.event_ids.iter().enumerate() {
            transaction.transaction().execute(
                "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES (?1, ?2, ?3)",
                params![receipt.command_id.to_string(), i64::try_from(ordinal).map_err(|_| PersistenceError::InvalidEventRecord)?, event_id.to_string()],
            ).map_err(map_sqlite)?;
        }
        Ok(())
    }
}

fn map_sqlite(error: SqliteError) -> PersistenceError {
    match error {
        SqliteError::SqliteFailure(error, _) if matches!(error.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => PersistenceError::Contention,
        SqliteError::SqliteFailure(error, _) if error.code == ErrorCode::ConstraintViolation => PersistenceError::IdempotencyConflict,
        _ => PersistenceError::QueryFailed,
    }
}
