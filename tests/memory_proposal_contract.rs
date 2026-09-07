use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, EventId, MemoryEntryId,
        MemoryEntryVersionId, MemoryNamespaceId, MemoryProposalId,
    },
    memory::{
        ExpectedMemoryEntryState, MemoryEntryDraft, MemoryProposal, MemoryProposalOperation,
        MemoryProposalResolution, MemoryProposalStatus,
    },
    policy::{ApprovalAction, ApprovalRecord, ApprovalStatus},
};
use uuid::Uuid;

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn profile() -> AgentProfileVersion {
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

fn draft() -> MemoryEntryDraft {
    MemoryEntryDraft::new(
        "Portfolio Thesis".to_owned(),
        "Own durable companies.".to_owned(),
        vec!["investing".to_owned()],
    )
    .unwrap()
}

fn pending_proposal() -> MemoryProposal {
    let profile = profile();
    MemoryProposal::new(
        MemoryProposalId::from_uuid(uuid(4)),
        &profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set { candidate: draft() },
        "Portfolio Thesis".to_owned(),
        ExpectedMemoryEntryState::Absent,
        "Preserves the current investment thesis.".to_owned(),
        20,
        EventId::from_uuid(uuid(5)),
        ApprovalId::from_uuid(uuid(6)),
    )
    .unwrap()
}

#[test]
fn proposal_requires_the_exact_profile_agent_and_derives_namespace_and_key() {
    let profile = profile();
    let proposal = pending_proposal();

    assert_eq!(proposal.proposer(), &profile.reference());
    assert_eq!(proposal.namespace_id(), profile.memory_namespace_id());
    assert_eq!(proposal.normalized_key().as_str(), "portfolio thesis");
    assert_eq!(proposal.reference().version().get(), 1);
    assert!(
        MemoryProposal::new(
            MemoryProposalId::from_uuid(uuid(7)),
            &profile,
            &Actor::Agent(AgentProfileId::from_uuid(uuid(8))),
            MemoryProposalOperation::Set { candidate: draft() },
            "Portfolio Thesis".to_owned(),
            ExpectedMemoryEntryState::Absent,
            "Mismatch must fail.".to_owned(),
            20,
            EventId::from_uuid(uuid(9)),
            ApprovalId::from_uuid(uuid(10)),
        )
        .is_err()
    );
}

#[test]
fn proposal_enforces_operation_expected_state_and_plaintext_rationale() {
    let profile = profile();
    assert!(
        MemoryProposal::new(
            MemoryProposalId::from_uuid(uuid(11)),
            &profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Delete,
            "Portfolio Thesis".to_owned(),
            ExpectedMemoryEntryState::Absent,
            "Deleting an absent entry has no effect.".to_owned(),
            20,
            EventId::from_uuid(uuid(12)),
            ApprovalId::from_uuid(uuid(13)),
        )
        .is_err()
    );
    assert!(
        MemoryProposal::new(
            MemoryProposalId::from_uuid(uuid(14)),
            &profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Set { candidate: draft() },
            "Portfolio Thesis".to_owned(),
            ExpectedMemoryEntryState::Absent,
            format!("Authorization: Bearer {}", "a".repeat(24)),
            20,
            EventId::from_uuid(uuid(15)),
            ApprovalId::from_uuid(uuid(16)),
        )
        .is_err()
    );
    let present = ai_stock_forum::memory::MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(uuid(19)),
        MemoryEntryVersionId::from_uuid(uuid(20)),
        draft(),
        Actor::Human,
        10,
        None,
        EventId::from_uuid(uuid(21)),
    )
    .unwrap();
    let tombstone = present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(uuid(22)),
            Actor::Human,
            11,
            None,
            EventId::from_uuid(uuid(23)),
        )
        .unwrap();
    assert!(
        MemoryProposal::new(
            MemoryProposalId::from_uuid(uuid(24)),
            &profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Set { candidate: draft() },
            "Portfolio Thesis".to_owned(),
            ExpectedMemoryEntryState::Present(tombstone.reference()),
            "The state tag must match the referenced entry.".to_owned(),
            20,
            EventId::from_uuid(uuid(25)),
            ApprovalId::from_uuid(uuid(26)),
        )
        .is_err()
    );
    assert!(
        MemoryProposal::new(
            MemoryProposalId::from_uuid(uuid(27)),
            &profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Set { candidate: draft() },
            "Portfolio Thesis".to_owned(),
            ExpectedMemoryEntryState::Present(present.reference()),
            "Identical candidates must not consume a proposal.".to_owned(),
            20,
            EventId::from_uuid(uuid(28)),
            ApprovalId::from_uuid(uuid(29)),
        )
        .is_err()
    );
    for unsafe_rationale in ["Tabs\tare deceptive", "Bidi \u{202e}review"] {
        assert!(
            MemoryProposal::new(
                MemoryProposalId::from_uuid(uuid(30)),
                &profile,
                &Actor::Agent(profile.profile_id()),
                MemoryProposalOperation::Set { candidate: draft() },
                "Portfolio Thesis".to_owned(),
                ExpectedMemoryEntryState::Absent,
                unsafe_rationale.to_owned(),
                20,
                EventId::from_uuid(uuid(31)),
                ApprovalId::from_uuid(uuid(32)),
            )
            .is_err(),
            "{unsafe_rationale:?} must be rejected"
        );
    }
}

