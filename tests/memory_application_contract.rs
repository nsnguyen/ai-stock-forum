mod support;

use std::sync::Arc;

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, AgentProfileSummary, AgentProfilesView, AppError, ApplicationCommand,
        AuthorizationDecision, CommandEnvelope, CommandOutcome, CommandView, DatabaseReadiness,
        EpisodicSummariesView, EpisodicSummaryListItem, EpisodicSummaryView, MemoryEntriesView,
        MemoryEntryHistorySummary, MemoryEntryHistoryView, MemoryEntryMutationView,
        MemoryEntrySummary, MemoryEntryVersionView, MemoryEntryView, MemoryProfileIdentityView,
        MemoryProposalCreatedView, MemoryProposalResolutionView, MemoryProposalSummary,
        MemoryProposalView, MemoryProposalsView, MemorySnapshotView, PresentationSnapshot,
        ProcessGuardOwnership, ShutdownDisposition,
    },
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, CommandId, CorrelationId,
        EpisodicSummaryId, EventId, InstallationId, MemoryEntryId, MemoryEntryVersionId,
        MemoryNamespaceId, MemoryProposalId, MemoryReviewToken, ObjectVersion, SessionId,
        canonical_json_bytes, sha256,
    },
    memory::{
        EpisodicQualification, EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState,
        MemoryEntryDraft, MemoryEntryVersion, MemoryProposal, MemoryProposalFilter,
        MemoryProposalOperation, MemoryProposalOperationKind, MemoryProposalRef,
        MemoryProposalResolution, MemoryProposalStatus, MemoryPurposeScope, MemoryRetrievalBudget,
        MemoryRetrievalRequest, MemoryRetrievalScope, select_snapshot,
    },
    policy::{ApprovalStatus, Capability},
    setup::SetupStatus,
    ui::{
        command::TextRenderer,
        tui::{
            ControllerEffect, apply_outcome,
            model::{AgentsPane, Focus, TuiModel, View},
        },
    },
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
            "Memory Agent".into(),
            "Memory contract fixture.".into(),
            AgentRole::Custom,
            "research".into(),
            vec![],
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

fn entry(profile: &AgentProfileVersion) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(uuid(4)),
        MemoryEntryVersionId::from_uuid(uuid(5)),
        MemoryEntryDraft::new("Earnings Thesis".into(), "private thesis".into(), vec![]).unwrap(),
        Actor::Human,
        20,
        None,
        EventId::from_uuid(uuid(6)),
    )
    .unwrap()
}

