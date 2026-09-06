use std::{collections::BTreeMap, path::PathBuf};

use rusqlite::{
    Connection, Error as SqliteError, ErrorCode, OpenFlags, Transaction, TransactionBehavior,
};
use thiserror::Error;

use crate::{
    config::{AppPaths, ProcessGuard, StartupError},
    domain::Sha256Digest,
};

use super::migrations::{
    APPLICATION_ID, AppliedMigration, LATEST_SCHEMA_VERSION, MIGRATION_BOUNDARY_PREFIX, Migration,
    SCHEMA_MIGRATIONS_SQL, migration_boundary_names, ordered,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PersistenceError {
    #[error("database query failed")]
    QueryFailed,
    #[error("database migration record is invalid")]
    InvalidMigrationRecord,
    #[error("event record is invalid")]
    InvalidEventRecord,
    #[error("event schema version is unsupported")]
    UnsupportedEventSchema,
    #[error("event id conflicts with an existing event")]
    IdempotencyConflict,
    #[error("database write is contended")]
    Contention,
    #[error("event stream is immutable")]
    ImmutableEventStream,
    #[error("event predecessor digest is invalid")]
    PreviousEventDigestMismatch,
    #[error("event digest is invalid")]
    EventDigestMismatch,
    #[error("projection state conflicts with the authoritative event stream")]
    ProjectionStateConflict,
    #[error("database agent profile history does not match the authoritative event stream")]
    AgentProfileHistoryMismatch,
    #[error("an active agent profile already uses that normalized name")]
    DuplicateAgentProfileName,
    #[error("agent profile memory namespace conflicts with stable profile identity")]
    AgentProfileNamespaceConflict,
    #[error("agent profile payload is invalid")]
    InvalidAgentProfilePayload,
    #[error("active agent profile projection rebuild failed")]
    ActiveAgentProfileRebuildFailed,
    #[error("database skill version integrity check failed")]
    SkillVersionIntegrityMismatch,
    #[error("agent profile skill reference does not match an immutable skill version")]
    SkillVersionReferenceMismatch,
    #[error("an active skill already uses that normalized name")]
    DuplicateSkillName,
    #[error("active skill projection rebuild failed")]
    ActiveSkillRebuildFailed,
}

impl PersistenceError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::QueryFailed => "database_write_failed",
            Self::InvalidMigrationRecord => "invalid_migration_record",
            Self::InvalidEventRecord => "invalid_event_record",
            Self::UnsupportedEventSchema => "unsupported_event_schema",
            Self::IdempotencyConflict => "event_id_conflict",
            Self::Contention => "database_write_contended",
            Self::ImmutableEventStream => "event_stream_immutable",
            Self::PreviousEventDigestMismatch => "previous_event_digest_mismatch",
            Self::EventDigestMismatch => "event_digest_mismatch",
            Self::ProjectionStateConflict => "projection_state_conflict",
            Self::AgentProfileHistoryMismatch => "database_agent_profile_history_mismatch",
            Self::DuplicateAgentProfileName => "active_name_conflict",
            Self::AgentProfileNamespaceConflict => "agent_profile_namespace_conflict",
            Self::InvalidAgentProfilePayload => "invalid_agent_profile_payload",
            Self::ActiveAgentProfileRebuildFailed => "active_agent_profile_rebuild_failed",
            Self::SkillVersionIntegrityMismatch => "skill_version_integrity_mismatch",
            Self::SkillVersionReferenceMismatch => "skill_version_reference_mismatch",
            Self::DuplicateSkillName => "active_skill_name_conflict",
            Self::ActiveSkillRebuildFailed => "active_skill_rebuild_failed",
        }
    }
}

pub struct Database {
    connection: Connection,
    schema_version: u32,
    state_dir: PathBuf,
}

pub struct ImmediateTransaction<'connection> {
    transaction: Transaction<'connection>,
}

impl<'connection> ImmediateTransaction<'connection> {
    pub fn commit(self) -> Result<(), PersistenceError> {
        self.transaction.commit().map_err(persistence_error)
    }

    pub fn rollback(self) -> Result<(), PersistenceError> {
        self.transaction.rollback().map_err(persistence_error)
    }

    pub(crate) fn transaction(&self) -> &Transaction<'connection> {
        &self.transaction
    }
}

impl Database {
    pub fn open(paths: &AppPaths) -> Result<Self, StartupError> {
        Self::open_with_before_sqlite_open(paths, || {})
    }

    fn open_with_before_sqlite_open<F>(
        paths: &AppPaths,
        before_sqlite_open: F,
    ) -> Result<Self, StartupError>
    where
        F: FnOnce(),
    {
        let mut migration_boundary_hook = |_: u32, _: &str| Ok(());
        Self::open_with_hooks(paths, before_sqlite_open, &mut migration_boundary_hook)
    }

