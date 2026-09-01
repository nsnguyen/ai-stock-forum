use std::fs;

use ai_stock_forum::config::AppPaths;
use ai_stock_forum::persistence::{Database, LATEST_SCHEMA_VERSION};

#[test]
fn fresh_database_has_the_complete_phase_zero_schema() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    let database = Database::open(&paths).unwrap();

    assert_eq!(database.schema_version(), LATEST_SCHEMA_VERSION);
    for table in [
        "event_stream",
        "installation_projection",
        "process_session_projection",
        "projection_metadata",
        "setup_drafts",
        "installation_configuration_versions",
        "active_installation_configuration",
        "setup_step_outcomes",
        "capability_readiness",
        "approval_records",
    ] {
        assert!(database.has_table(table).unwrap(), "missing {table}");
    }
}

#[test]
fn reopening_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());

    drop(Database::open(&paths).unwrap());

    assert_eq!(
        Database::open(&paths).unwrap().applied_migrations().unwrap().len(),
        1
    );
}

#[test]
fn newer_schema_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());

    drop(Database::open(&paths).unwrap());
    let raw = rusqlite::Connection::open(paths.database_path()).unwrap();
    raw.pragma_update(None, "user_version", 99).unwrap();
    drop(raw);

    assert!(matches!(Database::open(&paths), Err(error) if error.code() == "database_schema_newer"));
}

#[test]
fn corrupt_database_is_rejected_without_recreation() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    let bytes = b"not a sqlite database";
    fs::write(paths.database_path(), bytes).unwrap();

    assert!(matches!(Database::open(&paths), Err(error) if error.code() == "database_corrupt"));
    assert_eq!(fs::read(paths.database_path()).unwrap(), bytes);
}

#[test]
fn foreign_application_database_is_rejected_without_recreation() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    let raw = rusqlite::Connection::open(paths.database_path()).unwrap();
    raw.pragma_update(None, "application_id", 0x1234_5678_i64)
        .unwrap();
    drop(raw);
    let before = fs::read(paths.database_path()).unwrap();

    assert!(matches!(Database::open(&paths), Err(error) if error.code() == "database_application_mismatch"));
    assert_eq!(fs::read(paths.database_path()).unwrap(), before);
}
