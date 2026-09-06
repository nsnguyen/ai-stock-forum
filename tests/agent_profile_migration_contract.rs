use std::collections::BTreeMap;

use ai_stock_forum::{
    config::AppPaths,
    domain::sha256,
    persistence::{Database, LATEST_SCHEMA_VERSION},
};
use rusqlite::{Connection, Error as SqliteError, params};
use uuid::Uuid;

#[test]
fn fresh_database_reaches_schema_version_two_with_strict_profile_storage() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection();

    assert_eq!(LATEST_SCHEMA_VERSION, 2);
    assert_eq!(database.schema_version(), 2);
    assert_eq!(
        database
            .applied_migrations()
            .unwrap()
            .iter()
            .map(|migration| migration.version())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    for table in ["agent_profile_versions", "active_agent_profiles"] {
        assert!(database.has_table(table).unwrap(), "missing {table}");
        let sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(sql.ends_with(" STRICT"), "{table} is not STRICT");
    }
    for object in [
        "agent_profile_versions_history_idx",
        "active_agent_profiles_normalized_name_idx",
        "agent_profile_versions_no_update",
        "agent_profile_versions_no_delete",
    ] {
        assert!(
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
                    [object],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap(),
            "missing {object}"
        );
    }
}

#[test]
fn schema_v1_fixture_upgrades_without_changing_legacy_rows() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    create_schema_v1_fixture(&paths);

    let before = {
        let connection = Connection::open(paths.database_path()).unwrap();
        legacy_snapshot(&connection)
    };

    let database = Database::open(&paths).unwrap();

    assert_eq!(database.schema_version(), 2);
    assert!(database.has_table("agent_profile_versions").unwrap());
    assert!(database.has_table("active_agent_profiles").unwrap());
    assert_eq!(legacy_snapshot(database.connection()), before);
}

#[test]
fn every_ordered_v2_boundary_rolls_back_rows_inventory_and_version_record() {
    let expected_boundaries = vec![
        "drop_command_event_refs_no_update",
        "drop_command_event_refs_no_delete",
        "drop_command_event_refs_event_idx",
        "rename_command_event_refs_v1",
        "drop_command_receipts_no_update",
        "drop_command_receipts_no_delete",
        "rename_command_receipts_v1",
        "create_command_receipts",
        "rebuild_command_receipts",
        "create_command_receipts_no_update",
        "create_command_receipts_no_delete",
        "create_command_event_refs",
        "rebuild_command_event_refs",
        "create_command_event_refs_event_idx",
        "create_command_event_refs_no_update",
        "create_command_event_refs_no_delete",
        "drop_command_event_refs_v1",
        "drop_command_receipts_v1",
        "create_agent_profile_versions",
        "create_agent_profile_versions_history_idx",
        "create_agent_profile_namespace_insert_guard",
        "create_active_agent_profiles",
        "create_active_agent_profiles_normalized_name_idx",
        "create_agent_profile_versions_no_update",
        "create_agent_profile_versions_no_delete",
        "schema_migration_record",
    ];
    assert_eq!(Database::v2_migration_boundaries(), expected_boundaries);

    for boundary in expected_boundaries {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        create_schema_v1_fixture(&paths);
        let before = {
            let connection = Connection::open(paths.database_path()).unwrap();
            (
                legacy_snapshot(&connection),
                schema_inventory(&connection),
                migration_records(&connection),
                pragma_i64(&connection, "application_id"),
                pragma_i64(&connection, "user_version"),
            )
        };

        assert!(matches!(
            Database::open_with_migration_fault(&paths, 2, boundary),
            Err(error) if error.code() == "database_unavailable"
        ));

        let connection = Connection::open(paths.database_path()).unwrap();
        assert_eq!(
            legacy_snapshot(&connection),
            before.0,
            "boundary {boundary}"
        );
        assert_eq!(
            schema_inventory(&connection),
            before.1,
            "boundary {boundary}"
        );
        assert_eq!(
            migration_records(&connection),
            before.2,
            "boundary {boundary}"
        );
        assert_eq!(
            pragma_i64(&connection, "application_id"),
            before.3,
            "boundary {boundary}"
        );
        assert_eq!(
            pragma_i64(&connection, "user_version"),
            before.4,
            "boundary {boundary}"
        );
        assert!(!database_object_exists(
            &connection,
            "agent_profile_versions"
        ));
        assert!(!database_object_exists(
            &connection,
            "active_agent_profiles"
        ));
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations WHERE version = 2",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
            "boundary {boundary}"
        );
    }
}

