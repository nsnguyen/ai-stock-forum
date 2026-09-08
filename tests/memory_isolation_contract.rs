mod support;

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, ApplicationCommand, CommandEnvelope, CommandOutcome, CommandView,
        MemoryEditPreview,
    },
    domain::{Actor, CommandId, CorrelationId, EventId, ObjectVersion},
    memory::{
        ExpectedMemoryEntryState, MemoryEntryDraft, MemoryProposalFilter, MemoryProposalOperation,
        MemoryProposalRef, MemoryPurposeScope, MemoryRetrievalBudget, MemoryRetrievalRequest,
        MemoryRetrievalScope,
    },
};
use uuid::Uuid;

fn envelope(id: u128, actor: Actor, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 100_000)),
        actor,
        command,
    }
}

fn draft(name: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.into(),
        "Namespace isolation fixture.".into(),
        AgentRole::Custom,
        "research".into(),
        vec![],
        "Careful.".into(),
        "Use evidence.".into(),
        AgentBindings::default(),
        vec![],
        vec![],
    )
    .unwrap()
}

fn create_profile(app: &mut support::TestApp, id: u128, name: &str) -> AgentProfileVersion {
    app.execute(envelope(
        id,
        Actor::Human,
        ApplicationCommand::CreateAgentProfile {
            draft: draft(name),
            template_provenance: None,
        },
    ))
    .unwrap();
    app.projection()
        .agent_profiles
        .active_profiles()
        .into_iter()
        .find(|profile| profile.display_name() == name)
        .unwrap()
}

fn memory_candidate(key: &str, value: &str, tag: &str) -> MemoryEntryDraft {
    MemoryEntryDraft::new(key.into(), value.into(), vec![tag.into()]).unwrap()
}

fn set_entry(
    app: &mut support::TestApp,
    profile: &AgentProfileVersion,
    id: u128,
    key: &str,
    value: &str,
) -> (ai_stock_forum::memory::MemoryEntryRef, EventId) {
    let review = match app
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            memory_candidate(key, value, "shared"),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected review, got {other:?}"),
    };
    let outcome = app
        .execute(envelope(
            id,
            Actor::Human,
            ApplicationCommand::SetMemoryEntry {
                profile: review.profile,
                expected: review.expected,
                candidate: review.candidate.unwrap(),
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap();
    let event_id = outcome.committed_events[0].event_id;
    match outcome.view {
        CommandView::MemoryEntryMutation(view) => (view.entry, event_id),
        other => panic!("expected mutation view, got {other:?}"),
    }
}

fn propose(
    app: &mut support::TestApp,
    profile: &AgentProfileVersion,
    id: u128,
) -> MemoryProposalRef {
    let outcome = app
        .execute(envelope(
            id,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: memory_candidate(
                        "Shared proposal key",
                        &format!("{} proposal plaintext", profile.display_name()),
                        "shared",
                    ),
                },
                rationale: format!("{} private rationale", profile.display_name()),
            },
        ))
        .unwrap();
    match outcome.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("expected proposal view, got {other:?}"),
    }
}

fn run_read(
    app: &mut support::TestApp,
    id: &mut u128,
    command: ApplicationCommand,
) -> CommandOutcome {
    let outcome = app.execute(envelope(*id, Actor::Human, command)).unwrap();
    *id += 1;
    outcome
}

