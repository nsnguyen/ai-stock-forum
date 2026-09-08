use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, AgentProfilesView, ApplicationCommand, CommandView,
        DatabaseReadiness, EpisodicSummariesView, EpisodicSummaryListItem, EpisodicSummaryView,
        HelpView, InputRejectedView, InputRejectionCategory, MemoryEntriesView,
        MemoryEntryHistorySummary, MemoryEntryHistoryView, MemoryEntryMutationView,
        MemoryEntrySummary, MemoryEntryVersionView, MemoryEntryView, MemoryProfileIdentityView,
        MemoryProposalCreatedView, MemoryProposalResolutionView, MemoryProposalSummary,
        MemoryProposalView, MemoryProposalsView, PresentationSnapshot, ProcessGuardOwnership,
    },
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, EpisodicSummaryId, EventId,
        InstallationId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId, MemoryProposalId,
        ObjectVersion, SessionId, sha256,
    },
    memory::{
        EpisodicQualification, EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState,
        MemoryEntryDraft, MemoryEntryVersion, MemoryProposal, MemoryProposalFilter,
        MemoryProposalOperation, MemoryProposalOperationKind, MemoryProposalResolution,
        MemoryProposalStatus,
    },
    setup::SetupStatus,
    ui::{
        command::{MemoryWorkflowCommand, ParsedLine, TextRenderer, parse_line},
        tui::{
            ControllerEffect, TuiEvent, handle_event,
            model::{AgentsPane, Focus, Severity, TuiModel, UiMessage, View},
        },
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

fn command(input: &[u8]) -> ApplicationCommand {
    match parse_line(input) {
        ParsedLine::Command(command) => command,
        ParsedLine::AgentWorkflow(_) => panic!("expected a direct command"),
        ParsedLine::SkillWorkflow(_) => panic!("expected a direct command"),
        ParsedLine::MemoryWorkflow(_) => panic!("expected a direct command"),
        ParsedLine::Ignored => panic!("expected a command"),
    }
}

fn malformed(input: &[u8]) {
    let ApplicationCommand::RejectInput(rejection) = command(input) else {
        panic!("expected a typed rejection for {input:?}");
    };
    assert_eq!(rejection.category, InputRejectionCategory::Malformed);
    assert_eq!(rejection.safe_token.as_deref(), Some("/memory"));
    let encoded = serde_json::to_string(&rejection).unwrap();
    assert!(!encoded.contains("raw_input"));
}

#[test]
fn parser_maps_all_documented_memory_read_routes_exactly() {
    let proposal_id = MemoryProposalId::from_uuid(Uuid::from_u128(41));
    let summary_id = EpisodicSummaryId::from_uuid(Uuid::from_u128(42));

    assert_eq!(
        command(b"/memory list analyst"),
        ApplicationCommand::ListMemoryEntries {
            selector: AgentProfileSelector::Name("analyst".into()),
        }
    );
    assert_eq!(
        command(b"/memory get \"Research Agent\" \"Risk Appetite\""),
        ApplicationCommand::ShowMemoryEntry {
            selector: AgentProfileSelector::Name("Research Agent".into()),
            display_key: "Risk Appetite".into(),
        }
    );
    assert_eq!(
        command(b"/memory history analyst thesis"),
        ApplicationCommand::ShowMemoryEntryHistory {
            selector: AgentProfileSelector::Name("analyst".into()),
            display_key: "thesis".into(),
        }
    );
    assert_eq!(
        command(b"/memory history analyst thesis 7"),
        ApplicationCommand::ShowMemoryEntryVersion {
            selector: AgentProfileSelector::Name("analyst".into()),
            display_key: "thesis".into(),
            version: ObjectVersion::new(7).unwrap(),
        }
    );
    assert_eq!(
        command(b"/memory proposals analyst"),
        ApplicationCommand::ListMemoryProposals {
            selector: AgentProfileSelector::Name("analyst".into()),
            filter: MemoryProposalFilter::Pending,
        }
    );
    assert_eq!(
        command(b"/memory proposals analyst pending"),
        ApplicationCommand::ListMemoryProposals {
            selector: AgentProfileSelector::Name("analyst".into()),
            filter: MemoryProposalFilter::Pending,
        }
    );
    assert_eq!(
        command(b"/memory proposals analyst all"),
        ApplicationCommand::ListMemoryProposals {
            selector: AgentProfileSelector::Name("analyst".into()),
            filter: MemoryProposalFilter::All,
        }
    );
    assert_eq!(
        command(format!("/memory proposal {proposal_id}").as_bytes()),
        ApplicationCommand::ShowMemoryProposal { proposal_id }
    );
    assert_eq!(
        command(b"/memory episodes analyst"),
        ApplicationCommand::ListEpisodicSummaries {
            selector: AgentProfileSelector::Name("analyst".into()),
        }
    );
    assert_eq!(
        command(format!("/memory episode {summary_id}").as_bytes()),
        ApplicationCommand::ShowEpisodicSummary { summary_id }
    );
}

#[test]
fn parser_maps_memory_writes_only_to_typed_workflow_intents() {
    let proposal_id = MemoryProposalId::from_uuid(Uuid::from_u128(51));

    assert_eq!(
        parse_line(b"/memory set \"Research Agent\" \"Risk Appetite\""),
        ParsedLine::MemoryWorkflow(MemoryWorkflowCommand::Set {
            agent: AgentProfileSelector::Name("Research Agent".into()),
            key: "Risk Appetite".into(),
        })
    );
    assert_eq!(
        parse_line(b"/memory delete analyst thesis"),
        ParsedLine::MemoryWorkflow(MemoryWorkflowCommand::Delete {
            agent: AgentProfileSelector::Name("analyst".into()),
            key: "thesis".into(),
        })
    );
    assert_eq!(
        parse_line(format!("/memory approve {proposal_id}").as_bytes()),
        ParsedLine::MemoryWorkflow(MemoryWorkflowCommand::Approve { proposal_id })
    );
    assert_eq!(
        parse_line(format!("/memory reject {proposal_id}").as_bytes()),
        ParsedLine::MemoryWorkflow(MemoryWorkflowCommand::Reject { proposal_id })
    );
}

#[test]
fn parser_rejects_every_unsupported_or_ambiguous_memory_shape() {
    for input in [
        b"/memory".as_slice(),
        b"/memory list",
        b"/memory list analyst extra",
        b"/memory get analyst",
        b"/memory get analyst password",
        b"/memory get analyst thesis extra",
        b"/memory history analyst",
        b"/memory history analyst \"api key\"",
        b"/memory history analyst thesis 0",
        b"/memory history analyst thesis nope",
        b"/memory history analyst thesis 18446744073709551616",
        b"/memory history analyst thesis 1 extra",
        b"/memory set analyst",
        b"/memory set analyst password",
        b"/memory set analyst thesis extra",
        b"/memory delete analyst",
        b"/memory delete analyst \"api key\"",
        b"/memory proposals",
        b"/memory proposals analyst accepted",
        b"/memory proposals analyst all extra",
        b"/memory proposal not-a-uuid",
        b"/memory approve not-a-uuid",
        b"/memory reject not-a-uuid",
        b"/memory episodes",
        b"/memory episodes analyst extra",
        b"/memory episode not-a-uuid",
        b"/memory propose analyst thesis",
        b"/memory summary create analyst",
        b"/memory snapshot analyst",
        b"/memory get \"unterminated",
        b"/memory unknown hunter2",
    ] {
        malformed(input);
    }
}

#[test]
fn bare_memory_remains_unknown_and_memory_rejections_retain_no_arguments() {
    let ApplicationCommand::RejectInput(bare) = command(b"memory list analyst") else {
        panic!("bare memory must be rejected");
    };
    assert_eq!(bare.category, InputRejectionCategory::Unknown);

    let ApplicationCommand::RejectInput(rejection) = command(b"/memory unknown hunter2") else {
        panic!("unsupported memory syntax must be rejected");
    };
    assert_eq!(rejection.category, InputRejectionCategory::Malformed);
    assert_eq!(rejection.safe_token.as_deref(), Some("/memory"));
    let encoded = serde_json::to_string(&rejection).unwrap();
    assert!(!encoded.contains("hunter2"));
    assert!(!encoded.contains("raw_input"));
}

fn profile() -> AgentProfileVersion {
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(101)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(102)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(103)),
        1_700_000_000_000,
        AgentProfileDraft::new(
            "Memory Agent".into(),
            "Fallback renderer fixture.".into(),
            AgentRole::Custom,
            "research".into(),
            vec!["analysis".into()],
            "Careful.".into(),
            "Use evidence.".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
        None,
    )
    .unwrap()
}