    fn open_with_hooks<F>(
        paths: &AppPaths,
        before_sqlite_open: F,
        migration_boundary_hook: &mut dyn FnMut(u32, &str) -> Result<(), StartupError>,
    ) -> Result<Self, StartupError>
    where
        F: FnOnce(),
    {
        paths.ensure()?;
        let database_path = paths.sqlite_open_path();

        before_sqlite_open();
        let mut connection = Connection::open_with_flags(database_path, database_open_flags())
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
        run_migrations_with_hook(
            &mut connection,
            user_version as u32,
            &ordered(),
            migration_boundary_hook,
        )?;
        quick_check(&connection)?;

        Ok(Self {
            connection,
            schema_version: LATEST_SCHEMA_VERSION,
            state_dir: paths.state_dir().to_path_buf(),
        })
    }

    #[doc(hidden)]
    pub fn v2_migration_boundaries() -> Vec<&'static str> {
        let migrations = ordered();
        let migration = migrations
            .iter()
            .find(|migration| migration.version == 2)
            .expect("schema v2 migration is registered");
        let mut boundaries = migration_boundary_names(migration.sql);
        boundaries.push("schema_migration_record");
        boundaries
    }

    #[doc(hidden)]
    pub fn v3_migration_boundaries() -> Vec<&'static str> {
        let migrations = ordered();
        let migration = migrations
            .iter()
            .find(|migration| migration.version == 3)
            .expect("schema v3 migration is registered");
        let mut boundaries = migration_boundary_names(migration.sql);
        boundaries.push("schema_migration_record");
        boundaries
    }

    #[doc(hidden)]
    pub fn open_with_migration_fault(
        paths: &AppPaths,
        migration_version: u32,
        fail_after_boundary: &str,
    ) -> Result<Self, StartupError> {
        let mut migration_boundary_hook = |version: u32, boundary: &str| {
            if version == migration_version && boundary == fail_after_boundary {
                Err(StartupError::DatabaseUnavailable)
            } else {
                Ok(())
            }
        };
        Self::open_with_hooks(paths, || {}, &mut migration_boundary_hook)
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

    pub fn immediate_transaction(&mut self) -> Result<ImmediateTransaction<'_>, PersistenceError> {
        self.connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map(|transaction| ImmediateTransaction { transaction })
            .map_err(persistence_error)
    }

    pub fn acquire_process_guard(&self) -> Result<ProcessGuard, StartupError> {
        ProcessGuard::acquire(&self.state_dir)
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
            let version =
                u32::try_from(version).map_err(|_| PersistenceError::InvalidMigrationRecord)?;
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
        .map_err(startup_error)?;
    verify_connection_pragmas(connection)
}

fn verify_connection_pragmas(connection: &Connection) -> Result<(), StartupError> {
    let foreign_keys = pragma_i64(connection, "foreign_keys")?;
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(startup_error)?;
    let synchronous = pragma_i64(connection, "synchronous")?;
    let busy_timeout = pragma_i64(connection, "busy_timeout")?;

    if foreign_keys == 1
        && journal_mode.eq_ignore_ascii_case("wal")
        && synchronous == 2
        && busy_timeout == 5_000
    {
        Ok(())
    } else {
        Err(StartupError::DatabasePragmaMismatch)
    }
}

fn pragma_i64(connection: &Connection, name: &str) -> Result<i64, StartupError> {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(startup_error)
}

#[cfg(test)]
fn run_migrations_with(
    connection: &mut Connection,
    user_version: u32,
    migrations: &[Migration],
) -> Result<(), StartupError> {
    let mut migration_boundary_hook = |_: u32, _: &str| Ok(());
    run_migrations_with_hook(
        connection,
        user_version,
        migrations,
        &mut migration_boundary_hook,
    )
}

fn run_migrations_with_hook(
    connection: &mut Connection,
    user_version: u32,
    migrations: &[Migration],
    migration_boundary_hook: &mut dyn FnMut(u32, &str) -> Result<(), StartupError>,
) -> Result<(), StartupError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(startup_error)?;
    transaction
        .execute_batch(SCHEMA_MIGRATIONS_SQL)
        .map_err(startup_error)?;

    let applied = read_applied_migrations(&transaction)?;
    verify_applied_migrations(&applied, user_version, migrations)?;

    for migration in migrations {
        let checksum = migration.checksum();
        if applied.contains_key(&migration.version) {
            continue;
        }
        if migration.version <= user_version {
            return Err(StartupError::DatabaseMigrationState);
        }
        execute_migration_sql(&transaction, migration, migration_boundary_hook)?;
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, checksum) VALUES (?1, ?2)",
                (i64::from(migration.version), checksum.as_str()),
            )
            .map_err(startup_error)?;
        migration_boundary_hook(migration.version, "schema_migration_record")?;
    }

    let target_version = migrations
        .last()
        .map(|migration| migration.version)
        .ok_or(StartupError::DatabaseMigrationState)?;
    if user_version > target_version {
        return Err(StartupError::DatabaseMigrationState);
    }
    transaction
        .pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(startup_error)?;
    transaction
        .pragma_update(None, "user_version", i64::from(target_version))
        .map_err(startup_error)?;
    transaction.commit().map_err(startup_error)
}

