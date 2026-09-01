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

#[test]
fn migration_checksum_mismatch_is_rejected_as_invalid_migration_state() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());

    drop(Database::open(&paths).unwrap());
    let raw = rusqlite::Connection::open(paths.database_path()).unwrap();
    raw.execute(
        "UPDATE schema_migrations SET checksum = ?1 WHERE version = 1",
        ["0".repeat(64)],
    )
    .unwrap();
    drop(raw);

    assert!(matches!(Database::open(&paths), Err(error) if error.code() == "database_migration_state_invalid"));
}

#[test]
fn migration_records_ahead_of_user_version_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());

    drop(Database::open(&paths).unwrap());
    let raw = rusqlite::Connection::open(paths.database_path()).unwrap();
    raw.execute(
        "INSERT INTO schema_migrations (version, checksum) VALUES (?1, ?2)",
        (2_i64, "0".repeat(64)),
    )
    .unwrap();
    drop(raw);

    assert!(matches!(Database::open(&paths), Err(error) if error.code() == "database_migration_state_invalid"));
}

#[test]
fn migration_record_holes_and_version_count_mismatches_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());

    drop(Database::open(&paths).unwrap());
    let raw = rusqlite::Connection::open(paths.database_path()).unwrap();
    raw.execute("DELETE FROM schema_migrations WHERE version = 1", [])
        .unwrap();
    drop(raw);

    assert!(matches!(Database::open(&paths), Err(error) if error.code() == "database_migration_state_invalid"));

    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    drop(Database::open(&paths).unwrap());
    let raw = rusqlite::Connection::open(paths.database_path()).unwrap();
    raw.pragma_update(None, "user_version", 0).unwrap();
    drop(raw);

    assert!(matches!(Database::open(&paths), Err(error) if error.code() == "database_migration_state_invalid"));
}

#[test]
fn migration_records_and_complete_schema_are_exact() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    let database = Database::open(&paths).unwrap();
    let connection = database.connection();

    let migration = database.applied_migrations().unwrap();
    assert_eq!(migration.len(), 1);
    assert_eq!(migration[0].version(), 1);
    assert_eq!(migration[0].checksum().as_str(), ai_stock_forum::domain::sha256(include_str!("../migrations/0001_phase0.sql").as_bytes()).as_str());

    for (table, expected_columns) in [
        ("event_stream", vec!["sequence", "event_id", "event_schema_version", "event_type", "actor_kind", "actor_id", "occurred_at_ms", "correlation_id", "causation_id", "object_kind", "object_id", "object_version", "object_digest", "previous_event_digest", "payload_json", "event_digest"]),
        ("installation_projection", vec!["singleton", "installation_id", "created_event_id", "created_at_ms"]),
        ("process_session_projection", vec!["session_id", "started_event_id", "started_at_ms", "ended_event_id", "ended_at_ms", "end_reason"]),
        ("projection_metadata", vec!["singleton", "last_event_sequence", "last_event_digest", "projection_digest"]),
        ("setup_drafts", vec!["draft_id", "schema_version", "state", "path", "current_review_digest", "payload_json", "created_at_ms", "updated_at_ms"]),
        ("installation_configuration_versions", vec!["configuration_id", "version", "source_draft_id", "review_digest", "object_digest", "payload_json", "created_event_id", "created_at_ms"]),
        ("active_installation_configuration", vec!["singleton", "configuration_id", "activated_event_id", "activated_at_ms"]),
        ("setup_step_outcomes", vec!["draft_id", "step_key", "attempt", "status", "safe_code", "occurred_at_ms"]),
        ("capability_readiness", vec!["configuration_id", "capability", "status", "reason_code", "checked_at_ms", "projection_digest"]),
        ("approval_records", vec!["approval_id", "action_kind", "object_kind", "object_id", "object_version", "object_digest", "actor_kind", "actor_id", "status", "created_at_ms", "expires_at_ms", "resolved_at_ms", "resolution_kind", "resolution_event_id"]),
    ] {
        let mut statement = connection.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let actual = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected_columns, "columns for {table}");
    }

    for index in [
        "event_stream_correlation_idx",
        "event_stream_type_idx",
        "setup_drafts_state_idx",
        "approval_records_status_idx",
    ] {
        assert!(connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
                [index],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
    }

    for trigger in [
        "event_stream_no_update",
        "event_stream_no_delete",
        "installation_configuration_versions_no_update",
        "installation_configuration_versions_no_delete",
    ] {
        assert!(connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = ?1)",
                [trigger],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
    }

    for (table, child_column, parent_table, parent_column) in [
        ("installation_projection", "created_event_id", "event_stream", "event_id"),
        ("process_session_projection", "started_event_id", "event_stream", "event_id"),
        ("process_session_projection", "ended_event_id", "event_stream", "event_id"),
        ("installation_configuration_versions", "source_draft_id", "setup_drafts", "draft_id"),
        ("installation_configuration_versions", "created_event_id", "event_stream", "event_id"),
        ("active_installation_configuration", "configuration_id", "installation_configuration_versions", "configuration_id"),
        ("active_installation_configuration", "activated_event_id", "event_stream", "event_id"),
        ("setup_step_outcomes", "draft_id", "setup_drafts", "draft_id"),
        ("capability_readiness", "configuration_id", "installation_configuration_versions", "configuration_id"),
        ("approval_records", "resolution_event_id", "event_stream", "event_id"),
    ] {
        let mut statement = connection.prepare(&format!("PRAGMA foreign_key_list({table})")).unwrap();
        let found = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .any(|(actual_parent, actual_child, actual_parent_column)| {
                actual_parent == parent_table
                    && actual_child == child_column
                    && actual_parent_column == parent_column
            });
        assert!(found, "foreign key {table}.{child_column}");
    }

    let event_sql: String = connection
        .query_row("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'event_stream'", [], |row| row.get(0))
        .unwrap();
    assert!(event_sql.contains("CHECK (json_valid(payload_json))"));
    assert!(event_sql.contains("CHECK (object_version IS NULL OR object_version > 0)"));
    assert!(event_sql.ends_with(" STRICT"));
}

