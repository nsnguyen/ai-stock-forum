use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    config::AppPaths,
    domain::{AgentProfileId, AgentProfileVersionId, MemoryNamespaceId, SkillId, SkillVersionId},
    persistence::{
        Database, PersistenceError, insert_expected_version, insert_skill_version,
        load_active_skill, load_active_skill_by_name, load_all_skill_versions, load_skill_history,
        load_skill_version, load_skill_version_by_id, reconcile_skill_versions,
        replace_active_skills, set_active_skill,
    },
    skills::{SkillDraft, SkillProvenance, SkillVersion, SkillVersionRef},
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn immutable_insert_is_idempotent_and_rejects_identity_substitution() {
    let mut database = database();
    let first = skill_v1(1, 11, "Evidence Review");
    let substituted = skill_v1(1, 12, "Different Content");
    let transaction = database.connection_mut().transaction().unwrap();

    insert_skill_version(&transaction, &first).unwrap();
    insert_skill_version(&transaction, &first).unwrap();
    let error = insert_skill_version(&transaction, &substituted).unwrap_err();
    assert_eq!(error, PersistenceError::SkillVersionIntegrityMismatch);
    transaction.commit().unwrap();

    assert_eq!(
        load_all_skill_versions(database.connection()).unwrap(),
        vec![first]
    );
    assert!(
        database
            .connection()
            .execute(
                "UPDATE skill_versions SET created_at_ms = created_at_ms + 1",
                []
            )
            .is_err()
    );
    assert!(
        database
            .connection()
            .execute("DELETE FROM skill_versions", [])
            .is_err()
    );
}

#[test]
fn exact_active_name_and_history_reads_return_validated_domain_versions() {
    let mut database = database();
    let first = skill_v1(1, 11, "Evidence Review");
    let second = SkillVersion::next_version(
        &first,
        skill_version_id(12),
        1_726_000_000_001,
        skill_draft(
            "Forensic Evidence Review",
            "Require primary-source citations.",
        ),
    )
    .unwrap();
    let transaction = database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &first).unwrap();
    insert_skill_version(&transaction, &second).unwrap();
    set_active_skill(&transaction, &first).unwrap();
    transaction.commit().unwrap();

    assert_eq!(
        load_skill_version(database.connection(), &first.reference()).unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        load_skill_version_by_id(database.connection(), second.skill_version_id()).unwrap(),
        Some(second.clone())
    );
    assert_eq!(
        load_skill_history(database.connection(), first.skill_id()).unwrap(),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(
        load_active_skill(database.connection(), first.skill_id()).unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        load_active_skill_by_name(database.connection(), &first.normalized_name().unwrap())
            .unwrap(),
        Some(first.clone())
    );

    let transaction = database.connection_mut().transaction().unwrap();
    set_active_skill(&transaction, &second).unwrap();
    transaction.commit().unwrap();

    assert_eq!(
        load_active_skill(database.connection(), first.skill_id()).unwrap(),
        Some(second.clone())
    );
    assert_eq!(
        load_active_skill_by_name(database.connection(), &first.normalized_name().unwrap())
            .unwrap(),
        None
    );
    assert_eq!(
        load_active_skill_by_name(database.connection(), &second.normalized_name().unwrap())
            .unwrap(),
        Some(second)
    );
    assert_eq!(
        load_skill_version(database.connection(), &first.reference()).unwrap(),
        Some(first)
    );
}

#[test]
fn active_normalized_names_cannot_select_two_logical_skills() {
    let mut database = database();
    let first = skill_v1(1, 11, "Evidence Review");
    let second = skill_v1(2, 21, "evidence   review");
    let transaction = database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &first).unwrap();
    insert_skill_version(&transaction, &second).unwrap();
    set_active_skill(&transaction, &first).unwrap();

    let error = set_active_skill(&transaction, &second).unwrap_err();
    assert_eq!(error, PersistenceError::DuplicateSkillName);
    transaction.rollback().unwrap();
}