#[test]
fn terminal_memory_resolution_requires_human_and_matching_terminal_status() {
    let proposal = pending_proposal();
    assert!(
        MemoryProposalResolution::new(
            proposal.reference(),
            MemoryProposalStatus::Pending,
            proposal.approval_id(),
            Actor::Human,
            30,
            EventId::from_uuid(uuid(17)),
        )
        .is_err()
    );
    assert!(
        MemoryProposalResolution::new(
            proposal.reference(),
            MemoryProposalStatus::Accepted,
            proposal.approval_id(),
            Actor::Agent(profile().profile_id()),
            30,
            EventId::from_uuid(uuid(18)),
        )
        .is_err()
    );

    let approval = ApprovalRecord::builder(ApprovalAction::MemoryMutation)
        .approval_id(proposal.approval_id())
        .object(proposal.object_ref().unwrap())
        .actor(Actor::Agent(profile().profile_id()))
        .created_at_millis(20)
        .build()
        .unwrap();
    assert!(
        approval
            .resolve(ApprovalStatus::Cancelled, Actor::Human, 30)
            .is_err()
    );
    assert!(
        approval
            .resolve(
                ApprovalStatus::Accepted,
                Actor::Agent(profile().profile_id()),
                30
            )
            .is_err()
    );
    let resolved = approval
        .resolve(ApprovalStatus::Accepted, Actor::Human, 30)
        .unwrap();
    assert_eq!(resolved.status(), ApprovalStatus::Accepted);
    assert!(
        resolved
            .resolve(ApprovalStatus::Rejected, Actor::Human, 31)
            .is_err()
    );
}

#[test]
fn proposal_and_resolution_serde_reject_tampered_or_nonterminal_records() {
    let proposal = pending_proposal();
    let mut encoded = serde_json::to_value(&proposal).unwrap();
    encoded["rationale"] = serde_json::json!("Tampered rationale.");
    assert!(serde_json::from_value::<MemoryProposal>(encoded).is_err());

    let resolution = MemoryProposalResolution::new(
        proposal.reference(),
        MemoryProposalStatus::Rejected,
        proposal.approval_id(),
        Actor::Human,
        30,
        EventId::from_uuid(uuid(40)),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_value::<MemoryProposalResolution>(
            serde_json::to_value(&resolution).unwrap()
        )
        .unwrap(),
        resolution
    );
    let mut invalid = serde_json::to_value(resolution).unwrap();
    invalid["status"] = serde_json::json!("Pending");
    assert!(serde_json::from_value::<MemoryProposalResolution>(invalid).is_err());
}
