use ai_stock_forum::{
    domain::{ObjectVersion, SkillId, SkillVersionId, canonical_json_bytes, sha256},
    skills::{
        SkillDraft, SkillProvenance, SkillResource, SkillVersion, SkillsProjection,
    },
};
use uuid::Uuid;

fn id(value: u128) -> SkillId {
    SkillId::from_uuid(Uuid::from_u128(value))
}

fn version_id(value: u128) -> SkillVersionId {
    SkillVersionId::from_uuid(Uuid::from_u128(value))
}

fn draft() -> SkillDraft {
    SkillDraft::new(
        "Evidence Review".to_owned(),
        "Check sources before drawing conclusions.".to_owned(),
        "Use when evidence quality matters.".to_owned(),
        vec!["research".to_owned(), "evidence".to_owned()],
        "Compare claims to their supporting material.".to_owned(),
        vec![SkillResource {
            name: "Checklist".to_owned(),
            body: "Verify date, source, and context.".to_owned(),
        }],
    )
    .expect("valid draft")
}

fn user_provenance() -> SkillProvenance {
    SkillProvenance::User
}

#[test]
fn skill_ids_are_distinct_typed_ids() {
    fn accepts_skill_id(_: SkillId) {}
    fn accepts_skill_version_id(_: SkillVersionId) {}

    accepts_skill_id(id(1));
    accepts_skill_version_id(version_id(2));
    assert_ne!(id(1).to_string(), version_id(2).to_string());
}

#[test]
fn valid_draft_accepts_into_immutable_version_one() {
    let accepted = SkillVersion::create(id(1), version_id(101), 1_800_000_000_000, user_provenance(), draft())
        .expect("accepted version one");

    assert_eq!(accepted.skill_id(), id(1));
    assert_eq!(accepted.skill_version_id(), version_id(101));
    assert_eq!(accepted.version(), ObjectVersion::new(1).unwrap());
    assert!(accepted.predecessor().is_none());
    assert_eq!(accepted.content().tags, ["evidence", "research"]);
}

#[test]
fn next_version_keeps_logical_identity_and_advances_once() {
    let first = SkillVersion::create(id(1), version_id(101), 1_800_000_000_000, user_provenance(), draft())
        .unwrap();
    let mut changed = first.content().clone();
    changed.instructions.push_str(" Cite the primary source.");

    let second = SkillVersion::next_version(&first, version_id(102), 1_800_000_000_001, changed)
        .expect("next immutable version");

    assert_eq!(second.skill_id(), first.skill_id());
    assert_ne!(second.skill_version_id(), first.skill_version_id());
    assert_eq!(second.version().get(), first.version().get() + 1);
    assert_eq!(second.predecessor(), Some(first.skill_version_id()));
}

#[test]
fn next_version_rejects_reusing_its_predecessor_version_id() {
    let first = SkillVersion::create(id(1), version_id(101), 1_800_000_000_000, user_provenance(), draft())
        .unwrap();
    let mut changed = first.content().clone();
    changed.instructions.push_str(" Cite the primary source.");

    assert!(SkillVersion::next_version(
        &first,
        first.skill_version_id(),
        1_800_000_000_001,
        changed,
    )
    .is_err());
}

#[test]
fn unchanged_normalized_candidate_is_rejected() {
    let first = SkillVersion::create(id(1), version_id(101), 1_800_000_000_000, user_provenance(), draft())
        .unwrap();
    let unchanged = SkillDraft::new(
        " Evidence Review ".to_owned(),
        "Check sources before drawing conclusions.".to_owned(),
        "Use when evidence quality matters.".to_owned(),
        vec!["research".to_owned(), "evidence".to_owned()],
        "Compare claims to their supporting material.".to_owned(),
        vec![SkillResource {
            name: "Checklist".to_owned(),
            body: "Verify date, source, and context.".to_owned(),
        }],
    )
    .unwrap();

    assert!(SkillVersion::next_version(&first, version_id(102), 1_800_000_000_001, unchanged).is_err());
}

