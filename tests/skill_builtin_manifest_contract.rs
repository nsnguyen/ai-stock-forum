use ai_stock_forum::{
    domain::{ObjectVersion, SkillId, SkillVersionId},
    skills::{SkillProvenance, SkillsProjection, builtin_manifests, reconcile_builtin_manifests},
};
use uuid::Uuid;

fn skill_id(value: u128) -> SkillId {
    SkillId::from_uuid(Uuid::from_u128(value))
}

fn skill_version_id(value: u128) -> SkillVersionId {
    SkillVersionId::from_uuid(Uuid::from_u128(value))
}

#[test]
fn starter_manifests_are_stable_accepted_version_one_records() {
    let manifests = builtin_manifests().expect("valid static manifests");
    let expected = [
        (
            "Evidence Review",
            skill_id(0x1001),
            skill_version_id(0x2001),
            "evidence-review",
            "f9df82257cf896c23e65af4d4f45987d80fc2624fcade850f146e2f960e7d27c",
        ),
        (
            "Filing Analysis",
            skill_id(0x1002),
            skill_version_id(0x2002),
            "filing-analysis",
            "00b1aeae3bd3be4706e88ec47cfbbe68296a60700e659ca277bc5fa5fa391099",
        ),
        (
            "Catalyst Mapping",
            skill_id(0x1003),
            skill_version_id(0x2003),
            "catalyst-mapping",
            "78a982df8c3fc763d241f3850e6dd898f9344d0d3dbb00473658e0b4bd1e49a0",
        ),
        (
            "Risk Checklist",
            skill_id(0x1004),
            skill_version_id(0x2004),
            "risk-checklist",
            "eddd5889fafe3e93f9ecbd1a767197acfa3f9468cfcbc8b8c2e5f3b2d89a2f1d",
        ),
    ];

    assert_eq!(manifests.len(), 4);
    for (manifest, (name, id, version_id, manifest_id, digest)) in manifests.iter().zip(expected) {
        let skill = manifest.skill();
        assert_eq!(skill.content().display_name, name);
        assert_eq!(skill.skill_id(), id);
        assert_eq!(skill.skill_version_id(), version_id);
        assert_eq!(skill.version(), ObjectVersion::new(1).unwrap());
        assert!(skill.predecessor().is_none());
        assert_eq!(manifest.manifest_id(), manifest_id);
        assert_eq!(manifest.manifest_version(), 1);
        assert_eq!(manifest.expected_digest().as_str(), digest);
        assert_eq!(skill.content_digest(), manifest.expected_digest());
        assert_eq!(
            skill.recompute_content_digest().unwrap(),
            *manifest.expected_digest()
        );
        assert!(
            skill
                .content()
                .instructions
                .contains("unsupported financial conclusions")
        );
        match skill.provenance() {
            SkillProvenance::BuiltIn {
                manifest_id: actual_id,
                manifest_version,
                manifest_digest,
            } => {
                assert_eq!(actual_id, manifest_id);
                assert_eq!(*manifest_version, 1);
                assert_eq!(manifest_digest, manifest.expected_digest());
            }
            SkillProvenance::User => panic!("starter skill must retain built-in provenance"),
        }
    }
}

#[test]
fn reconciliation_is_independent_of_manifest_input_order() {
    let manifests = builtin_manifests().expect("valid static manifests");
    let mut reversed = manifests.clone();
    reversed.reverse();
    let mut forward_projection = SkillsProjection::default();
    let mut reversed_projection = SkillsProjection::default();

    let forward = reconcile_builtin_manifests(&mut forward_projection, &manifests)
        .expect("forward reconciliation");
    let reversed = reconcile_builtin_manifests(&mut reversed_projection, &reversed)
        .expect("reversed reconciliation");

    assert_eq!(forward, reversed);
    for skill in forward {
        assert_eq!(
            forward_projection.active_skill(skill.skill_id()),
            reversed_projection.active_skill(skill.skill_id())
        );
    }
}
