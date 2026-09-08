use std::collections::BTreeMap;

use ai_stock_forum::{config::AppPaths, domain::sha256, persistence::Database};
use rusqlite::{Connection, params};

const LEGACY_APPROVAL_ID: &str = "legacy-terminal-approval";

#[test]
fn fresh_database_reaches_v4_and_registers_hybrid_memory_storage() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();

    assert_eq!(database.schema_version(), 4);
    for table in [
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
fn v3_terminal_non_memory_approval_keeps_null_resolution_event() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    create_schema_v3_fixture(&paths);
    let before = legacy_rows(&Connection::open(paths.database_path()).unwrap());

    let database = Database::open(&paths).unwrap();

    assert_eq!(database.schema_version(), 4);
    assert_eq!(legacy_rows(database.connection()), before);
    let (event_id, kind, actor_id): (Option<String>, Option<String>, Option<String>) = database
        .connection()
        .query_row(
            "SELECT resolution_event_id, resolution_actor_kind, resolution_actor_id
             FROM approval_records WHERE approval_id = ?1",
            [LEGACY_APPROVAL_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(event_id, None);
    assert_eq!(kind, None);
    assert_eq!(actor_id, None);
}

#[test]
fn migration_failure_at_every_v4_boundary_rolls_back() {
    let expected_boundaries = vec![
        "drop_approval_records_status_idx",
        "rename_approval_records_v3",
        "create_approval_records",
        "copy_approval_records_v3",
        "drop_approval_records_v3",
        "create_approval_records_status_idx",
        "create_approval_records_identity_guard",
        "create_approval_records_requester_insert_guard",
        "create_approval_records_pending_insert_guard",
        "create_approval_records_memory_insert_guard",
        "create_approval_records_transition_guard",
        "create_approval_records_terminal_insert_guard",
        "create_approval_records_no_delete",
        "drop_command_event_refs_no_update_v4",
        "drop_command_event_refs_no_delete_v4",
        "drop_command_event_refs_event_idx_v4",
        "rename_command_event_refs_v3",
        "drop_command_receipts_no_update_v4",
        "drop_command_receipts_no_delete_v4",
        "rename_command_receipts_v3",
        "create_command_receipts_v4",
        "copy_command_receipts_v3",
        "create_command_receipts_no_update_v4",
        "create_command_receipts_no_delete_v4",
        "create_command_event_refs_v4",
        "copy_command_event_refs_v3",
        "create_command_event_refs_event_idx_v4",
        "create_command_event_refs_no_update_v4",
        "create_command_event_refs_no_delete_v4",
        "drop_command_event_refs_v3",
        "drop_command_receipts_v3",
        "create_agent_profile_versions_memory_ref_idx",
        "create_event_stream_memory_source_ref_idx",
        "create_memory_entry_versions",
        "create_memory_entry_versions_history_idx",
        "create_memory_entry_versions_identity_guard",
        "create_memory_entry_versions_predecessor_guard",
        "create_memory_entry_versions_no_update",
        "create_memory_entry_versions_no_delete",
        "create_current_memory_entries",
        "create_current_memory_entries_list_idx",
        "create_memory_proposals",
        "create_memory_proposals_pending_order_idx",
        "create_memory_proposals_no_update",
        "create_memory_proposals_no_delete",
        "create_memory_proposal_resolutions",
        "create_memory_proposal_resolutions_event_idx",
        "create_memory_proposal_resolutions_no_update",
        "create_memory_proposal_resolutions_no_delete",
        "create_current_memory_proposal_status",
        "create_current_memory_proposals_pending_idx",
        "create_current_memory_proposals_all_idx",
        "create_episodic_summaries",
        "create_episodic_summaries_list_idx",
        "create_episodic_summary_sources",
        "create_episodic_summary_sources_event_idx",
        "create_episodic_summary_sources_order_guard",
        "create_episodic_summaries_no_update",
        "create_episodic_summaries_no_delete",
        "create_episodic_summary_sources_no_update",
        "create_episodic_summary_sources_no_delete",
        "schema_migration_record",
    ];
    assert_eq!(Database::v4_migration_boundaries(), expected_boundaries);

    for boundary in expected_boundaries {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        create_schema_v3_fixture(&paths);
        let before_connection = Connection::open(paths.database_path()).unwrap();
        let before = (
            schema_inventory(&before_connection),
            migration_records(&before_connection),
            legacy_rows(&before_connection),
            pragma(&before_connection, "application_id"),
            pragma(&before_connection, "user_version"),
        );
        drop(before_connection);

        assert!(Database::open_with_migration_fault(&paths, 4, boundary).is_err());

        let after = Connection::open(paths.database_path()).unwrap();
        assert_eq!(schema_inventory(&after), before.0, "boundary {boundary}");
        assert_eq!(migration_records(&after), before.1, "boundary {boundary}");
        assert_eq!(legacy_rows(&after), before.2, "boundary {boundary}");
        assert_eq!(
            pragma(&after, "application_id"),
            before.3,
            "boundary {boundary}"
        );
        assert_eq!(
            pragma(&after, "user_version"),
            before.4,
            "boundary {boundary}"
        );
        assert!(!object_exists(&after, "memory_entry_versions"));
        assert!(!object_exists(&after, "memory_proposals"));
    }
}

fn create_schema_v3_fixture(paths: &AppPaths) {
    let connection = Connection::open(paths.database_path()).unwrap();
    for migration in [
        include_str!("../migrations/0001_phase0.sql"),
        include_str!("../migrations/0002_agent_profiles.sql"),
        include_str!("../migrations/0003_declarative_skills.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY CHECK (version > 0),
                checksum TEXT NOT NULL
            ) STRICT;",
        )
        .unwrap();
    for (version, sql) in [
        (1, include_str!("../migrations/0001_phase0.sql")),
        (2, include_str!("../migrations/0002_agent_profiles.sql")),
        (3, include_str!("../migrations/0003_declarative_skills.sql")),
    ] {
        connection
            .execute(
                "INSERT INTO schema_migrations (version, checksum) VALUES (?1, ?2)",
                params![version, sha256(sql.as_bytes()).as_str()],
            )
            .unwrap();
    }
    connection
        .pragma_update(None, "application_id", 0x4149_4653_i64)
        .unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    connection.execute_batch(
        "INSERT INTO event_stream (
             sequence, event_id, event_schema_version, event_type, actor_kind,
             occurred_at_ms, correlation_id, payload_json, event_digest
         ) VALUES (1, 'legacy-event', 1, 'legacy.created', 'system', 1,
                   'legacy-correlation', '{}', 'legacy-digest');
         INSERT INTO command_receipts (
             command_id, command_fingerprint, request_json, capability,
             policy_decision, outcome_json
         ) VALUES ('legacy-command', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                   '{\"legacy\":true}', 'skill_read', 'granted', '{\"ok\":true}');
         INSERT INTO command_event_refs (command_id, event_ordinal, event_id)
         VALUES ('legacy-command', 0, 'legacy-event');
         INSERT INTO approval_records (
             approval_id, action_kind, object_kind, object_id, object_version,
             object_digest, actor_kind, actor_id, status, created_at_ms,
             expires_at_ms, resolved_at_ms, resolution_kind, resolution_event_id
         ) VALUES ('legacy-terminal-approval', 'legacy_action', 'legacy_object',
                   'legacy-object-id', 1, 'legacy-object-digest', 'system', NULL,
                   'accepted', 1, NULL, 2, 'accepted', NULL);",
    )
    .unwrap();
}

fn legacy_rows(connection: &Connection) -> BTreeMap<&'static str, Vec<String>> {
    [
        (
            "approval_records",
            "SELECT json_object('approval_id', approval_id, 'action_kind', action_kind,
                'object_kind', object_kind, 'object_id', object_id,
                'object_version', object_version, 'object_digest', object_digest,
                'actor_kind', actor_kind, 'actor_id', actor_id, 'status', status,
                'created_at_ms', created_at_ms, 'expires_at_ms', expires_at_ms,
                'resolved_at_ms', resolved_at_ms, 'resolution_kind', resolution_kind,
                'resolution_event_id', resolution_event_id) FROM approval_records ORDER BY approval_id",
        ),
        (
            "command_receipts",
            "SELECT json_object('command_id', command_id, 'command_fingerprint', command_fingerprint,
                'request_json', request_json, 'capability', capability,
                'policy_decision', policy_decision, 'outcome_json', outcome_json)
             FROM command_receipts ORDER BY command_id",
        ),
        (
            "command_event_refs",
            "SELECT json_object('command_id', command_id, 'event_ordinal', event_ordinal,
                'event_id', event_id) FROM command_event_refs ORDER BY command_id, event_ordinal",
        ),
    ]
    .into_iter()
    .map(|(table, query)| {
        let mut statement = connection.prepare(query).unwrap();
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
            "SELECT type, name, tbl_name, sql FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
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

fn pragma(connection: &Connection, name: &str) -> i64 {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .unwrap()
}

fn object_exists(connection: &Connection, name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}