fn execute_migration_sql(
    transaction: &Transaction<'_>,
    migration: &Migration,
    migration_boundary_hook: &mut dyn FnMut(u32, &str) -> Result<(), StartupError>,
) -> Result<(), StartupError> {
    let mut batch = String::new();
    for line in migration.sql.lines() {
        if let Some(boundary) = line.trim().strip_prefix(MIGRATION_BOUNDARY_PREFIX) {
            if batch.trim().is_empty() || boundary.trim().is_empty() {
                return Err(StartupError::DatabaseMigrationState);
            }
            transaction.execute_batch(&batch).map_err(startup_error)?;
            migration_boundary_hook(migration.version, boundary.trim())?;
            batch.clear();
        } else {
            batch.push_str(line);
            batch.push('\n');
        }
    }
    if !batch.trim().is_empty() {
        transaction.execute_batch(&batch).map_err(startup_error)?;
    }
    Ok(())
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
        let checksum =
            Sha256Digest::parse(&checksum).map_err(|_| StartupError::DatabaseMigrationState)?;
        applied.insert(version, checksum);
    }
    Ok(applied)
}

fn verify_applied_migrations(
    applied: &BTreeMap<u32, Sha256Digest>,
    user_version: u32,
    migrations: &[Migration],
) -> Result<(), StartupError> {
    if applied.len() != user_version as usize {
        return Err(StartupError::DatabaseMigrationState);
    }
    for (offset, migration) in migrations.iter().enumerate() {
        if migration.version != (offset as u32) + 1 {
            return Err(StartupError::DatabaseMigrationState);
        }
    }
    for version in 1..=user_version {
        let Some(migration) = migrations
            .iter()
            .find(|migration| migration.version == version)
        else {
            return Err(StartupError::DatabaseMigrationState);
        };
        let Some(checksum) = applied.get(&version) else {
            return Err(StartupError::DatabaseMigrationState);
        };
        if checksum != &migration.checksum() {
            return Err(StartupError::DatabaseMigrationState);
        }
    }
    Ok(())
}

fn database_open_flags() -> OpenFlags {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_EXRESCODE;
    #[cfg(unix)]
    let flags = flags | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    flags
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
            if matches!(
                error.code,
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase
            ) =>
        {
            StartupError::DatabaseCorrupt
        }
        SqliteError::SqliteFailure(error, _) if error.code == ErrorCode::PermissionDenied => {
            StartupError::StatePermissions
        }
        SqliteError::SqliteFailure(error, _)
            if error.extended_code == rusqlite::ffi::SQLITE_CANTOPEN_SYMLINK =>
        {
            StartupError::DatabaseTerminalPathRejected
        }
        _ => StartupError::DatabaseUnavailable,
    }
}

fn persistence_error(error: SqliteError) -> PersistenceError {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn rejects_connections_with_ineffective_required_pragmas() {
        let connection = Connection::open_in_memory().unwrap();

        let error = verify_connection_pragmas(&connection).unwrap_err();

        assert_eq!(error.code(), "database_pragma_mismatch");
    }

    #[cfg(unix)]
    #[test]
    fn terminal_replacement_is_rejected_at_the_sqlite_open_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let target = temp.path().join("target.sqlite3");
        let paths = AppPaths::for_test(&state);

        fs::write(&target, b"not a sqlite database").unwrap();

        let result = Database::open_with_before_sqlite_open(&paths, || {
            fs::remove_file(paths.database_path()).unwrap();
            std::os::unix::fs::symlink(&target, paths.database_path()).unwrap();
        });

        assert!(matches!(result, Err(error) if error.code() == "database_terminal_path_rejected"));
    }

    #[test]
    fn failed_migration_rolls_back_every_schema_change() {
        let mut connection = Connection::open_in_memory().unwrap();
        let migrations = [Migration {
            version: 1,
            sql: "CREATE TABLE rollback_marker (id INTEGER PRIMARY KEY) STRICT; invalid sql;",
        }];

        assert!(run_migrations_with(&mut connection, 0, &migrations).is_err());
        assert!(!connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'rollback_marker')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
        assert_eq!(pragma_i64(&connection, "application_id").unwrap(), 0);
        assert_eq!(pragma_i64(&connection, "user_version").unwrap(), 0);
    }
}