fn current_profile(historical: &AgentProfileVersion) -> AgentProfileVersion {
    AgentProfileVersion::next_version(
        historical,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(120)),
        1_700_000_000_001,
        AgentProfileDraft::new(
            "Memory Agent Current".into(),
            "Current fallback renderer fixture.".into(),
            AgentRole::Custom,
            "research".into(),
            vec!["analysis".into()],
            "Careful.".into(),
            "Use current evidence.".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
    )
    .unwrap()
}

fn entry(profile: &AgentProfileVersion) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(104)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(105)),
        MemoryEntryDraft::new(
            "Earnings Thesis".into(),
            "private thesis\nsecond line".into(),
            vec!["Catalyst".into()],
        )
        .unwrap(),
        Actor::Human,
        1_700_000_000_010,
        None,
        EventId::from_uuid(Uuid::from_u128(106)),
    )
    .unwrap()
}

fn proposal(profile: &AgentProfileVersion) -> MemoryProposal {
    MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(107)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                "Earnings Thesis".into(),
                "private proposed value\nsecond line".into(),
                vec!["Catalyst".into()],
            )
            .unwrap(),
        },
        "Earnings Thesis".into(),
        ExpectedMemoryEntryState::Absent,
        "private rationale\nsecond line".into(),
        1_700_000_000_020,
        EventId::from_uuid(Uuid::from_u128(108)),
        ApprovalId::from_uuid(Uuid::from_u128(109)),
    )
    .unwrap()
}

