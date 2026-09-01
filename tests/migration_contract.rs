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
        "command_receipts",
        "command_event_refs",
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
        ("command_receipts", vec!["command_id", "command_fingerprint", "request_json", "capability", "policy_decision", "outcome_json"]),
        ("command_event_refs", vec!["command_id", "event_ordinal", "event_id"]),
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
        "command_event_refs_event_idx",
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
        "command_receipts_no_update",
        "command_receipts_no_delete",
        "command_event_refs_no_update",
        "command_event_refs_no_delete",
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
        ("command_event_refs", "command_id", "command_receipts", "command_id"),
        ("command_event_refs", "event_id", "event_stream", "event_id"),
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

    let receipt_sql: String = connection
        .query_row("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'command_receipts'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        normalize_sql(&receipt_sql),
        normalize_sql(
            "CREATE TABLE command_receipts (
                command_id TEXT PRIMARY KEY,
                command_fingerprint TEXT NOT NULL CHECK (
                    length(command_fingerprint) = 64
                    AND command_fingerprint NOT GLOB '*[^0-9a-f]*'
                ),
                request_json TEXT NOT NULL CHECK (json_valid(request_json)),
                capability TEXT NOT NULL CHECK (capability IN (
                    'help_read', 'status_read', 'setup_status_read', 'audit_read', 'shutdown',
                    'discussion_run', 'mcp_use', 'engineering_job_run', 'git_merge', 'git_push',
                    'finance_recommendation'
                )),
                policy_decision TEXT NOT NULL CHECK (policy_decision IN (
                    'granted', 'denied', 'denied_by_default', 'approval_required'
                )),
                outcome_json TEXT NOT NULL CHECK (json_valid(outcome_json))
            ) STRICT"
        )
    );
    let refs_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'command_event_refs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        normalize_sql(&refs_sql),
        normalize_sql(
            "CREATE TABLE command_event_refs (
                command_id TEXT NOT NULL REFERENCES command_receipts(command_id),
                event_ordinal INTEGER NOT NULL CHECK (event_ordinal >= 0),
                event_id TEXT NOT NULL REFERENCES event_stream(event_id),
                PRIMARY KEY (command_id, event_ordinal)
            ) STRICT"
        )
    );
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

    connection
        .execute(
            "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-1', ?1, '{}', 'help_read', 'granted', '{}')",
            ["a".repeat(64)],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('command-1', 0, 'event-1')",
            [],
        )
        .unwrap();
    assert!(connection
        .execute("UPDATE command_receipts SET capability = 'status_read' WHERE command_id = 'command-1'", [])
        .is_err());
    assert!(connection
        .execute("DELETE FROM command_receipts WHERE command_id = 'command-1'", [])
        .is_err());
    assert!(connection
        .execute("UPDATE command_event_refs SET event_ordinal = 1 WHERE command_id = 'command-1'", [])
        .is_err());
    assert!(connection
        .execute("DELETE FROM command_event_refs WHERE command_id = 'command-1'", [])
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

#[test]
fn semantic_index_oracle_detects_an_extra_autoindex_constraint() {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE oracle_fixture (
                id TEXT PRIMARY KEY,
                expected_unique TEXT UNIQUE,
                unexpected_unique TEXT UNIQUE
            ) STRICT;",
        )
        .unwrap();

    let error = assert_semantic_index_set(
        &connection,
        &["oracle_fixture"],
        &[
            semantic_index("oracle_fixture", "pk", true, false, &["id"]),
            semantic_index("oracle_fixture", "u", true, false, &["expected_unique"]),
        ],
    )
    .unwrap_err();

    assert!(error.contains("unexpected_unique"));
}

#[test]
fn every_enumerated_check_value_is_accepted() {
    assert_every_enumerated_check_value_is_accepted();
}

#[test]
fn command_receipt_fingerprint_and_enumerated_domains_reject_every_extra_form() {
    for fingerprint in [
        "A".repeat(64),
        "g".repeat(64),
        "a".repeat(65),
        "a".repeat(63),
    ] {
        let (_temp, database) = fresh_database();
        assert!(database
            .connection()
            .execute(
                "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command', ?1, '{}', 'help_read', 'granted', '{}')",
                [fingerprint],
            )
            .is_err());
    }

    for (capability, decision) in [
        ("HELP_READ", "granted"),
        ("help_read_extra", "granted"),
        ("help_read", "GRANTED"),
        ("help_read", "granted_extra"),
    ] {
        let (_temp, database) = fresh_database();
        assert!(database
            .connection()
            .execute(
                "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command', ?1, '{}', ?2, ?3, '{}')",
                rusqlite::params!["a".repeat(64), capability, decision],
            )
            .is_err());
    }
}