#[test]
fn canonical_limits_and_forbidden_controls_hold_at_boundaries() {
    let valid = SkillDraft::new(
        "n".repeat(64),
        "d".repeat(256),
        "u".repeat(512),
        (0..8).map(|index| format!("t{index:0>31}")).collect(),
        "i".repeat(4_096),
        (0..8)
            .map(|index| SkillResource {
                name: format!("r{index:0>63}"),
                body: "b".repeat(2_048),
            })
            .collect(),
    );
    assert!(valid.is_ok());

    for invalid in [
        SkillDraft::new("n".repeat(65), "d".repeat(256), "u".repeat(512), vec![], "i".repeat(4_096), vec![]),
        SkillDraft::new("n".repeat(64), "d".repeat(257), "u".repeat(512), vec![], "i".repeat(4_096), vec![]),
        SkillDraft::new("n".repeat(64), "d".repeat(256), "u".repeat(513), vec![], "i".repeat(4_096), vec![]),
        SkillDraft::new("n".repeat(64), "d".repeat(256), "u".repeat(512), vec!["t".repeat(32); 9], "i".repeat(4_096), vec![]),
        SkillDraft::new("n".repeat(64), "d".repeat(256), "u".repeat(512), vec!["t".repeat(33)], "i".repeat(4_096), vec![]),
        SkillDraft::new("n".repeat(64), "d".repeat(256), "u".repeat(512), vec![], "i".repeat(4_097), vec![]),
        SkillDraft::new("n".repeat(64), "d".repeat(256), "u".repeat(512), vec![], "i".repeat(4_096), vec![SkillResource { name: "r".to_owned(), body: "b".repeat(4_097) }]),
        SkillDraft::new("n".repeat(64), "d".repeat(256), "u".repeat(512), vec![], "i".repeat(4_096), vec![SkillResource { name: "r".repeat(65), body: "b".to_owned() }]),
        SkillDraft::new("n".repeat(64), "d".repeat(256), "u".repeat(512), vec![], "i".repeat(4_096), (0..9).map(|index| SkillResource { name: format!("r{index}"), body: String::new() }).collect()),
        SkillDraft::new("n".repeat(64), "d".repeat(256), "u".repeat(512), vec![], "i".repeat(4_096), vec![
            SkillResource { name: "r1".to_owned(), body: "b".repeat(4_096) },
            SkillResource { name: "r2".to_owned(), body: "b".repeat(4_096) },
            SkillResource { name: "r3".to_owned(), body: "b".repeat(4_096) },
            SkillResource { name: "r4".to_owned(), body: "b".repeat(4_096) },
            SkillResource { name: "r5".to_owned(), body: "b".to_owned() },
        ]),
    ] {
        assert!(invalid.is_err());
    }

    for control in ["\0", "\u{001b}", "\u{0085}", "\u{202e}"] {
        assert!(SkillDraft::new(
            format!("unsafe{control}"),
            "description".to_owned(),
            "usage".to_owned(),
            vec![],
            "instructions".to_owned(),
            vec![],
        )
        .is_err());
    }
}

#[test]
fn complete_canonical_payload_limit_accepts_32768_bytes_and_rejects_32769() {
    let escaped_bytes = (0..=16_384)
        .find(|escaped_bytes| canonical_json_bytes(&maximum_payload_draft(*escaped_bytes)).unwrap().len() == 32_768)
        .expect("a payload exactly at the canonical limit");
    let at_limit = maximum_payload_draft(escaped_bytes);
    let over_limit = maximum_payload_draft(escaped_bytes + 1);

    assert_eq!(canonical_json_bytes(&at_limit).unwrap().len(), 32_768);
    assert_eq!(canonical_json_bytes(&over_limit).unwrap().len(), 32_769);
    assert!(accept_draft(at_limit).is_ok());
    assert!(accept_draft(over_limit).is_err());
}

#[test]
fn canonicalization_normalizes_line_endings_orders_content_and_rejects_duplicates() {
    let canonical = SkillDraft::new(
        "Evidence Review".to_owned(),
        "A\r\nB\rC".to_owned(),
        "Use\rwhen".to_owned(),
        vec!["Zulu".to_owned(), "alpha".to_owned()],
        "First\r\nSecond\rThird".to_owned(),
        vec![
            SkillResource { name: "Zulu".to_owned(), body: "Z".to_owned() },
            SkillResource { name: "alpha".to_owned(), body: "A".to_owned() },
        ],
    )
    .unwrap();
    assert_eq!(canonical.description, "A\nB\nC");
    assert_eq!(canonical.instructions, "First\nSecond\nThird");
    assert_eq!(canonical.tags, ["alpha", "Zulu"]);
    assert_eq!(canonical.resources[0].name, "alpha");
    assert!(SkillDraft::new(
        "Evidence Review".to_owned(), "description".to_owned(), "usage".to_owned(),
        vec!["evidence".to_owned(), "EVIDENCE".to_owned()], "instructions".to_owned(), vec![],
    ).is_err());
    assert!(SkillDraft::new(
        "Evidence Review".to_owned(), "description".to_owned(), "usage".to_owned(), vec![],
        "instructions".to_owned(), vec![
            SkillResource { name: "checklist".to_owned(), body: "one".to_owned() },
            SkillResource { name: "CHECKLIST".to_owned(), body: "two".to_owned() },
        ],
    ).is_err());
}