fn commands() -> Vec<(Actor, ApplicationCommand, Capability)> {
    let profile = profile();
    let entry = entry(&profile).reference();
    let proposal = MemoryProposalRef::new(
        MemoryProposalId::from_uuid(uuid(7)),
        ObjectVersion::new(1).unwrap(),
        sha256(b"proposal"),
    )
    .unwrap();
    let candidate = MemoryEntryDraft::new(
        "Earnings Thesis".into(),
        "new private thesis".into(),
        vec![],
    )
    .unwrap();
    let selector = AgentProfileSelector::from(profile.profile_id());
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(&profile, MemoryPurposeScope::General).unwrap(),
        MemoryRetrievalBudget::default(),
    )
    .unwrap();
    vec![
        (
            Actor::Human,
            ApplicationCommand::SetMemoryEntry {
                profile: profile.reference(),
                expected: ExpectedMemoryEntryState::Present(entry.clone()),
                candidate: candidate.clone(),
                review_token: MemoryReviewToken::from_uuid(uuid(8)),
                review_digest: sha256(b"set review"),
            },
            Capability::MemoryMutate,
        ),
        (
            Actor::Human,
            ApplicationCommand::DeleteMemoryEntry {
                profile: profile.reference(),
                expected: entry,
                review_token: MemoryReviewToken::from_uuid(uuid(9)),
                review_digest: sha256(b"delete review"),
            },
            Capability::MemoryMutate,
        ),
        (
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: candidate.clone(),
                },
                rationale: "private rationale".into(),
            },
            Capability::MemoryPropose,
        ),
        (
            Actor::Human,
            ApplicationCommand::ApproveMemoryProposal {
                proposal: proposal.clone(),
                approval_id: ApprovalId::from_uuid(uuid(10)),
                expected_approval_status: ApprovalStatus::Pending,
                expected_entry: ExpectedMemoryEntryState::Absent,
                review_token: MemoryReviewToken::from_uuid(uuid(11)),
                review_digest: sha256(b"approve review"),
            },
            Capability::MemoryResolve,
        ),
        (
            Actor::Human,
            ApplicationCommand::RejectMemoryProposal {
                proposal,
                approval_id: ApprovalId::from_uuid(uuid(10)),
                expected_approval_status: ApprovalStatus::Pending,
                expected_entry: ExpectedMemoryEntryState::Absent,
                review_token: MemoryReviewToken::from_uuid(uuid(12)),
                review_digest: sha256(b"reject review"),
            },
            Capability::MemoryResolve,
        ),
        (
            Actor::Human,
            ApplicationCommand::ListMemoryEntries {
                selector: selector.clone(),
            },
            Capability::MemoryRead,
        ),
        (
            Actor::Human,
            ApplicationCommand::ShowMemoryEntry {
                selector: selector.clone(),
                display_key: "Earnings Thesis".into(),
            },
            Capability::MemoryRead,
        ),
        (
            Actor::Human,
            ApplicationCommand::ShowMemoryEntryHistory {
                selector: selector.clone(),
                display_key: "Earnings Thesis".into(),
            },
            Capability::MemoryRead,
        ),
        (
            Actor::Human,
            ApplicationCommand::ShowMemoryEntryVersion {
                selector: selector.clone(),
                display_key: "Earnings Thesis".into(),
                version: ObjectVersion::new(1).unwrap(),
            },
            Capability::MemoryRead,
        ),
        (
            Actor::Human,
            ApplicationCommand::ListMemoryProposals {
                selector: selector.clone(),
                filter: MemoryProposalFilter::Pending,
            },
            Capability::MemoryRead,
        ),
        (
            Actor::Human,
            ApplicationCommand::ShowMemoryProposal {
                proposal_id: MemoryProposalId::from_uuid(uuid(7)),
            },
            Capability::MemoryRead,
        ),
        (
            Actor::Human,
            ApplicationCommand::ListEpisodicSummaries { selector },
            Capability::MemoryRead,
        ),
        (
            Actor::Human,
            ApplicationCommand::ShowEpisodicSummary {
                summary_id: EpisodicSummaryId::from_uuid(uuid(13)),
            },
            Capability::MemoryRead,
        ),
        (
            Actor::Human,
            ApplicationCommand::BuildMemorySnapshot { request },
            Capability::MemoryRead,
        ),
    ]
}

#[test]
fn memory_application_vocabulary_has_exact_capabilities_and_mappings() {
    let capabilities = [
        Capability::MemoryRead,
        Capability::MemoryPreview,
        Capability::MemoryMutate,
        Capability::MemoryPropose,
        Capability::MemoryResolve,
    ];
    assert_eq!(
        serde_json::to_value(capabilities).unwrap(),
        serde_json::json!([
            "memory_read",
            "memory_preview",
            "memory_mutate",
            "memory_propose",
            "memory_resolve"
        ])
    );
    let commands = commands();
    assert_eq!(commands.len(), 14);
    for (_, command, capability) in commands {
        assert_eq!(command.required_capability(), capability);
    }
}

#[test]
fn defensive_dispatch_error_is_content_free_and_has_an_exact_code() {
    assert_eq!(
        AppError::WrongMemoryCommandDispatcher.code(),
        "wrong_memory_command_dispatcher"
    );
    assert!(
        !AppError::WrongMemoryCommandDispatcher
            .to_string()
            .contains("private")
    );
}

#[test]
fn service_dispatch_reaches_all_fourteen_memory_command_variants() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    for (index, (actor, command, capability)) in commands().into_iter().enumerate() {
        let envelope = CommandEnvelope {
            command_id: CommandId::from_uuid(uuid(1_000 + index as u128)),
            correlation_id: CorrelationId::from_uuid(uuid(2_000 + index as u128)),
            actor,
            command,
        };
        let result = app.execute(envelope);
        assert!(!matches!(
            result,
            Err(AppError::WrongMemoryCommandDispatcher)
        ));
        assert_eq!(policy.capabilities().last(), Some(&capability));
    }
}

#[test]
fn default_policy_grants_memory_read_before_profile_resolution() {
    let mut app = support::app();
    assert_eq!(
        app.execute_user(ApplicationCommand::ListMemoryEntries {
            selector: AgentProfileSelector::from(profile().profile_id()),
        }),
        Err(AppError::AgentProfileNotFound)
    );
}

