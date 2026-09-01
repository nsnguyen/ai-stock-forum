use std::collections::BTreeMap;

use rusqlite::{
    Connection, Error as SqliteError, ErrorCode, OpenFlags, Transaction, TransactionBehavior,
};
use thiserror::Error;

use crate::{
    config::{AppPaths, StartupError},
    domain::Sha256Digest,
};

use super::migrations::{
    ordered, AppliedMigration, APPLICATION_ID, LATEST_SCHEMA_VERSION, SCHEMA_MIGRATIONS_SQL,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PersistenceError {
    #[error("database query failed")]
    QueryFailed,
    #[error("database migration record is invalid")]
    InvalidMigrationRecord,
}

pub struct Database {
    connection: Connection,
    schema_version: u32,
}

impl Database {
    pub fn open(paths: &AppPaths) -> Result<Self, StartupError> {
        paths.ensure()?;

        let mut connection = Connection::open_with_flags(
            paths.database_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(startup_error)?;

        let application_id = pragma_i64(&connection, "application_id")?;
        if application_id != 0 && application_id != APPLICATION_ID {
            return Err(StartupError::DatabaseApplicationMismatch);
        }

        let user_version = pragma_i64(&connection, "user_version")?;
        if user_version > i64::from(LATEST_SCHEMA_VERSION) {
            return Err(StartupError::DatabaseSchemaNewer);
        }
        if user_version < 0 {
            return Err(StartupError::DatabaseMigrationState);
        }

        configure_connection(&connection)?;
        run_migrations(&mut connection, user_version as u32)?;
        quick_check(&connection)?;

        Ok(Self {
            connection,
            schema_version: LATEST_SCHEMA_VERSION,
        })
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }

    pub fn has_table(&self, name: &str) -> Result<bool, PersistenceError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                [name],
                |row| row.get(0),
            )
            .map_err(persistence_error)
    }

    pub fn applied_migrations(&self) -> Result<Vec<AppliedMigration>, PersistenceError> {
        let mut statement = self
            .connection
            .prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")
            .map_err(persistence_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(persistence_error)?;

        rows.map(|row| {
            let (version, checksum) = row.map_err(persistence_error)?;
            let version = u32::try_from(version).map_err(|_| PersistenceError::InvalidMigrationRecord)?;
            let checksum = Sha256Digest::parse(&checksum)
                .map_err(|_| PersistenceError::InvalidMigrationRecord)?;
            Ok(AppliedMigration { version, checksum })
        })
        .collect()
    }
}

fn configure_connection(connection: &Connection) -> Result<(), StartupError> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(startup_error)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(startup_error)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(startup_error)?;
    connection
        .pragma_update(None, "busy_timeout", 5_000_i64)
        .map_err(startup_error)
}

fn pragma_i64(connection: &Connection, name: &str) -> Result<i64, StartupError> {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(startup_error)
}

fn run_migrations(connection: &mut Connection, user_version: u32) -> Result<(), StartupError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(startup_error)?;
    transaction
        .execute_batch(SCHEMA_MIGRATIONS_SQL)
        .map_err(startup_error)?;

    let applied = read_applied_migrations(&transaction)?;
    verify_applied_migrations(&applied)?;

    for migration in ordered() {
        let checksum = migration.checksum();
        if applied.contains_key(&migration.version) {
            continue;
        }
        if migration.version <= user_version {
            return Err(StartupError::DatabaseMigrationState);
        }
        transaction.execute_batch(migration.sql).map_err(startup_error)?;
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, checksum) VALUES (?1, ?2)",
                (i64::from(migration.version), checksum.as_str()),
            )
            .map_err(startup_error)?;
    }

    if user_version != 0 && user_version != LATEST_SCHEMA_VERSION {
        return Err(StartupError::DatabaseMigrationState);
    }
    transaction
        .pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(startup_error)?;
    transaction
        .pragma_update(None, "user_version", i64::from(LATEST_SCHEMA_VERSION))
        .map_err(startup_error)?;
    transaction.commit().map_err(startup_error)
}

fn read_applied_migrations(
    transaction: &Transaction<'_>,
) -> Result<BTreeMap<u32, Sha256Digest>, StartupError> {
    let mut statement = transaction
        .prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")
        .map_err(startup_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(startup_error)?;
    let mut applied = BTreeMap::new();
    for row in rows {
        let (version, checksum) = row.map_err(startup_error)?;
        let version = u32::try_from(version).map_err(|_| StartupError::DatabaseMigrationState)?;
        let checksum = Sha256Digest::parse(&checksum)
            .map_err(|_| StartupError::DatabaseMigrationChecksumMismatch)?;
        applied.insert(version, checksum);
    }
    Ok(applied)
}

fn verify_applied_migrations(
    applied: &BTreeMap<u32, Sha256Digest>,
) -> Result<(), StartupError> {
    let migrations = ordered();
    for (version, checksum) in applied {
        let Some(migration) = migrations.iter().find(|migration| migration.version == *version) else {
            return Err(StartupError::DatabaseMigrationChecksumMismatch);
        };
        if checksum != &migration.checksum() {
            return Err(StartupError::DatabaseMigrationChecksumMismatch);
        }
    }
    Ok(())
}

fn quick_check(connection: &Connection) -> Result<(), StartupError> {
    let result: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(startup_error)?;
    if result == "ok" {
        Ok(())
    } else {
        Err(StartupError::DatabaseCorrupt)
    }
}

fn startup_error(error: SqliteError) -> StartupError {
    match error {
        SqliteError::SqliteFailure(error, _)
            if matches!(error.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) =>
        {
            StartupError::DatabaseCorrupt
        }
        SqliteError::SqliteFailure(error, _) if error.code == ErrorCode::PermissionDenied => {
            StartupError::StatePermissions
        }
        _ => StartupError::DatabaseUnavailable,
    }
}

fn persistence_error(_: SqliteError) -> PersistenceError {
    PersistenceError::QueryFailed
}
