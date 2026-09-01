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

#[test]
fn task_six_schema_contract_is_exact_and_every_constraint_is_enforced() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();

    assert_complete_task_six_schema_contract(database.connection());
    assert_every_task_six_constraint_is_enforced();
}

#[derive(Clone, Copy)]
struct ExpectedColumn {
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    default: Option<&'static str>,
    primary_key_position: i64,
}

const fn column(
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    primary_key_position: i64,
) -> ExpectedColumn {
    ExpectedColumn {
        name,
        declared_type,
        not_null,
        default: None,
        primary_key_position,
    }
}

fn assert_complete_task_six_schema_contract(connection: &rusqlite::Connection) {
    let mut statement = connection
        .prepare(
            "SELECT type, name FROM sqlite_schema WHERE type IN ('table', 'index', 'trigger') AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .unwrap();
    let objects = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(
        objects,
        vec![
            ("index".to_owned(), "approval_records_status_idx".to_owned()),
            ("index".to_owned(), "event_stream_correlation_idx".to_owned()),
            ("index".to_owned(), "event_stream_type_idx".to_owned()),
            ("index".to_owned(), "setup_drafts_state_idx".to_owned()),
            ("table".to_owned(), "active_installation_configuration".to_owned()),
            ("table".to_owned(), "approval_records".to_owned()),
            ("table".to_owned(), "capability_readiness".to_owned()),
            ("table".to_owned(), "event_stream".to_owned()),
            ("table".to_owned(), "installation_configuration_versions".to_owned()),
            ("table".to_owned(), "installation_projection".to_owned()),
            ("table".to_owned(), "process_session_projection".to_owned()),
            ("table".to_owned(), "projection_metadata".to_owned()),
            ("table".to_owned(), "schema_migrations".to_owned()),
            ("table".to_owned(), "setup_drafts".to_owned()),
            ("table".to_owned(), "setup_step_outcomes".to_owned()),
            ("trigger".to_owned(), "event_stream_no_delete".to_owned()),
            ("trigger".to_owned(), "event_stream_no_update".to_owned()),
            ("trigger".to_owned(), "installation_configuration_versions_no_delete".to_owned()),
            ("trigger".to_owned(), "installation_configuration_versions_no_update".to_owned()),
        ]
    );

    let expected_columns: &[(&str, &[ExpectedColumn])] = &[
        ("schema_migrations", &[column("version", "INTEGER", false, 1), column("checksum", "TEXT", true, 0)]),
        ("event_stream", &[
            column("sequence", "INTEGER", false, 1), column("event_id", "TEXT", true, 0),
            column("event_schema_version", "INTEGER", true, 0), column("event_type", "TEXT", true, 0),
            column("actor_kind", "TEXT", true, 0), column("actor_id", "TEXT", false, 0),
            column("occurred_at_ms", "INTEGER", true, 0), column("correlation_id", "TEXT", true, 0),
            column("causation_id", "TEXT", false, 0), column("object_kind", "TEXT", false, 0),
            column("object_id", "TEXT", false, 0), column("object_version", "INTEGER", false, 0),
            column("object_digest", "TEXT", false, 0), column("previous_event_digest", "TEXT", false, 0),
            column("payload_json", "TEXT", true, 0), column("event_digest", "TEXT", true, 0),
        ]),
        ("installation_projection", &[column("singleton", "INTEGER", false, 1), column("installation_id", "TEXT", true, 0), column("created_event_id", "TEXT", true, 0), column("created_at_ms", "INTEGER", true, 0)]),
        ("process_session_projection", &[column("session_id", "TEXT", true, 1), column("started_event_id", "TEXT", true, 0), column("started_at_ms", "INTEGER", true, 0), column("ended_event_id", "TEXT", false, 0), column("ended_at_ms", "INTEGER", false, 0), column("end_reason", "TEXT", false, 0)]),
        ("projection_metadata", &[column("singleton", "INTEGER", false, 1), column("last_event_sequence", "INTEGER", true, 0), column("last_event_digest", "TEXT", false, 0), column("projection_digest", "TEXT", true, 0)]),
        ("setup_drafts", &[column("draft_id", "TEXT", true, 1), column("schema_version", "INTEGER", true, 0), column("state", "TEXT", true, 0), column("path", "TEXT", true, 0), column("current_review_digest", "TEXT", false, 0), column("payload_json", "TEXT", true, 0), column("created_at_ms", "INTEGER", true, 0), column("updated_at_ms", "INTEGER", true, 0)]),
        ("installation_configuration_versions", &[column("configuration_id", "TEXT", true, 1), column("version", "INTEGER", true, 0), column("source_draft_id", "TEXT", true, 0), column("review_digest", "TEXT", true, 0), column("object_digest", "TEXT", true, 0), column("payload_json", "TEXT", true, 0), column("created_event_id", "TEXT", true, 0), column("created_at_ms", "INTEGER", true, 0)]),
        ("active_installation_configuration", &[column("singleton", "INTEGER", false, 1), column("configuration_id", "TEXT", true, 0), column("activated_event_id", "TEXT", true, 0), column("activated_at_ms", "INTEGER", true, 0)]),
        ("setup_step_outcomes", &[column("draft_id", "TEXT", true, 1), column("step_key", "TEXT", true, 2), column("attempt", "INTEGER", true, 3), column("status", "TEXT", true, 0), column("safe_code", "TEXT", false, 0), column("occurred_at_ms", "INTEGER", true, 0)]),
        ("capability_readiness", &[column("configuration_id", "TEXT", true, 1), column("capability", "TEXT", true, 2), column("status", "TEXT", true, 0), column("reason_code", "TEXT", false, 0), column("checked_at_ms", "INTEGER", true, 0), column("projection_digest", "TEXT", true, 0)]),
        ("approval_records", &[column("approval_id", "TEXT", true, 1), column("action_kind", "TEXT", true, 0), column("object_kind", "TEXT", true, 0), column("object_id", "TEXT", true, 0), column("object_version", "INTEGER", true, 0), column("object_digest", "TEXT", true, 0), column("actor_kind", "TEXT", true, 0), column("actor_id", "TEXT", false, 0), column("status", "TEXT", true, 0), column("created_at_ms", "INTEGER", true, 0), column("expires_at_ms", "INTEGER", false, 0), column("resolved_at_ms", "INTEGER", false, 0), column("resolution_kind", "TEXT", false, 0), column("resolution_event_id", "TEXT", false, 0)]),
    ];
    for (table, expected) in expected_columns {
        let mut statement = connection.prepare(&format!("PRAGMA table_xinfo({table})")).unwrap();
        let actual = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)? != 0,
                    row.get::<_, Option<String>>(4)?, row.get::<_, i64>(5)?, row.get::<_, i64>(6)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let expected = expected.iter().map(|column| (
            column.name.to_owned(), column.declared_type.to_owned(), column.not_null,
            column.default.map(str::to_owned), column.primary_key_position, 0_i64,
        )).collect::<Vec<_>>();
        assert_eq!(actual, expected, "column contract for {table}");
        let sql: String = connection.query_row("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1", [*table], |row| row.get(0)).unwrap();
        assert!(normalize_sql(&sql).ends_with(" strict"), "{table} must remain STRICT");
    }

    let mut foreign_keys = Vec::new();
    for table in expected_columns.iter().map(|(table, _)| *table) {
        let mut statement = connection.prepare(&format!("PRAGMA foreign_key_list({table})")).unwrap();
        foreign_keys.extend(statement.query_map([], |row| Ok((
            table.to_owned(), row.get::<_, String>(3)?, row.get::<_, String>(2)?, row.get::<_, String>(4)?,
            row.get::<_, String>(5)?, row.get::<_, String>(6)?,
        ))).unwrap().map(Result::unwrap));
    }
    foreign_keys.sort();
    let mut expected_foreign_keys = vec![
        ("active_installation_configuration", "activated_event_id", "event_stream", "event_id"),
        ("active_installation_configuration", "configuration_id", "installation_configuration_versions", "configuration_id"),
        ("approval_records", "resolution_event_id", "event_stream", "event_id"),
        ("capability_readiness", "configuration_id", "installation_configuration_versions", "configuration_id"),
        ("installation_configuration_versions", "created_event_id", "event_stream", "event_id"),
        ("installation_configuration_versions", "source_draft_id", "setup_drafts", "draft_id"),
        ("installation_projection", "created_event_id", "event_stream", "event_id"),
        ("process_session_projection", "ended_event_id", "event_stream", "event_id"),
        ("process_session_projection", "started_event_id", "event_stream", "event_id"),
        ("setup_step_outcomes", "draft_id", "setup_drafts", "draft_id"),
    ].into_iter().map(|(table, from, parent, to)| (table.to_owned(), from.to_owned(), parent.to_owned(), to.to_owned(), "NO ACTION".to_owned(), "NO ACTION".to_owned())).collect::<Vec<_>>();
    expected_foreign_keys.sort();
    assert_eq!(foreign_keys, expected_foreign_keys);

    for (name, table, columns) in [
        ("event_stream_correlation_idx", "event_stream", &["correlation_id", "sequence"][..]),
        ("event_stream_type_idx", "event_stream", &["event_type", "sequence"][..]),
        ("setup_drafts_state_idx", "setup_drafts", &["state", "updated_at_ms"][..]),
        ("approval_records_status_idx", "approval_records", &["status", "created_at_ms"][..]),
    ] {
        let (unique, origin): (i64, String) = connection.query_row(&format!("SELECT \"unique\", origin FROM pragma_index_list('{table}') WHERE name = ?1"), [name], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!((unique, origin.as_str()), (0, "c"), "index metadata for {name}");
        let mut statement = connection.prepare(&format!("SELECT name FROM pragma_index_xinfo('{name}') WHERE key = 1 ORDER BY seqno")).unwrap();
        let actual = statement.query_map([], |row| row.get::<_, String>(0)).unwrap().map(Result::unwrap).collect::<Vec<_>>();
        assert_eq!(actual, columns, "ordered index columns for {name}");
        let sql: String = connection.query_row("SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1", [name], |row| row.get(0)).unwrap();
        assert_eq!(normalize_sql(&sql), format!("create index {name} on {table}({})", columns.join(", ")));
    }

    for (name, expected_sql) in [
        ("event_stream_no_update", "create trigger event_stream_no_update before update on event_stream begin select raise(abort, 'event_stream is append-only'); end"),
        ("event_stream_no_delete", "create trigger event_stream_no_delete before delete on event_stream begin select raise(abort, 'event_stream is append-only'); end"),
        ("installation_configuration_versions_no_update", "create trigger installation_configuration_versions_no_update before update on installation_configuration_versions begin select raise(abort, 'installation configuration versions are immutable'); end"),
        ("installation_configuration_versions_no_delete", "create trigger installation_configuration_versions_no_delete before delete on installation_configuration_versions begin select raise(abort, 'installation configuration versions are immutable'); end"),
    ] {
        let sql: String = connection.query_row("SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?1", [name], |row| row.get(0)).unwrap();
        assert_eq!(normalize_sql(&sql), expected_sql, "trigger definition for {name}");
    }
}

fn assert_every_task_six_constraint_is_enforced() {
    for statement in [
        "INSERT INTO schema_migrations (version, checksum) VALUES (0, 'x')",
        "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (2, 'event-2', 0, 'created', 'system', 2, 'corr-2', '{}', 'digest-2')",
        "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, object_version, payload_json, event_digest) VALUES (2, 'event-2', 1, 'created', 'system', 2, 'corr-2', 0, '{}', 'digest-2')",
        "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (2, 'event-2', 1, 'created', 'system', 2, 'corr-2', 'not-json', 'digest-2')",
        "INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (2, 'installation-2', 'event-1', 2)",
        "INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms, ended_at_ms, end_reason) VALUES ('session-bad-1', 'event-1', 1, 2, 'quit')",
        "INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms, ended_event_id) VALUES ('session-bad-2', 'event-1', 1, 'event-1')",
        "INSERT INTO projection_metadata (singleton, last_event_sequence, projection_digest) VALUES (2, 0, 'projection-2')",
        "INSERT INTO projection_metadata (singleton, last_event_sequence, projection_digest) VALUES (2, -1, 'projection-2')",
        "INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms) VALUES ('draft-bad-1', 0, 'drafting', 'quick_start', '{}', 1, 1)",
        "INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms) VALUES ('draft-bad-2', 1, 'unknown', 'quick_start', '{}', 1, 1)",
        "INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms) VALUES ('draft-bad-3', 1, 'drafting', 'unknown', '{}', 1, 1)",
        "INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms) VALUES ('draft-bad-4', 1, 'drafting', 'quick_start', 'not-json', 1, 1)",
        "INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('config-bad-1', 0, 'draft-1', 'review-bad-1', 'object-bad-1', '{}', 'event-1', 1)",
        "INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('config-bad-2', 2, 'draft-1', 'review-bad-2', 'object-bad-2', 'not-json', 'event-1', 1)",
        "INSERT INTO active_installation_configuration (singleton, configuration_id, activated_event_id, activated_at_ms) VALUES (2, 'configuration-1', 'event-1', 1)",
        "INSERT INTO setup_step_outcomes (draft_id, step_key, attempt, status, occurred_at_ms) VALUES ('draft-1', 'step-bad-1', 0, 'passed', 1)",
        "INSERT INTO setup_step_outcomes (draft_id, step_key, attempt, status, occurred_at_ms) VALUES ('draft-1', 'step-bad-2', 1, 'unknown', 1)",
        "INSERT INTO capability_readiness (configuration_id, capability, status, checked_at_ms, projection_digest) VALUES ('configuration-1', 'capability-bad', 'unknown', 1, 'projection-1')",
        "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms) VALUES ('approval-bad-1', 'apply', 'configuration', 'configuration-1', 0, 'object-1', 'system', 'pending', 1)",
        "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms) VALUES ('approval-bad-2', 'apply', 'configuration', 'configuration-1', 1, 'object-1', 'system', 'unknown', 1)",
        "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms, expires_at_ms) VALUES ('approval-bad-3', 'apply', 'configuration', 'configuration-1', 1, 'object-1', 'system', 'pending', 2, 2)",
        "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms, resolved_at_ms, resolution_kind) VALUES ('approval-bad-4', 'apply', 'configuration', 'configuration-1', 1, 'object-1', 'system', 'pending', 1, 2, 'accepted')",
        "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms) VALUES ('approval-bad-5', 'apply', 'configuration', 'configuration-1', 1, 'object-1', 'system', 'accepted', 1)",
    ] { assert_rejected_after(seed_complete, statement); }

    for statement in [
        "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (2, 'event-1', 1, 'created', 'system', 2, 'corr-2', '{}', 'digest-2')",
        "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (2, 'event-2', 1, 'created', 'system', 2, 'corr-2', '{}', 'digest-1')",
        "INSERT INTO schema_migrations (version, checksum) VALUES (1, 'other')",
        "INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms) VALUES ('session-1', 'event-1', 1)",
        "INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms) VALUES ('draft-1', 1, 'drafting', 'quick_start', '{}', 1, 1)",
        "INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('configuration-2', 2, 'draft-1', 'review-1', 'object-2', '{}', 'event-1', 1)",
        "INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('configuration-2', 1, 'draft-1', 'review-2', 'object-2', '{}', 'event-1', 1)",
        "INSERT INTO setup_step_outcomes (draft_id, step_key, attempt, status, occurred_at_ms) VALUES ('draft-1', 'step-1', 1, 'passed', 1)",
        "INSERT INTO capability_readiness (configuration_id, capability, status, checked_at_ms, projection_digest) VALUES ('configuration-1', 'capability-1', 'ready', 1, 'projection-1')",
        "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms) VALUES ('approval-1', 'apply', 'configuration', 'configuration-1', 1, 'object-1', 'system', 'pending', 1)",
    ] { assert_rejected_after(seed_complete, statement); }

    assert_rejected_after(seed_event, "INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (1, 'installation-missing', 'missing-event', 1)");
    assert_rejected_after(seed_event, "INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms) VALUES ('session-missing-start', 'missing-event', 1)");
    assert_rejected_after(seed_event, "INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms, ended_event_id, ended_at_ms, end_reason) VALUES ('session-missing-end', 'event-1', 1, 'missing-event', 2, 'quit')");
    assert_rejected_after(seed_event, "INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('config-missing-source', 1, 'missing-draft', 'review', 'object', '{}', 'event-1', 1)");
    assert_rejected_after(seed_draft, "INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('config-missing-event', 1, 'draft-1', 'review', 'object', '{}', 'missing-event', 1)");
    assert_rejected_after(seed_event, "INSERT INTO active_installation_configuration (singleton, configuration_id, activated_event_id, activated_at_ms) VALUES (1, 'missing-configuration', 'event-1', 1)");
    assert_rejected_after(seed_configuration, "INSERT INTO active_installation_configuration (singleton, configuration_id, activated_event_id, activated_at_ms) VALUES (1, 'configuration-1', 'missing-event', 1)");
    assert_rejected_after(seed_event, "INSERT INTO setup_step_outcomes (draft_id, step_key, attempt, status, occurred_at_ms) VALUES ('missing-draft', 'step', 1, 'passed', 1)");
    assert_rejected_after(seed_event, "INSERT INTO capability_readiness (configuration_id, capability, status, checked_at_ms, projection_digest) VALUES ('missing-configuration', 'capability', 'ready', 1, 'projection')");
    assert_rejected_after(seed_event, "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms, resolved_at_ms, resolution_kind, resolution_event_id) VALUES ('approval-missing-event', 'apply', 'configuration', 'configuration-1', 1, 'object-1', 'system', 'accepted', 1, 2, 'accepted', 'missing-event')");

    assert_rejected_after(seed_complete, "UPDATE event_stream SET event_type = 'changed' WHERE sequence = 1");
    assert_rejected_after(seed_complete, "DELETE FROM event_stream WHERE sequence = 1");
    assert_rejected_after(seed_complete, "UPDATE installation_configuration_versions SET review_digest = 'changed' WHERE configuration_id = 'configuration-1'");
    assert_rejected_after(seed_complete, "DELETE FROM installation_configuration_versions WHERE configuration_id = 'configuration-1'");

    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_complete(connection);
    connection.pragma_update(None, "ignore_check_constraints", "ON").unwrap();
    assert!(connection.execute("INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (2, 'installation-1', 'event-1', 2)", []).is_err());
    connection.pragma_update(None, "ignore_check_constraints", "OFF").unwrap();
}

fn fresh_database() -> (tempfile::TempDir, Database) {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    (temp, database)
}

fn assert_rejected_after(seed: fn(&rusqlite::Connection), statement: &str) {
    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed(connection);
    assert!(connection.execute(statement, []).is_err(), "expected rejection: {statement}");
}

fn seed_event(connection: &rusqlite::Connection) {
    connection.execute("INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (1, 'event-1', 1, 'created', 'system', 1, 'corr-1', '{}', 'digest-1')", []).unwrap();
}

fn seed_draft(connection: &rusqlite::Connection) {
    seed_event(connection);
    connection.execute("INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms) VALUES ('draft-1', 1, 'drafting', 'quick_start', '{}', 1, 1)", []).unwrap();
}

fn seed_configuration(connection: &rusqlite::Connection) {
    seed_draft(connection);
    connection.execute("INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('configuration-1', 1, 'draft-1', 'review-1', 'object-1', '{}', 'event-1', 1)", []).unwrap();
}

fn seed_complete(connection: &rusqlite::Connection) {
    seed_configuration(connection);
    connection.execute("INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (1, 'installation-1', 'event-1', 1)", []).unwrap();
    connection.execute("INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms) VALUES ('session-1', 'event-1', 1)", []).unwrap();
    connection.execute("INSERT INTO projection_metadata (singleton, last_event_sequence, projection_digest) VALUES (1, 0, 'projection-1')", []).unwrap();
    connection.execute("INSERT INTO active_installation_configuration (singleton, configuration_id, activated_event_id, activated_at_ms) VALUES (1, 'configuration-1', 'event-1', 1)", []).unwrap();
    connection.execute("INSERT INTO setup_step_outcomes (draft_id, step_key, attempt, status, occurred_at_ms) VALUES ('draft-1', 'step-1', 1, 'passed', 1)", []).unwrap();
    connection.execute("INSERT INTO capability_readiness (configuration_id, capability, status, checked_at_ms, projection_digest) VALUES ('configuration-1', 'capability-1', 'ready', 1, 'projection-1')", []).unwrap();
    connection.execute("INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms) VALUES ('approval-1', 'apply', 'configuration', 'configuration-1', 1, 'object-1', 'system', 'pending', 1)", []).unwrap();
}

fn normalize_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
}
