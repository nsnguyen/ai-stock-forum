//! Application state path discovery and permission enforcement.

mod paths;
mod process_guard;

pub use paths::AppPaths;
pub use process_guard::ProcessGuard;

use thiserror::Error;

use crate::persistence::{PersistenceError, RecoveryError};

pub const MODULE_NAME: &str = "config";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StartupError {
    #[error("application state directory is unavailable")]
    StateDirectoryUnavailable,
    #[error("application state permissions could not be established")]
    StatePermissions,
    #[error("database could not be opened")]
    DatabaseUnavailable,
    #[error("database contents are corrupt")]
    DatabaseCorrupt,
    #[error("database belongs to a different application")]
    DatabaseApplicationMismatch,
    #[error("database schema is newer than this application supports")]
    DatabaseSchemaNewer,
    #[error("database migration checksum does not match this application")]
    DatabaseMigrationChecksumMismatch,
    #[error("database migration state is invalid")]
    DatabaseMigrationState,
    #[error("database did not apply required connection settings")]
    DatabasePragmaMismatch,
    #[error("database terminal path was rejected")]
    DatabaseTerminalPathRejected,
    #[error("another application process is already running")]
    AlreadyRunning,
    #[error("event stream recovery failed: {0}")]
    EventStreamRecovery(RecoveryError),
    #[error("bootstrap persistence failed: {0}")]
    Persistence(PersistenceError),
}

impl StartupError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::StateDirectoryUnavailable => "state_directory_unavailable",
            Self::StatePermissions => "state_permissions",
            Self::DatabaseUnavailable => "database_unavailable",
            Self::DatabaseCorrupt => "database_corrupt",
            Self::DatabaseApplicationMismatch => "database_application_mismatch",
            Self::DatabaseSchemaNewer => "database_schema_newer",
            Self::DatabaseMigrationChecksumMismatch => "database_migration_checksum_mismatch",
            Self::DatabaseMigrationState => "database_migration_state_invalid",
            Self::DatabasePragmaMismatch => "database_pragma_mismatch",
            Self::DatabaseTerminalPathRejected => "database_terminal_path_rejected",
            Self::AlreadyRunning => "already_running",
            Self::EventStreamRecovery(error) => error.code(),
            Self::Persistence(error) => match error {
                PersistenceError::QueryFailed => "database_write_failed",
                PersistenceError::InvalidMigrationRecord => "invalid_migration_record",
                PersistenceError::InvalidEventRecord => "invalid_event_record",
                PersistenceError::UnsupportedEventSchema => "unsupported_event_schema",
                PersistenceError::IdempotencyConflict => "event_id_conflict",
                PersistenceError::Contention => "database_write_contended",
                PersistenceError::ImmutableEventStream => "event_stream_immutable",
                PersistenceError::PreviousEventDigestMismatch => "previous_event_digest_mismatch",
                PersistenceError::EventDigestMismatch => "event_digest_mismatch",
                PersistenceError::ProjectionStateConflict => "projection_state_conflict",
            },
        }
    }
}
