use std::collections::BTreeMap;

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    config::AppPaths,
    domain::{
        AgentProfileId, AgentProfileVersionId, MemoryNamespaceId, canonical_json_bytes, sha256,
    },
    persistence::{Database, LATEST_SCHEMA_VERSION, load_all_versions},
};
use rusqlite::{Connection, params};
use uuid::Uuid;

#[test]
fn schema_v2_migrates_to_exactly_v4_without_changing_agent_rows() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    let expected_profile = legacy_profile();
    assert!(
        expected_profile.skill_refs().is_empty(),
        "valid released-v2 compatibility fixtures cannot contain skill references"
    );
    create_schema_v2_fixture(&paths, &expected_profile);
    let before = agent_rows(&Connection::open(paths.database_path()).unwrap());

    let database = Database::open(&paths).unwrap();

    assert_eq!(LATEST_SCHEMA_VERSION, 5);
    assert_eq!(database.schema_version(), 5);
    assert_eq!(
        database
            .applied_migrations()
            .unwrap()
            .iter()
            .map(|migration| migration.version())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(agent_rows(database.connection()), before);
    assert_eq!(
        load_all_versions(database.connection()).unwrap()[0].profile,
        expected_profile
    );
}

#[test]
fn migration_v3_creates_strict_skill_storage_and_accepts_only_inert_capabilities() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection();

    for table in ["skill_versions", "active_skills"] {
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
        "skill_versions_history_idx",
        "active_skills_normalized_name_idx",
        "skill_versions_predecessor_guard",
        "skill_versions_no_update",
        "skill_versions_no_delete",
    ] {
        assert!(object_exists(connection, object), "missing {object}");
    }

    for (index, capability) in [
        "skill_read",
        "skill_create",
        "skill_version",
        "skill_assign",
        "skill_unassign",
    ]
    .into_iter()
    .enumerate()
    {
        connection
            .execute(
                "INSERT INTO command_receipts (
                    command_id, command_fingerprint, request_json, capability,
                    policy_decision, outcome_json
                 ) VALUES (?1, ?2, '{}', ?3, 'granted', '{}')",
                params![format!("skill-command-{index}"), "a".repeat(64), capability],
            )
            .unwrap();
    }
    assert!(
        connection
            .execute(
                "INSERT INTO command_receipts (
                command_id, command_fingerprint, request_json, capability,
                policy_decision, outcome_json
             ) VALUES ('execute-command', ?1, '{}', 'skill_execute', 'granted', '{}')",
                ["b".repeat(64)],
            )
            .is_err()
    );
}

#[test]
fn skill_schema_enforces_identity_predecessor_and_active_pointer_constraints() {
    let temp = tempfile::tempdir().unwrap();
    let database = Database::open(&AppPaths::for_test(temp.path())).unwrap();
    let connection = database.connection();
    let digest = "a".repeat(64);
    connection
        .execute(
            "INSERT INTO skill_versions (
                skill_id, skill_version_id, version, predecessor_version_id,
                display_name, normalized_name, content_digest, content_json,
                provenance_json, created_at_ms, record_digest, record_json
             ) VALUES (?1, ?2, 1, NULL, 'Evidence Review', 'evidence review', ?3,
                       CAST('{}' AS BLOB), CAST('\"user\"' AS BLOB), 1, ?3,
                       CAST('{}' AS BLOB))",
            params![
                Uuid::from_u128(1).to_string(),
                Uuid::from_u128(11).to_string(),
                digest
            ],
        )
        .unwrap();

    assert!(
        connection
            .execute(
                "INSERT INTO skill_versions SELECT skill_id, ?1, version,
                    predecessor_version_id, display_name, normalized_name, content_digest,
                    content_json, provenance_json, created_at_ms, record_digest, record_json
             FROM skill_versions",
                [Uuid::from_u128(12).to_string()],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO skill_versions SELECT ?1, skill_version_id, version,
                    predecessor_version_id, display_name, normalized_name, content_digest,
                    content_json, provenance_json, created_at_ms, record_digest, record_json
             FROM skill_versions",
                [Uuid::from_u128(2).to_string()],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO skill_versions SELECT skill_id, ?1, 2, ?2,
                    display_name, normalized_name, content_digest, content_json,
                    provenance_json, created_at_ms, record_digest, record_json
             FROM skill_versions",
                params![
                    Uuid::from_u128(13).to_string(),
                    Uuid::from_u128(99).to_string()
                ],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO active_skills (
                skill_id, skill_version_id, version, normalized_name,
                content_digest, record_digest
             ) VALUES (?1, ?2, 1, 'missing', ?3, ?3)",
                params![
                    Uuid::from_u128(9).to_string(),
                    Uuid::from_u128(99).to_string(),
                    "b".repeat(64)
                ],
            )
            .is_err()
    );
}

