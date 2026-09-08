//! SQLite persistence and ordered schema migrations.

mod agent_profile_repository;
mod command_receipt_repository;
mod database;
mod event_repository;
mod memory_repository;
mod migrations;
mod projection_repository;
mod skill_repository;

pub use agent_profile_repository::{
    StoredAgentProfileVersion, insert_expected_version, load_all_versions,
    load_exact_profile_version, replace_active_profiles,
};
pub use command_receipt_repository::{CommandReceiptRecord, CommandReceiptRepository};
pub use database::{Database, ImmediateTransaction, PersistenceError};
pub use event_repository::{EventRepository, RecoveryError};
pub(crate) use memory_repository::reconcile_verified_memory;
pub use memory_repository::{
    EpisodicSummariesPage, EpisodicSummaryListRecord, MemoryEntriesPage, MemoryEntryHistoryPage,
    MemoryEntryListRecord, MemoryProposalListRecord, MemoryProposalsPage, MemoryRepository,
};
pub use migrations::{AppliedMigration, LATEST_SCHEMA_VERSION};
pub use projection_repository::ProjectionRepository;
pub use skill_repository::{
    insert_skill_version, load_active_skill, load_active_skill_by_name, load_all_skill_versions,
    load_skill_history, load_skill_version, load_skill_version_by_id, reconcile_skill_versions,
    replace_active_skills, set_active_skill, validate_skill_version_ref,
};

pub const MODULE_NAME: &str = "persistence";
