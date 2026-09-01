//! Application state path discovery and permission enforcement.

mod paths;

pub use paths::AppPaths;

use thiserror::Error;

use crate::persistence::RecoveryError;

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
    #[error("event stream recovery failed: {0}")]
    EventStreamRecovery(RecoveryError),
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
            Self::EventStreamRecovery(error) => error.code(),
        }
    }
}
