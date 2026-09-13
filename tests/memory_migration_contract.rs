use std::collections::BTreeMap;

use ai_stock_forum::{config::AppPaths, domain::sha256, persistence::Database};
use rusqlite::{Connection, params};

const LEGACY_APPROVAL_ID: &str = "legacy-terminal-approval";

#[test]
fn fresh_database_reaches_v5_and_registers_hybrid_memory_storage() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();

    assert_eq!(database.schema_version(), 5);
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
    let before_connection = Connection::open(paths.database_path()).unwrap();
    let before = v3_snapshot(&before_connection);
    assert_eq!(migration_records(&before_connection).len(), 3);
    assert_eq!(pragma(&before_connection, "application_id"), 0x4149_4653);
    assert_eq!(pragma(&before_connection, "user_version"), 3);

    let database = Database::open(&paths).unwrap();

    assert_eq!(database.schema_version(), 5);
    assert_eq!(v3_snapshot(database.connection()), before);
    assert_eq!(migration_records(database.connection()).len(), 5);
    assert_eq!(pragma(database.connection(), "application_id"), 0x4149_4653);
    assert_eq!(pragma(database.connection(), "user_version"), 5);
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
            v3_snapshot(&before_connection),
            pragma(&before_connection, "application_id"),
            pragma(&before_connection, "user_version"),
        );
        drop(before_connection);

        assert!(Database::open_with_migration_fault(&paths, 4, boundary).is_err());

        let after = Connection::open(paths.database_path()).unwrap();
        assert_eq!(schema_inventory(&after), before.0, "boundary {boundary}");
        assert_eq!(migration_records(&after), before.1, "boundary {boundary}");
        assert_eq!(v3_snapshot(&after), before.2, "boundary {boundary}");
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

#[test]
fn migration_failure_at_every_v5_boundary_rolls_back_to_the_exact_v4_guard() {
    let expected_boundaries = vec![
        "drop_episodic_summary_sources_order_guard_v4",
        "create_episodic_summary_sources_recovery_order_guard",
        "schema_migration_record",
    ];
    assert_eq!(Database::v5_migration_boundaries(), expected_boundaries);

    for boundary in expected_boundaries {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        create_schema_v4_fixture(&paths);

        assert!(Database::open_with_migration_fault(&paths, 5, boundary).is_err());

        let after = Connection::open(paths.database_path()).unwrap();
        assert_eq!(pragma(&after, "user_version"), 4, "boundary {boundary}");
        assert_eq!(migration_records(&after).len(), 4, "boundary {boundary}");
        let v4_guard: String = after
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='episodic_summary_sources_order_guard'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(v4_guard.contains("SELECT COUNT(*)"), "boundary {boundary}");
        assert!(
            !v4_guard.contains("source_ordinal - 1"),
            "boundary {boundary}"
        );
        drop(after);

        let migrated = Database::open(&paths).unwrap();
        assert_eq!(migrated.schema_version(), 5, "boundary {boundary}");
        assert_eq!(
            migrated.applied_migrations().unwrap().len(),
            5,
            "boundary {boundary}"
        );
        let v5_guard: String = migrated
            .connection()
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='episodic_summary_sources_order_guard'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            v5_guard.contains("source_ordinal - 1"),
            "boundary {boundary}"
        );
        assert!(!v5_guard.contains("SELECT COUNT(*)"), "boundary {boundary}");
    }
}

