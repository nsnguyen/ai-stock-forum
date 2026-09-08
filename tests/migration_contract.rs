use std::fs;

use ai_stock_forum::config::AppPaths;
use ai_stock_forum::persistence::{Database, LATEST_SCHEMA_VERSION};
use ai_stock_forum::policy::ApprovalStatus;

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
        "memory_entry_versions",
        "current_memory_entries",
        "memory_proposals",
        "memory_proposal_resolutions",
        "current_memory_proposal_status",
        "episodic_summaries",
        "episodic_summary_sources",
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
        Database::open(&paths)
            .unwrap()
            .applied_migrations()
            .unwrap()
            .len(),
        LATEST_SCHEMA_VERSION as usize
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

    assert!(
        matches!(Database::open(&paths), Err(error) if error.code() == "database_schema_newer")
    );
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

    assert!(
        matches!(Database::open(&paths), Err(error) if error.code() == "database_application_mismatch")
    );
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

    assert!(
        matches!(Database::open(&paths), Err(error) if error.code() == "database_migration_state_invalid")
    );
}

#[test]
fn migration_records_ahead_of_user_version_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());

    drop(Database::open(&paths).unwrap());
    let raw = rusqlite::Connection::open(paths.database_path()).unwrap();
    raw.execute(
        "INSERT INTO schema_migrations (version, checksum) VALUES (?1, ?2)",
        (5_i64, "0".repeat(64)),
    )
    .unwrap();
    drop(raw);

    assert!(
        matches!(Database::open(&paths), Err(error) if error.code() == "database_migration_state_invalid")
    );
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

    assert!(
        matches!(Database::open(&paths), Err(error) if error.code() == "database_migration_state_invalid")
    );

    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    drop(Database::open(&paths).unwrap());
    let raw = rusqlite::Connection::open(paths.database_path()).unwrap();
    raw.pragma_update(None, "user_version", 0).unwrap();
    drop(raw);

    assert!(
        matches!(Database::open(&paths), Err(error) if error.code() == "database_migration_state_invalid")
    );
}

#[test]
fn migration_records_and_complete_schema_are_exact() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    let database = Database::open(&paths).unwrap();
    let connection = database.connection();

    let migration = database.applied_migrations().unwrap();
    assert_eq!(migration.len(), 4);
    assert_eq!(migration[0].version(), 1);
    assert_eq!(
        migration[0].checksum().as_str(),
        ai_stock_forum::domain::sha256(include_str!("../migrations/0001_phase0.sql").as_bytes())
            .as_str()
    );
    assert_eq!(migration[1].version(), 2);
    assert_eq!(
        migration[1].checksum().as_str(),
        ai_stock_forum::domain::sha256(
            include_str!("../migrations/0002_agent_profiles.sql").as_bytes()
        )
        .as_str()
    );
    assert_eq!(migration[2].version(), 3);
    assert_eq!(
        migration[2].checksum().as_str(),
        ai_stock_forum::domain::sha256(
            include_str!("../migrations/0003_declarative_skills.sql").as_bytes()
        )
        .as_str()
    );

    for (table, expected_columns) in [
        (
            "event_stream",
            vec![
                "sequence",
                "event_id",
                "event_schema_version",
                "event_type",
                "actor_kind",
                "actor_id",
                "occurred_at_ms",
                "correlation_id",
                "causation_id",
                "object_kind",
                "object_id",
                "object_version",
                "object_digest",
                "previous_event_digest",
                "payload_json",
                "event_digest",
            ],
        ),
        (
            "command_receipts",
            vec![
                "command_id",
                "command_fingerprint",
                "request_json",
                "capability",
                "policy_decision",
                "outcome_json",
            ],
        ),
        (
            "command_event_refs",
            vec!["command_id", "event_ordinal", "event_id"],
        ),
        (
            "installation_projection",
            vec![
                "singleton",
                "installation_id",
                "created_event_id",
                "created_at_ms",
            ],
        ),
        (
            "process_session_projection",
            vec![
                "session_id",
                "started_event_id",
                "started_at_ms",
                "ended_event_id",
                "ended_at_ms",
                "end_reason",
            ],
        ),
        (
            "projection_metadata",
            vec![
                "singleton",
                "last_event_sequence",
                "last_event_digest",
                "projection_digest",
            ],
        ),
        (
            "setup_drafts",
            vec![
                "draft_id",
                "schema_version",
                "state",
                "path",
                "current_review_digest",
                "payload_json",
                "created_at_ms",
                "updated_at_ms",
            ],
        ),
        (
            "installation_configuration_versions",
            vec![
                "configuration_id",
                "version",
                "source_draft_id",
                "review_digest",
                "object_digest",
                "payload_json",
                "created_event_id",
                "created_at_ms",
            ],
        ),
        (
            "active_installation_configuration",
            vec![
                "singleton",
                "configuration_id",
                "activated_event_id",
                "activated_at_ms",
            ],
        ),
        (
            "setup_step_outcomes",
            vec![
                "draft_id",
                "step_key",
                "attempt",
                "status",
                "safe_code",
                "occurred_at_ms",
            ],
        ),
        (
            "capability_readiness",
            vec![
                "configuration_id",
                "capability",
                "status",
                "reason_code",
                "checked_at_ms",
                "projection_digest",
            ],
        ),
        (
            "approval_records",
            vec![
                "approval_id",
                "action_kind",
                "object_kind",
                "object_id",
                "object_version",
                "object_digest",
                "actor_kind",
                "actor_id",
                "status",
                "created_at_ms",
                "expires_at_ms",
                "resolved_at_ms",
                "resolution_kind",
                "resolution_event_id",
                "resolution_actor_kind",
                "resolution_actor_id",
            ],
        ),
    ] {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
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
        assert!(
            connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1)",
                    [index],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap()
        );
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
        (
            "installation_projection",
            "created_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "command_event_refs",
            "command_id",
            "command_receipts",
            "command_id",
        ),
        ("command_event_refs", "event_id", "event_stream", "event_id"),
        (
            "process_session_projection",
            "started_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "process_session_projection",
            "ended_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "installation_configuration_versions",
            "source_draft_id",
            "setup_drafts",
            "draft_id",
        ),
        (
            "installation_configuration_versions",
            "created_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "active_installation_configuration",
            "configuration_id",
            "installation_configuration_versions",
            "configuration_id",
        ),
        (
            "active_installation_configuration",
            "activated_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "setup_step_outcomes",
            "draft_id",
            "setup_drafts",
            "draft_id",
        ),
        (
            "capability_readiness",
            "configuration_id",
            "installation_configuration_versions",
            "configuration_id",
        ),
        (
            "approval_records",
            "resolution_event_id",
            "event_stream",
            "event_id",
        ),
    ] {
        let mut statement = connection
            .prepare(&format!("PRAGMA foreign_key_list({table})"))
            .unwrap();
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
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'event_stream'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(event_sql.contains("CHECK (json_valid(payload_json))"));
    assert!(event_sql.contains("CHECK (object_version IS NULL OR object_version > 0)"));
    assert!(event_sql.ends_with(" STRICT"));

    let receipt_sql: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'command_receipts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        normalize_sql(&receipt_sql),
        normalize_sql(
            "CREATE TABLE command_receipts (
                command_id TEXT PRIMARY KEY,
                command_fingerprint TEXT NOT NULL CHECK (
                    typeof(command_fingerprint) = 'text'
                    AND length(CAST(command_fingerprint AS BLOB)) = 64
                    AND instr(command_fingerprint, char(0)) = 0
                    AND command_fingerprint NOT GLOB '*[^0-9a-f]*'
                ),
                request_json TEXT NOT NULL CHECK (json_valid(request_json)),
                capability TEXT NOT NULL CHECK (capability IN (
                    'help_read', 'status_read', 'setup_status_read', 'audit_read',
                    'agent_profile_read', 'agent_profile_create', 'agent_profile_preview',
                    'agent_profile_activate', 'skill_read', 'skill_create', 'skill_version',
                    'skill_assign', 'skill_unassign', 'memory_read', 'memory_preview',
                    'memory_mutate', 'memory_propose', 'memory_resolve', 'shutdown',
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

    assert_eq!(
        connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        connection
            .query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5_000
    );
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
    assert!(
        connection
            .execute(
                "UPDATE event_stream SET event_type = 'changed' WHERE sequence = 1",
                []
            )
            .is_err()
    );

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
    assert!(
        connection
            .execute(
                "DELETE FROM command_receipts WHERE command_id = 'command-1'",
                []
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "UPDATE command_event_refs SET event_ordinal = 1 WHERE command_id = 'command-1'",
                []
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "DELETE FROM command_event_refs WHERE command_id = 'command-1'",
                []
            )
            .is_err()
    );
    assert!(
        connection
            .execute("DELETE FROM event_stream WHERE sequence = 1", [])
            .is_err()
    );
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

    assert_eq!(
        fs::metadata(paths.database_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn task_six_schema_contract_is_exact_and_every_constraint_is_enforced() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();

    assert_complete_task_six_schema_contract(database.connection());
    assert_every_task_six_constraint_is_enforced();
}

#[test]
fn v4_approval_actor_transition_and_memory_capability_guards_are_effective() {
    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_event(connection);
    let agent = "00000000-0000-0000-0000-000000000001";
    let digest = "a".repeat(64);
    for (id, kind, actor_id) in [
        ("human", "human", None),
        ("system", "system", None),
        ("agent", "agent", Some(agent)),
    ] {
        connection.execute(
            "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, actor_id, status, created_at_ms) VALUES (?1, 'apply', 'configuration', ?1, 1, ?2, ?3, ?4, 'pending', 1)",
            rusqlite::params![id, digest, kind, actor_id],
        ).unwrap();
    }
    for (id, kind, actor_id) in [
        ("bad-human", "human", Some(agent)),
        ("bad-system", "system", Some(agent)),
        ("bad-agent-null", "agent", None),
        (
            "bad-agent-uppercase",
            "agent",
            Some("00000000-0000-0000-0000-00000000000A"),
        ),
    ] {
        assert!(connection.execute(
            "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, actor_id, status, created_at_ms) VALUES (?1, 'apply', 'configuration', ?1, 1, ?2, ?3, ?4, 'pending', 1)",
            rusqlite::params![id, digest, kind, actor_id],
        ).is_err(), "invalid requester {id} was accepted");
    }
    connection.execute(
        "UPDATE approval_records SET status = 'accepted', resolved_at_ms = 2, resolution_kind = 'accepted', resolution_event_id = 'event-1', resolution_actor_kind = 'human', resolution_actor_id = NULL WHERE approval_id = 'agent'",
        [],
    ).unwrap();
    assert!(connection.execute("UPDATE approval_records SET resolution_kind = 'rejected' WHERE approval_id = 'agent'", []).is_err());

    for (id, status, resolver) in [
        ("memory-pending", "pending", None),
        ("memory-accepted", "accepted", Some("human")),
    ] {
        connection.execute(
            "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, actor_id, status, created_at_ms, resolved_at_ms, resolution_kind, resolution_event_id, resolution_actor_kind, resolution_actor_id) VALUES (?1, 'memory_mutation', 'memory_proposal', 'proposal', 1, ?2, 'agent', ?3, ?4, 1, CASE WHEN ?4 = 'pending' THEN NULL ELSE 2 END, CASE WHEN ?4 = 'pending' THEN NULL ELSE ?4 END, CASE WHEN ?4 = 'pending' THEN NULL ELSE 'event-1' END, ?5, NULL)",
            rusqlite::params![id, digest, agent, status, resolver],
        ).unwrap();
    }
    for (id, status, resolver) in [
        ("memory-cancelled", "cancelled", "human"),
        ("memory-system-resolver", "accepted", "system"),
    ] {
        assert!(connection.execute(
            "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, actor_id, status, created_at_ms, resolved_at_ms, resolution_kind, resolution_event_id, resolution_actor_kind, resolution_actor_id) VALUES (?1, 'memory_mutation', 'memory_proposal', 'proposal', 1, ?2, 'agent', ?3, ?4, 1, 2, ?4, 'event-1', ?5, NULL)",
            rusqlite::params![id, digest, agent, status, resolver],
        ).is_err(), "invalid memory approval {id} was accepted");
    }
    for (ordinal, capability) in [
        "memory_read",
        "memory_preview",
        "memory_mutate",
        "memory_propose",
        "memory_resolve",
    ]
    .into_iter()
    .enumerate()
    {
        connection.execute("INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES (?1, ?2, '{}', ?3, 'granted', '{}')", rusqlite::params![format!("memory-capability-{ordinal}"), "b".repeat(64), capability]).unwrap();
    }
}

#[test]
fn exhaustive_v4_schema_oracle_detects_added_and_omitted_objects() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection();
    connection
        .execute_batch("CREATE INDEX unexpected_v4_index ON memory_proposals (proposal_id);")
        .unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_complete_task_six_schema_contract(connection)
        }))
        .is_err()
    );
    connection
        .execute_batch(
            "DROP INDEX unexpected_v4_index; DROP INDEX current_memory_entries_list_idx;",
        )
        .unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_complete_task_six_schema_contract(connection)
        }))
        .is_err()
    );
}