#[test]
fn overlapping_namespaces_remain_isolated_across_every_read_and_retrieval_surface() {
    let mut app = support::app();
    let alpha = create_profile(&mut app, 2_000_000, "Isolation Alpha");
    let beta = create_profile(&mut app, 2_000_001, "Isolation Beta");
    assert_ne!(alpha.memory_namespace_id(), beta.memory_namespace_id());
    let (alpha_entry, alpha_event) = set_entry(
        &mut app,
        &alpha,
        2_000_010,
        "Shared key",
        "alpha-only plaintext",
    );
    let (beta_entry, beta_event) = set_entry(
        &mut app,
        &beta,
        2_000_011,
        "Shared key",
        "beta-only plaintext",
    );
    let alpha_proposal = propose(&mut app, &alpha, 2_000_020);
    let beta_proposal = propose(&mut app, &beta, 2_000_021);
    let alpha_summary = app
        .record_test_episodic_summary(
            alpha.reference(),
            "Alpha summary".into(),
            "alpha-only episode".into(),
            vec!["shared".into()],
            vec![alpha_event],
        )
        .unwrap();
    let beta_summary = app
        .record_test_episodic_summary(
            beta.reference(),
            "Beta summary".into(),
            "beta-only episode".into(),
            vec!["shared".into()],
            vec![beta_event],
        )
        .unwrap();

    let mut id = 2_001_000;
    let CommandView::MemoryEntries(entries) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::ListMemoryEntries {
            selector: alpha.profile_id().into(),
        },
    )
    .view
    else {
        panic!("memory entries view")
    };
    assert_eq!(entries.namespace_id, alpha.memory_namespace_id());
    assert_eq!(entries.entries.len(), 1);
    assert_eq!(entries.entries[0].entry, alpha_entry);

    let CommandView::MemoryEntry(detail) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::ShowMemoryEntry {
            selector: alpha.profile_id().into(),
            display_key: "Shared key".into(),
        },
    )
    .view
    else {
        panic!("memory entry view")
    };
    assert_eq!(detail.entry.reference(), alpha_entry);
    assert_eq!(detail.entry.value(), Some("alpha-only plaintext"));

    let CommandView::MemoryEntryHistory(history) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::ShowMemoryEntryHistory {
            selector: alpha.profile_id().into(),
            display_key: "Shared key".into(),
        },
    )
    .view
    else {
        panic!("memory history view")
    };
    assert_eq!(history.current, alpha_entry);
    assert_eq!(history.versions.len(), 1);
    assert_eq!(history.versions[0].entry, alpha_entry);

    let CommandView::MemoryEntryVersion(version) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::ShowMemoryEntryVersion {
            selector: alpha.profile_id().into(),
            display_key: "Shared key".into(),
            version: ObjectVersion::new(1).unwrap(),
        },
    )
    .view
    else {
        panic!("memory version view")
    };
    assert_eq!(version.entry.reference(), alpha_entry);

    let CommandView::MemoryProposals(proposals) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::ListMemoryProposals {
            selector: alpha.profile_id().into(),
            filter: MemoryProposalFilter::All,
        },
    )
    .view
    else {
        panic!("memory proposals view")
    };
    assert_eq!(proposals.namespace_id, alpha.memory_namespace_id());
    assert_eq!(proposals.proposals.len(), 1);
    assert_eq!(proposals.proposals[0].proposal, alpha_proposal);

    let CommandView::MemoryProposal(proposal) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::ShowMemoryProposal {
            proposal_id: alpha_proposal.proposal_id(),
        },
    )
    .view
    else {
        panic!("memory proposal detail view")
    };
    assert_eq!(proposal.proposal.reference(), alpha_proposal);
    assert_eq!(
        proposal.proposal.namespace_id(),
        alpha.memory_namespace_id()
    );

    let CommandView::EpisodicSummaries(summaries) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::ListEpisodicSummaries {
            selector: alpha.profile_id().into(),
        },
    )
    .view
    else {
        panic!("episodic summaries view")
    };
    assert_eq!(summaries.namespace_id, alpha.memory_namespace_id());
    assert_eq!(summaries.summaries.len(), 1);
    assert_eq!(summaries.summaries[0].summary, alpha_summary);

    let CommandView::EpisodicSummary(summary) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::ShowEpisodicSummary {
            summary_id: alpha_summary.summary_id(),
        },
    )
    .view
    else {
        panic!("episodic summary detail view")
    };
    assert_eq!(summary.summary.reference(), alpha_summary);
    assert_eq!(
        summary.summary.reference().namespace_id(),
        alpha.memory_namespace_id()
    );

    let snapshot_request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &alpha,
            MemoryPurposeScope::tagged(vec!["shared".into()]).unwrap(),
        )
        .unwrap(),
        MemoryRetrievalBudget::default(),
    )
    .unwrap();
    let CommandView::MemorySnapshot(snapshot) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::BuildMemorySnapshot {
            request: snapshot_request,
        },
    )
    .view
    else {
        panic!("memory snapshot view")
    };
    assert_eq!(snapshot.snapshot.entries().len(), 1);
    assert_eq!(snapshot.snapshot.entries()[0].entry(), &alpha_entry);
    assert_eq!(snapshot.snapshot.summaries().len(), 1);
    assert_eq!(snapshot.snapshot.summaries()[0].summary(), &alpha_summary);

    let alpha_json = serde_json::to_string(&(
        entries, detail, history, version, proposals, proposal, summaries, summary, snapshot,
    ))
    .unwrap();
    for foreign in [
        beta_entry.entry_id().to_string(),
        beta_proposal.proposal_id().to_string(),
        beta_summary.summary_id().to_string(),
        "beta-only plaintext".into(),
        "beta-only episode".into(),
    ] {
        assert!(!alpha_json.contains(&foreign), "leaked {foreign}");
    }

    let beta_snapshot = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &beta,
            MemoryPurposeScope::tagged(vec!["shared".into()]).unwrap(),
        )
        .unwrap(),
        MemoryRetrievalBudget::default(),
    )
    .unwrap();
    let CommandView::MemorySnapshot(beta_view) = run_read(
        &mut app,
        &mut id,
        ApplicationCommand::BuildMemorySnapshot {
            request: beta_snapshot,
        },
    )
    .view
    else {
        panic!("beta snapshot view")
    };
    assert_eq!(beta_view.snapshot.entries()[0].entry(), &beta_entry);
    assert_eq!(beta_view.snapshot.summaries()[0].summary(), &beta_summary);
}