#[test]
fn failed_v1_migration_rolls_back_every_earlier_schema_object() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    let connection = Connection::open(paths.database_path()).unwrap();
    connection
        .execute_batch("CREATE TABLE approval_records (marker TEXT) STRICT;")
        .unwrap();
    drop(connection);

    assert!(matches!(
        Database::open(&paths),
        Err(error) if error.code() == "database_unavailable"
    ));

    let connection = Connection::open(paths.database_path()).unwrap();
    assert_eq!(pragma_i64(&connection, "user_version"), 0);
    for object in [
        "schema_migrations",
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
        "approval_records_status_idx",
    ] {
        assert!(
            !connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
                    [object],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap(),
            "unexpected rolled-back object {object}"
        );
    }
    assert!(
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'approval_records')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap(),
        "pre-existing conflict object was altered"
    );
}

#[test]
fn profile_version_mirror_is_immutable_and_active_pointer_replaces_transactionally() {
    let temp = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection_mut();
    insert_version(connection, "profile-1", "version-1", 1, None, "research", 1);
    insert_version(
        connection,
        "profile-1",
        "version-2",
        2,
        Some("version-1"),
        "research",
        2,
    );
    insert_active(connection, "profile-1", "version-1", 1, "research");

    assert_immutable(
        connection
            .execute(
                "UPDATE agent_profile_versions SET normalized_name = 'changed' WHERE profile_id = 'profile-1'",
                [],
            )
            .unwrap_err(),
    );
    assert_immutable(
        connection
            .execute(
                "DELETE FROM agent_profile_versions WHERE profile_id = 'profile-1' AND version = 1",
                [],
            )
            .unwrap_err(),
    );

    let transaction = connection.transaction().unwrap();
    transaction
        .execute(
            "DELETE FROM active_agent_profiles WHERE profile_id = 'profile-1'",
            [],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO active_agent_profiles
                (profile_id, profile_version_id, version, normalized_name, content_digest)
             SELECT profile_id, profile_version_id, version, 'research', content_digest
             FROM agent_profile_versions
             WHERE profile_id = 'profile-1' AND profile_version_id = 'version-2'",
            [],
        )
        .unwrap();
    transaction.commit().unwrap();

    assert_eq!(
        connection
            .query_row(
                "SELECT profile_version_id FROM active_agent_profiles WHERE profile_id = 'profile-1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "version-2"
    );
}

#[test]
fn foreign_keys_and_binary_folded_name_uniqueness_are_effective() {
    let temp = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection_mut();

    assert!(
        connection
            .execute(
                "INSERT INTO active_agent_profiles
                (profile_id, profile_version_id, version, normalized_name, content_digest)
             VALUES ('missing', 'missing-version', 1, 'missing',
                     '0000000000000000000000000000000000000000000000000000000000000000')",
                [],
            )
            .is_err()
    );

    insert_version(connection, "profile-1", "version-1", 1, None, "Research", 1);
    insert_version(connection, "profile-2", "version-2", 1, None, "research", 2);
    insert_version(connection, "profile-3", "version-3", 1, None, "Research", 3);
    insert_active(connection, "profile-1", "version-1", 1, "Research");
    insert_active(connection, "profile-2", "version-2", 1, "research");

    assert!(
        connection
            .execute(
                "INSERT INTO active_agent_profiles
                (profile_id, profile_version_id, version, normalized_name, content_digest)
             SELECT profile_id, profile_version_id, version, 'Research', content_digest
             FROM agent_profile_versions
             WHERE profile_id = 'profile-3' AND profile_version_id = 'version-3'",
                [],
            )
            .is_err()
    );
}

#[test]
fn memory_namespace_is_global_to_one_profile_and_stable_across_versions() {
    let temp = tempfile::tempdir().unwrap();
    let mut database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection_mut();
    let namespace = Uuid::from_u128(41).to_string();
    insert_version_with_namespace(
        connection,
        "profile-1",
        "version-1",
        1,
        None,
        "research one",
        &namespace,
        1,
    );

    let cross_profile = insert_version_with_namespace_result(
        connection,
        "profile-2",
        "version-2",
        1,
        None,
        "research two",
        &namespace,
        2,
    )
    .unwrap_err();
    assert!(matches!(
        cross_profile,
        SqliteError::SqliteFailure(_, Some(message))
            if message == "agent_profile_namespace_conflict"
    ));

    let drift = insert_version_with_namespace_result(
        connection,
        "profile-1",
        "version-3",
        2,
        Some("version-1"),
        "research one",
        &Uuid::from_u128(42).to_string(),
        3,
    )
    .unwrap_err();
    assert!(matches!(
        drift,
        SqliteError::SqliteFailure(_, Some(message))
            if message == "agent_profile_namespace_conflict"
    ));
}

#[test]
fn startup_is_idempotent_after_version_two_is_applied() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());

    drop(Database::open(&paths).unwrap());
    let database = Database::open(&paths).unwrap();

    assert_eq!(database.schema_version(), 2);
    assert_eq!(database.applied_migrations().unwrap().len(), 2);
}