#[test]
fn memory_actor_matrix_is_exhaustive_and_runs_before_policy() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let examples = commands();

    for (index, (_, command, capability)) in examples.iter().cloned().enumerate() {
        let calls = policy.calls();
        assert_eq!(
            app.execute(CommandEnvelope {
                command_id: CommandId::from_uuid(uuid(10_000 + index as u128)),
                correlation_id: CorrelationId::from_uuid(uuid(11_000 + index as u128)),
                actor: Actor::System,
                command,
            }),
            Err(AppError::CapabilityDenied {
                capability,
                decision: ai_stock_forum::policy::PolicyDecision::Denied,
            })
        );
        assert_eq!(policy.calls(), calls);
    }

    for (index, (_, command, capability)) in examples.iter().cloned().enumerate() {
        let calls = policy.calls();
        let result = app.execute(CommandEnvelope {
            command_id: CommandId::from_uuid(uuid(12_000 + index as u128)),
            correlation_id: CorrelationId::from_uuid(uuid(13_000 + index as u128)),
            actor: Actor::Human,
            command,
        });
        if index == 2 {
            assert_eq!(
                result,
                Err(AppError::CapabilityDenied {
                    capability,
                    decision: ai_stock_forum::policy::PolicyDecision::Denied,
                })
            );
            assert_eq!(policy.calls(), calls);
        } else {
            assert!(!matches!(result, Err(AppError::CapabilityDenied { .. })));
            assert_eq!(policy.calls(), calls + 1);
        }
    }

    for (index, (valid_actor, command, capability)) in examples.into_iter().enumerate() {
        let actor = match &valid_actor {
            Actor::Agent(id) => Actor::Agent(*id),
            _ => Actor::Agent(AgentProfileId::from_uuid(uuid(99_999))),
        };
        let calls = policy.calls();
        let result = app.execute(CommandEnvelope {
            command_id: CommandId::from_uuid(uuid(14_000 + index as u128)),
            correlation_id: CorrelationId::from_uuid(uuid(15_000 + index as u128)),
            actor,
            command,
        });
        if index == 2 {
            assert!(!matches!(result, Err(AppError::CapabilityDenied { .. })));
            assert!(!matches!(
                result,
                Err(AppError::WrongMemoryCommandDispatcher)
            ));
            assert_eq!(policy.calls(), calls + 1);
        } else {
            assert_eq!(
                result,
                Err(AppError::CapabilityDenied {
                    capability,
                    decision: ai_stock_forum::policy::PolicyDecision::Denied,
                })
            );
            assert_eq!(policy.calls(), calls);
        }
    }
}

#[test]
fn nested_memory_candidates_are_canonical_before_fingerprint_material() {
    let profile = profile();
    let left = MemoryEntryDraft::new(
        "  Earnings   Thesis ".into(),
        "line one\r\nline two".into(),
        vec!["  Macro  ".into(), "Catalyst".into()],
    )
    .unwrap();
    let right = MemoryEntryDraft::new(
        "Earnings Thesis".into(),
        "line one\nline two".into(),
        vec!["Catalyst".into(), "Macro".into()],
    )
    .unwrap();
    let command = |candidate| ApplicationCommand::ProposeMemoryMutation {
        proposer: profile.reference(),
        expected: ExpectedMemoryEntryState::Absent,
        operation: MemoryProposalOperation::Set { candidate },
        rationale: "reason".into(),
    };
    let left = command(left);
    let right = command(right);
    assert_eq!(left, right);
    assert_eq!(
        sha256(&canonical_json_bytes(&left).unwrap()),
        sha256(&canonical_json_bytes(&right).unwrap())
    );

    let mut noncanonical = serde_json::to_value(right).unwrap();
    noncanonical["data"]["operation"]["Set"]["candidate"]["display_key"] =
        serde_json::json!(" Earnings Thesis ");
    assert!(serde_json::from_value::<ApplicationCommand>(noncanonical).is_err());
}

#[test]
fn command_and_view_payloads_reject_unknown_fields() {
    let command = commands().pop().unwrap().1;
    let mut command_json = serde_json::to_value(command).unwrap();
    command_json["data"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ApplicationCommand>(command_json).is_err());

    let profile = profile();
    let view = MemoryEntriesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        entries: vec![],
        total_count: 0,
        returned_count: 0,
        omitted_count: 0,
    };
    let mut view_json = serde_json::to_value(view).unwrap();
    view_json["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<MemoryEntriesView>(view_json).is_err());
}