fn episodic_summary(profile: &AgentProfileVersion) -> EpisodicSummary {
    EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::from_u128(110)),
        profile,
        "Quarterly synthesis".into(),
        "private summary body\nverify each source".into(),
        vec!["Catalyst".into()],
        vec![
            EpisodicSourceRef::new(
                17,
                EventId::from_uuid(Uuid::from_u128(111)),
                "memory_entry_set".into(),
                sha256(b"episodic source"),
            )
            .unwrap(),
        ],
        1_700_000_000_030,
        18,
        EventId::from_uuid(Uuid::from_u128(112)),
    )
    .unwrap()
}

fn render(view: CommandView) -> String {
    let mut bytes = Vec::new();
    TextRenderer::render_view(&view, &mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn tui_model() -> TuiModel {
    TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(201)),
            session_id: SessionId::from_uuid(Uuid::from_u128(202)),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: vec![],
            agent_profiles: AgentProfilesView {
                profiles: vec![],
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            selected_agent_profile: None,
            selected_agent_profile_history: None,
        },
        false,
    )
}

fn assert_exact_memory_usage(text: &str) {
    for route in [
        "/memory list <agent>",
        "/memory get <agent> <key>",
        "/memory history <agent> <key> [positive-version]",
        "/memory set <agent> <key>",
        "/memory delete <agent> <key>",
        "/memory proposals <agent> [pending|all]",
        "/memory proposal <proposal-id>",
        "/memory approve <proposal-id>",
        "/memory reject <proposal-id>",
        "/memory episodes <agent>",
        "/memory episode <summary-id>",
    ] {
        assert_exact_line(text, &format!("  {route}"));
    }
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("  /memory "))
            .count(),
        11
    );
    for unsupported in [
        "  /memory propose ",
        "  /memory summary ",
        "  /memory snapshot ",
    ] {
        assert!(!text.contains(unsupported));
    }
    assert!(!text.lines().any(|line| line == "  /memory"));
}