#[test]
fn exhaustive_v4_schema_oracle_detects_inline_constraint_and_trigger_body_mutations() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection();
    connection.execute_batch("PRAGMA writable_schema = ON; UPDATE sqlite_schema SET sql = replace(sql, 'UNIQUE (proposal_id, status, resolution_event_id)', 'CHECK (proposal_id IS NOT NULL)') WHERE type = 'table' AND name = 'memory_proposal_resolutions'; PRAGMA writable_schema = OFF;").unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_complete_task_six_schema_contract(connection)
        }))
        .is_err()
    );

    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection();
    connection.execute_batch("DROP TRIGGER memory_proposals_no_update; CREATE TRIGGER memory_proposals_no_update BEFORE UPDATE ON memory_proposals BEGIN SELECT 1; END;").unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_complete_task_six_schema_contract(connection)
        }))
        .is_err()
    );
}

#[test]
fn v4_memory_history_pointer_and_episodic_source_guards_are_effective() {
    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_v4_memory_parents(connection);
    let namespace = "00000000-0000-0000-0000-000000000003";
    let entry = "00000000-0000-0000-0000-000000000021";
    let entry_version = "00000000-0000-0000-0000-000000000022";
    let digest = "d".repeat(64);
    connection.execute(
        "INSERT INTO memory_entry_versions (memory_namespace_id, entry_id, entry_version_id, version, predecessor_version_id, display_key, normalized_key, state, value_text, value_bytes, purpose_tags_json, created_by_kind, created_by_id, created_at_ms, accepted_proposal_id, accepted_proposal_version, accepted_proposal_digest, plaintext_validation_version, creation_event_sequence, creation_event_id, content_digest, record_digest, record_json) VALUES (?1, ?2, ?3, 1, NULL, 'Key', 'key', 'present', 'value', 5, CAST('[]' AS BLOB), 'human', NULL, 1, NULL, NULL, NULL, 1, 1, 'event-1', ?4, ?4, CAST('{}' AS BLOB))",
        rusqlite::params![namespace, entry, entry_version, digest],
    ).unwrap();
    connection.execute(
        "INSERT INTO current_memory_entries (memory_namespace_id, normalized_key, entry_id, entry_version_id, version, state, content_digest) VALUES (?1, 'key', ?2, ?3, 1, 'present', ?4)",
        rusqlite::params![namespace, entry, entry_version, digest],
    ).unwrap();
    assert!(connection.execute(
        "INSERT INTO current_memory_entries (memory_namespace_id, normalized_key, entry_id, entry_version_id, version, state, content_digest) VALUES (?1, 'wrong', ?2, '00000000-0000-0000-0000-000000000099', 1, 'present', ?3)",
        rusqlite::params![namespace, entry, digest],
    ).is_err());
    let successor_version = "00000000-0000-0000-0000-000000000023";
    let transaction = connection.unchecked_transaction().unwrap();
    transaction.execute(
        "INSERT INTO memory_entry_versions (memory_namespace_id, entry_id, entry_version_id, version, predecessor_version_id, display_key, normalized_key, state, value_text, value_bytes, purpose_tags_json, created_by_kind, created_by_id, created_at_ms, accepted_proposal_id, accepted_proposal_version, accepted_proposal_digest, plaintext_validation_version, creation_event_sequence, creation_event_id, content_digest, record_digest, record_json) VALUES (?1, ?2, ?3, 2, ?4, 'Key', 'key', 'present', 'next', 4, CAST('[]' AS BLOB), 'human', NULL, 2, NULL, NULL, NULL, 1, 2, 'event-2', ?5, ?5, CAST('{}' AS BLOB))",
        rusqlite::params![namespace, entry, successor_version, entry_version, digest],
    ).unwrap();
    transaction.execute("DELETE FROM current_memory_entries WHERE memory_namespace_id = ?1 AND normalized_key = 'key'", [namespace]).unwrap();
    transaction.execute("INSERT INTO current_memory_entries (memory_namespace_id, normalized_key, entry_id, entry_version_id, version, state, content_digest) VALUES (?1, 'key', ?2, ?3, 2, 'present', ?4)", rusqlite::params![namespace, entry, successor_version, digest]).unwrap();
    transaction.commit().unwrap();
    assert_eq!(connection.query_row("SELECT entry_version_id, version, content_digest FROM current_memory_entries WHERE memory_namespace_id = ?1 AND normalized_key = 'key'", [namespace], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))).unwrap(), (successor_version.to_owned(), 2, digest.clone()));
    assert!(
        connection
            .execute(
                "UPDATE memory_entry_versions SET value_text = 'changed' WHERE entry_id = ?1",
                [entry]
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "DELETE FROM memory_entry_versions WHERE entry_id = ?1",
                [entry]
            )
            .is_err()
    );

    let summary = "00000000-0000-0000-0000-000000000031";
    connection.execute(
        "INSERT INTO episodic_summaries (summary_id, version, memory_namespace_id, profile_id, profile_version_id, profile_version, profile_content_digest, label, body, purpose_tags_json, source_count, plaintext_validation_version, created_at_ms, creation_event_sequence, creation_event_id, source_set_digest, content_digest, record_digest, record_json) VALUES (?1, 1, ?2, '00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 1, ?3, 'Summary', 'Body', CAST('[]' AS BLOB), 1, 1, 2, 2, 'event-2', ?3, ?3, ?3, CAST('{}' AS BLOB))",
        rusqlite::params![summary, namespace, "a".repeat(64)],
    ).unwrap();
    connection.execute(
        "INSERT INTO episodic_summary_sources (summary_id, source_ordinal, event_sequence, event_id, event_type, event_digest) VALUES (?1, 0, 1, 'event-1', 'legacy.created', ?2)",
        rusqlite::params![summary, "e".repeat(64)],
    ).unwrap();
    assert!(connection.execute(
        "INSERT INTO episodic_summary_sources (summary_id, source_ordinal, event_sequence, event_id, event_type, event_digest) VALUES (?1, 1, 2, 'event-2', 'summary.created', ?2)",
        rusqlite::params![summary, "f".repeat(64)],
    ).is_err(), "source event at or after the summary event was accepted");
    assert!(
        connection
            .execute(
                "UPDATE episodic_summaries SET body = 'changed' WHERE summary_id = ?1",
                [summary]
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "DELETE FROM episodic_summary_sources WHERE summary_id = ?1",
                [summary]
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "DELETE FROM episodic_summaries WHERE summary_id = ?1",
                [summary]
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "UPDATE episodic_summary_sources SET event_type = 'changed' WHERE summary_id = ?1",
                [summary]
            )
            .is_err()
    );
}