fn proposal(profile: &AgentProfileVersion) -> MemoryProposal {
    MemoryProposal::new(
        MemoryProposalId::from_uuid(uuid(20)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                "Earnings Thesis".into(),
                "private proposed value".into(),
                vec!["Catalyst".into()],
            )
            .unwrap(),
        },
        "Earnings Thesis".into(),
        ExpectedMemoryEntryState::Absent,
        "private rationale".into(),
        30,
        EventId::from_uuid(uuid(21)),
        ApprovalId::from_uuid(uuid(22)),
    )
    .unwrap()
}

fn summary(profile: &AgentProfileVersion) -> EpisodicSummary {
    EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(uuid(30)),
        profile,
        "private summary label".into(),
        "private summary body".into(),
        vec!["Catalyst".into()],
        vec![
            EpisodicSourceRef::new(
                1,
                EventId::from_uuid(uuid(31)),
                "help_viewed".into(),
                sha256(b"source"),
            )
            .unwrap(),
        ],
        40,
        2,
        EventId::from_uuid(uuid(32)),
    )
    .unwrap()
}

fn memory_views() -> Vec<CommandView> {
    let profile = profile();
    let entry = entry(&profile);
    let entry_ref = entry.reference();
    let proposal = proposal(&profile);
    let proposal_ref = proposal.reference();
    let resolution = MemoryProposalResolution::new(
        proposal_ref.clone(),
        MemoryProposalStatus::Rejected,
        proposal.approval_id(),
        Actor::Human,
        50,
        EventId::from_uuid(uuid(50)),
    )
    .unwrap();
    let summary = summary(&profile);
    let summary_ref = summary.reference();
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(&profile, MemoryPurposeScope::General).unwrap(),
        MemoryRetrievalBudget::default(),
    )
    .unwrap();
    let snapshot = select_snapshot(&request, std::iter::empty(), std::iter::empty()).unwrap();
    vec![
        CommandView::MemoryEntries(MemoryEntriesView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            entries: vec![MemoryEntrySummary {
                entry: entry_ref.clone(),
                display_key: entry.display_key().into(),
                purpose_tags: entry.purpose_tags().to_vec(),
                value_bytes: u64::try_from(entry.value().unwrap().len()).unwrap(),
                created_at_ms: entry.created_at_ms(),
            }],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        }),
        CommandView::MemoryEntry(MemoryEntryView {
            profile: profile.reference(),
            entry: entry.clone(),
        }),
        CommandView::MemoryEntryHistory(MemoryEntryHistoryView {
            profile: profile.reference(),
            current: entry_ref.clone(),
            versions: vec![MemoryEntryHistorySummary {
                entry: entry_ref.clone(),
                display_key: entry.display_key().into(),
                created_at_ms: entry.created_at_ms(),
                accepted_proposal: None,
            }],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        }),
        CommandView::MemoryEntryVersion(MemoryEntryVersionView {
            profile: profile.reference(),
            entry: entry.clone(),
        }),
        CommandView::MemoryProposals(MemoryProposalsView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            filter: MemoryProposalFilter::Pending,
            proposals: vec![MemoryProposalSummary {
                proposal: proposal_ref.clone(),
                proposer: profile.reference(),
                operation: MemoryProposalOperationKind::Set,
                display_key: proposal.display_key().into(),
                status: MemoryProposalStatus::Pending,
                created_at_ms: proposal.created_at_ms(),
            }],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        }),
        CommandView::MemoryProposal(MemoryProposalView {
            proposal: proposal.clone(),
            status: MemoryProposalStatus::Pending,
            resolution: None,
            current_entry: ExpectedMemoryEntryState::Absent,
            proposer_is_historical: false,
            proposer_identity: MemoryProfileIdentityView {
                profile: profile.reference(),
                display_name: profile.display_name().into(),
            },
            namespace_owner_identity: MemoryProfileIdentityView {
                profile: profile.reference(),
                display_name: profile.display_name().into(),
            },
        }),
        CommandView::EpisodicSummaries(EpisodicSummariesView {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            summaries: vec![EpisodicSummaryListItem {
                summary: summary_ref,
                label: summary.label().into(),
                purpose_tags: summary.purpose_tags().to_vec(),
                source_count: u64::try_from(summary.sources().len()).unwrap(),
                created_at_ms: summary.created_at_ms(),
            }],
            total_count: 1,
            returned_count: 1,
            omitted_count: 0,
        }),
        CommandView::EpisodicSummary(EpisodicSummaryView {
            summary,
            qualification: EpisodicQualification::SummaryVerifySources,
        }),
        CommandView::MemoryEntryMutation(MemoryEntryMutationView {
            entry: entry_ref.clone(),
            expired_proposals: vec![proposal_ref.clone()],
        }),
        CommandView::MemoryProposalCreated(MemoryProposalCreatedView {
            proposal: proposal_ref.clone(),
            approval_id: proposal.approval_id(),
            status: MemoryProposalStatus::Pending,
        }),
        CommandView::MemoryProposalResolution(MemoryProposalResolutionView {
            resolution,
            entry: Some(entry_ref),
            expired_proposals: vec![proposal_ref],
        }),
        CommandView::MemorySnapshot(MemorySnapshotView { snapshot }),
    ]
}

