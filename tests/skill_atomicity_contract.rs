use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    config::AppPaths,
    domain::{AgentProfileId, AgentProfileVersionId, MemoryNamespaceId, SkillId, SkillVersionId},
    persistence::{Database, insert_expected_version, insert_skill_version, set_active_skill},
    skills::{SkillDraft, SkillProvenance, SkillVersion},
};
use uuid::Uuid;

#[derive(Debug, PartialEq, Eq)]
struct DurableSnapshot {
    skill_versions: i64,
    active_skill: Vec<(String, String, i64)>,
    profile_versions: i64,
    active_profile: Vec<(String, String, i64)>,
    events: i64,
    receipts: i64,
    event_refs: i64,
}

#[test]
fn failed_assignment_transaction_leaves_every_durable_record_unchanged() {
    let mut database = database();
    let first = skill_v1();
    let base_profile = profile_v1(Vec::new());
    let transaction = database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &first).unwrap();
    set_active_skill(&transaction, &first).unwrap();
    insert_expected_version(&transaction, 1, &base_profile).unwrap();
    transaction
        .execute(
            "INSERT INTO active_agent_profiles (
                profile_id, profile_version_id, version, normalized_name, content_digest
             ) SELECT profile_id, profile_version_id, version, normalized_name, content_digest
               FROM agent_profile_versions WHERE profile_version_id = ?1",
            [base_profile.profile_version_id().to_string()],
        )
        .unwrap();
    seed_event_and_receipt(&transaction);
    transaction.commit().unwrap();
    let before = snapshot(database.connection());

    let second = SkillVersion::next_version(
        &first,
        SkillVersionId::from_uuid(Uuid::from_u128(12)),
        1_726_000_000_001,
        skill_draft("Require primary-source citations."),
    )
    .unwrap();
    let assigned_profile = AgentProfileVersion::next_version(
        &base_profile,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(102)),
        1_726_000_000_001,
        profile_draft(vec![second.reference()]),
    )
    .unwrap();
    let transaction = database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &second).unwrap();
    set_active_skill(&transaction, &second).unwrap();
    insert_expected_version(&transaction, 2, &assigned_profile).unwrap();
    transaction
        .execute("DELETE FROM active_agent_profiles", [])
        .unwrap();
    transaction
        .execute(
            "INSERT INTO active_agent_profiles (
                profile_id, profile_version_id, version, normalized_name, content_digest
             ) SELECT profile_id, profile_version_id, version, normalized_name, content_digest
               FROM agent_profile_versions WHERE profile_version_id = ?1",
            [assigned_profile.profile_version_id().to_string()],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO event_stream (
                sequence, event_id, event_schema_version, event_type, actor_kind,
                occurred_at_ms, correlation_id, previous_event_digest, payload_json, event_digest
             ) VALUES (2, 'event-2', 1, 'agent_skill_assigned', 'human', 2,
                       'correlation-2', 'digest-1', '{}', 'digest-2')",
            [],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO command_receipts (
                command_id, command_fingerprint, request_json, capability,
                policy_decision, outcome_json
             ) VALUES ('command-2', ?1, '{}', 'skill_assign', 'granted', '{}')",
            ["b".repeat(64)],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO command_event_refs (command_id, event_ordinal, event_id)
             VALUES ('command-2', 0, 'event-2')",
            [],
        )
        .unwrap();

    assert!(
        transaction
            .execute(
                "INSERT INTO command_receipts (
                command_id, command_fingerprint, request_json, capability,
                policy_decision, outcome_json
             ) VALUES ('command-2', ?1, '{}', 'skill_assign', 'granted', '{}')",
                ["c".repeat(64)],
            )
            .is_err()
    );
    transaction.rollback().unwrap();

    assert_eq!(snapshot(database.connection()), before);
}

fn database() -> Database {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.keep();
    Database::open(&AppPaths::for_test(&path)).unwrap()
}

fn skill_draft(instructions: &str) -> SkillDraft {
    SkillDraft::new(
        "Evidence Review".to_owned(),
        "Evidence-led research guidance.".to_owned(),
        "Use when reviewing an investment thesis.".to_owned(),
        vec!["research".to_owned()],
        instructions.to_owned(),
        Vec::new(),
    )
    .unwrap()
}

fn skill_v1() -> SkillVersion {
    SkillVersion::create(
        SkillId::from_uuid(Uuid::from_u128(1)),
        SkillVersionId::from_uuid(Uuid::from_u128(11)),
        1_726_000_000_000,
        SkillProvenance::User,
        skill_draft("State the evidence and the disconfirming case."),
    )
    .unwrap()
}

fn profile_draft(skill_refs: Vec<ai_stock_forum::skills::SkillVersionRef>) -> AgentProfileDraft {
    AgentProfileDraft::new(
        "Research Analyst".to_owned(),
        "Evidence-led equity research.".to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical.".to_owned(),
        "Cite evidence before making a claim.".to_owned(),
        AgentBindings::default(),
        skill_refs,
        Vec::new(),
    )
    .unwrap()
}

fn profile_v1(skill_refs: Vec<ai_stock_forum::skills::SkillVersionRef>) -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(1)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(101)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(1_001)),
        1_726_000_000_000,
        profile_draft(skill_refs),
        None,
    )
    .unwrap()
}

fn seed_event_and_receipt(transaction: &rusqlite::Transaction<'_>) {
    transaction
        .execute(
            "INSERT INTO event_stream (
                sequence, event_id, event_schema_version, event_type, actor_kind,
                occurred_at_ms, correlation_id, payload_json, event_digest
             ) VALUES (1, 'event-1', 1, 'agent_profile_created', 'human', 1,
                       'correlation-1', '{}', 'digest-1')",
            [],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO command_receipts (
                command_id, command_fingerprint, request_json, capability,
                policy_decision, outcome_json
             ) VALUES ('command-1', ?1, '{}', 'agent_profile_create', 'granted', '{}')",
            ["a".repeat(64)],
        )
        .unwrap();
    transaction
        .execute(
            "INSERT INTO command_event_refs (command_id, event_ordinal, event_id)
             VALUES ('command-1', 0, 'event-1')",
            [],
        )
        .unwrap();
}

fn snapshot(connection: &rusqlite::Connection) -> DurableSnapshot {
    DurableSnapshot {
        skill_versions: count(connection, "skill_versions"),
        active_skill: rows(
            connection,
            "active_skills",
            "skill_id, skill_version_id, version",
        ),
        profile_versions: count(connection, "agent_profile_versions"),
        active_profile: rows(
            connection,
            "active_agent_profiles",
            "profile_id, profile_version_id, version",
        ),
        events: count(connection, "event_stream"),
        receipts: count(connection, "command_receipts"),
        event_refs: count(connection, "command_event_refs"),
    }
}

fn count(connection: &rusqlite::Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn rows(
    connection: &rusqlite::Connection,
    table: &str,
    columns: &str,
) -> Vec<(String, String, i64)> {
    let mut statement = connection
        .prepare(&format!("SELECT {columns} FROM {table} ORDER BY 1"))
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