fn assert_exact_line(text: &str, expected: &str) {
    assert!(
        text.lines().any(|line| line == expected),
        "missing exact line `{expected}` in:\n{text}"
    );
}

#[test]
fn memory_list_renderers_cap_rows_escape_terminal_text_and_omit_detail_prose() {
    let profile = profile();
    let entry = entry(&profile);
    let proposal = proposal(&profile);
    let summary = episodic_summary(&profile);

    let mut entries = (0..100)
        .map(|index| MemoryEntrySummary {
            entry: entry.reference(),
            display_key: if index == 0 {
                "\u{1b}[31mHostile Key".into()
            } else {
                format!("Visible Key {index}")
            },
            purpose_tags: vec!["tag\nline".into()],
            value_bytes: 26,
            created_at_ms: entry.created_at_ms(),
        })
        .collect::<Vec<_>>();
    entries.push(MemoryEntrySummary {
        entry: entry.reference(),
        display_key: "ENTRY_SENTINEL_MUST_NOT_RENDER".into(),
        purpose_tags: vec![],
        value_bytes: 26,
        created_at_ms: entry.created_at_ms(),
    });
    let entries_text = render(CommandView::MemoryEntries(MemoryEntriesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        entries,
        total_count: 101,
        returned_count: 101,
        omitted_count: 0,
    }));
    assert!(entries_text.contains("returned 100 of 101 (1 omitted)"));
    assert!(entries_text.contains("\\u{1b}[31mHostile Key"));
    assert!(entries_text.contains("tag\\nline"));
    assert!(entries_text.contains("1 entries omitted"));
    assert!(!entries_text.contains("ENTRY_SENTINEL_MUST_NOT_RENDER"));
    assert!(!entries_text.contains("private thesis"));
    assert!(!entries_text.contains('\u{1b}'));

    let mut history = (0..100)
        .map(|index| MemoryEntryHistorySummary {
            entry: entry.reference(),
            display_key: format!("History Key {index}"),
            created_at_ms: entry.created_at_ms(),
            accepted_proposal: None,
        })
        .collect::<Vec<_>>();
    history.push(MemoryEntryHistorySummary {
        entry: entry.reference(),
        display_key: "HISTORY_SENTINEL_MUST_NOT_RENDER".into(),
        created_at_ms: entry.created_at_ms(),
        accepted_proposal: None,
    });
    let history_text = render(CommandView::MemoryEntryHistory(MemoryEntryHistoryView {
        profile: profile.reference(),
        current: entry.reference(),
        versions: history,
        total_count: 101,
        returned_count: 101,
        omitted_count: 0,
    }));
    assert!(history_text.contains("returned 100 of 101 (1 omitted)"));
    assert!(history_text.contains("1 versions omitted"));
    assert!(!history_text.contains("HISTORY_SENTINEL_MUST_NOT_RENDER"));
    assert!(!history_text.contains("private thesis"));

    let mut proposals = (0..100)
        .map(|index| MemoryProposalSummary {
            proposal: proposal.reference(),
            proposer: profile.reference(),
            operation: MemoryProposalOperationKind::Set,
            display_key: format!("Proposal Key {index}"),
            status: MemoryProposalStatus::Pending,
            created_at_ms: proposal.created_at_ms(),
        })
        .collect::<Vec<_>>();
    proposals.push(MemoryProposalSummary {
        proposal: proposal.reference(),
        proposer: profile.reference(),
        operation: MemoryProposalOperationKind::Set,
        display_key: "PROPOSAL_SENTINEL_MUST_NOT_RENDER".into(),
        status: MemoryProposalStatus::Pending,
        created_at_ms: proposal.created_at_ms(),
    });
    let proposals_text = render(CommandView::MemoryProposals(MemoryProposalsView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        filter: MemoryProposalFilter::Pending,
        proposals,
        total_count: 101,
        returned_count: 101,
        omitted_count: 0,
    }));
    assert!(proposals_text.contains("returned 100 of 101 (1 omitted)"));
    assert!(proposals_text.contains("1 proposals omitted"));
    assert!(!proposals_text.contains("PROPOSAL_SENTINEL_MUST_NOT_RENDER"));
    assert!(!proposals_text.contains("private rationale"));
    assert!(!proposals_text.contains("private proposed value"));

    let mut summaries = (0..100)
        .map(|index| EpisodicSummaryListItem {
            summary: summary.reference(),
            label: format!("Summary {index}"),
            purpose_tags: vec!["Catalyst".into()],
            source_count: 1,
            created_at_ms: summary.created_at_ms(),
        })
        .collect::<Vec<_>>();
    summaries.push(EpisodicSummaryListItem {
        summary: summary.reference(),
        label: "SUMMARY_SENTINEL_MUST_NOT_RENDER".into(),
        purpose_tags: vec![],
        source_count: 1,
        created_at_ms: summary.created_at_ms(),
    });
    let summaries_text = render(CommandView::EpisodicSummaries(EpisodicSummariesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        summaries,
        total_count: 101,
        returned_count: 101,
        omitted_count: 0,
    }));
    assert!(summaries_text.contains("returned 100 of 101 (1 omitted)"));
    assert!(summaries_text.contains("1 summaries omitted"));
    assert_eq!(
        summaries_text.matches("Summary — verify sources").count(),
        100,
        "each visible episodic list item must carry the verification qualification"
    );
    assert!(!summaries_text.contains("SUMMARY_SENTINEL_MUST_NOT_RENDER"));
    assert!(!summaries_text.contains("private summary body"));
    assert!(!summaries_text.contains("memory_entry_set"));
}