#[test]
fn equal_accepted_content_has_stable_digest_and_exact_reference() {
    let first = SkillVersion::create(id(1), version_id(101), 1_800_000_000_000, user_provenance(), draft()).unwrap();
    let second = SkillVersion::create(id(1), version_id(101), 1_800_000_000_000, user_provenance(), draft()).unwrap();
    let reference = first.reference();

    assert_eq!(first.content_digest(), second.content_digest());
    assert_eq!(reference.skill_id(), id(1));
    assert_eq!(reference.skill_version_id(), version_id(101));
    assert_eq!(reference.version(), ObjectVersion::new(1).unwrap());
    assert_eq!(reference.content_digest(), first.content_digest());
}

#[test]
fn content_digest_ignores_record_metadata_and_normalizes_line_endings() {
    let lf = SkillDraft::new(
        "Evidence Review".to_owned(),
        "Check sources.\nKeep context.".to_owned(),
        "Use when evidence quality matters.".to_owned(),
        vec!["research".to_owned()],
        "Compare claims to evidence.\nCite the primary source.".to_owned(),
        vec![SkillResource {
            name: "Checklist".to_owned(),
            body: "Verify the date.\nVerify the source.".to_owned(),
        }],
    )
    .unwrap();
    let crlf = SkillDraft::new(
        "Evidence Review".to_owned(),
        "Check sources.\r\nKeep context.".to_owned(),
        "Use when evidence quality matters.".to_owned(),
        vec!["research".to_owned()],
        "Compare claims to evidence.\r\nCite the primary source.".to_owned(),
        vec![SkillResource {
            name: "Checklist".to_owned(),
            body: "Verify the date.\r\nVerify the source.".to_owned(),
        }],
    )
    .unwrap();
    let user_record = SkillVersion::create(id(1), version_id(101), 1_800_000_000_000, user_provenance(), lf)
        .unwrap();
    let builtin_record = SkillVersion::create(
        id(2),
        version_id(202),
        1_800_000_000_123,
        SkillProvenance::BuiltIn {
            manifest_id: "evidence-review".to_owned(),
            manifest_version: 7,
            manifest_digest: sha256(b"manifest-v7"),
        },
        crlf,
    )
    .unwrap();

    assert_eq!(user_record.content_digest(), builtin_record.content_digest());
}

#[test]
fn projection_deterministically_inserts_and_moves_active_version() {
    let first = SkillVersion::create(id(1), version_id(101), 1_800_000_000_000, user_provenance(), draft()).unwrap();
    let mut changed = first.content().clone();
    changed.description.push_str(" Updated.");
    let second = SkillVersion::next_version(&first, version_id(102), 1_800_000_000_001, changed).unwrap();
    let mut projection = SkillsProjection::default();

    projection.insert(&first).unwrap();
    projection.activate(&second, first.skill_version_id()).unwrap();

    assert_eq!(projection.active_skill(id(1)).unwrap().skill_version_id(), version_id(102));
    assert_eq!(projection.history(id(1)), vec![first, second]);
}

fn accept_draft(draft: SkillDraft) -> Result<SkillDraft, ai_stock_forum::domain::DomainError> {
    SkillDraft::new(
        draft.display_name,
        draft.description,
        draft.use_when,
        draft.tags,
        draft.instructions,
        draft.resources,
    )
}

fn maximum_payload_draft(escaped_bytes: usize) -> SkillDraft {
    let body = format!(
        "{}{}",
        "\"".repeat(escaped_bytes),
        "b".repeat(16_384 - escaped_bytes),
    );
    SkillDraft {
        display_name: "n".repeat(64),
        description: "d".repeat(256),
        use_when: "u".repeat(512),
        tags: (0..8).map(|index| format!("t{index:0>31}")).collect(),
        instructions: "i".repeat(4_096),
        resources: body
            .as_bytes()
            .chunks(2_048)
            .enumerate()
            .map(|(index, chunk)| SkillResource {
                name: format!("r{index:0>63}"),
                body: String::from_utf8(chunk.to_vec()).expect("ASCII body"),
            })
            .collect(),
    }
}