#[test]
fn active_profile_v2_retrieves_v1_memory_from_the_same_namespace() {
    let mut app = support::app();
    let v1 = create_profile(&mut app, 2_100_000, "Versioned memory owner");
    let (entry, _) = set_entry(&mut app, &v1, 2_100_001, "Thesis", "version-one memory");
    let memory_rows = app.count_rows("memory_entry_versions");
    let mut successor = v1.to_draft();
    successor.description = "Active version two.".into();
    let preview = app
        .preview_agent_profile_edit(v1.profile_id(), v1.profile_version_id(), successor.clone())
        .unwrap();
    app.execute(envelope(
        2_100_002,
        Actor::Human,
        ApplicationCommand::ActivateAgentProfileVersion {
            profile_id: v1.profile_id(),
            expected_active_version_id: v1.profile_version_id(),
            candidate: successor,
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    ))
    .unwrap();
    let v2 = app
        .projection()
        .agent_profiles
        .active_profile(v1.profile_id())
        .unwrap()
        .clone();
    assert_eq!(v2.version().get(), 2);
    assert_eq!(v2.memory_namespace_id(), v1.memory_namespace_id());
    assert_eq!(app.count_rows("memory_entry_versions"), memory_rows);

    let CommandView::MemoryEntries(list) = app
        .execute(envelope(
            2_100_003,
            Actor::Human,
            ApplicationCommand::ListMemoryEntries {
                selector: v2.profile_id().into(),
            },
        ))
        .unwrap()
        .view
    else {
        panic!("memory entries view")
    };
    assert_eq!(list.profile, v2.reference());
    assert_eq!(list.entries[0].entry, entry);
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &v2,
            MemoryPurposeScope::tagged(vec!["shared".into()]).unwrap(),
        )
        .unwrap(),
        MemoryRetrievalBudget::default(),
    )
    .unwrap();
    let CommandView::MemorySnapshot(view) = app
        .execute(envelope(
            2_100_004,
            Actor::Human,
            ApplicationCommand::BuildMemorySnapshot { request },
        ))
        .unwrap()
        .view
    else {
        panic!("memory snapshot view")
    };
    assert_eq!(view.snapshot.entries()[0].entry(), &entry);
    assert_eq!(view.snapshot.entries()[0].value(), "version-one memory");
}

#[test]
fn copied_profile_starts_with_a_fresh_empty_namespace() {
    let mut app = support::app();
    let source = create_profile(&mut app, 2_200_000, "Copy source");
    set_entry(&mut app, &source, 2_200_001, "Thesis", "source-only memory");
    let memory_rows = app.count_rows("memory_entry_versions");
    let mut copied_draft = source.to_draft();
    copied_draft.display_name = "Copied profile".into();
    app.execute(envelope(
        2_200_002,
        Actor::Human,
        ApplicationCommand::CreateAgentProfile {
            draft: copied_draft,
            template_provenance: None,
        },
    ))
    .unwrap();
    let copy = app
        .projection()
        .agent_profiles
        .active_profiles()
        .into_iter()
        .find(|profile| profile.display_name() == "Copied profile")
        .unwrap();
    assert_ne!(copy.memory_namespace_id(), source.memory_namespace_id());
    assert_eq!(app.count_rows("memory_entry_versions"), memory_rows);

    let CommandView::MemoryEntries(list) = app
        .execute(envelope(
            2_200_003,
            Actor::Human,
            ApplicationCommand::ListMemoryEntries {
                selector: copy.profile_id().into(),
            },
        ))
        .unwrap()
        .view
    else {
        panic!("memory entries view")
    };
    assert!(list.entries.is_empty());
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(&copy, MemoryPurposeScope::General).unwrap(),
        MemoryRetrievalBudget::default(),
    )
    .unwrap();
    let CommandView::MemorySnapshot(view) = app
        .execute(envelope(
            2_200_004,
            Actor::Human,
            ApplicationCommand::BuildMemorySnapshot { request },
        ))
        .unwrap()
        .view
    else {
        panic!("memory snapshot view")
    };
    assert!(view.snapshot.entries().is_empty());
    assert!(view.snapshot.summaries().is_empty());
}