#[test]
fn opened_connection_has_the_required_effective_pragmas() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection();

    assert_eq!(connection.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(connection.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0)).unwrap(), "wal");
    assert_eq!(connection.query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(connection.query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0)).unwrap(), 5_000);
}

#[test]
fn schema_constraints_foreign_keys_and_immutable_triggers_are_enforced() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection();

    assert!(connection
        .execute(
            "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (1, 'event-1', 1, 'created', 'system', 1, 'correlation-1', '{}', 'digest-1')",
            [],
        )
        .is_ok());
    assert!(connection
        .execute("UPDATE event_stream SET event_type = 'changed' WHERE sequence = 1", [])
        .is_err());
    assert!(connection
        .execute("DELETE FROM event_stream WHERE sequence = 1", [])
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (2, 'event-2', 0, 'created', 'system', 2, 'correlation-2', 'not-json', 'digest-2')",
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (1, 'installation-1', 'missing-event', 1)",
            [],
        )
        .is_err());

    connection
        .execute(
            "INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms) VALUES ('draft-1', 1, 'drafting', 'quick_start', '{}', 1, 1)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('configuration-1', 1, 'draft-1', 'review-1', 'object-1', '{}', 'event-1', 1)",
            [],
        )
        .unwrap();
    assert!(connection
        .execute(
            "UPDATE installation_configuration_versions SET review_digest = 'changed' WHERE configuration_id = 'configuration-1'",
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms, resolved_at_ms, resolution_kind) VALUES ('approval-1', 'apply', 'configuration', 'configuration-1', 1, 'object-1', 'system', 'pending', 1, 2, 'accepted')",
            [],
        )
        .is_err());
}

#[cfg(unix)]
#[test]
fn database_open_corrects_the_database_to_owner_only_mode() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    drop(Database::open(&paths).unwrap());
    fs::set_permissions(paths.database_path(), fs::Permissions::from_mode(0o644)).unwrap();

    drop(Database::open(&paths).unwrap());

    assert_eq!(fs::metadata(paths.database_path()).unwrap().permissions().mode() & 0o777, 0o600);
}