#[test]
fn v4_memory_proposal_resolution_and_current_status_lifecycle_is_exact() {
    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_v4_memory_parents(connection);
    for (sequence, event_id, event_type, event_digest) in [
        (3_i64, "event-3", "memory.proposal.created", "c".repeat(64)),
        (4_i64, "event-4", "memory.proposal.resolved", "d".repeat(64)),
        (5_i64, "event-5", "memory.proposal.created", "g".repeat(64)),
        (6_i64, "event-6", "memory.proposal.resolved", "h".repeat(64)),
    ] {
        connection.execute(
            "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES (?1, ?2, 1, ?3, 'system', ?1, ?4, '{}', ?5)",
            rusqlite::params![sequence, event_id, event_type, format!("corr-{sequence}"), event_digest],
        ).unwrap();
    }
    let profile = "00000000-0000-0000-0000-000000000001";
    let namespace = "00000000-0000-0000-0000-000000000003";
    let proposal = "00000000-0000-0000-0000-000000000041";
    let approval = "00000000-0000-0000-0000-000000000042";
    let digest = "a".repeat(64);
    connection.execute("INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, actor_id, status, created_at_ms) VALUES (?1, 'memory_mutation', 'memory_proposal', ?2, 1, ?3, 'agent', ?4, 'pending', 1)", rusqlite::params![approval, proposal, digest, profile]).unwrap();
    connection.execute("INSERT INTO memory_proposals (proposal_id, version, proposer_profile_id, proposer_profile_version_id, proposer_profile_version, proposer_profile_digest, memory_namespace_id, operation, display_key, normalized_key, expected_kind, expected_entry_id, expected_entry_version_id, expected_entry_version, expected_entry_digest, candidate_value, candidate_value_bytes, candidate_purpose_tags_json, rationale, plaintext_validation_version, created_at_ms, creation_event_sequence, creation_event_id, approval_id, content_digest, record_digest, record_json) VALUES (?1, 1, ?2, '00000000-0000-0000-0000-000000000002', 1, ?3, ?4, 'set', 'Key', 'key', 'absent', NULL, NULL, NULL, NULL, 'value', 5, CAST('[]' AS BLOB), 'because', 1, 1, 1, 'event-1', ?5, ?3, ?3, CAST('{}' AS BLOB))", rusqlite::params![proposal, profile, digest, namespace, approval]).unwrap();
    connection.execute("INSERT INTO current_memory_proposal_status (proposal_id, proposal_version, proposal_content_digest, memory_namespace_id, normalized_key, status, resolution_event_id, created_at_ms) VALUES (?1, 1, ?2, ?3, 'key', 'pending', NULL, 1)", rusqlite::params![proposal, digest, namespace]).unwrap();
    assert!(connection.execute("INSERT INTO current_memory_proposal_status (proposal_id, proposal_version, proposal_content_digest, memory_namespace_id, normalized_key, status, resolution_event_id, created_at_ms) VALUES ('00000000-0000-0000-0000-000000000099', 1, ?1, ?2, 'key', 'pending', NULL, 1)", rusqlite::params![digest, namespace]).is_err());
    connection.execute("UPDATE approval_records SET status = 'accepted', resolved_at_ms = 2, resolution_kind = 'accepted', resolution_event_id = 'event-2', resolution_actor_kind = 'human', resolution_actor_id = NULL WHERE approval_id = ?1", [approval]).unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    transaction.execute("INSERT INTO memory_proposal_resolutions (proposal_id, proposal_version, proposal_content_digest, status, approval_id, resolved_by_kind, resolved_by_id, resolved_at_ms, resolution_event_sequence, resolution_event_id, resolution_json) VALUES (?1, 1, ?2, 'accepted', ?3, 'human', NULL, 2, 2, 'event-2', CAST('{}' AS BLOB))", rusqlite::params![proposal, digest, approval]).unwrap();
    transaction
        .execute(
            "DELETE FROM current_memory_proposal_status WHERE proposal_id = ?1",
            [proposal],
        )
        .unwrap();
    transaction.execute("INSERT INTO current_memory_proposal_status (proposal_id, proposal_version, proposal_content_digest, memory_namespace_id, normalized_key, status, resolution_event_id, created_at_ms) VALUES (?1, 1, ?2, ?3, 'key', 'accepted', 'event-2', 1)", rusqlite::params![proposal, digest, namespace]).unwrap();
    transaction.commit().unwrap();
    assert!(connection.execute("INSERT INTO memory_proposal_resolutions (proposal_id, proposal_version, proposal_content_digest, status, approval_id, resolved_by_kind, resolved_by_id, resolved_at_ms, resolution_event_sequence, resolution_event_id, resolution_json) VALUES (?1, 1, ?2, 'accepted', ?3, 'human', NULL, 2, 2, 'event-2', CAST('{}' AS BLOB))", rusqlite::params![proposal, digest, approval]).is_err());

    let mismatched_resolution_proposal = "00000000-0000-0000-0000-000000000051";
    let mismatched_resolution_approval = "00000000-0000-0000-0000-000000000052";
    seed_pending_memory_proposal(
        connection,
        mismatched_resolution_proposal,
        mismatched_resolution_approval,
        "resolution-mismatch",
        3,
        "event-3",
        &digest,
    );
    connection.execute(
        "UPDATE approval_records SET status = 'accepted', resolved_at_ms = 4, resolution_kind = 'accepted', resolution_event_id = 'event-4', resolution_actor_kind = 'human', resolution_actor_id = NULL WHERE approval_id = ?1",
        [mismatched_resolution_approval],
    ).unwrap();
    let pending_status_before =
        current_memory_proposal_status_values(connection, mismatched_resolution_proposal);
    let transaction = connection.unchecked_transaction().unwrap();
    transaction.execute(
        "INSERT INTO memory_proposal_resolutions (proposal_id, proposal_version, proposal_content_digest, status, approval_id, resolved_by_kind, resolved_by_id, resolved_at_ms, resolution_event_sequence, resolution_event_id, resolution_json) VALUES (?1, 1, ?2, 'accepted', ?3, 'human', NULL, 4, 4, 'event-4', CAST('{}' AS BLOB))",
        rusqlite::params![mismatched_resolution_proposal, "b".repeat(64), mismatched_resolution_approval],
    ).unwrap();
    assert_single_foreign_key_violation(
        &transaction,
        "memory_proposal_resolutions",
        "memory_proposals",
    );
    assert_foreign_key_commit_failure(transaction.commit());
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM memory_proposal_resolutions WHERE proposal_id = ?1",
                [mismatched_resolution_proposal],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        current_memory_proposal_status_values(connection, mismatched_resolution_proposal),
        pending_status_before
    );

    let mismatched_status_proposal = "00000000-0000-0000-0000-000000000061";
    let mismatched_status_approval = "00000000-0000-0000-0000-000000000062";
    seed_pending_memory_proposal(
        connection,
        mismatched_status_proposal,
        mismatched_status_approval,
        "status-mismatch",
        5,
        "event-5",
        &digest,
    );
    connection.execute(
        "UPDATE approval_records SET status = 'accepted', resolved_at_ms = 6, resolution_kind = 'accepted', resolution_event_id = 'event-6', resolution_actor_kind = 'human', resolution_actor_id = NULL WHERE approval_id = ?1",
        [mismatched_status_approval],
    ).unwrap();
    let transaction = connection.unchecked_transaction().unwrap();
    transaction.execute(
        "INSERT INTO memory_proposal_resolutions (proposal_id, proposal_version, proposal_content_digest, status, approval_id, resolved_by_kind, resolved_by_id, resolved_at_ms, resolution_event_sequence, resolution_event_id, resolution_json) VALUES (?1, 1, ?2, 'accepted', ?3, 'human', NULL, 6, 6, 'event-6', CAST('{}' AS BLOB))",
        rusqlite::params![mismatched_status_proposal, digest, mismatched_status_approval],
    ).unwrap();
    transaction
        .execute(
            "DELETE FROM current_memory_proposal_status WHERE proposal_id = ?1",
            [mismatched_status_proposal],
        )
        .unwrap();
    transaction.execute(
        "INSERT INTO current_memory_proposal_status (proposal_id, proposal_version, proposal_content_digest, memory_namespace_id, normalized_key, status, resolution_event_id, created_at_ms) VALUES (?1, 1, ?2, ?3, 'status-mismatch', 'accepted', 'event-6', 5)",
        rusqlite::params![mismatched_status_proposal, digest, namespace],
    ).unwrap();
    transaction.commit().unwrap();
    let canonical_status_before =
        current_memory_proposal_status_values(connection, mismatched_status_proposal);
    let transaction = connection.unchecked_transaction().unwrap();
    transaction
        .execute(
            "DELETE FROM current_memory_proposal_status WHERE proposal_id = ?1",
            [mismatched_status_proposal],
        )
        .unwrap();
    transaction.execute(
        "INSERT INTO current_memory_proposal_status (proposal_id, proposal_version, proposal_content_digest, memory_namespace_id, normalized_key, status, resolution_event_id, created_at_ms) VALUES (?1, 1, ?2, ?3, 'status-mismatch', 'rejected', 'event-6', 5)",
        rusqlite::params![mismatched_status_proposal, digest, namespace],
    ).unwrap();
    assert_single_foreign_key_violation(
        &transaction,
        "current_memory_proposal_status",
        "memory_proposal_resolutions",
    );
    assert_foreign_key_commit_failure(transaction.commit());
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM memory_proposal_resolutions WHERE proposal_id = ?1",
                [mismatched_status_proposal],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        current_memory_proposal_status_values(connection, mismatched_status_proposal),
        canonical_status_before
    );
    for table in ["memory_proposals", "memory_proposal_resolutions"] {
        assert!(
            connection
                .execute(&format!("UPDATE {table} SET proposal_id = proposal_id"), [])
                .is_err(),
            "{table} update was accepted"
        );
        assert!(
            connection
                .execute(&format!("DELETE FROM {table}"), [])
                .is_err(),
            "{table} delete was accepted"
        );
    }
}

fn seed_v4_memory_parents(connection: &rusqlite::Connection) {
    connection.execute_batch(&format!(
        "INSERT INTO event_stream (sequence, event_id, event_schema_version, event_type, actor_kind, occurred_at_ms, correlation_id, payload_json, event_digest) VALUES
             (1, 'event-1', 1, 'legacy.created', 'system', 1, 'corr-1', '{{}}', '{}'),
             (2, 'event-2', 1, 'summary.created', 'system', 2, 'corr-2', '{{}}', '{}');
         INSERT INTO agent_profile_versions (profile_id, profile_version_id, version, supersedes_version_id, template_id, template_version, template_digest, role, display_name, normalized_name, memory_namespace_id, policy_profile_ref, content_digest, payload_json, source_event_sequence, created_at_ms) VALUES
             ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 1, NULL, NULL, NULL, NULL, 'custom', 'Profile', 'profile', '00000000-0000-0000-0000-000000000003', 'policy', '{}', CAST('{{}}' AS BLOB), 1, 1);",
        "e".repeat(64), "f".repeat(64), "a".repeat(64)
    )).unwrap();
}