#[test]
fn memory_entry_details_render_deliberate_escaped_plaintext() {
    let historical_profile = profile();
    let entry = entry(&historical_profile);

    for view in [
        CommandView::MemoryEntry(MemoryEntryView {
            profile: historical_profile.reference(),
            entry: entry.clone(),
        }),
        CommandView::MemoryEntryVersion(MemoryEntryVersionView {
            profile: historical_profile.reference(),
            entry: entry.clone(),
        }),
    ] {
        let text = render(view);
        assert!(text.contains("Earnings Thesis"));
        assert!(text.contains("private thesis\\nsecond line"));
        assert!(text.contains("Catalyst"));
        assert!(text.contains(entry.reference().content_digest().as_str()));
        assert!(!text.contains("private thesis\nsecond line"));
    }
}

#[test]
fn proposal_detail_renders_canonical_key_and_complete_historical_profile_provenance() {
    let historical_profile = profile();
    let current_profile = current_profile(&historical_profile);
    let proposal = proposal(&historical_profile);
    let proposal_text = render(CommandView::MemoryProposal(MemoryProposalView {
        proposal: proposal.clone(),
        status: MemoryProposalStatus::Pending,
        resolution: None,
        current_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: true,
        proposer_identity: MemoryProfileIdentityView {
            profile: historical_profile.reference(),
            display_name: "\u{1b}[31mHistorical Proposer".into(),
        },
        namespace_owner_identity: MemoryProfileIdentityView {
            profile: current_profile.reference(),
            display_name: "Memory Owner".into(),
        },
    }));
    assert_exact_line(&proposal_text, "Normalized key: earnings thesis");
    assert_exact_line(
        &proposal_text,
        &format!(
            "Proposer profile: {}@{}",
            historical_profile.profile_id(),
            historical_profile.version().get()
        ),
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Proposer profile version ID: {}",
            historical_profile.profile_version_id()
        ),
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Proposer profile digest: {}",
            historical_profile.content_digest()
        ),
    );
    assert_exact_line(
        &proposal_text,
        "Proposer warning: historical profile version",
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Namespace owner profile: {}@{}",
            current_profile.profile_id(),
            current_profile.version().get()
        ),
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Namespace owner profile version ID: {}",
            current_profile.profile_version_id()
        ),
    );
    assert_exact_line(
        &proposal_text,
        &format!(
            "Namespace owner profile digest: {}",
            current_profile.content_digest()
        ),
    );
    assert_ne!(
        historical_profile.profile_version_id(),
        current_profile.profile_version_id()
    );
    assert_ne!(
        historical_profile.content_digest(),
        current_profile.content_digest()
    );
    assert!(proposal_text.contains("private proposed value\\nsecond line"));
    assert!(proposal_text.contains("private rationale\\nsecond line"));
    assert!(proposal_text.contains("historical"));
    assert!(proposal_text.contains("\\u{1b}[31mHistorical Proposer"));
    assert!(!proposal_text.contains('\u{1b}'));
}