#[test]
fn sql_token_normalization_preserves_literal_case_but_ignores_sql_formatting() {
    assert_ne!(normalize_sql("SELECT 'Append-Only'"), normalize_sql("select 'append-only'"));
    assert_eq!(
        normalize_sql("CREATE TRIGGER sample BEFORE UPDATE ON item BEGIN SELECT RAISE(ABORT, 'Append-Only'); END"),
        normalize_sql("create trigger SAMPLE before update on ITEM begin select raise ( abort , 'Append-Only' ) ; end"),
    );
    assert_eq!(normalize_sql("SELECT 'it''s safe'"), normalize_sql("select 'it''s safe'"));
}

#[derive(Clone, Copy)]
struct ExpectedColumn {
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    default: Option<&'static str>,
    primary_key_position: i64,
}

#[derive(Clone, Copy)]
struct SemanticIndex {
    table: &'static str,
    origin: &'static str,
    unique: bool,
    partial: bool,
    columns: &'static [&'static str],
}

fn semantic_index(
    table: &'static str,
    origin: &'static str,
    unique: bool,
    partial: bool,
    columns: &'static [&'static str],
) -> SemanticIndex {
    SemanticIndex {
        table,
        origin,
        unique,
        partial,
        columns,
    }
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
            ("index".to_owned(), "command_event_refs_event_idx".to_owned()),
            ("index".to_owned(), "event_stream_correlation_idx".to_owned()),
            ("index".to_owned(), "event_stream_type_idx".to_owned()),
            ("index".to_owned(), "setup_drafts_state_idx".to_owned()),
            ("table".to_owned(), "active_installation_configuration".to_owned()),
            ("table".to_owned(), "approval_records".to_owned()),
            ("table".to_owned(), "capability_readiness".to_owned()),
            ("table".to_owned(), "command_event_refs".to_owned()),
            ("table".to_owned(), "command_receipts".to_owned()),
            ("table".to_owned(), "event_stream".to_owned()),
            ("table".to_owned(), "installation_configuration_versions".to_owned()),
            ("table".to_owned(), "installation_projection".to_owned()),
            ("table".to_owned(), "process_session_projection".to_owned()),
            ("table".to_owned(), "projection_metadata".to_owned()),
            ("table".to_owned(), "schema_migrations".to_owned()),
            ("table".to_owned(), "setup_drafts".to_owned()),
            ("table".to_owned(), "setup_step_outcomes".to_owned()),
            ("trigger".to_owned(), "command_event_refs_no_delete".to_owned()),
            ("trigger".to_owned(), "command_event_refs_no_update".to_owned()),
            ("trigger".to_owned(), "command_receipts_no_delete".to_owned()),
            ("trigger".to_owned(), "command_receipts_no_update".to_owned()),
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
        ("command_receipts", &[
            column("command_id", "TEXT", true, 1),
            column("command_fingerprint", "TEXT", true, 0),
            column("request_json", "TEXT", true, 0),
            column("capability", "TEXT", true, 0),
            column("policy_decision", "TEXT", true, 0),
            column("outcome_json", "TEXT", true, 0),
        ]),
        ("command_event_refs", &[
            column("command_id", "TEXT", true, 1),
            column("event_ordinal", "INTEGER", true, 2),
            column("event_id", "TEXT", true, 0),
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
        assert!(matches!(normalize_sql(&sql).last(), Some(token) if token == "strict"), "{table} must remain STRICT");
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
        ("command_event_refs", "command_id", "command_receipts", "command_id"),
        ("command_event_refs", "event_id", "event_stream", "event_id"),
        ("installation_configuration_versions", "created_event_id", "event_stream", "event_id"),
        ("installation_configuration_versions", "source_draft_id", "setup_drafts", "draft_id"),
        ("installation_projection", "created_event_id", "event_stream", "event_id"),
        ("process_session_projection", "ended_event_id", "event_stream", "event_id"),
        ("process_session_projection", "started_event_id", "event_stream", "event_id"),
        ("setup_step_outcomes", "draft_id", "setup_drafts", "draft_id"),
    ].into_iter().map(|(table, from, parent, to)| (table.to_owned(), from.to_owned(), parent.to_owned(), to.to_owned(), "NO ACTION".to_owned(), "NO ACTION".to_owned())).collect::<Vec<_>>();
    expected_foreign_keys.sort();
    assert_eq!(foreign_keys, expected_foreign_keys);

    let application_tables = expected_columns.iter().map(|(table, _)| *table).collect::<Vec<_>>();
    assert_semantic_index_set(connection, &application_tables, &[
        semantic_index("event_stream", "c", false, false, &["correlation_id", "sequence"]),
        semantic_index("event_stream", "c", false, false, &["event_type", "sequence"]),
        semantic_index("event_stream", "u", true, false, &["event_id"]),
        semantic_index("event_stream", "u", true, false, &["event_digest"]),
        semantic_index("command_receipts", "pk", true, false, &["command_id"]),
        semantic_index("command_event_refs", "pk", true, false, &["command_id", "event_ordinal"]),
        semantic_index("command_event_refs", "c", true, false, &["event_id"]),
        semantic_index("installation_projection", "u", true, false, &["installation_id"]),
        semantic_index("process_session_projection", "pk", true, false, &["session_id"]),
        semantic_index("setup_drafts", "c", false, false, &["state", "updated_at_ms"]),
        semantic_index("setup_drafts", "pk", true, false, &["draft_id"]),
        semantic_index("installation_configuration_versions", "pk", true, false, &["configuration_id"]),
        semantic_index("installation_configuration_versions", "u", true, false, &["source_draft_id", "review_digest"]),
        semantic_index("installation_configuration_versions", "u", true, false, &["version"]),
        semantic_index("setup_step_outcomes", "pk", true, false, &["draft_id", "step_key", "attempt"]),
        semantic_index("capability_readiness", "pk", true, false, &["configuration_id", "capability"]),
        semantic_index("approval_records", "c", false, false, &["status", "created_at_ms"]),
        semantic_index("approval_records", "pk", true, false, &["approval_id"]),
    ]).unwrap();
    for (name, table, columns) in [
        ("event_stream_correlation_idx", "event_stream", &["correlation_id", "sequence"][..]),
        ("event_stream_type_idx", "event_stream", &["event_type", "sequence"][..]),
        ("command_event_refs_event_idx", "command_event_refs", &["event_id"][..]),
        ("setup_drafts_state_idx", "setup_drafts", &["state", "updated_at_ms"][..]),
        ("approval_records_status_idx", "approval_records", &["status", "created_at_ms"][..]),
    ] {
        let sql: String = connection.query_row("SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1", [name], |row| row.get(0)).unwrap();
        let unique = if name == "command_event_refs_event_idx" { "unique " } else { "" };
        assert_eq!(normalize_sql(&sql), normalize_sql(&format!("create {unique}index {name} on {table}({})", columns.join(", "))));
    }

    for (name, expected_sql) in [
        ("event_stream_no_update", "create trigger event_stream_no_update before update on event_stream begin select raise(abort, 'event_stream is append-only'); end"),
        ("event_stream_no_delete", "create trigger event_stream_no_delete before delete on event_stream begin select raise(abort, 'event_stream is append-only'); end"),
        ("command_receipts_no_update", "create trigger command_receipts_no_update before update on command_receipts begin select raise(abort, 'command receipts are immutable'); end"),
        ("command_receipts_no_delete", "create trigger command_receipts_no_delete before delete on command_receipts begin select raise(abort, 'command receipts are immutable'); end"),
        ("command_event_refs_no_update", "create trigger command_event_refs_no_update before update on command_event_refs begin select raise(abort, 'command event refs are immutable'); end"),
        ("command_event_refs_no_delete", "create trigger command_event_refs_no_delete before delete on command_event_refs begin select raise(abort, 'command event refs are immutable'); end"),
        ("installation_configuration_versions_no_update", "create trigger installation_configuration_versions_no_update before update on installation_configuration_versions begin select raise(abort, 'installation configuration versions are immutable'); end"),
        ("installation_configuration_versions_no_delete", "create trigger installation_configuration_versions_no_delete before delete on installation_configuration_versions begin select raise(abort, 'installation configuration versions are immutable'); end"),
    ] {
        let sql: String = connection.query_row("SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?1", [name], |row| row.get(0)).unwrap();
        assert_eq!(normalize_sql(&sql), normalize_sql(expected_sql), "trigger definition for {name}");
    }
}

fn assert_every_task_six_constraint_is_enforced() {
    for statement in [
        "INSERT INTO schema_migrations (version, checksum) VALUES (0, 'x')",
        "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-bad-fingerprint', 'short', '{}', 'help_read', 'granted', '{}')",
        "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-bad-request', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 'not-json', 'help_read', 'granted', '{}')",
        "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-bad-capability', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', '{}', 'unknown', 'granted', '{}')",
        "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-bad-policy', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', '{}', 'help_read', 'unknown', '{}')",
        "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-bad-outcome', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', '{}', 'help_read', 'granted', 'not-json')",
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
    assert_rejected_after(seed_receipt_without_ref, "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('command-1', -1, 'event-1')");
    assert_rejected_after(seed_event, "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('missing-command', 0, 'event-1')");
    assert_rejected_after(seed_receipt_without_ref, "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('command-1', 0, 'missing-event')");
    assert_rejected_after(seed_receipt_without_ref, "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-1', 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', '{}', 'status_read', 'denied', '{}')");
    assert_rejected_after(seed_two_receipts_one_ref, "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('command-2', 0, 'event-1')");
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
    assert_rejected_after(seed_receipt, "UPDATE command_receipts SET capability = 'status_read' WHERE command_id = 'command-1'");
    assert_rejected_after(seed_receipt, "DELETE FROM command_receipts WHERE command_id = 'command-1'");
    assert_rejected_after(seed_receipt, "UPDATE command_event_refs SET event_ordinal = 1 WHERE command_id = 'command-1'");
    assert_rejected_after(seed_receipt, "DELETE FROM command_event_refs WHERE command_id = 'command-1'");

    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_complete(connection);
    connection.pragma_update(None, "ignore_check_constraints", "ON").unwrap();
    assert!(connection.execute("INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (2, 'installation-1', 'event-1', 2)", []).is_err());
    connection.pragma_update(None, "ignore_check_constraints", "OFF").unwrap();
}

fn assert_semantic_index_set(
    connection: &rusqlite::Connection,
    tables: &[&str],
    expected: &[SemanticIndex],
) -> Result<(), String> {
    let mut actual = Vec::new();
    for table in tables {
        let mut statement = connection.prepare(&format!("PRAGMA index_list({table})")).map_err(|error| error.to_string())?;
        let indexes = statement.query_map([], |row| Ok((
            row.get::<_, String>(1)?, row.get::<_, i64>(2)? != 0, row.get::<_, String>(3)?, row.get::<_, i64>(4)? != 0,
        ))).map_err(|error| error.to_string())?;
        for index in indexes {
            let (name, unique, origin, partial) = index.map_err(|error| error.to_string())?;
            let escaped_name = name.replace('\'', "''");
            let mut columns = connection.prepare(&format!("SELECT name FROM pragma_index_xinfo('{escaped_name}') WHERE key = 1 ORDER BY seqno")).map_err(|error| error.to_string())?;
            let columns = columns.query_map([], |row| row.get::<_, String>(0)).map_err(|error| error.to_string())?.map(|column| column.map_err(|error| error.to_string())).collect::<Result<Vec<_>, _>>()?;
            actual.push(((*table).to_owned(), origin, unique, partial, columns));
        }
    }
    actual.sort();
    let mut expected = expected.iter().map(|index| (
        index.table.to_owned(), index.origin.to_owned(), index.unique, index.partial,
        index.columns.iter().map(|column| (*column).to_owned()).collect::<Vec<_>>(),
    )).collect::<Vec<_>>();
    expected.sort();
    if actual == expected { Ok(()) } else { Err(format!("semantic index mismatch: actual={actual:?}, expected={expected:?}")) }
}

fn assert_every_enumerated_check_value_is_accepted() {
    let (_temp, database) = fresh_database();
    let connection = database.connection();
    for (index, capability) in [
        "help_read", "status_read", "setup_status_read", "audit_read", "shutdown",
        "discussion_run", "mcp_use", "engineering_job_run", "git_merge", "git_push",
        "finance_recommendation",
    ]
    .into_iter()
    .enumerate()
    {
        connection.execute(
            "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES (?1, ?2, '{}', ?3, 'granted', '{}')",
            rusqlite::params![format!("capability-{index}"), format!("{index:064x}"), capability],
        ).unwrap();
    }
    let (_temp, database) = fresh_database();
    let connection = database.connection();
    for (index, decision) in ["granted", "denied", "denied_by_default", "approval_required"]
        .into_iter()
        .enumerate()
    {
        connection.execute(
            "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES (?1, ?2, '{}', 'help_read', ?3, '{}')",
            rusqlite::params![format!("decision-{index}"), format!("{index:064x}"), decision],
        ).unwrap();
    }

    let (_temp, database) = fresh_database();
    let connection = database.connection();
    for (index, state) in ["drafting", "reviewed", "applied", "superseded"].into_iter().enumerate() {
        connection.execute("INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms) VALUES (?1, 1, ?2, 'quick_start', '{}', 1, 1)", rusqlite::params![format!("state-{index}"), state]).unwrap();
    }
    for (index, path) in ["quick_start", "customize"].into_iter().enumerate() {
        connection.execute("INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms) VALUES (?1, 1, 'drafting', ?2, '{}', 1, 1)", rusqlite::params![format!("path-{index}"), path]).unwrap();
    }

    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_draft(connection);
    for (index, status) in ["passed", "failed", "skipped"].into_iter().enumerate() {
        connection.execute("INSERT INTO setup_step_outcomes (draft_id, step_key, attempt, status, occurred_at_ms) VALUES ('draft-1', ?1, 1, ?2, 1)", rusqlite::params![format!("status-{index}"), status]).unwrap();
    }

    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_configuration(connection);
    for (index, status) in ["ready", "unavailable"].into_iter().enumerate() {
        connection.execute("INSERT INTO capability_readiness (configuration_id, capability, status, checked_at_ms, projection_digest) VALUES ('configuration-1', ?1, ?2, 1, 'projection')", rusqlite::params![format!("readiness-{index}"), status]).unwrap();
    }

    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_event(connection);
    connection.execute("INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms) VALUES ('approval-pending', 'apply', 'configuration', 'configuration-1', 1, 'object', 'system', 'pending', 1)", []).unwrap();
    for status in ["accepted", "rejected", "expired", "cancelled"] {
        connection.execute("INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms, resolved_at_ms, resolution_kind, resolution_event_id) VALUES (?1, 'apply', 'configuration', 'configuration-1', 1, 'object', 'system', ?2, 1, 2, 'resolved', 'event-1')", rusqlite::params![format!("approval-{status}"), status]).unwrap();
    }
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

fn seed_receipt_without_ref(connection: &rusqlite::Connection) {
    seed_event(connection);
    connection.execute(
        "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-1', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', '{}', 'help_read', 'granted', '{}')",
        [],
    ).unwrap();
}

fn seed_receipt(connection: &rusqlite::Connection) {
    seed_receipt_without_ref(connection);
    connection.execute(
        "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('command-1', 0, 'event-1')",
        [],
    ).unwrap();
}

fn seed_two_receipts_one_ref(connection: &rusqlite::Connection) {
    seed_receipt(connection);
    connection.execute(
        "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-2', 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', '{}', 'help_read', 'granted', '{}')",
        [],
    ).unwrap();
}

fn normalize_sql(sql: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut characters = sql.chars().peekable();
    while let Some(character) = characters.next() {
        if character.is_whitespace() { continue; }
        if character == '\'' {
            let mut quoted = String::from("'");
            while let Some(next) = characters.next() {
                quoted.push(next);
                if next == '\'' {
                    if characters.peek() == Some(&'\'') { quoted.push(characters.next().unwrap()); } else { break; }
                }
            }
            tokens.push(quoted);
        } else if character.is_ascii_alphanumeric() || character == '_' {
            let mut word = character.to_string();
            while let Some(next) = characters.peek() {
                if next.is_ascii_alphanumeric() || *next == '_' { word.push(characters.next().unwrap()); } else { break; }
            }
            tokens.push(word.to_ascii_lowercase());
        } else { tokens.push(character.to_string()); }
    }
    tokens
}