fn tui_model() -> TuiModel {
    let first = profile();
    let second = AgentProfileVersion::create(
        AgentProfileId::from_uuid(uuid(60)),
        AgentProfileVersionId::from_uuid(uuid(61)),
        MemoryNamespaceId::from_uuid(uuid(62)),
        10,
        AgentProfileDraft::new(
            "Second Agent".into(),
            "Fixture profile.".into(),
            AgentRole::Custom,
            "research".into(),
            vec![],
            "Careful.".into(),
            "Use evidence.".into(),
            AgentBindings::default(),
            vec![],
            vec![],
        )
        .unwrap(),
        None,
    )
    .unwrap();
    let summary = |profile: &AgentProfileVersion| AgentProfileSummary {
        profile_id: profile.profile_id(),
        profile_version_id: profile.profile_version_id(),
        version: profile.version(),
        display_name: profile.display_name().into(),
        role: profile.role(),
        primary_specialty: profile.primary_specialty().into(),
        readiness: ai_stock_forum::agents::AgentReadiness::Unbound,
        content_digest: profile.content_digest().clone(),
    };
    let mut model = TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(uuid(70)),
            session_id: SessionId::from_uuid(uuid(71)),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: vec![],
            agent_profiles: AgentProfilesView {
                profiles: vec![summary(&first), summary(&second)],
                total_count: 2,
                returned_count: 2,
                truncated: false,
            },
            selected_agent_profile: None,
            selected_agent_profile_history: None,
        },
        false,
    );
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::List;
    model.agents.selected_profile = 1;
    model.agents.list_scroll = 1;
    model.focus = Focus::Inspector;
    model.inspector_open = true;
    model
}

#[test]
fn all_twelve_memory_views_are_strict_safe_and_navigation_neutral() {
    let views = memory_views();
    assert_eq!(views.len(), 12);
    for (index, view) in views.into_iter().enumerate() {
        let encoded = serde_json::to_value(&view).unwrap();
        assert_eq!(
            serde_json::from_value::<CommandView>(encoded.clone()).unwrap(),
            view
        );
        let mut forged = encoded;
        forged["data"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<CommandView>(forged).is_err());

        let mut rendered = Vec::new();
        TextRenderer::render_view(&view, &mut rendered).unwrap();
        let rendered = String::from_utf8(rendered).unwrap();
        assert!(!rendered.is_empty());
        for private in [
            "private thesis",
            "private proposed value",
            "private rationale",
            "private summary label",
            "private summary body",
        ] {
            assert!(!rendered.contains(private), "view {index}");
        }

        let mut model = tui_model();
        let before = model.clone();
        let effect = apply_outcome(
            &mut model,
            CommandOutcome {
                command_id: CommandId::from_uuid(uuid(80 + index as u128)),
                correlation_id: CorrelationId::from_uuid(uuid(100 + index as u128)),
                committed_events: vec![],
                view,
                shutdown: ShutdownDisposition::Continue,
            },
        );
        assert_eq!(effect, ControllerEffect::Redraw);
        assert_eq!(model.active_view, before.active_view);
        assert_eq!(model.agents, before.agents);
        assert_eq!(model.skills, before.skills);
        assert_eq!(model.focus, before.focus);
        assert_eq!(model.inspector_open, before.inspector_open);
        assert_eq!(model.workspace_scroll, before.workspace_scroll);
        assert_eq!(model.audit_selection, before.audit_selection);
    }
}