#[test]
fn episodic_detail_renders_pinned_profile_creation_and_source_provenance() {
    let historical_profile = profile();
    let summary = episodic_summary(&historical_profile);
    let summary_text = render(CommandView::EpisodicSummary(EpisodicSummaryView {
        summary: summary.clone(),
        qualification: EpisodicQualification::SummaryVerifySources,
    }));
    assert!(summary_text.contains("Summary — verify sources"));
    assert!(summary_text.contains("Quarterly synthesis"));
    assert!(summary_text.contains("private summary body\\nverify each source"));
    assert_exact_line(
        &summary_text,
        &format!(
            "Profile: {}@{}",
            historical_profile.profile_id(),
            historical_profile.version().get()
        ),
    );
    assert_exact_line(
        &summary_text,
        &format!(
            "Profile version ID: {}",
            historical_profile.profile_version_id()
        ),
    );
    assert_exact_line(
        &summary_text,
        &format!("Profile digest: {}", historical_profile.content_digest()),
    );
    assert_exact_line(
        &summary_text,
        &format!(
            "Creation event sequence: {}",
            summary.creation_event_sequence()
        ),
    );
    assert_exact_line(
        &summary_text,
        &format!("Creation event ID: {}", summary.creation_event_id()),
    );
    assert_exact_line(
        &summary_text,
        &format!("Source set digest: {}", summary.source_set_digest()),
    );
    assert_exact_line(
        &summary_text,
        &format!(
            "  sequence {} event {} type {} digest {}",
            summary.sources()[0].sequence(),
            summary.sources()[0].event_id(),
            summary.sources()[0].event_type(),
            summary.sources()[0].event_digest()
        ),
    );
    assert!(!summary_text.contains("private summary body\nverify each source"));
}

#[test]
fn memory_mutation_renderers_show_identifiers_versions_statuses_and_digests() {
    let profile = profile();
    let entry = entry(&profile);
    let proposal = proposal(&profile);
    let resolution = MemoryProposalResolution::new(
        proposal.reference(),
        MemoryProposalStatus::Accepted,
        proposal.approval_id(),
        Actor::Human,
        1_700_000_000_040,
        EventId::from_uuid(Uuid::from_u128(113)),
    )
    .unwrap();

    let mutation = render(CommandView::MemoryEntryMutation(MemoryEntryMutationView {
        entry: entry.reference(),
        expired_proposals: vec![proposal.reference()],
    }));
    assert!(mutation.contains(&entry.reference().entry_version_id().to_string()));
    assert!(mutation.contains(entry.reference().content_digest().as_str()));
    assert!(mutation.contains(proposal.reference().content_digest().as_str()));

    let created = render(CommandView::MemoryProposalCreated(
        MemoryProposalCreatedView {
            proposal: proposal.reference(),
            approval_id: proposal.approval_id(),
            status: MemoryProposalStatus::Pending,
        },
    ));
    assert!(created.contains(&proposal.reference().proposal_id().to_string()));
    assert!(created.contains(proposal.reference().content_digest().as_str()));
    assert!(created.contains("Pending"));

    let resolved = render(CommandView::MemoryProposalResolution(
        MemoryProposalResolutionView {
            resolution,
            entry: Some(entry.reference()),
            expired_proposals: vec![proposal.reference()],
        },
    ));
    assert!(resolved.contains(proposal.reference().content_digest().as_str()));
    assert!(resolved.contains(entry.reference().content_digest().as_str()));
    assert!(resolved.contains("Accepted"));
}