#[test]
fn every_v3_migration_boundary_rolls_back_to_the_exact_v2_schema() {
    let expected_boundaries = vec![
        "drop_command_event_refs_no_update",
        "drop_command_event_refs_no_delete",
        "drop_command_event_refs_event_idx",
        "rename_command_event_refs_v2",
        "drop_command_receipts_no_update",
        "drop_command_receipts_no_delete",
        "rename_command_receipts_v2",
        "create_command_receipts",
        "rebuild_command_receipts",
        "create_command_receipts_no_update",
        "create_command_receipts_no_delete",
        "create_command_event_refs",
        "rebuild_command_event_refs",
        "create_command_event_refs_event_idx",
        "create_command_event_refs_no_update",
        "create_command_event_refs_no_delete",
        "drop_command_event_refs_v2",
        "drop_command_receipts_v2",
        "create_skill_versions",
        "create_skill_versions_history_idx",
        "create_skill_versions_predecessor_guard",
        "create_active_skills",
        "create_active_skills_normalized_name_idx",
        "create_skill_versions_no_update",
        "create_skill_versions_no_delete",
        "schema_migration_record",
    ];
    assert_eq!(Database::v3_migration_boundaries(), expected_boundaries);

    for boundary in expected_boundaries {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        create_schema_v2_fixture(&paths, &legacy_profile());
        let before = {
            let connection = Connection::open(paths.database_path()).unwrap();
            (
                schema_inventory(&connection),
                migration_records(&connection),
                agent_rows(&connection),
                pragma_i64(&connection, "application_id"),
                pragma_i64(&connection, "user_version"),
            )
        };

        assert!(Database::open_with_migration_fault(&paths, 3, boundary).is_err());

        let connection = Connection::open(paths.database_path()).unwrap();
        assert_eq!(
            schema_inventory(&connection),
            before.0,
            "boundary {boundary}"
        );
        assert_eq!(
            migration_records(&connection),
            before.1,
            "boundary {boundary}"
        );
        assert_eq!(agent_rows(&connection), before.2, "boundary {boundary}");
        assert_eq!(pragma_i64(&connection, "application_id"), before.3);
        assert_eq!(pragma_i64(&connection, "user_version"), before.4);
        assert!(!object_exists(&connection, "skill_versions"));
        assert!(!object_exists(&connection, "active_skills"));
    }
}

#[test]
fn legacy_agent_snapshot_detects_a_changed_stored_value() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    create_schema_v2_fixture(&paths, &legacy_profile());
    let connection = Connection::open(paths.database_path()).unwrap();
    let before = agent_rows(&connection);

    connection
        .execute_batch(
            "DROP TRIGGER agent_profile_versions_no_update;
             UPDATE agent_profile_versions SET created_at_ms = created_at_ms + 1;",
        )
        .unwrap();

    assert_ne!(agent_rows(&connection), before);
}

fn legacy_profile() -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(1)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(2)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(3)),
        1_726_000_000_000,
        AgentProfileDraft::new(
            "Legacy Analyst".to_owned(),
            "Existing profile bytes must survive migration.".to_owned(),
            AgentRole::Custom,
            "equity research".to_owned(),
            vec!["research".to_owned()],
            "Calm and skeptical.".to_owned(),
            "Cite evidence before making a claim.".to_owned(),
            AgentBindings::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn create_schema_v2_fixture(paths: &AppPaths, profile: &AgentProfileVersion) {
    let connection = Connection::open(paths.database_path()).unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_phase0.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0002_agent_profiles.sql"))
        .unwrap();
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
    connection.pragma_update(None, "user_version", 2).unwrap();
    insert_profile_row(&connection, profile);
    connection
        .execute(
            "INSERT INTO active_agent_profiles (
                profile_id, profile_version_id, version, normalized_name, content_digest
             ) SELECT profile_id, profile_version_id, version, normalized_name, content_digest
               FROM agent_profile_versions WHERE profile_version_id = ?1",
            [profile.profile_version_id().to_string()],
        )
        .unwrap();
}

fn insert_profile_row(connection: &Connection, profile: &AgentProfileVersion) {
    connection
        .execute(
            "INSERT INTO agent_profile_versions (
                profile_id, profile_version_id, version, supersedes_version_id,
                template_id, template_version, template_digest, role, display_name,
                normalized_name, memory_namespace_id, policy_profile_ref, content_digest,
                payload_json, source_event_sequence, created_at_ms
             ) VALUES (?1, ?2, ?3, NULL, NULL, NULL, NULL, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11)",
            params![
                profile.profile_id().to_string(),
                profile.profile_version_id().to_string(),
                i64::try_from(profile.version().get()).unwrap(),
                profile.role().as_str(),
                profile.display_name(),
                profile.normalized_name().as_str(),
                profile.memory_namespace_id().to_string(),
                profile.default_policy_ref(),
                profile.content_digest().as_str(),
                canonical_json_bytes(profile).unwrap(),
                profile.created_at_ms(),
            ],
        )
        .unwrap();
}

fn agent_rows(connection: &Connection) -> BTreeMap<&'static str, Vec<String>> {
    [
        (
            "agent_profile_versions",
            "SELECT json_object(
                'profile_id', profile_id,
                'profile_version_id', profile_version_id,
                'version', version,
                'supersedes_version_id', supersedes_version_id,
                'template_id', template_id,
                'template_version', template_version,
                'template_digest', template_digest,
                'role', role,
                'display_name', display_name,
                'normalized_name', normalized_name,
                'memory_namespace_id', memory_namespace_id,
                'policy_profile_ref', policy_profile_ref,
                'content_digest', content_digest,
                'payload_hex', hex(payload_json),
                'source_event_sequence', source_event_sequence,
                'created_at_ms', created_at_ms
             ) FROM agent_profile_versions
             ORDER BY profile_id, version, profile_version_id",
        ),
        (
            "active_agent_profiles",
            "SELECT json_object(
                'profile_id', profile_id,
                'profile_version_id', profile_version_id,
                'version', version,
                'normalized_name', normalized_name,
                'content_digest', content_digest
             ) FROM active_agent_profiles
             ORDER BY profile_id, version, profile_version_id",
        ),
    ]
    .into_iter()
    .map(|(table, sql)| {
        let mut statement = connection.prepare(sql).unwrap();
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

fn object_exists(connection: &Connection, name: &str) -> bool {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
            [name],
            |row| row.get(0),
        )
        .unwrap()
}

fn pragma_i64(connection: &Connection, name: &str) -> i64 {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .unwrap()
}