#[test]
fn v5_guard_permits_only_an_ordered_authenticated_middle_source_hole_fill() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    create_schema_v4_fixture(&paths);
    let connection = Connection::open(paths.database_path()).unwrap();
    connection
        .execute_batch(
            "INSERT INTO event_stream (
                 sequence,event_id,event_schema_version,event_type,actor_kind,occurred_at_ms,
                 correlation_id,payload_json,event_digest
             ) VALUES
                 (2,'source-event-1',1,'source.one','system',2,'source-correlation-1','{}',
                  'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'),
                 (3,'source-event-2',1,'source.two','system',3,'source-correlation-2','{}',
                  'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'),
                 (4,'source-event-3',1,'source.three','system',4,'source-correlation-3','{}',
                  'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'),
                 (5,'summary-event',1,'episodic_summary_recorded','system',5,
                  'summary-correlation','{}',
                  'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee');
             INSERT INTO episodic_summaries (
                 summary_id,version,memory_namespace_id,profile_id,profile_version_id,
                 profile_version,profile_content_digest,label,body,purpose_tags_json,
                 source_count,plaintext_validation_version,created_at_ms,
                 creation_event_sequence,creation_event_id,source_set_digest,content_digest,
                 record_digest,record_json
             ) VALUES (
                 '00000000-0000-0000-0000-000000000099',1,
                 '00000000-0000-0000-0000-000000000003',
                 '00000000-0000-0000-0000-000000000001',
                 '00000000-0000-0000-0000-000000000002',1,
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 'Recovery hole','Recovery body',CAST('[]' AS BLOB),3,1,5,5,
                 'summary-event',
                 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',
                 '9999999999999999999999999999999999999999999999999999999999999999',
                 '8888888888888888888888888888888888888888888888888888888888888888',
                 CAST('{}' AS BLOB));
             INSERT INTO episodic_summary_sources VALUES
                 ('00000000-0000-0000-0000-000000000099',0,2,'source-event-1',
                  'source.one','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'),
                 ('00000000-0000-0000-0000-000000000099',1,3,'source-event-2',
                  'source.two','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'),
                 ('00000000-0000-0000-0000-000000000099',2,4,'source-event-3',
                  'source.three','dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd');
             DROP TRIGGER episodic_summary_sources_no_delete;
             DELETE FROM episodic_summary_sources
             WHERE summary_id='00000000-0000-0000-0000-000000000099' AND source_ordinal=1;",
        )
        .unwrap();
    drop(connection);

    let database = Database::open(&paths).unwrap();
    assert_eq!(database.schema_version(), 5);
    for (sequence, event_id, event_type, digest) in [
        (
            2,
            "source-event-1",
            "source.one",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
        (
            4,
            "source-event-3",
            "source.three",
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
    ] {
        assert!(
            database
                .connection()
                .execute(
                    "INSERT INTO episodic_summary_sources VALUES (
                         '00000000-0000-0000-0000-000000000099',1,?1,?2,?3,?4
                     )",
                    params![sequence, event_id, event_type, digest],
                )
                .is_err(),
            "middle source event sequence {sequence} must be bounded by both neighbours"
        );
    }
    database
        .connection()
        .execute(
            "INSERT INTO episodic_summary_sources VALUES (
                 '00000000-0000-0000-0000-000000000099',1,3,'source-event-2',
                 'source.two','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'
             )",
            [],
        )
        .unwrap();
    assert_eq!(
        database
            .connection()
            .query_row(
                "SELECT group_concat(source_ordinal||':'||event_sequence,',')
                 FROM episodic_summary_sources
                 WHERE summary_id='00000000-0000-0000-0000-000000000099'
                 ORDER BY source_ordinal",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "0:2,1:3,2:4"
    );
    database
        .connection()
        .execute(
            "DELETE FROM episodic_summary_sources
             WHERE summary_id='00000000-0000-0000-0000-000000000099'",
            [],
        )
        .unwrap();
    assert!(
        database
            .connection()
            .execute(
                "INSERT INTO episodic_summary_sources VALUES (
                     '00000000-0000-0000-0000-000000000099',1,3,'source-event-2',
                     'source.two','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'
                 )",
                [],
            )
            .is_err(),
        "a missing immediate predecessor must be rejected"
    );
    for statement in [
        "INSERT INTO episodic_summary_sources VALUES (
             '00000000-0000-0000-0000-000000000099',0,2,'source-event-1',
             'source.one','bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb')",
        "INSERT INTO episodic_summary_sources VALUES (
             '00000000-0000-0000-0000-000000000099',1,3,'source-event-2',
             'source.two','cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc')",
        "INSERT INTO episodic_summary_sources VALUES (
             '00000000-0000-0000-0000-000000000099',2,4,'source-event-3',
             'source.three','dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd')",
    ] {
        database.connection().execute(statement, []).unwrap();
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
                   'accepted', 1, NULL, 2, 'accepted', NULL);
         INSERT INTO installation_projection (singleton, installation_id, created_event_id, created_at_ms)
         VALUES (1, 'legacy-installation', 'legacy-event', 1);
         INSERT INTO process_session_projection (session_id, started_event_id, started_at_ms)
         VALUES ('legacy-session', 'legacy-event', 1);
         INSERT INTO projection_metadata (singleton, last_event_sequence, last_event_digest, projection_digest)
         VALUES (1, 1, 'legacy-digest', 'legacy-projection-digest');
         INSERT INTO setup_drafts (draft_id, schema_version, state, path, payload_json, created_at_ms, updated_at_ms)
         VALUES ('legacy-draft', 1, 'drafting', 'quick_start', '{\"draft\":true}', 1, 2);
         INSERT INTO installation_configuration_versions (configuration_id, version, source_draft_id, review_digest, object_digest, payload_json, created_event_id, created_at_ms)
         VALUES ('legacy-configuration', 1, 'legacy-draft', 'legacy-review', 'legacy-config-digest', '{\"config\":true}', 'legacy-event', 2);
         INSERT INTO active_installation_configuration (singleton, configuration_id, activated_event_id, activated_at_ms)
         VALUES (1, 'legacy-configuration', 'legacy-event', 2);
         INSERT INTO setup_step_outcomes (draft_id, step_key, attempt, status, safe_code, occurred_at_ms)
         VALUES ('legacy-draft', 'review', 1, 'passed', 'ok', 2);
         INSERT INTO capability_readiness (configuration_id, capability, status, reason_code, checked_at_ms, projection_digest)
         VALUES ('legacy-configuration', 'audit_read', 'ready', NULL, 2, 'legacy-projection-digest');
         INSERT INTO agent_profile_versions (profile_id, profile_version_id, version, supersedes_version_id, template_id, template_version, template_digest, role, display_name, normalized_name, memory_namespace_id, policy_profile_ref, content_digest, payload_json, source_event_sequence, created_at_ms)
         VALUES ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 1, NULL, NULL, NULL, NULL, 'custom', 'Legacy Profile', 'legacy profile', '00000000-0000-0000-0000-000000000003', 'legacy-policy', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', CAST('{\"profile\":true}' AS BLOB), 1, 2);
         INSERT INTO active_agent_profiles (profile_id, profile_version_id, version, normalized_name, content_digest)
         VALUES ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', 1, 'legacy profile', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
         INSERT INTO skill_versions (skill_id, skill_version_id, version, predecessor_version_id, display_name, normalized_name, content_digest, content_json, provenance_json, created_at_ms, record_digest, record_json)
         VALUES ('00000000-0000-0000-0000-000000000011', '00000000-0000-0000-0000-000000000012', 1, NULL, 'Legacy Skill', 'legacy skill', 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', CAST('{\"skill\":true}' AS BLOB), CAST('{\"source\":\"legacy\"}' AS BLOB), 2, 'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc', CAST('{\"record\":true}' AS BLOB));
         INSERT INTO active_skills (skill_id, skill_version_id, version, normalized_name, content_digest, record_digest)
         VALUES ('00000000-0000-0000-0000-000000000011', '00000000-0000-0000-0000-000000000012', 1, 'legacy skill', 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', 'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc');",
    )
    .unwrap();
}

fn create_schema_v4_fixture(paths: &AppPaths) {
    create_schema_v3_fixture(paths);
    let connection = Connection::open(paths.database_path()).unwrap();
    let sql = include_str!("../migrations/0004_hybrid_memory.sql");
    connection.execute_batch(sql).unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations (version,checksum) VALUES (4,?1)",
            [ai_stock_forum::domain::sha256(sql.as_bytes()).as_str()],
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 4).unwrap();
}

fn v3_snapshot(connection: &Connection) -> BTreeMap<String, Vec<String>> {
    let tables = [
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
        "agent_profile_versions",
        "active_agent_profiles",
        "skill_versions",
        "active_skills",
    ];
    tables
        .into_iter()
        .map(|table| {
            let columns = table_columns(connection, table);
            let quoted = columns
                .iter()
                .map(|column| format!("quote(\"{column}\")"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut statement = connection
                .prepare(&format!(
                    "SELECT json_array({quoted}) FROM \"{table}\" ORDER BY rowid"
                ))
                .unwrap();
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            (table.to_owned(), rows)
        })
        .collect()
}

fn table_columns(connection: &Connection, table: &str) -> Vec<String> {
    if table == "approval_records" {
        return [
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
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
    }
    let mut statement = connection
        .prepare(&format!("PRAGMA table_xinfo(\"{table}\")"))
        .unwrap();
    statement
        .query_map([], |row| row.get(1))
        .unwrap()
        .map(Result::unwrap)
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