#[test]
fn help_lists_only_supported_memory_routes_and_no_internal_route() {
    let text = render(CommandView::Help(HelpView));

    for route in [
        "/memory list <agent>",
        "/memory get <agent> <key>",
        "/memory history <agent> <key> [version]",
        "/memory set <agent> <key>",
        "/memory delete <agent> <key>",
        "/memory proposals <agent> [pending|all]",
        "/memory proposal <proposal-id>",
        "/memory approve <proposal-id>",
        "/memory reject <proposal-id>",
        "/memory episodes <agent>",
        "/memory episode <summary-id>",
    ] {
        assert!(text.contains(route), "missing {route}");
    }
    assert!(!text.contains("/memory propose"));
    assert!(!text.contains("/memory snapshot"));
    assert!(!text.contains("/memory summary"));
    assert!(text.contains("internal producers are unavailable"));
}

#[test]
fn malformed_memory_rejection_renders_only_static_actionable_usage() {
    let ApplicationCommand::RejectInput(rejection) =
        command(b"/memory unknown \x1b[31mcredential=top-secret-password")
    else {
        panic!("expected malformed memory rejection");
    };
    let input_digest = rejection.input_digest.to_string();

    let text = render(CommandView::InputRejected(InputRejectedView { rejection }));

    assert!(text.starts_with("Input rejected: malformed command.\nUsage:\n"));
    assert_exact_memory_usage(&text);
    assert!(!text.contains("credential"));
    assert!(!text.contains("top-secret-password"));
    assert!(!text.contains(&input_digest));
    assert!(!text.contains('\x1b'));
}

#[test]
fn tui_malformed_memory_is_consumed_without_history_and_its_usage_is_visible() {
    let mut model = tui_model();
    model.set_focus(Focus::Command);
    model
        .command
        .ingest("/memory unknown credential=top-secret-password");

    let effect = handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );

    let ControllerEffect::Submit(ApplicationCommand::RejectInput(rejection)) = effect else {
        panic!("malformed memory must submit its typed rejection");
    };
    assert_eq!(model.command.text(), "");
    assert_eq!(model.command.cursor_byte(), 0);
    assert_eq!(model.command.history_len(), 0);
    assert!(model.command_in_flight);
    let text = render(CommandView::InputRejected(InputRejectedView { rejection }));
    assert_exact_memory_usage(&text);
    assert!(!text.contains("top-secret-password"));
}

#[test]
fn tui_direct_memory_read_is_consumed_submitted_and_remembered() {
    let mut model = tui_model();
    model.set_focus(Focus::Command);
    model.command.remember("/status".into());
    model.command.ingest("/memory list analyst");

    let effect = handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );

    assert_eq!(
        effect,
        ControllerEffect::Submit(ApplicationCommand::ListMemoryEntries {
            selector: AgentProfileSelector::Name("analyst".into()),
        })
    );
    assert_eq!(model.command.text(), "");
    assert_eq!(model.command.cursor_byte(), 0);
    assert_eq!(model.command.history_len(), 2);
    assert_eq!(model.command.history_back(), Some("/memory list analyst"));
    assert!(model.command_in_flight);
}

#[test]
fn tui_memory_workflow_only_shows_guidance_without_navigation_or_history_mutation() {
    let mut model = tui_model();
    model.active_view = View::Help;
    model.set_focus(Focus::Command);
    model.workspace_body_width = 80;
    model.workspace_body_height = 1;
    model.agents.pane = AgentsPane::History;
    model.agents.selected_profile = 7;
    model.agents.selected_history_version = 9;
    model.agents.list_scroll = 5;
    model.agents.detail_scroll = 6;
    model.agents.history_scroll = 8;
    model.workspace_scroll = 11;
    model.command.remember("/help".into());
    model.command.ingest("/memory delete analyst thesis");
    model.command.move_left();
    let mut expected = model.clone();
    expected.message = Some(UiMessage {
        severity: Severity::Info,
        text: "Open Agents → Memory to edit or resolve memory.".into(),
    });

    let effect = handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
    );

    assert_eq!(effect, ControllerEffect::Redraw);
    assert_eq!(model, expected);
}
