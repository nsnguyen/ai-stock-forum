//! SQLite persistence and ordered schema migrations.

mod database;
mod migrations;

pub use database::{Database, PersistenceError};
pub use migrations::{AppliedMigration, LATEST_SCHEMA_VERSION};

pub const MODULE_NAME: &str = "persistence";