fn seed_pending_memory_proposal(
    connection: &rusqlite::Connection,
    proposal_id: &str,
    approval_id: &str,
    normalized_key: &str,
    creation_event_sequence: i64,
    creation_event_id: &str,
    digest: &str,
) {
    let profile_id = "00000000-0000-0000-0000-000000000001";
    let namespace_id = "00000000-0000-0000-0000-000000000003";
    connection.execute(
        "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, actor_id, status, created_at_ms) VALUES (?1, 'memory_mutation', 'memory_proposal', ?2, 1, ?3, 'agent', ?4, 'pending', ?5)",
        rusqlite::params![approval_id, proposal_id, digest, profile_id, creation_event_sequence],
    ).unwrap();
    connection.execute(
        "INSERT INTO memory_proposals (proposal_id, version, proposer_profile_id, proposer_profile_version_id, proposer_profile_version, proposer_profile_digest, memory_namespace_id, operation, display_key, normalized_key, expected_kind, expected_entry_id, expected_entry_version_id, expected_entry_version, expected_entry_digest, candidate_value, candidate_value_bytes, candidate_purpose_tags_json, rationale, plaintext_validation_version, created_at_ms, creation_event_sequence, creation_event_id, approval_id, content_digest, record_digest, record_json) VALUES (?1, 1, ?2, '00000000-0000-0000-0000-000000000002', 1, ?3, ?4, 'set', ?5, ?5, 'absent', NULL, NULL, NULL, NULL, 'value', 5, CAST('[]' AS BLOB), 'because', 1, ?6, ?6, ?7, ?8, ?3, ?3, CAST('{}' AS BLOB))",
        rusqlite::params![proposal_id, profile_id, digest, namespace_id, normalized_key, creation_event_sequence, creation_event_id, approval_id],
    ).unwrap();
    connection.execute(
        "INSERT INTO current_memory_proposal_status (proposal_id, proposal_version, proposal_content_digest, memory_namespace_id, normalized_key, status, resolution_event_id, created_at_ms) VALUES (?1, 1, ?2, ?3, ?4, 'pending', NULL, ?5)",
        rusqlite::params![proposal_id, digest, namespace_id, normalized_key, creation_event_sequence],
    ).unwrap();
}

fn current_memory_proposal_status_values(
    connection: &rusqlite::Connection,
    proposal_id: &str,
) -> Vec<rusqlite::types::Value> {
    connection.query_row(
        "SELECT proposal_id, proposal_version, proposal_content_digest, memory_namespace_id, normalized_key, status, resolution_event_id, created_at_ms FROM current_memory_proposal_status WHERE proposal_id = ?1",
        [proposal_id],
        |row| (0..8).map(|index| row.get(index)).collect(),
    ).unwrap()
}

fn assert_single_foreign_key_violation(
    connection: &rusqlite::Connection,
    table: &str,
    expected_parent: &str,
) {
    let mut statement = connection
        .prepare(&format!("PRAGMA foreign_key_check({table})"))
        .unwrap();
    let violations = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(violations.len(), 1, "unexpected violations: {violations:?}");
    assert_eq!(violations[0].0, table);
    assert_eq!(violations[0].2, expected_parent);
}

