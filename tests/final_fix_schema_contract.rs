const PHASE_2_MIGRATION: &str = include_str!("../migrations/0002_agent_profiles.sql");

#[test]
fn immutable_version_mirror_has_independent_stable_contract_columns() {
    for required in [
        "supersedes_version_id TEXT",
        "template_id TEXT",
        "template_version INTEGER",
        "template_digest TEXT",
        "role TEXT NOT NULL",
        "display_name TEXT NOT NULL",
        "memory_namespace_id TEXT NOT NULL",
        "policy_profile_ref TEXT NOT NULL",
    ] {
        assert!(
            PHASE_2_MIGRATION.contains(required),
            "migration is missing {required}"
        );
    }
    assert!(PHASE_2_MIGRATION.contains("agent_profile_namespace_conflict"));
}

#[test]
fn active_projection_pins_the_content_digest_without_persisting_readiness() {
    let active_start = PHASE_2_MIGRATION
        .find("CREATE TABLE active_agent_profiles (")
        .expect("active profile projection table");
    let active_sql = &PHASE_2_MIGRATION[active_start..];

    assert!(active_sql.contains("content_digest TEXT NOT NULL"));
    assert!(!active_sql.contains("readiness TEXT"));
}

#[test]
fn receipt_schema_names_all_four_profile_capabilities() {
    for capability in [
        "agent_profile_read",
        "agent_profile_create",
        "agent_profile_preview",
        "agent_profile_activate",
    ] {
        assert!(PHASE_2_MIGRATION.contains(capability));
    }
    assert!(!PHASE_2_MIGRATION.contains("agent_profile_edit"));
}