#[test]
fn every_immutable_record_field_is_covered_by_verified_integrity() {
    for mutation in [
        "UPDATE skill_versions SET skill_id = '00000000-0000-0000-0000-000000000099'",
        "UPDATE skill_versions SET version = 2, predecessor_version_id = skill_version_id",
        "UPDATE skill_versions SET created_at_ms = created_at_ms + 1",
        "UPDATE skill_versions SET provenance_json = CAST('{\"user\":null}' AS BLOB)",
        "UPDATE skill_versions SET content_json = CAST('{\"display_name\":\"Altered\",\"description\":\"\",\"use_when\":\"x\",\"tags\":[],\"instructions\":\"x\",\"resources\":[]}' AS BLOB)",
        "UPDATE skill_versions SET content_digest = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
        "UPDATE skill_versions SET record_json = CAST('{}' AS BLOB)",
        "UPDATE skill_versions SET record_digest = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'",
    ] {
        let mut database = database();
        let first = skill_v1(1, 11, "Evidence Review");
        let transaction = database.connection_mut().transaction().unwrap();
        insert_skill_version(&transaction, &first).unwrap();
        transaction.commit().unwrap();
        database
            .connection()
            .execute_batch("DROP TRIGGER skill_versions_no_update")
            .unwrap();
        database.connection().execute(mutation, []).unwrap();

        let error = load_all_skill_versions(database.connection()).unwrap_err();
        assert_eq!(
            error,
            PersistenceError::SkillVersionIntegrityMismatch,
            "{mutation}"
        );
    }
}

#[test]
fn profile_insert_requires_an_existing_exact_verified_skill_reference() {
    let mut database = database();
    let skill = skill_v1(1, 11, "Evidence Review");
    let transaction = database.connection_mut().transaction().unwrap();
    insert_skill_version(&transaction, &skill).unwrap();
    insert_expected_version(&transaction, 1, &profile(1, 101, vec![skill.reference()])).unwrap();
    transaction.commit().unwrap();

    let mismatches = [
        forged_ref(2, 11, 1, skill.content_digest().as_str()),
        forged_ref(1, 12, 1, skill.content_digest().as_str()),
        forged_ref(1, 11, 2, skill.content_digest().as_str()),
        forged_ref(1, 11, 1, &"f".repeat(64)),
    ];
    for (index, reference) in mismatches.into_iter().enumerate() {
        let transaction = database.connection_mut().transaction().unwrap();
        let error = insert_expected_version(
            &transaction,
            i64::try_from(index + 2).unwrap(),
            &profile(
                u128::try_from(index + 2).unwrap(),
                102 + index as u128,
                vec![reference],
            ),
        )
        .unwrap_err();
        assert_eq!(error, PersistenceError::SkillVersionReferenceMismatch);
        transaction.rollback().unwrap();
    }
}

#[test]
fn reconciliation_and_active_projection_rebuild_are_idempotent_and_transactional() {
    let mut database = database();
    let first = skill_v1(1, 11, "Evidence Review");
    let second = skill_v1(2, 21, "Filing Analysis");
    let transaction = database.connection_mut().transaction().unwrap();
    reconcile_skill_versions(&transaction, &[first.clone(), second.clone()]).unwrap();
    reconcile_skill_versions(&transaction, &[first.clone(), second.clone()]).unwrap();
    replace_active_skills(&transaction, &[second.clone(), first.clone()]).unwrap();
    transaction.commit().unwrap();

    assert_eq!(
        load_all_skill_versions(database.connection()).unwrap(),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(
        load_active_skill(database.connection(), first.skill_id()).unwrap(),
        Some(first)
    );
    assert_eq!(
        load_active_skill(database.connection(), second.skill_id()).unwrap(),
        Some(second)
    );
}

fn database() -> Database {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.keep();
    Database::open(&AppPaths::for_test(&path)).unwrap()
}

fn skill_id(value: u128) -> SkillId {
    SkillId::from_uuid(Uuid::from_u128(value))
}

fn skill_version_id(value: u128) -> SkillVersionId {
    SkillVersionId::from_uuid(Uuid::from_u128(value))
}

fn skill_draft(name: &str, instructions: &str) -> SkillDraft {
    SkillDraft::new(
        name.to_owned(),
        "Evidence-led research guidance.".to_owned(),
        "Use when reviewing an investment thesis.".to_owned(),
        vec!["research".to_owned()],
        instructions.to_owned(),
        Vec::new(),
    )
    .unwrap()
}

fn skill_v1(skill: u128, version: u128, name: &str) -> SkillVersion {
    SkillVersion::create(
        skill_id(skill),
        skill_version_id(version),
        1_726_000_000_000,
        SkillProvenance::User,
        skill_draft(name, "State the evidence and the disconfirming case."),
    )
    .unwrap()
}

fn forged_ref(skill: u128, version_id: u128, version: u64, digest: &str) -> SkillVersionRef {
    serde_json::from_value(json!({
        "skill_id": Uuid::from_u128(skill),
        "skill_version_id": Uuid::from_u128(version_id),
        "version": version,
        "content_digest": digest,
    }))
    .unwrap()
}

fn profile(profile: u128, version: u128, skill_refs: Vec<SkillVersionRef>) -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(profile)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(version)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(profile + 1_000)),
        1_726_000_000_000,
        AgentProfileDraft::new(
            format!("Research Analyst {profile}"),
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
        .unwrap(),
        None,
    )
    .unwrap()
}
