use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    domain::{
        AgentProfileId, AgentProfileVersionId, EpisodicSummaryId, EventId, MemoryNamespaceId,
        canonical_json_bytes, sha256,
    },
    memory::{EpisodicQualification, EpisodicSourceRef, EpisodicSummary},
};
use serde_json::Value;
use uuid::Uuid;

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn event_id(value: u128) -> EventId {
    EventId::from_uuid(uuid(value))
}

fn summary_id(value: u128) -> EpisodicSummaryId {
    EpisodicSummaryId::from_uuid(uuid(value))
}

fn profile_version_fixture() -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(uuid(1)),
        AgentProfileVersionId::from_uuid(uuid(2)),
        MemoryNamespaceId::from_uuid(uuid(3)),
        10,
        AgentProfileDraft::new(
            "Research Analyst".to_owned(),
            "An immutable research profile.".to_owned(),
            AgentRole::Custom,
            "equity research".to_owned(),
            vec!["valuation".to_owned()],
            "Deliberate and concise.".to_owned(),
            "Assess evidence before answering.".to_owned(),
            AgentBindings::default(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn source_refs_fixture() -> Vec<EpisodicSourceRef> {
    vec![
        EpisodicSourceRef::new(
            4,
            event_id(4),
            "MemoryEntryCreated".to_owned(),
            sha256(b"4"),
        )
        .unwrap(),
        EpisodicSourceRef::new(
            5,
            event_id(5),
            "MemoryEntryUpdated".to_owned(),
            sha256(b"5"),
        )
        .unwrap(),
    ]
}

fn summary_fixture() -> EpisodicSummary {
    EpisodicSummary::new(
        summary_id(1),
        &profile_version_fixture(),
        "Weekly thesis".to_owned(),
        "Evidence-backed summary".to_owned(),
        vec!["analysis".to_owned()],
        source_refs_fixture(),
        30,
        10,
        event_id(10),
    )
    .unwrap()
}

#[test]
fn episodic_sources_must_be_unique_and_strictly_in_sequence_order() {
    let mut sources = source_refs_fixture();
    sources.swap(0, 1);
    let error = EpisodicSummary::new(
        summary_id(1),
        &profile_version_fixture(),
        "Weekly thesis".to_owned(),
        "Evidence-backed summary".to_owned(),
        vec!["analysis".to_owned()],
        sources,
        30,
        10,
        event_id(10),
    )
    .unwrap_err();
    assert_eq!(error.code(), "episodic_sources_not_ordered");
}

#[test]
fn summary_pins_the_exact_profile_namespace_and_immutable_digests() {
    let profile = profile_version_fixture();
    let summary = summary_fixture();

    assert_eq!(summary.reference().profile(), &profile.reference());
    assert_eq!(
        summary.reference().namespace_id(),
        profile.memory_namespace_id()
    );
    assert_eq!(summary.reference().version().get(), 1);
    assert_eq!(summary.record_digest(), summary_fixture().record_digest());
    assert_ne!(summary.source_set_digest(), &sha256(b"different"));
}

#[test]
fn summary_rejects_boundaries_credentials_and_invalid_sources() {
    let profile = profile_version_fixture();
    assert!(
        EpisodicSummary::new(
            summary_id(2),
            &profile,
            "".to_owned(),
            "body".to_owned(),
            vec![],
            source_refs_fixture(),
            30,
            10,
            event_id(10),
        )
        .is_err()
    );
    assert!(
        EpisodicSummary::new(
            summary_id(2),
            &profile,
            "label".to_owned(),
            format!("Authorization: Bearer {}", "a".repeat(24)),
            vec![],
            source_refs_fixture(),
            30,
            10,
            event_id(10),
        )
        .is_err()
    );
    assert!(
        EpisodicSummary::new(
            summary_id(2),
            &profile,
            "label".to_owned(),
            "body".to_owned(),
            vec!["general".to_owned()],
            source_refs_fixture(),
            30,
            10,
            event_id(10),
        )
        .is_err()
    );
    assert!(
        EpisodicSummary::new(
            summary_id(2),
            &profile,
            "label".to_owned(),
            "body".to_owned(),
            vec![],
            vec![
                EpisodicSourceRef::new(10, event_id(10), "Event".to_owned(), sha256(b"10"))
                    .unwrap()
            ],
            30,
            10,
            event_id(10),
        )
        .is_err()
    );
    assert!(
        EpisodicSummary::new(
            summary_id(2),
            &profile,
            "label".to_owned(),
            "body".to_owned(),
            vec![],
            Vec::new(),
            30,
            10,
            event_id(10),
        )
        .is_err()
    );
}

#[test]
fn summary_rejects_duplicate_sources_and_more_than_the_hard_source_limit() {
    let profile = profile_version_fixture();
    let duplicate = vec![
        EpisodicSourceRef::new(1, event_id(1), "Event".to_owned(), sha256(b"1")).unwrap(),
        EpisodicSourceRef::new(2, event_id(1), "Event".to_owned(), sha256(b"2")).unwrap(),
    ];
    assert_eq!(
        EpisodicSummary::new(
            summary_id(3),
            &profile,
            "label".to_owned(),
            "body".to_owned(),
            vec![],
            duplicate,
            30,
            10,
            event_id(10),
        )
        .unwrap_err()
        .code(),
        "episodic_sources_not_unique"
    );
    let many = (1..=129)
        .map(|sequence| {
            EpisodicSourceRef::new(
                sequence,
                event_id(sequence as u128),
                "Event".to_owned(),
                sha256(&sequence.to_le_bytes()),
            )
            .unwrap()
        })
        .collect();
    assert!(
        EpisodicSummary::new(
            summary_id(4),
            &profile,
            "label".to_owned(),
            "body".to_owned(),
            vec![],
            many,
            30,
            130,
            event_id(130),
        )
        .is_err()
    );
}

#[test]
fn summary_serialization_rejects_tampering_and_noncanonical_plaintext() {
    let summary = summary_fixture();
    let encoded: Value = serde_json::from_slice(&canonical_json_bytes(&summary).unwrap()).unwrap();

    let mut digest = encoded.clone();
    digest["record_digest"] = serde_json::json!(sha256(b"tampered").as_str());
    assert!(serde_json::from_value::<EpisodicSummary>(digest).is_err());

    let mut label = encoded;
    label["label"] = serde_json::json!(" Weekly thesis ");
    assert!(serde_json::from_value::<EpisodicSummary>(label).is_err());
}

#[test]
fn summary_accepts_exact_utf8_boundaries_and_rejects_one_byte_over() {
    let profile = profile_version_fixture();
    let label = "é".repeat(64);
    let body = "é".repeat(4_096);
    assert!(
        EpisodicSummary::new(
            summary_id(20),
            &profile,
            label.clone(),
            body.clone(),
            vec![],
            source_refs_fixture(),
            30,
            10,
            event_id(10),
        )
        .is_ok()
    );
    assert!(
        EpisodicSummary::new(
            summary_id(21),
            &profile,
            format!("{label}a"),
            body.clone(),
            vec![],
            source_refs_fixture(),
            30,
            10,
            event_id(10),
        )
        .is_err()
    );
    assert!(
        EpisodicSummary::new(
            summary_id(22),
            &profile,
            label,
            format!("{body}a"),
            vec![],
            source_refs_fixture(),
            30,
            10,
            event_id(10),
        )
        .is_err()
    );
}

#[test]
fn summary_accepts_exact_source_limit_and_rejects_fixed_version_and_source_tampering() {
    let profile = profile_version_fixture();
    let sources = (1..=128)
        .map(|sequence| {
            EpisodicSourceRef::new(
                sequence,
                event_id(sequence as u128),
                "MemoryEntryCreated".to_owned(),
                sha256(&sequence.to_le_bytes()),
            )
            .unwrap()
        })
        .collect();
    let summary = EpisodicSummary::new(
        summary_id(30),
        &profile,
        "label".to_owned(),
        "body".to_owned(),
        vec![],
        sources,
        30,
        129,
        event_id(129),
    )
    .unwrap();
    let encoded: Value = serde_json::from_slice(&canonical_json_bytes(&summary).unwrap()).unwrap();
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("plaintext_validation_version", serde_json::json!(2)),
    ] {
        let mut tampered = encoded.clone();
        tampered[field] = value;
        assert!(serde_json::from_value::<EpisodicSummary>(tampered).is_err());
    }
    let mut source_type = encoded.clone();
    source_type["sources"][0]["event_type"] = serde_json::json!("OtherEvent");
    assert!(serde_json::from_value::<EpisodicSummary>(source_type).is_err());
    let mut source_digest = encoded;
    source_digest["sources"][0]["event_digest"] = serde_json::json!(sha256(b"other").as_str());
    assert!(serde_json::from_value::<EpisodicSummary>(source_digest).is_err());
}

#[test]
fn summary_exposes_the_required_verify_sources_qualification() {
    let qualification = EpisodicQualification::SummaryVerifySources;
    assert_eq!(qualification.label(), "Summary — verify sources");
    assert_eq!(
        serde_json::to_value(qualification).unwrap(),
        "Summary — verify sources"
    );
}