fn create_schema_v1_fixture(paths: &AppPaths) {
    let connection = Connection::open(paths.database_path()).unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_phase0.sql"))
        .unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY CHECK (version > 0),
                checksum TEXT NOT NULL
            ) STRICT;",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations (version, checksum) VALUES (1, ?1)",
            [sha256(include_str!("../migrations/0001_phase0.sql").as_bytes()).as_str()],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    connection
        .execute(
            "INSERT INTO event_stream (
                sequence, event_id, event_schema_version, event_type, actor_kind, actor_id,
                occurred_at_ms, correlation_id, causation_id, object_kind, object_id,
                object_version, object_digest, previous_event_digest, payload_json, event_digest
            ) VALUES (
                1, 'event-1', 1, 'installation_initialized', 'human', NULL,
                1, 'correlation-1', NULL, NULL, NULL, NULL, NULL, NULL, '{}', 'digest-1'
            )",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO command_receipts (
                command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json
            ) VALUES ('command-1', ?1, '{}', 'audit_read', 'granted', '{}')",
            ["a".repeat(64)],
        )
        .unwrap();
    connection
        .execute_batch(
            "INSERT INTO command_event_refs (command_id, event_ordinal, event_id)
                VALUES ('command-1', 0, 'event-1');
             INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms)
                VALUES (1, 'installation-1', 'event-1', 1);
             INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms)
                VALUES ('session-1', 'event-1', 1);
             INSERT INTO projection_metadata (singleton, last_event_sequence, last_event_digest, projection_digest)
                VALUES (1, 1, 'digest-1', 'projection-digest-1');
             INSERT INTO setup_drafts (
                draft_id, schema_version, state, path, current_review_digest, payload_json, created_at_ms, updated_at_ms
             ) VALUES ('draft-1', 1, 'drafting', 'customize', NULL, '{}', 1, 1);
             INSERT INTO installation_configuration_versions (
                configuration_id, version, source_draft_id, review_digest, object_digest,
                payload_json, created_event_id, created_at_ms
             ) VALUES ('configuration-1', 1, 'draft-1', 'review-1', 'object-1', '{}', 'event-1', 1);
             INSERT INTO active_installation_configuration (
                singleton, configuration_id, activated_event_id, activated_at_ms
             ) VALUES (1, 'configuration-1', 'event-1', 1);
             INSERT INTO setup_step_outcomes (draft_id, step_key, attempt, status, safe_code, occurred_at_ms)
                VALUES ('draft-1', 'validate', 1, 'passed', NULL, 1);
             INSERT INTO capability_readiness (
                configuration_id, capability, status, reason_code, checked_at_ms, projection_digest
             ) VALUES ('configuration-1', 'status_read', 'ready', NULL, 1, 'projection-digest-1');
             INSERT INTO approval_records (
                approval_id, action_kind, object_kind, object_id, object_version, object_digest,
                actor_kind, actor_id, status, created_at_ms, expires_at_ms, resolved_at_ms,
                resolution_kind, resolution_event_id
             ) VALUES (
                'approval-1', 'merge', 'change', 'change-1', 1, 'object-1', 'human', NULL,
                'pending', 1, NULL, NULL, NULL, NULL
             );",
        )
        .unwrap();
}

