//! SQLite persistence and ordered schema migrations.

mod database;
mod event_repository;
mod migrations;

pub use database::{Database, ImmediateTransaction, PersistenceError};
pub use event_repository::{EventRepository, RecoveryError};
pub use migrations::{AppliedMigration, LATEST_SCHEMA_VERSION};

pub const MODULE_NAME: &str = "persistence";
