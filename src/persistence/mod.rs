//! SQLite persistence and ordered schema migrations.

mod command_receipt_repository;
mod database;
mod event_repository;
mod migrations;
mod projection_repository;

pub use command_receipt_repository::{CommandReceiptRecord, CommandReceiptRepository};
pub use database::{Database, ImmediateTransaction, PersistenceError};
pub use event_repository::{EventRepository, RecoveryError};
pub use migrations::{AppliedMigration, LATEST_SCHEMA_VERSION};
pub use projection_repository::ProjectionRepository;

pub const MODULE_NAME: &str = "persistence";