fn legacy_snapshot(connection: &Connection) -> BTreeMap<&'static str, Vec<String>> {
    [
        ("event_stream", "sequence, event_id, event_schema_version, event_type, actor_kind, actor_id, occurred_at_ms, correlation_id, causation_id, object_kind, object_id, object_version, object_digest, previous_event_digest, payload_json, event_digest"),
        ("command_receipts", "command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json"),
        ("command_event_refs", "command_id, event_ordinal, event_id"),
        ("installation_projection", "singleton, installation_id, created_event_id, created_at_ms"),
        ("process_session_projection", "session_id, started_event_id, started_at_ms, ended_event_id, ended_at_ms, end_reason"),
        ("projection_metadata", "singleton, last_event_sequence, last_event_digest, projection_digest"),
        ("setup_drafts", "draft_id, schema_version, state, path, current_review_digest, payload_json, created_at_ms, updated_at_ms"),
        ("installation_configuration_versions", "configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms"),
        ("active_installation_configuration", "singleton, configuration_id, activated_event_id, activated_at_ms"),
        ("setup_step_outcomes", "draft_id, step_key, attempt, status, safe_code, occurred_at_ms"),
        ("capability_readiness", "configuration_id, capability, status, reason_code, checked_at_ms, projection_digest"),
        ("approval_records", "approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, actor_id, status, created_at_ms, expires_at_ms, resolved_at_ms, resolution_kind, resolution_event_id"),
    ]
    .into_iter()
    .map(|(table, columns)| {
        let mut statement = connection
            .prepare(&format!("SELECT json_array({columns}) FROM {table} ORDER BY rowid"))
            .unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        (table, rows)
    })
    .collect()
}

fn schema_inventory(connection: &Connection) -> Vec<(String, String, String, Option<String>)> {
    let mut statement = connection
        .prepare(
            "SELECT type, name, tbl_name, sql
             FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%'
             ORDER BY type, name",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn migration_records(connection: &Connection) -> Vec<(i64, String)> {
    let mut statement = connection
        .prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn database_object_exists(connection: &Connection, name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

fn insert_version(
    connection: &Connection,
    profile_id: &str,
    profile_version_id: &str,
    version: i64,
    supersedes_version_id: Option<&str>,
    normalized_name: &str,
    source_event_sequence: i64,
) {
    let namespace_seed = profile_id.bytes().fold(0_u128, |seed, byte| {
        seed.wrapping_mul(257) + u128::from(byte)
    });
    insert_version_with_namespace(
        connection,
        profile_id,
        profile_version_id,
        version,
        supersedes_version_id,
        normalized_name,
        &Uuid::from_u128(namespace_seed).to_string(),
        source_event_sequence,
    );
}

#[allow(clippy::too_many_arguments)]
fn insert_version_with_namespace(
    connection: &Connection,
    profile_id: &str,
    profile_version_id: &str,
    version: i64,
    supersedes_version_id: Option<&str>,
    normalized_name: &str,
    memory_namespace_id: &str,
    source_event_sequence: i64,
) {
    insert_version_with_namespace_result(
        connection,
        profile_id,
        profile_version_id,
        version,
        supersedes_version_id,
        normalized_name,
        memory_namespace_id,
        source_event_sequence,
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn insert_version_with_namespace_result(
    connection: &Connection,
    profile_id: &str,
    profile_version_id: &str,
    version: i64,
    supersedes_version_id: Option<&str>,
    normalized_name: &str,
    memory_namespace_id: &str,
    source_event_sequence: i64,
) -> rusqlite::Result<usize> {
    let content_digest = format!("{source_event_sequence:064x}");
    connection.execute(
        "INSERT INTO agent_profile_versions (
            profile_id, profile_version_id, version, supersedes_version_id,
            template_id, template_version, template_digest, role, display_name,
            normalized_name, memory_namespace_id, policy_profile_ref, content_digest,
            payload_json, source_event_sequence, created_at_ms
        ) VALUES (
            ?1, ?2, ?3, ?4, NULL, NULL, NULL, 'custom', ?5, ?5, ?6,
            'profile-default/v1', ?7, ?8, ?9, 1
        )",
        params![
            profile_id,
            profile_version_id,
            version,
            supersedes_version_id,
            normalized_name,
            memory_namespace_id,
            content_digest,
            b"{}".as_slice(),
            source_event_sequence,
        ],
    )
}

fn insert_active(
    connection: &Connection,
    profile_id: &str,
    profile_version_id: &str,
    version: i64,
    normalized_name: &str,
) {
    connection
        .execute(
            "INSERT INTO active_agent_profiles
                (profile_id, profile_version_id, version, normalized_name, content_digest)
             SELECT profile_id, profile_version_id, version, ?4, content_digest
             FROM agent_profile_versions
             WHERE profile_id = ?1 AND profile_version_id = ?2 AND version = ?3",
            params![profile_id, profile_version_id, version, normalized_name],
        )
        .unwrap();
}

fn assert_immutable(error: SqliteError) {
    assert!(matches!(
        error,
        SqliteError::SqliteFailure(_, Some(message)) if message == "agent_profile_versions_immutable"
    ));
}

fn pragma_i64(connection: &Connection, name: &str) -> i64 {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .unwrap()
}