fn assert_foreign_key_commit_failure(result: rusqlite::Result<()>) {
    let error = result.unwrap_err();
    assert!(
        matches!(
            &error,
            rusqlite::Error::SqliteFailure(sqlite, _)
                if sqlite.code == rusqlite::ErrorCode::ConstraintViolation
                    && sqlite.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
        ),
        "expected deferred foreign-key failure at commit, got {error:?}"
    );
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
        format!("{}\0tail", "a".repeat(64)),
        format!("{}\0", "a".repeat(63)),
        "A".repeat(64),
        "g".repeat(64),
        "é".repeat(32),
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

    let (_temp, database) = fresh_database();
    assert!(database
        .connection()
        .execute(
            "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('blob-command', zeroblob(64), '{}', 'help_read', 'granted', '{}')",
            [],
        )
        .is_err());
    let (_temp, database) = fresh_database();
    assert!(database
        .connection()
        .execute(
            "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('valid-command', ?1, '{}', 'help_read', 'granted', '{}')",
            ["a".repeat(64)],
        )
        .is_ok());

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
    assert_ne!(
        normalize_sql("SELECT 'Append-Only'"),
        normalize_sql("select 'append-only'")
    );
    assert_eq!(
        normalize_sql(
            "CREATE TRIGGER sample BEFORE UPDATE ON item BEGIN SELECT RAISE(ABORT, 'Append-Only'); END"
        ),
        normalize_sql(
            "create trigger SAMPLE before update on ITEM begin select raise ( abort , 'Append-Only' ) ; end"
        ),
    );
    assert_eq!(
        normalize_sql("SELECT 'it''s safe'"),
        normalize_sql("select 'it''s safe'")
    );
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
            "SELECT type, name FROM sqlite_schema WHERE type IN ('table', 'index', 'trigger')
             AND name NOT LIKE 'sqlite_%' AND name NOT IN (
                 'agent_profile_versions_memory_ref_idx', 'event_stream_memory_source_ref_idx',
                 'memory_entry_versions', 'current_memory_entries', 'memory_proposals',
                 'memory_proposal_resolutions', 'current_memory_proposal_status',
                 'episodic_summaries', 'episodic_summary_sources',
                 'memory_entry_versions_history_idx', 'current_memory_entries_list_idx',
                 'memory_proposals_pending_order_idx', 'memory_proposal_resolutions_event_idx',
                 'current_memory_proposals_pending_idx', 'current_memory_proposals_all_idx',
                 'episodic_summaries_list_idx', 'episodic_summary_sources_event_idx',
                 'approval_records_identity_guard', 'approval_records_requester_insert_guard',
                 'approval_records_pending_insert_guard', 'approval_records_memory_insert_guard',
                 'approval_records_transition_guard', 'approval_records_terminal_insert_guard',
                 'approval_records_no_delete', 'memory_entry_versions_identity_guard',
                 'memory_entry_versions_predecessor_guard', 'memory_entry_versions_no_update',
                 'memory_entry_versions_no_delete', 'memory_proposals_no_update',
                 'memory_proposals_no_delete', 'memory_proposal_resolutions_no_update',
                 'memory_proposal_resolutions_no_delete', 'episodic_summary_sources_order_guard',
                 'episodic_summaries_no_update', 'episodic_summaries_no_delete',
                 'episodic_summary_sources_no_update', 'episodic_summary_sources_no_delete'
             ) ORDER BY type, name",
        )
        .unwrap();
    let objects = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(
        objects,
        vec![
            (
                "index".to_owned(),
                "active_agent_profiles_normalized_name_idx".to_owned()
            ),
            (
                "index".to_owned(),
                "active_skills_normalized_name_idx".to_owned()
            ),
            (
                "index".to_owned(),
                "agent_profile_versions_history_idx".to_owned()
            ),
            ("index".to_owned(), "approval_records_status_idx".to_owned()),
            (
                "index".to_owned(),
                "command_event_refs_event_idx".to_owned()
            ),
            (
                "index".to_owned(),
                "event_stream_correlation_idx".to_owned()
            ),
            ("index".to_owned(), "event_stream_type_idx".to_owned()),
            ("index".to_owned(), "setup_drafts_state_idx".to_owned()),
            ("index".to_owned(), "skill_versions_history_idx".to_owned()),
            ("table".to_owned(), "active_agent_profiles".to_owned()),
            (
                "table".to_owned(),
                "active_installation_configuration".to_owned()
            ),
            ("table".to_owned(), "active_skills".to_owned()),
            ("table".to_owned(), "agent_profile_versions".to_owned()),
            ("table".to_owned(), "approval_records".to_owned()),
            ("table".to_owned(), "capability_readiness".to_owned()),
            ("table".to_owned(), "command_event_refs".to_owned()),
            ("table".to_owned(), "command_receipts".to_owned()),
            ("table".to_owned(), "event_stream".to_owned()),
            (
                "table".to_owned(),
                "installation_configuration_versions".to_owned()
            ),
            ("table".to_owned(), "installation_projection".to_owned()),
            ("table".to_owned(), "process_session_projection".to_owned()),
            ("table".to_owned(), "projection_metadata".to_owned()),
            ("table".to_owned(), "schema_migrations".to_owned()),
            ("table".to_owned(), "setup_drafts".to_owned()),
            ("table".to_owned(), "setup_step_outcomes".to_owned()),
            ("table".to_owned(), "skill_versions".to_owned()),
            (
                "trigger".to_owned(),
                "agent_profile_namespace_insert_guard".to_owned()
            ),
            (
                "trigger".to_owned(),
                "agent_profile_versions_no_delete".to_owned()
            ),
            (
                "trigger".to_owned(),
                "agent_profile_versions_no_update".to_owned()
            ),
            (
                "trigger".to_owned(),
                "command_event_refs_no_delete".to_owned()
            ),
            (
                "trigger".to_owned(),
                "command_event_refs_no_update".to_owned()
            ),
            (
                "trigger".to_owned(),
                "command_receipts_no_delete".to_owned()
            ),
            (
                "trigger".to_owned(),
                "command_receipts_no_update".to_owned()
            ),
            ("trigger".to_owned(), "event_stream_no_delete".to_owned()),
            ("trigger".to_owned(), "event_stream_no_update".to_owned()),
            (
                "trigger".to_owned(),
                "installation_configuration_versions_no_delete".to_owned()
            ),
            (
                "trigger".to_owned(),
                "installation_configuration_versions_no_update".to_owned()
            ),
            ("trigger".to_owned(), "skill_versions_no_delete".to_owned()),
            ("trigger".to_owned(), "skill_versions_no_update".to_owned()),
            (
                "trigger".to_owned(),
                "skill_versions_predecessor_guard".to_owned()
            ),
        ]
    );

    let expected_columns: &[(&str, &[ExpectedColumn])] = &[
        (
            "schema_migrations",
            &[
                column("version", "INTEGER", false, 1),
                column("checksum", "TEXT", true, 0),
            ],
        ),
        (
            "agent_profile_versions",
            &[
                column("profile_id", "TEXT", true, 1),
                column("profile_version_id", "TEXT", true, 0),
                column("version", "INTEGER", true, 2),
                column("supersedes_version_id", "TEXT", false, 0),
                column("template_id", "TEXT", false, 0),
                column("template_version", "INTEGER", false, 0),
                column("template_digest", "TEXT", false, 0),
                column("role", "TEXT", true, 0),
                column("display_name", "TEXT", true, 0),
                column("normalized_name", "TEXT", true, 0),
                column("memory_namespace_id", "TEXT", true, 0),
                column("policy_profile_ref", "TEXT", true, 0),
                column("content_digest", "TEXT", true, 0),
                column("payload_json", "BLOB", true, 0),
                column("source_event_sequence", "INTEGER", true, 0),
                column("created_at_ms", "INTEGER", true, 0),
            ],
        ),
        (
            "active_agent_profiles",
            &[
                column("profile_id", "TEXT", true, 1),
                column("profile_version_id", "TEXT", true, 0),
                column("version", "INTEGER", true, 0),
                column("normalized_name", "TEXT", true, 0),
                column("content_digest", "TEXT", true, 0),
            ],
        ),
        (
            "event_stream",
            &[
                column("sequence", "INTEGER", false, 1),
                column("event_id", "TEXT", true, 0),
                column("event_schema_version", "INTEGER", true, 0),
                column("event_type", "TEXT", true, 0),
                column("actor_kind", "TEXT", true, 0),
                column("actor_id", "TEXT", false, 0),
                column("occurred_at_ms", "INTEGER", true, 0),
                column("correlation_id", "TEXT", true, 0),
                column("causation_id", "TEXT", false, 0),
                column("object_kind", "TEXT", false, 0),
                column("object_id", "TEXT", false, 0),
                column("object_version", "INTEGER", false, 0),
                column("object_digest", "TEXT", false, 0),
                column("previous_event_digest", "TEXT", false, 0),
                column("payload_json", "TEXT", true, 0),
                column("event_digest", "TEXT", true, 0),
            ],
        ),
        (
            "command_receipts",
            &[
                column("command_id", "TEXT", true, 1),
                column("command_fingerprint", "TEXT", true, 0),
                column("request_json", "TEXT", true, 0),
                column("capability", "TEXT", true, 0),
                column("policy_decision", "TEXT", true, 0),
                column("outcome_json", "TEXT", true, 0),
            ],
        ),
        (
            "command_event_refs",
            &[
                column("command_id", "TEXT", true, 1),
                column("event_ordinal", "INTEGER", true, 2),
                column("event_id", "TEXT", true, 0),
            ],
        ),
        (
            "installation_projection",
            &[
                column("singleton", "INTEGER", false, 1),
                column("installation_id", "TEXT", true, 0),
                column("created_event_id", "TEXT", true, 0),
                column("created_at_ms", "INTEGER", true, 0),
            ],
        ),
        (
            "process_session_projection",
            &[
                column("session_id", "TEXT", true, 1),
                column("started_event_id", "TEXT", true, 0),
                column("started_at_ms", "INTEGER", true, 0),
                column("ended_event_id", "TEXT", false, 0),
                column("ended_at_ms", "INTEGER", false, 0),
                column("end_reason", "TEXT", false, 0),
            ],
        ),
        (
            "projection_metadata",
            &[
                column("singleton", "INTEGER", false, 1),
                column("last_event_sequence", "INTEGER", true, 0),
                column("last_event_digest", "TEXT", false, 0),
                column("projection_digest", "TEXT", true, 0),
            ],
        ),
        (
            "setup_drafts",
            &[
                column("draft_id", "TEXT", true, 1),
                column("schema_version", "INTEGER", true, 0),
                column("state", "TEXT", true, 0),
                column("path", "TEXT", true, 0),
                column("current_review_digest", "TEXT", false, 0),
                column("payload_json", "TEXT", true, 0),
                column("created_at_ms", "INTEGER", true, 0),
                column("updated_at_ms", "INTEGER", true, 0),
            ],
        ),
        (
            "installation_configuration_versions",
            &[
                column("configuration_id", "TEXT", true, 1),
                column("version", "INTEGER", true, 0),
                column("source_draft_id", "TEXT", true, 0),
                column("review_digest", "TEXT", true, 0),
                column("object_digest", "TEXT", true, 0),
                column("payload_json", "TEXT", true, 0),
                column("created_event_id", "TEXT", true, 0),
                column("created_at_ms", "INTEGER", true, 0),
            ],
        ),
        (
            "active_installation_configuration",
            &[
                column("singleton", "INTEGER", false, 1),
                column("configuration_id", "TEXT", true, 0),
                column("activated_event_id", "TEXT", true, 0),
                column("activated_at_ms", "INTEGER", true, 0),
            ],
        ),
        (
            "setup_step_outcomes",
            &[
                column("draft_id", "TEXT", true, 1),
                column("step_key", "TEXT", true, 2),
                column("attempt", "INTEGER", true, 3),
                column("status", "TEXT", true, 0),
                column("safe_code", "TEXT", false, 0),
                column("occurred_at_ms", "INTEGER", true, 0),
            ],
        ),
        (
            "capability_readiness",
            &[
                column("configuration_id", "TEXT", true, 1),
                column("capability", "TEXT", true, 2),
                column("status", "TEXT", true, 0),
                column("reason_code", "TEXT", false, 0),
                column("checked_at_ms", "INTEGER", true, 0),
                column("projection_digest", "TEXT", true, 0),
            ],
        ),
        (
            "approval_records",
            &[
                column("approval_id", "TEXT", true, 1),
                column("action_kind", "TEXT", true, 0),
                column("object_kind", "TEXT", true, 0),
                column("object_id", "TEXT", true, 0),
                column("object_version", "INTEGER", true, 0),
                column("object_digest", "TEXT", true, 0),
                column("actor_kind", "TEXT", true, 0),
                column("actor_id", "TEXT", false, 0),
                column("status", "TEXT", true, 0),
                column("created_at_ms", "INTEGER", true, 0),
                column("expires_at_ms", "INTEGER", false, 0),
                column("resolved_at_ms", "INTEGER", false, 0),
                column("resolution_kind", "TEXT", false, 0),
                column("resolution_event_id", "TEXT", false, 0),
                column("resolution_actor_kind", "TEXT", false, 0),
                column("resolution_actor_id", "TEXT", false, 0),
            ],
        ),
    ];
    for (table, expected) in expected_columns {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_xinfo({table})"))
            .unwrap();
        let actual = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        let expected = expected
            .iter()
            .map(|column| {
                (
                    column.name.to_owned(),
                    column.declared_type.to_owned(),
                    column.not_null,
                    column.default.map(str::to_owned),
                    column.primary_key_position,
                    0_i64,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "column contract for {table}");
        let sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [*table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            matches!(normalize_sql(&sql).last(), Some(token) if token == "strict"),
            "{table} must remain STRICT"
        );
    }

    let mut foreign_keys = Vec::new();
    for table in expected_columns.iter().map(|(table, _)| *table) {
        let mut statement = connection
            .prepare(&format!("PRAGMA foreign_key_list({table})"))
            .unwrap();
        foreign_keys.extend(
            statement
                .query_map([], |row| {
                    Ok((
                        table.to_owned(),
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .unwrap()
                .map(Result::unwrap),
        );
    }
    foreign_keys.sort();
    let mut expected_foreign_keys = vec![
        (
            "active_agent_profiles",
            "content_digest",
            "agent_profile_versions",
            "content_digest",
        ),
        (
            "active_agent_profiles",
            "profile_id",
            "agent_profile_versions",
            "profile_id",
        ),
        (
            "active_agent_profiles",
            "profile_version_id",
            "agent_profile_versions",
            "profile_version_id",
        ),
        (
            "active_installation_configuration",
            "activated_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "active_installation_configuration",
            "configuration_id",
            "installation_configuration_versions",
            "configuration_id",
        ),
        (
            "agent_profile_versions",
            "profile_id",
            "agent_profile_versions",
            "profile_id",
        ),
        (
            "agent_profile_versions",
            "supersedes_version_id",
            "agent_profile_versions",
            "profile_version_id",
        ),
        (
            "approval_records",
            "resolution_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "capability_readiness",
            "configuration_id",
            "installation_configuration_versions",
            "configuration_id",
        ),
        (
            "command_event_refs",
            "command_id",
            "command_receipts",
            "command_id",
        ),
        ("command_event_refs", "event_id", "event_stream", "event_id"),
        (
            "installation_configuration_versions",
            "created_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "installation_configuration_versions",
            "source_draft_id",
            "setup_drafts",
            "draft_id",
        ),
        (
            "installation_projection",
            "created_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "process_session_projection",
            "ended_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "process_session_projection",
            "started_event_id",
            "event_stream",
            "event_id",
        ),
        (
            "setup_step_outcomes",
            "draft_id",
            "setup_drafts",
            "draft_id",
        ),
    ]
    .into_iter()
    .map(|(table, from, parent, to)| {
        (
            table.to_owned(),
            from.to_owned(),
            parent.to_owned(),
            to.to_owned(),
            "NO ACTION".to_owned(),
            "NO ACTION".to_owned(),
        )
    })
    .collect::<Vec<_>>();
    expected_foreign_keys.sort();
    assert_eq!(foreign_keys, expected_foreign_keys);

    let application_tables = expected_columns
        .iter()
        .map(|(table, _)| *table)
        .collect::<Vec<_>>();
    assert_semantic_index_set(
        connection,
        &application_tables,
        &[
            semantic_index(
                "agent_profile_versions",
                "c",
                false,
                false,
                &["profile_id", "version"],
            ),
            semantic_index(
                "agent_profile_versions",
                "pk",
                true,
                false,
                &["profile_id", "version"],
            ),
            semantic_index(
                "agent_profile_versions",
                "u",
                true,
                false,
                &["profile_version_id"],
            ),
            semantic_index(
                "agent_profile_versions",
                "u",
                true,
                false,
                &["profile_id", "profile_version_id"],
            ),
            semantic_index(
                "agent_profile_versions",
                "u",
                true,
                false,
                &["profile_id", "profile_version_id", "content_digest"],
            ),
            semantic_index(
                "agent_profile_versions",
                "u",
                true,
                false,
                &["source_event_sequence"],
            ),
            semantic_index(
                "agent_profile_versions",
                "c",
                true,
                false,
                &[
                    "profile_id",
                    "profile_version_id",
                    "version",
                    "content_digest",
                    "memory_namespace_id",
                ],
            ),
            semantic_index(
                "active_agent_profiles",
                "c",
                false,
                false,
                &["normalized_name"],
            ),
            semantic_index("active_agent_profiles", "pk", true, false, &["profile_id"]),
            semantic_index(
                "active_agent_profiles",
                "u",
                true,
                false,
                &["profile_version_id"],
            ),
            semantic_index(
                "active_agent_profiles",
                "u",
                true,
                false,
                &["normalized_name"],
            ),
            semantic_index(
                "event_stream",
                "c",
                false,
                false,
                &["correlation_id", "sequence"],
            ),
            semantic_index(
                "event_stream",
                "c",
                false,
                false,
                &["event_type", "sequence"],
            ),
            semantic_index("event_stream", "u", true, false, &["event_id"]),
            semantic_index("event_stream", "u", true, false, &["event_digest"]),
            semantic_index(
                "event_stream",
                "c",
                true,
                false,
                &["sequence", "event_id", "event_type", "event_digest"],
            ),
            semantic_index("command_receipts", "pk", true, false, &["command_id"]),
            semantic_index(
                "command_event_refs",
                "pk",
                true,
                false,
                &["command_id", "event_ordinal"],
            ),
            semantic_index("command_event_refs", "c", true, false, &["event_id"]),
            semantic_index(
                "installation_projection",
                "u",
                true,
                false,
                &["installation_id"],
            ),
            semantic_index(
                "process_session_projection",
                "pk",
                true,
                false,
                &["session_id"],
            ),
            semantic_index(
                "setup_drafts",
                "c",
                false,
                false,
                &["state", "updated_at_ms"],
            ),
            semantic_index("setup_drafts", "pk", true, false, &["draft_id"]),
            semantic_index(
                "installation_configuration_versions",
                "pk",
                true,
                false,
                &["configuration_id"],
            ),
            semantic_index(
                "installation_configuration_versions",
                "u",
                true,
                false,
                &["source_draft_id", "review_digest"],
            ),
            semantic_index(
                "installation_configuration_versions",
                "u",
                true,
                false,
                &["version"],
            ),
            semantic_index(
                "setup_step_outcomes",
                "pk",
                true,
                false,
                &["draft_id", "step_key", "attempt"],
            ),
            semantic_index(
                "capability_readiness",
                "pk",
                true,
                false,
                &["configuration_id", "capability"],
            ),
            semantic_index(
                "approval_records",
                "c",
                false,
                false,
                &["status", "created_at_ms"],
            ),
            semantic_index("approval_records", "pk", true, false, &["approval_id"]),
        ],
    )
    .unwrap();
    for (name, table, columns) in [
        (
            "agent_profile_versions_history_idx",
            "agent_profile_versions",
            &["profile_id", "version DESC"][..],
        ),
        (
            "active_agent_profiles_normalized_name_idx",
            "active_agent_profiles",
            &["normalized_name"][..],
        ),
        (
            "event_stream_correlation_idx",
            "event_stream",
            &["correlation_id", "sequence"][..],
        ),
        (
            "event_stream_type_idx",
            "event_stream",
            &["event_type", "sequence"][..],
        ),
        (
            "command_event_refs_event_idx",
            "command_event_refs",
            &["event_id"][..],
        ),
        (
            "setup_drafts_state_idx",
            "setup_drafts",
            &["state", "updated_at_ms"][..],
        ),
        (
            "approval_records_status_idx",
            "approval_records",
            &["status", "created_at_ms"][..],
        ),
    ] {
        let sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        let unique = if name == "command_event_refs_event_idx" {
            "unique "
        } else {
            ""
        };
        assert_eq!(
            normalize_sql(&sql),
            normalize_sql(&format!(
                "create {unique}index {name} on {table}({})",
                columns.join(", ")
            ))
        );
    }

    for (name, expected_sql) in [
        (
            "agent_profile_namespace_insert_guard",
            "create trigger agent_profile_namespace_insert_guard before insert on agent_profile_versions when exists (select 1 from agent_profile_versions existing where existing.memory_namespace_id = new.memory_namespace_id and existing.profile_id <> new.profile_id) or exists (select 1 from agent_profile_versions existing where existing.profile_id = new.profile_id and existing.memory_namespace_id <> new.memory_namespace_id) begin select raise(abort, 'agent_profile_namespace_conflict'); end",
        ),
        (
            "agent_profile_versions_no_update",
            "create trigger agent_profile_versions_no_update before update on agent_profile_versions begin select raise(abort, 'agent_profile_versions_immutable'); end",
        ),
        (
            "agent_profile_versions_no_delete",
            "create trigger agent_profile_versions_no_delete before delete on agent_profile_versions begin select raise(abort, 'agent_profile_versions_immutable'); end",
        ),
        (
            "event_stream_no_update",
            "create trigger event_stream_no_update before update on event_stream begin select raise(abort, 'event_stream is append-only'); end",
        ),
        (
            "event_stream_no_delete",
            "create trigger event_stream_no_delete before delete on event_stream begin select raise(abort, 'event_stream is append-only'); end",
        ),
        (
            "command_receipts_no_update",
            "create trigger command_receipts_no_update before update on command_receipts begin select raise(abort, 'command receipts are immutable'); end",
        ),
        (
            "command_receipts_no_delete",
            "create trigger command_receipts_no_delete before delete on command_receipts begin select raise(abort, 'command receipts are immutable'); end",
        ),
        (
            "command_event_refs_no_update",
            "create trigger command_event_refs_no_update before update on command_event_refs begin select raise(abort, 'command event refs are immutable'); end",
        ),
        (
            "command_event_refs_no_delete",
            "create trigger command_event_refs_no_delete before delete on command_event_refs begin select raise(abort, 'command event refs are immutable'); end",
        ),
        (
            "installation_configuration_versions_no_update",
            "create trigger installation_configuration_versions_no_update before update on installation_configuration_versions begin select raise(abort, 'installation configuration versions are immutable'); end",
        ),
        (
            "installation_configuration_versions_no_delete",
            "create trigger installation_configuration_versions_no_delete before delete on installation_configuration_versions begin select raise(abort, 'installation configuration versions are immutable'); end",
        ),
    ] {
        let sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            normalize_sql(&sql),
            normalize_sql(expected_sql),
            "trigger definition for {name}"
        );
    }

    assert_complete_v4_memory_schema(connection);
}

fn assert_complete_v4_memory_schema(connection: &rusqlite::Connection) {
    assert_v4_sql_definitions(connection);
    let expected_columns: &[(&str, &[&str])] = &[
        (
            "memory_entry_versions",
            &[
                "memory_namespace_id TEXT",
                "entry_id TEXT",
                "entry_version_id TEXT",
                "version INTEGER",
                "predecessor_version_id TEXT",
                "display_key TEXT",
                "normalized_key TEXT",
                "state TEXT",
                "value_text TEXT",
                "value_bytes INTEGER",
                "purpose_tags_json BLOB",
                "created_by_kind TEXT",
                "created_by_id TEXT",
                "created_at_ms INTEGER",
                "accepted_proposal_id TEXT",
                "accepted_proposal_version INTEGER",
                "accepted_proposal_digest TEXT",
                "plaintext_validation_version INTEGER",
                "creation_event_sequence INTEGER",
                "creation_event_id TEXT",
                "content_digest TEXT",
                "record_digest TEXT",
                "record_json BLOB",
            ],
        ),
        (
            "current_memory_entries",
            &[
                "memory_namespace_id TEXT",
                "normalized_key TEXT",
                "entry_id TEXT",
                "entry_version_id TEXT",
                "version INTEGER",
                "state TEXT",
                "content_digest TEXT",
            ],
        ),
        (
            "memory_proposals",
            &[
                "proposal_id TEXT",
                "version INTEGER",
                "proposer_profile_id TEXT",
                "proposer_profile_version_id TEXT",
                "proposer_profile_version INTEGER",
                "proposer_profile_digest TEXT",
                "memory_namespace_id TEXT",
                "operation TEXT",
                "display_key TEXT",
                "normalized_key TEXT",
                "expected_kind TEXT",
                "expected_entry_id TEXT",
                "expected_entry_version_id TEXT",
                "expected_entry_version INTEGER",
                "expected_entry_digest TEXT",
                "candidate_value TEXT",
                "candidate_value_bytes INTEGER",
                "candidate_purpose_tags_json BLOB",
                "rationale TEXT",
                "plaintext_validation_version INTEGER",
                "created_at_ms INTEGER",
                "creation_event_sequence INTEGER",
                "creation_event_id TEXT",
                "approval_id TEXT",
                "content_digest TEXT",
                "record_digest TEXT",
                "record_json BLOB",
            ],
        ),
        (
            "memory_proposal_resolutions",
            &[
                "proposal_id TEXT",
                "proposal_version INTEGER",
                "proposal_content_digest TEXT",
                "status TEXT",
                "approval_id TEXT",
                "resolved_by_kind TEXT",
                "resolved_by_id TEXT",
                "resolved_at_ms INTEGER",
                "resolution_event_sequence INTEGER",
                "resolution_event_id TEXT",
                "resolution_json BLOB",
            ],
        ),
        (
            "current_memory_proposal_status",
            &[
                "proposal_id TEXT",
                "proposal_version INTEGER",
                "proposal_content_digest TEXT",
                "memory_namespace_id TEXT",
                "normalized_key TEXT",
                "status TEXT",
                "resolution_event_id TEXT",
                "created_at_ms INTEGER",
            ],
        ),
        (
            "episodic_summaries",
            &[
                "summary_id TEXT",
                "version INTEGER",
                "memory_namespace_id TEXT",
                "profile_id TEXT",
                "profile_version_id TEXT",
                "profile_version INTEGER",
                "profile_content_digest TEXT",
                "label TEXT",
                "body TEXT",
                "purpose_tags_json BLOB",
                "source_count INTEGER",
                "plaintext_validation_version INTEGER",
                "created_at_ms INTEGER",
                "creation_event_sequence INTEGER",
                "creation_event_id TEXT",
                "source_set_digest TEXT",
                "content_digest TEXT",
                "record_digest TEXT",
                "record_json BLOB",
            ],
        ),
        (
            "episodic_summary_sources",
            &[
                "summary_id TEXT",
                "source_ordinal INTEGER",
                "event_sequence INTEGER",
                "event_id TEXT",
                "event_type TEXT",
                "event_digest TEXT",
            ],
        ),
    ];
    for (table, expected) in expected_columns {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_xinfo({table})"))
            .unwrap();
        let actual = statement
            .query_map([], |row| {
                Ok(format!(
                    "{} {}",
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(actual, *expected, "exact v4 columns and types for {table}");
        let sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [*table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(sql.ends_with(" STRICT"), "{table} must be STRICT");
    }

    let expected_indexes = [
        (
            "agent_profile_versions_memory_ref_idx",
            "agent_profile_versions",
            "profile_id, profile_version_id, version, content_digest, memory_namespace_id",
        ),
        (
            "event_stream_memory_source_ref_idx",
            "event_stream",
            "sequence, event_id, event_type, event_digest",
        ),
        (
            "memory_entry_versions_history_idx",
            "memory_entry_versions",
            "memory_namespace_id, normalized_key, version DESC",
        ),
        (
            "current_memory_entries_list_idx",
            "current_memory_entries",
            "memory_namespace_id, state, normalized_key, entry_id",
        ),
        (
            "memory_proposals_pending_order_idx",
            "memory_proposals",
            "memory_namespace_id, created_at_ms, proposal_id",
        ),
        (
            "memory_proposal_resolutions_event_idx",
            "memory_proposal_resolutions",
            "resolution_event_id, proposal_id",
        ),
        (
            "current_memory_proposals_pending_idx",
            "current_memory_proposal_status",
            "memory_namespace_id, status, created_at_ms ASC, proposal_id ASC",
        ),
        (
            "current_memory_proposals_all_idx",
            "current_memory_proposal_status",
            "memory_namespace_id, created_at_ms DESC, proposal_id ASC",
        ),
        (
            "episodic_summaries_list_idx",
            "episodic_summaries",
            "memory_namespace_id, created_at_ms DESC, summary_id ASC",
        ),
        (
            "episodic_summary_sources_event_idx",
            "episodic_summary_sources",
            "event_id, summary_id",
        ),
    ];
    for (name, table, columns) in expected_indexes {
        let sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            normalize_sql(&sql),
            normalize_sql(&format!(
                "create {}index {name} on {table} ({columns})",
                if name.ends_with("ref_idx") {
                    "unique "
                } else {
                    ""
                }
            )),
            "index {name}"
        );
    }

    let expected_triggers = [
        ("approval_records_identity_guard", "approval_records"),
        (
            "approval_records_requester_insert_guard",
            "approval_records",
        ),
        ("approval_records_pending_insert_guard", "approval_records"),
        ("approval_records_memory_insert_guard", "approval_records"),
        ("approval_records_transition_guard", "approval_records"),
        ("approval_records_terminal_insert_guard", "approval_records"),
        ("approval_records_no_delete", "approval_records"),
        (
            "memory_entry_versions_identity_guard",
            "memory_entry_versions",
        ),
        (
            "memory_entry_versions_predecessor_guard",
            "memory_entry_versions",
        ),
        ("memory_entry_versions_no_update", "memory_entry_versions"),
        ("memory_entry_versions_no_delete", "memory_entry_versions"),
        ("memory_proposals_no_update", "memory_proposals"),
        ("memory_proposals_no_delete", "memory_proposals"),
        (
            "memory_proposal_resolutions_no_update",
            "memory_proposal_resolutions",
        ),
        (
            "memory_proposal_resolutions_no_delete",
            "memory_proposal_resolutions",
        ),
        (
            "episodic_summary_sources_order_guard",
            "episodic_summary_sources",
        ),
        ("episodic_summaries_no_update", "episodic_summaries"),
        ("episodic_summaries_no_delete", "episodic_summaries"),
        (
            "episodic_summary_sources_no_update",
            "episodic_summary_sources",
        ),
        (
            "episodic_summary_sources_no_delete",
            "episodic_summary_sources",
        ),
    ];
    for (name, table) in expected_triggers {
        let (actual_table, sql): (String, String) = connection
            .query_row(
                "SELECT tbl_name, sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?1",
                [name],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(actual_table, table, "trigger owner for {name}");
        assert!(
            normalize_sql(&sql).contains(&"begin".to_owned()),
            "trigger {name} must have a body"
        );
    }

    for (table, child, parent, parent_columns) in [
        (
            "memory_entry_versions",
            &["entry_id", "predecessor_version_id"][..],
            "memory_entry_versions",
            &["entry_id", "entry_version_id"][..],
        ),
        (
            "memory_entry_versions",
            &[
                "memory_namespace_id",
                "normalized_key",
                "accepted_proposal_id",
                "accepted_proposal_version",
                "accepted_proposal_digest",
            ],
            "memory_proposals",
            &[
                "memory_namespace_id",
                "normalized_key",
                "proposal_id",
                "version",
                "content_digest",
            ],
        ),
        (
            "current_memory_entries",
            &[
                "memory_namespace_id",
                "normalized_key",
                "entry_id",
                "entry_version_id",
                "version",
                "state",
                "content_digest",
            ],
            "memory_entry_versions",
            &[
                "memory_namespace_id",
                "normalized_key",
                "entry_id",
                "entry_version_id",
                "version",
                "state",
                "content_digest",
            ],
        ),
        (
            "memory_proposals",
            &[
                "proposer_profile_id",
                "proposer_profile_version_id",
                "proposer_profile_version",
                "proposer_profile_digest",
                "memory_namespace_id",
            ],
            "agent_profile_versions",
            &[
                "profile_id",
                "profile_version_id",
                "version",
                "content_digest",
                "memory_namespace_id",
            ],
        ),
        (
            "memory_proposals",
            &[
                "memory_namespace_id",
                "normalized_key",
                "expected_entry_id",
                "expected_entry_version_id",
                "expected_entry_version",
                "expected_kind",
                "expected_entry_digest",
            ],
            "memory_entry_versions",
            &[
                "memory_namespace_id",
                "normalized_key",
                "entry_id",
                "entry_version_id",
                "version",
                "state",
                "content_digest",
            ],
        ),
        (
            "memory_proposal_resolutions",
            &[
                "proposal_id",
                "proposal_version",
                "proposal_content_digest",
                "approval_id",
            ],
            "memory_proposals",
            &["proposal_id", "version", "content_digest", "approval_id"],
        ),
        (
            "current_memory_proposal_status",
            &[
                "memory_namespace_id",
                "normalized_key",
                "proposal_id",
                "proposal_version",
                "proposal_content_digest",
            ],
            "memory_proposals",
            &[
                "memory_namespace_id",
                "normalized_key",
                "proposal_id",
                "version",
                "content_digest",
            ],
        ),
        (
            "current_memory_proposal_status",
            &["proposal_id", "status", "resolution_event_id"],
            "memory_proposal_resolutions",
            &["proposal_id", "status", "resolution_event_id"],
        ),
        (
            "episodic_summaries",
            &[
                "profile_id",
                "profile_version_id",
                "profile_version",
                "profile_content_digest",
                "memory_namespace_id",
            ],
            "agent_profile_versions",
            &[
                "profile_id",
                "profile_version_id",
                "version",
                "content_digest",
                "memory_namespace_id",
            ],
        ),
        (
            "episodic_summary_sources",
            &["event_sequence", "event_id", "event_type", "event_digest"],
            "event_stream",
            &["sequence", "event_id", "event_type", "event_digest"],
        ),
    ] {
        assert_composite_foreign_key(connection, table, child, parent, parent_columns);
    }
}

fn assert_v4_sql_definitions(connection: &rusqlite::Connection) {
    for table in [
        "approval_records",
        "command_receipts",
        "command_event_refs",
        "memory_entry_versions",
        "current_memory_entries",
        "memory_proposals",
        "memory_proposal_resolutions",
        "current_memory_proposal_status",
        "episodic_summaries",
        "episodic_summary_sources",
    ] {
        let actual: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            normalize_sql(&actual),
            normalize_sql(&v4_object_sql("CREATE TABLE", table)),
            "exact table definition for {table}"
        );
    }
    for trigger in [
        "approval_records_identity_guard",
        "approval_records_requester_insert_guard",
        "approval_records_pending_insert_guard",
        "approval_records_memory_insert_guard",
        "approval_records_transition_guard",
        "approval_records_terminal_insert_guard",
        "approval_records_no_delete",
        "command_receipts_no_update",
        "command_receipts_no_delete",
        "command_event_refs_no_update",
        "command_event_refs_no_delete",
        "memory_entry_versions_identity_guard",
        "memory_entry_versions_predecessor_guard",
        "memory_entry_versions_no_update",
        "memory_entry_versions_no_delete",
        "memory_proposals_no_update",
        "memory_proposals_no_delete",
        "memory_proposal_resolutions_no_update",
        "memory_proposal_resolutions_no_delete",
        "episodic_summary_sources_order_guard",
        "episodic_summaries_no_update",
        "episodic_summaries_no_delete",
        "episodic_summary_sources_no_update",
        "episodic_summary_sources_no_delete",
    ] {
        let actual: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'trigger' AND name = ?1",
                [trigger],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            normalize_sql(&actual),
            normalize_sql(&v4_object_sql("CREATE TRIGGER", trigger)),
            "exact trigger definition for {trigger}"
        );
    }
}

fn v4_object_sql(kind: &str, name: &str) -> String {
    let migration = include_str!("../migrations/0004_hybrid_memory.sql");
    let needle = format!("{kind} {name}");
    let start = migration
        .find(&needle)
        .unwrap_or_else(|| panic!("missing {needle} in v4 migration"));
    migration[start..]
        .split("\n-- migration-boundary:")
        .next()
        .unwrap()
        .trim_end_matches(';')
        .to_owned()
}

fn assert_composite_foreign_key(
    connection: &rusqlite::Connection,
    table: &str,
    child: &[&str],
    parent: &str,
    parent_columns: &[&str],
) {
    let mut statement = connection
        .prepare(&format!("PRAGMA foreign_key_list({table})"))
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert!(
        rows.iter()
            .any(|(id, _, actual_parent, _, _)| actual_parent == parent
                && child
                    .iter()
                    .enumerate()
                    .all(
                        |(sequence, column)| rows.iter().any(|(candidate, seq, _, from, to)| {
                            candidate == id
                                && *seq == sequence as i64
                                && from == column
                                && to == parent_columns[sequence]
                        })
                    )),
        "missing composite FK {table}({}) -> {parent}({})",
        child.join(", "),
        parent_columns.join(", ")
    );
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
    ] {
        assert_rejected_after(seed_complete, statement);
    }

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
    ] {
        assert_rejected_after(seed_complete, statement);
    }

    assert_rejected_after(
        seed_event,
        "INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (1, 'installation-missing', 'missing-event', 1)",
    );
    assert_rejected_after(
        seed_receipt_without_ref,
        "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('command-1', -1, 'event-1')",
    );
    assert_rejected_after(
        seed_event,
        "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('missing-command', 0, 'event-1')",
    );
    assert_rejected_after(
        seed_receipt_without_ref,
        "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('command-1', 0, 'missing-event')",
    );
    assert_rejected_after(
        seed_receipt_without_ref,
        "INSERT INTO command_receipts (command_id, command_fingerprint, request_json, capability, policy_decision, outcome_json) VALUES ('command-1', 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', '{}', 'status_read', 'denied', '{}')",
    );
    assert_rejected_after(
        seed_two_receipts_one_ref,
        "INSERT INTO command_event_refs (command_id, event_ordinal, event_id) VALUES ('command-2', 0, 'event-1')",
    );
    assert_rejected_after(
        seed_event,
        "INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms) VALUES ('session-missing-start', 'missing-event', 1)",
    );
    assert_rejected_after(
        seed_event,
        "INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms, ended_event_id, ended_at_ms, end_reason) VALUES ('session-missing-end', 'event-1', 1, 'missing-event', 2, 'quit')",
    );
    assert_rejected_after(
        seed_event,
        "INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('config-missing-source', 1, 'missing-draft', 'review', 'object', '{}', 'event-1', 1)",
    );
    assert_rejected_after(
        seed_draft,
        "INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms) VALUES ('config-missing-event', 1, 'draft-1', 'review', 'object', '{}', 'missing-event', 1)",
    );
    assert_rejected_after(
        seed_event,
        "INSERT INTO active_installation_configuration (singleton, configuration_id, activated_event_id, activated_at_ms) VALUES (1, 'missing-configuration', 'event-1', 1)",
    );
    assert_rejected_after(
        seed_configuration,
        "INSERT INTO active_installation_configuration (singleton, configuration_id, activated_event_id, activated_at_ms) VALUES (1, 'configuration-1', 'missing-event', 1)",
    );
    assert_rejected_after(
        seed_event,
        "INSERT INTO setup_step_outcomes (draft_id, step_key, attempt, status, occurred_at_ms) VALUES ('missing-draft', 'step', 1, 'passed', 1)",
    );
    assert_rejected_after(
        seed_event,
        "INSERT INTO capability_readiness (configuration_id, capability, status, checked_at_ms, projection_digest) VALUES ('missing-configuration', 'capability', 'ready', 1, 'projection')",
    );
    assert_rejected_after(
        seed_event,
        "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms, resolved_at_ms, resolution_kind, resolution_event_id) VALUES ('approval-missing-event', 'apply', 'configuration', 'configuration-1', 1, 'object-1', 'system', 'accepted', 1, 2, 'accepted', 'missing-event')",
    );

    assert_rejected_after(
        seed_complete,
        "UPDATE event_stream SET event_type = 'changed' WHERE sequence = 1",
    );
    assert_rejected_after(seed_complete, "DELETE FROM event_stream WHERE sequence = 1");
    assert_rejected_after(
        seed_complete,
        "UPDATE installation_configuration_versions SET review_digest = 'changed' WHERE configuration_id = 'configuration-1'",
    );
    assert_rejected_after(
        seed_complete,
        "DELETE FROM installation_configuration_versions WHERE configuration_id = 'configuration-1'",
    );
    assert_rejected_after(
        seed_receipt,
        "UPDATE command_receipts SET capability = 'status_read' WHERE command_id = 'command-1'",
    );
    assert_rejected_after(
        seed_receipt,
        "DELETE FROM command_receipts WHERE command_id = 'command-1'",
    );
    assert_rejected_after(
        seed_receipt,
        "UPDATE command_event_refs SET event_ordinal = 1 WHERE command_id = 'command-1'",
    );
    assert_rejected_after(
        seed_receipt,
        "DELETE FROM command_event_refs WHERE command_id = 'command-1'",
    );

    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_complete(connection);
    connection
        .pragma_update(None, "ignore_check_constraints", "ON")
        .unwrap();
    assert!(connection.execute("INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms) VALUES (2, 'installation-1', 'event-1', 2)", []).is_err());
    connection
        .pragma_update(None, "ignore_check_constraints", "OFF")
        .unwrap();
}

fn assert_semantic_index_set(
    connection: &rusqlite::Connection,
    tables: &[&str],
    expected: &[SemanticIndex],
) -> Result<(), String> {
    let mut actual = Vec::new();
    for table in tables {
        let mut statement = connection
            .prepare(&format!("PRAGMA index_list({table})"))
            .map_err(|error| error.to_string())?;
        let indexes = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? != 0,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)? != 0,
                ))
            })
            .map_err(|error| error.to_string())?;
        for index in indexes {
            let (name, unique, origin, partial) = index.map_err(|error| error.to_string())?;
            let escaped_name = name.replace('\'', "''");
            let mut columns = connection.prepare(&format!("SELECT name FROM pragma_index_xinfo('{escaped_name}') WHERE key = 1 ORDER BY seqno")).map_err(|error| error.to_string())?;
            let columns = columns
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?
                .map(|column| column.map_err(|error| error.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            actual.push(((*table).to_owned(), origin, unique, partial, columns));
        }
    }
    actual.sort();
    let mut expected = expected
        .iter()
        .map(|index| {
            (
                index.table.to_owned(),
                index.origin.to_owned(),
                index.unique,
                index.partial,
                index
                    .columns
                    .iter()
                    .map(|column| (*column).to_owned())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    expected.sort();
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "semantic index mismatch: actual={actual:?}, expected={expected:?}"
        ))
    }
}

fn assert_every_enumerated_check_value_is_accepted() {
    let (_temp, database) = fresh_database();
    let connection = database.connection();
    for (index, capability) in [
        "help_read",
        "status_read",
        "setup_status_read",
        "audit_read",
        "shutdown",
        "discussion_run",
        "mcp_use",
        "engineering_job_run",
        "git_merge",
        "git_push",
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
    for (index, decision) in [
        "granted",
        "denied",
        "denied_by_default",
        "approval_required",
    ]
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
    for (index, state) in ["drafting", "reviewed", "applied", "superseded"]
        .into_iter()
        .enumerate()
    {
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
        connection.execute("INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms, resolved_at_ms, resolution_kind, resolution_event_id, resolution_actor_kind, resolution_actor_id) VALUES (?1, 'apply', 'configuration', 'configuration-1', 1, 'object', 'system', ?2, 1, 2, 'resolved', 'event-1', 'human', NULL)", rusqlite::params![format!("approval-{status}"), status]).unwrap();
    }
}

#[test]
fn every_typed_approval_status_round_trips_through_real_sqlite() {
    let (_temp, database) = fresh_database();
    let connection = database.connection();
    seed_event(connection);

    for (index, (status, canonical, terminal)) in [
        (ApprovalStatus::Pending, "pending", false),
        (ApprovalStatus::Accepted, "accepted", true),
        (ApprovalStatus::Rejected, "rejected", true),
        (ApprovalStatus::Expired, "expired", true),
        (ApprovalStatus::Cancelled, "cancelled", true),
    ]
    .into_iter()
    .enumerate()
    {
        let wire = serde_json::to_value(status).unwrap();
        assert_eq!(wire, serde_json::Value::String(canonical.to_owned()));
        let status_text = wire.as_str().unwrap();
        let resolved_at_ms = terminal.then_some(2_i64);
        let resolution_kind = terminal.then_some(canonical);
        let resolution_event_id = terminal.then_some("event-1");
        let approval_id = format!("approval-round-trip-{index}");

        connection
            .execute(
                "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms, expires_at_ms, resolved_at_ms, resolution_kind, resolution_event_id, resolution_actor_kind, resolution_actor_id) VALUES (?1, 'git_push', 'git_commit', 'commit-1', 1, 'object-1', 'human', ?2, 1, 3, ?3, ?4, ?5, ?6, NULL)",
                rusqlite::params![
                    approval_id,
                    status_text,
                    resolved_at_ms,
                    resolution_kind,
                    resolution_event_id,
                    terminal.then_some("human")
                ],
            )
            .unwrap();

        let persisted = connection
            .query_row(
                "SELECT status FROM approval_records WHERE approval_id = ?1",
                [approval_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let decoded =
            serde_json::from_value::<ApprovalStatus>(serde_json::Value::String(persisted.clone()))
                .unwrap();

        assert_eq!(persisted, canonical);
        assert_eq!(decoded, status);
        assert_eq!(decoded.is_terminal(), terminal);
    }
}

#[test]
fn every_terminal_approval_status_requires_resolution_metadata_in_sqlite() {
    let (_temp, database) = fresh_database();
    let connection = database.connection();

    for (index, status) in [
        ApprovalStatus::Accepted,
        ApprovalStatus::Rejected,
        ApprovalStatus::Expired,
        ApprovalStatus::Cancelled,
    ]
    .into_iter()
    .enumerate()
    {
        let wire = serde_json::to_value(status).unwrap();
        let status_text = wire.as_str().unwrap();

        assert!(
            connection
                .execute(
                    "INSERT INTO approval_records (approval_id, action_kind, object_kind, object_id, object_version, object_digest, actor_kind, status, created_at_ms) VALUES (?1, 'git_push', 'git_commit', 'commit-1', 1, 'object-1', 'human', ?2, 1)",
                    rusqlite::params![format!("approval-unresolved-{index}"), status_text],
                )
                .is_err(),
            "terminal status {status_text} must require resolution metadata"
        );
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
    assert!(
        connection.execute(statement, []).is_err(),
        "expected rejection: {statement}"
    );
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
        if character.is_whitespace() {
            continue;
        }
        if character == '\'' {
            let mut quoted = String::from("'");
            while let Some(next) = characters.next() {
                quoted.push(next);
                if next == '\'' {
                    if characters.peek() == Some(&'\'') {
                        quoted.push(characters.next().unwrap());
                    } else {
                        break;
                    }
                }
            }
            tokens.push(quoted);
        } else if character.is_ascii_alphanumeric() || character == '_' {
            let mut word = character.to_string();
            while let Some(next) = characters.peek() {
                if next.is_ascii_alphanumeric() || *next == '_' {
                    word.push(characters.next().unwrap());
                } else {
                    break;
                }
            }
            tokens.push(word.to_ascii_lowercase());
        } else {
            tokens.push(character.to_string());
        }
    }
    tokens
}
