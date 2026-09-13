mod support;

use std::{
    collections::BTreeSet,
    env, fs, io,
    path::Path,
    sync::{Arc, Barrier},
    thread,
};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, AppError, ApplicationCommand, ApplicationService, AuditLimit,
        CommandEnvelope, CommandOutcome, CommandView, MemoryProfileIdentityView,
        PresentationSnapshot, ShutdownReason,
    },
    config::AppPaths,
    domain::{Actor, CommandId, CorrelationId, EventId},
    memory::{
        EpisodicQualification, ExpectedMemoryEntryState, MemoryEntryDraft, MemoryEntryState,
        MemoryProposalFilter, MemoryProposalOperation, MemoryProposalStatus, MemoryPurposeScope,
        MemoryResolutionAction, MemoryRetrievalBudget, MemoryRetrievalRequest,
        MemoryRetrievalScope,
    },
    runtime::RuntimeError,
    ui::{
        command::{FallbackParsedLine, ParsedLine, TextRenderer, parse_fallback_line, parse_line},
        memory_editor::{MemoryEditor, MemoryEditorEffect, MemoryEditorStep},
        tui::{
            model::{AgentsPane, MemoryPane, TuiModel, View},
            render,
            theme::Theme,
        },
    },
};
use ratatui::{Terminal, backend::TestBackend};
use uuid::Uuid;

fn validate_manual_acceptance_target(
    requested: &Path,
    discovered: &Path,
    target_exists: bool,
) -> Result<(), &'static str> {
    if requested != discovered {
        return Err("manual acceptance target mismatch");
    }
    if target_exists {
        return Err("manual acceptance target already exists");
    }
    Ok(())
}

fn classify_target_metadata<T>(metadata: io::Result<T>) -> Result<bool, &'static str> {
    match metadata {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("manual acceptance target metadata unavailable"),
    }
}

fn manual_acceptance_output(seed: &support::ManualMemoryAcceptanceSeed) -> [String; 7] {
    [
        format!("profile_id={}", seed.profile.profile_id()),
        format!("tui_approval_proposal_id={}", seed.tui_approval_proposal_id),
        format!(
            "tui_rejection_proposal_id={}",
            seed.tui_rejection_proposal_id
        ),
        format!(
            "fallback_approval_proposal_id={}",
            seed.fallback_approval_proposal_id
        ),
        format!(
            "fallback_rejection_proposal_id={}",
            seed.fallback_rejection_proposal_id
        ),
        format!(
            "restart_pending_proposal_id={}",
            seed.restart_pending_proposal_id
        ),
        format!("summary_id={}", seed.summary_id),
    ]
}

fn acceptance_envelope(id: u128, actor: Actor, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 1_000_000)),
        actor,
        command,
    }
}

fn acceptance_profile_draft(name: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        "Synthetic acceptance profile.".to_owned(),
        AgentRole::Custom,
        "memory acceptance".to_owned(),
        vec!["acceptance".to_owned()],
        "Careful and deterministic.".to_owned(),
        "Exercise existing Hybrid Memory behavior.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .expect("valid acceptance profile draft")
}

fn create_acceptance_profile(
    app: &mut ApplicationService,
    id: u128,
    name: &str,
) -> AgentProfileVersion {
    let outcome = app
        .execute(acceptance_envelope(
            id,
            Actor::Human,
            ApplicationCommand::CreateAgentProfile {
                draft: acceptance_profile_draft(name),
                template_provenance: None,
            },
        ))
        .unwrap_or_else(|error| panic!("acceptance profile failed: {}", error.code()));
    outcome
        .committed_events
        .into_iter()
        .find_map(|event| match event.event {
            ai_stock_forum::app::ApplicationEvent::AgentProfileCreated { profile } => Some(profile),
            _ => None,
        })
        .unwrap_or_else(|| panic!("acceptance profile event was unavailable"))
}

fn acceptance_memory_candidate_with_tag(key: &str, value: &str, tag: &str) -> MemoryEntryDraft {
    MemoryEntryDraft::new(key.to_owned(), value.to_owned(), vec![tag.to_owned()])
        .expect("valid acceptance memory candidate")
}

fn set_acceptance_entry(
    app: &mut ApplicationService,
    profile: &AgentProfileVersion,
    id: u128,
    key: &str,
    value: &str,
) -> (ai_stock_forum::memory::MemoryEntryRef, EventId) {
    set_acceptance_entry_with_tag(app, profile, id, key, value, "isolation")
}

fn set_acceptance_entry_with_tag(
    app: &mut ApplicationService,
    profile: &AgentProfileVersion,
    id: u128,
    key: &str,
    value: &str,
    tag: &str,
) -> (ai_stock_forum::memory::MemoryEntryRef, EventId) {
    let preview = app
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            acceptance_memory_candidate_with_tag(key, value, tag),
        )
        .unwrap_or_else(|error| panic!("acceptance Set preview failed: {}", error.code()));
    let ai_stock_forum::app::MemoryEditPreview::Review(review) = preview else {
        panic!("acceptance Set preview was not a review")
    };
    let candidate = review
        .candidate
        .clone()
        .unwrap_or_else(|| panic!("acceptance Set review had no candidate"));
    let outcome = app
        .execute(acceptance_envelope(
            id,
            Actor::Human,
            ApplicationCommand::SetMemoryEntry {
                profile: review.profile,
                expected: review.expected,
                candidate,
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap_or_else(|error| panic!("acceptance Set failed: {}", error.code()));
    let event_id = outcome
        .committed_events
        .first()
        .map(|event| event.event_id)
        .unwrap_or_else(|| panic!("acceptance Set event was unavailable"));
    let CommandView::MemoryEntryMutation(view) = outcome.view else {
        panic!("acceptance Set returned the wrong view")
    };
    (view.entry, event_id)
}

fn propose_acceptance_entry(
    app: &mut ApplicationService,
    profile: &AgentProfileVersion,
    id: u128,
    key: &str,
    value: &str,
) -> ai_stock_forum::memory::MemoryProposalRef {
    propose_acceptance_entry_with_metadata(
        app,
        profile,
        id,
        key,
        value,
        "isolation",
        "Synthetic acceptance rationale.",
    )
}

#[allow(clippy::too_many_arguments)]
fn propose_acceptance_entry_with_metadata(
    app: &mut ApplicationService,
    profile: &AgentProfileVersion,
    id: u128,
    key: &str,
    value: &str,
    tag: &str,
    rationale: &str,
) -> ai_stock_forum::memory::MemoryProposalRef {
    let outcome = app
        .execute(acceptance_envelope(
            id,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: acceptance_memory_candidate_with_tag(key, value, tag),
                },
                rationale: rationale.to_owned(),
            },
        ))
        .unwrap_or_else(|error| panic!("acceptance proposal failed: {}", error.code()));
    let CommandView::MemoryProposalCreated(view) = outcome.view else {
        panic!("acceptance proposal returned the wrong view")
    };
    view.proposal
}

fn acceptance_read(
    app: &mut ApplicationService,
    id: &mut u128,
    command: ApplicationCommand,
) -> CommandOutcome {
    let outcome = app
        .execute(acceptance_envelope(*id, Actor::Human, command))
        .unwrap_or_else(|error| panic!("acceptance read failed: {}", error.code()));
    *id += 1;
    outcome
}

fn delete_acceptance_entry(
    app: &mut ApplicationService,
    profile: &AgentProfileVersion,
    id: u128,
    key: &str,
) -> ai_stock_forum::memory::MemoryEntryRef {
    let preview = app
        .preview_memory_delete(profile.profile_id().into(), key.to_owned())
        .unwrap_or_else(|error| panic!("acceptance Delete preview failed: {}", error.code()));
    let ai_stock_forum::app::MemoryEditPreview::Review(review) = preview else {
        panic!("acceptance Delete preview was not a review")
    };
    let outcome = app
        .execute(acceptance_envelope(
            id,
            Actor::Human,
            ApplicationCommand::DeleteMemoryEntry {
                profile: review.profile,
                expected: match review.expected {
                    ExpectedMemoryEntryState::Present(entry) => entry,
                    _ => panic!("acceptance Delete review lacked a present entry"),
                },
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap_or_else(|error| panic!("acceptance Delete failed: {}", error.code()));
    let CommandView::MemoryEntryMutation(view) = outcome.view else {
        panic!("acceptance Delete returned the wrong view")
    };
    view.entry
}

fn resolve_acceptance_proposal(
    app: &mut ApplicationService,
    id: u128,
    proposal: ai_stock_forum::memory::MemoryProposalRef,
    action: MemoryResolutionAction,
) -> ai_stock_forum::domain::ApprovalId {
    let review = match action {
        MemoryResolutionAction::Approve => app.preview_memory_proposal_approval(proposal),
        MemoryResolutionAction::Reject => app.preview_memory_proposal_rejection(proposal),
    }
    .unwrap_or_else(|error| panic!("acceptance resolution preview failed: {}", error.code()));
    let approval_id = review.approval_id;
    let command = match action {
        MemoryResolutionAction::Approve => ApplicationCommand::ApproveMemoryProposal {
            proposal: review.proposal.reference(),
            approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
        MemoryResolutionAction::Reject => ApplicationCommand::RejectMemoryProposal {
            proposal: review.proposal.reference(),
            approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
    };
    let outcome = app
        .execute(acceptance_envelope(id, Actor::Human, command))
        .unwrap_or_else(|error| panic!("acceptance resolution failed: {}", error.code()));
    if !matches!(outcome.view, CommandView::MemoryProposalResolution(_)) {
        panic!("acceptance resolution returned the wrong view");
    }
    approval_id
}

fn render_acceptance_view(view: &CommandView) -> String {
    let mut output = Vec::new();
    TextRenderer::render_view(view, &mut output)
        .unwrap_or_else(|_| panic!("acceptance view rendering failed"));
    String::from_utf8(output).unwrap_or_else(|_| panic!("acceptance view was not UTF-8"))
}

fn render_acceptance_error(error: &RuntimeError) -> String {
    let mut output = Vec::new();
    TextRenderer::render_runtime_error(error, &mut output)
        .unwrap_or_else(|_| panic!("acceptance error rendering failed"));
    String::from_utf8(output).unwrap_or_else(|_| panic!("acceptance error was not UTF-8"))
}

fn acceptance_memory_model(
    snapshot: PresentationSnapshot,
    profile: &AgentProfileVersion,
) -> TuiModel {
    let mut model = TuiModel::new(snapshot, false);
    model.select_view(View::Agents);
    model
        .agents
        .memory
        .bind_profile(
            MemoryProfileIdentityView {
                profile: profile.reference(),
                display_name: profile.display_name().to_owned(),
            },
            profile.memory_namespace_id(),
        )
        .unwrap_or_else(|error| panic!("acceptance Memory binding failed: {}", error.code()));
    model.agents.pane = AgentsPane::Memory;
    model.set_terminal_size(140, 30);
    model
}

fn render_acceptance_tui(model: &TuiModel) -> String {
    let width = model.terminal_width;
    let height = model.terminal_height;
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)
        .unwrap_or_else(|_| panic!("acceptance TUI terminal was unavailable"));
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .unwrap_or_else(|_| panic!("acceptance TUI render failed"));
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn manual_acceptance_target_must_be_the_absent_discovered_state_directory() {
    let discovered = Path::new("/synthetic/discovered-state");

    assert_eq!(
        validate_manual_acceptance_target(discovered, discovered, false),
        Ok(())
    );
    assert_eq!(
        validate_manual_acceptance_target(
            Path::new("/synthetic/different-state"),
            discovered,
            false,
        ),
        Err("manual acceptance target mismatch")
    );
    assert_eq!(
        validate_manual_acceptance_target(discovered, discovered, true),
        Err("manual acceptance target already exists")
    );
    assert_eq!(
        validate_manual_acceptance_target(
            &discovered.join("ai-stock-forum.sqlite3"),
            discovered,
            false,
        ),
        Err("manual acceptance target mismatch")
    );
}

#[test]
fn manual_acceptance_metadata_accepts_only_absent_or_existing_targets() {
    assert_eq!(classify_target_metadata(Ok(())), Ok(true));
    assert_eq!(
        classify_target_metadata::<()>(Err(io::Error::from(io::ErrorKind::NotFound))),
        Ok(false)
    );
    assert_eq!(
        classify_target_metadata::<()>(Err(io::Error::from(io::ErrorKind::PermissionDenied))),
        Err("manual acceptance target metadata unavailable")
    );
}

#[test]
fn manual_seed_atomically_refuses_an_existing_target_without_initializing_it() {
    let parent = tempfile::tempdir().expect("temporary parent");
    let target = parent.path().join("already-present");
    fs::create_dir(&target).expect("pre-existing target");
    let paths = AppPaths::for_test(&target);

    let error = support::seed_manual_memory_acceptance(&paths)
        .expect_err("existing target must be refused");

    assert_eq!(error.code(), "database_write_failed");
    assert_eq!(
        fs::read_dir(&target)
            .expect("unchanged existing target")
            .count(),
        0
    );
}

#[test]
fn concurrent_manual_seeders_atomically_claim_the_target_once() {
    let parent = tempfile::tempdir().expect("temporary parent");
    let target = parent.path().join("concurrent-manual-state");
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let target = target.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            support::seed_manual_memory_acceptance(&AppPaths::for_test(target))
        }));
    }
    barrier.wait();
    let results = workers
        .into_iter()
        .map(|worker| {
            worker
                .join()
                .unwrap_or_else(|_| panic!("manual seed worker panicked"))
        })
        .collect::<Vec<_>>();
    assert!(
        results.iter().filter(|result| result.is_ok()).count() == 1,
        "manual target had other than one atomic winner"
    );
    assert!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|error| error.code() == "database_write_failed"),
        "manual atomic loser returned a non-static error"
    );

    let paths = AppPaths::for_test(&target);
    let mut reopened = ApplicationService::bootstrap(
        &paths,
        Arc::new(support::TestClock::new()),
        Arc::new(support::TestIds::starting_at(8_500_000)),
    )
    .unwrap_or_else(|_| panic!("atomically seeded target did not reopen"));
    reopened
        .finish(ShutdownReason::InputClosed)
        .unwrap_or_else(|error| panic!("atomic target finish failed: {}", error.code()));
    drop(reopened);
}

#[test]
fn manual_seed_creates_five_separated_pending_proposals_one_source_linked_summary_and_closes() {
    let parent = tempfile::tempdir().expect("temporary parent");
    let paths = AppPaths::for_test(parent.path().join("manual-state"));
    let seed = support::seed_manual_memory_acceptance(&paths)
        .unwrap_or_else(|error| panic!("manual seed failed: {}", error.code()));

    assert!(paths.state_dir().is_dir());
    let mut service = ApplicationService::bootstrap(
        &paths,
        Arc::new(support::TestClock::new()),
        Arc::new(support::TestIds::starting_at(8_000_000)),
    )
    .unwrap_or_else(|error| panic!("seeded state did not reopen: {}", error.code()));

    let proposals = service
        .execute_user(ApplicationCommand::ListMemoryProposals {
            selector: AgentProfileSelector::from(seed.profile.profile_id()),
            filter: ai_stock_forum::memory::MemoryProposalFilter::All,
        })
        .unwrap_or_else(|error| panic!("proposal list failed: {}", error.code()));
    let CommandView::MemoryProposals(proposals) = proposals.view else {
        panic!("proposal list returned the wrong safe view")
    };
    assert_eq!(proposals.total_count, 5);
    assert_eq!(proposals.returned_count, 5);
    assert_eq!(proposals.omitted_count, 0);
    assert!(
        proposals
            .proposals
            .iter()
            .all(|proposal| proposal.status == MemoryProposalStatus::Pending)
    );
    let returned_ids = proposals
        .proposals
        .iter()
        .map(|proposal| proposal.proposal.proposal_id().to_string())
        .collect::<BTreeSet<_>>();
    let seeded_ids = [
        seed.tui_approval_proposal_id,
        seed.tui_rejection_proposal_id,
        seed.fallback_approval_proposal_id,
        seed.fallback_rejection_proposal_id,
        seed.restart_pending_proposal_id,
    ]
    .into_iter()
    .map(|proposal_id| proposal_id.to_string())
    .collect::<BTreeSet<_>>();
    assert_eq!(returned_ids, seeded_ids);

    let mut normalized_keys = BTreeSet::new();
    for proposal_id in [
        seed.tui_approval_proposal_id,
        seed.tui_rejection_proposal_id,
        seed.fallback_approval_proposal_id,
        seed.fallback_rejection_proposal_id,
        seed.restart_pending_proposal_id,
    ] {
        let detail = service
            .execute_user(ApplicationCommand::ShowMemoryProposal { proposal_id })
            .unwrap_or_else(|error| panic!("proposal detail failed: {}", error.code()));
        let CommandView::MemoryProposal(detail) = detail.view else {
            panic!("proposal detail returned the wrong safe view")
        };
        assert_eq!(detail.status, MemoryProposalStatus::Pending);
        assert!(normalized_keys.insert(detail.proposal.normalized_key().as_str().to_owned()));
    }
    assert_eq!(normalized_keys.len(), 5);
    assert!(!normalized_keys.contains("thesis"));
    assert!(!normalized_keys.contains("fallback_thesis"));

    let summaries = service
        .execute_user(ApplicationCommand::ListEpisodicSummaries {
            selector: AgentProfileSelector::from(seed.profile.profile_id()),
        })
        .unwrap_or_else(|error| panic!("summary list failed: {}", error.code()));
    let CommandView::EpisodicSummaries(summaries) = summaries.view else {
        panic!("summary list returned the wrong safe view")
    };
    assert_eq!(summaries.total_count, 1);
    assert_eq!(summaries.summaries[0].summary.summary_id(), seed.summary_id);
    assert_eq!(summaries.summaries[0].source_count, 1);

    service
        .finish(ShutdownReason::InputClosed)
        .unwrap_or_else(|error| panic!("seeded service teardown failed: {}", error.code()));
    drop(service);
}

#[test]
fn manual_seed_output_contains_only_stable_labels_and_synthetic_identifiers() {
    let parent = tempfile::tempdir().expect("temporary parent");
    let paths = AppPaths::for_test(parent.path().join("manual-output-state"));
    let seed = support::seed_manual_memory_acceptance(&paths)
        .unwrap_or_else(|error| panic!("manual seed failed: {}", error.code()));

    let lines = manual_acceptance_output(&seed);
    assert!(
        lines
            == [
                format!("profile_id={}", seed.profile.profile_id()),
                format!("tui_approval_proposal_id={}", seed.tui_approval_proposal_id),
                format!(
                    "tui_rejection_proposal_id={}",
                    seed.tui_rejection_proposal_id
                ),
                format!(
                    "fallback_approval_proposal_id={}",
                    seed.fallback_approval_proposal_id
                ),
                format!(
                    "fallback_rejection_proposal_id={}",
                    seed.fallback_rejection_proposal_id
                ),
                format!(
                    "restart_pending_proposal_id={}",
                    seed.restart_pending_proposal_id
                ),
                format!("summary_id={}", seed.summary_id),
            ],
        "manual output was not the exact seven stable ID lines"
    );
}

#[test]
fn fallback_and_tui_commit_equivalent_typed_memory_mutations() {
    let fallback = support::run_fallback_memory_set_scenario();
    let tui = support::run_tui_memory_set_scenario();

    assert!(fallback.command == tui.command, "host Set commands differ");
    assert!(
        fallback.event_payload == tui.event_payload,
        "host Set event payloads differ"
    );
    assert!(fallback.view == tui.view, "host Set views differ");
    assert_eq!(
        fallback.navigation_labels,
        [
            "1 Home",
            "2 Chat",
            "3 Agents",
            "4 Skills",
            "5 Connections",
            "6 Activity",
            "7 Setup",
            "8 Audit",
            "9 Help",
        ]
    );
    assert_eq!(fallback.navigation_labels, tui.navigation_labels);
}

fn assert_resolution_host_parity(action: MemoryResolutionAction) {
    let fallback = support::run_fallback_memory_resolution_scenario(action);
    let tui = support::run_tui_memory_resolution_scenario(action);

    assert!(
        fallback.command == tui.command,
        "host resolution commands differ"
    );
    assert!(
        fallback.event_payload == tui.event_payload,
        "host resolution event payloads differ"
    );
    assert!(fallback.view == tui.view, "host resolution views differ");
    assert!(
        fallback.navigation_labels == tui.navigation_labels,
        "host resolution navigation labels differ"
    );
}

#[test]
fn fallback_and_tui_approve_the_same_exact_proposal_with_equivalent_semantics() {
    assert_resolution_host_parity(MemoryResolutionAction::Approve);
}

#[test]
fn fallback_and_tui_reject_the_same_exact_proposal_with_equivalent_semantics() {
    assert_resolution_host_parity(MemoryResolutionAction::Reject);
}

#[test]
fn bounded_snapshot_is_deterministic_and_never_crosses_namespaces() {
    let mut app = support::app();
    let alpha = create_acceptance_profile(&mut app, 10_000_000, "Acceptance Alpha");
    let beta = create_acceptance_profile(&mut app, 10_000_001, "Acceptance Beta");
    assert!(
        alpha.memory_namespace_id() != beta.memory_namespace_id(),
        "profiles shared a memory namespace"
    );

    let (alpha_entry, alpha_event) = set_acceptance_entry(
        &mut app,
        &alpha,
        10_000_010,
        "Shared acceptance key",
        "alpha acceptance plaintext",
    );
    let (beta_entry, beta_event) = set_acceptance_entry(
        &mut app,
        &beta,
        10_000_011,
        "Shared acceptance key",
        "beta acceptance plaintext",
    );
    let alpha_proposal = propose_acceptance_entry(
        &mut app,
        &alpha,
        10_000_020,
        "Shared acceptance proposal",
        "alpha proposal plaintext",
    );
    let beta_proposal = propose_acceptance_entry(
        &mut app,
        &beta,
        10_000_021,
        "Shared acceptance proposal",
        "beta proposal plaintext",
    );
    let alpha_summary = app
        .record_test_episodic_summary(
            alpha.reference(),
            "Alpha acceptance summary".to_owned(),
            "alpha episodic plaintext".to_owned(),
            vec!["isolation".to_owned()],
            vec![alpha_event],
        )
        .unwrap_or_else(|error| panic!("alpha summary failed: {}", error.code()));
    let beta_summary = app
        .record_test_episodic_summary(
            beta.reference(),
            "Beta acceptance summary".to_owned(),
            "beta episodic plaintext".to_owned(),
            vec!["isolation".to_owned()],
            vec![beta_event],
        )
        .unwrap_or_else(|error| panic!("beta summary failed: {}", error.code()));

    let mut command_id = 10_001_000;
    let CommandView::MemoryEntries(entries) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ListMemoryEntries {
            selector: alpha.profile_id().into(),
        },
    )
    .view
    else {
        panic!("entry list returned the wrong view")
    };
    assert!(
        entries.profile == alpha.reference(),
        "entry list profile mismatch"
    );
    assert!(
        entries.namespace_id == alpha.memory_namespace_id(),
        "entry list namespace mismatch"
    );
    assert!(
        entries.entries.len() == 1 && entries.entries[0].entry == alpha_entry,
        "entry list crossed namespaces"
    );

    let CommandView::MemoryProposals(proposals) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ListMemoryProposals {
            selector: alpha.profile_id().into(),
            filter: MemoryProposalFilter::All,
        },
    )
    .view
    else {
        panic!("proposal list returned the wrong view")
    };
    assert!(
        proposals.profile == alpha.reference()
            && proposals.namespace_id == alpha.memory_namespace_id(),
        "proposal list identity mismatch"
    );
    assert!(
        proposals.proposals.len() == 1
            && proposals.proposals[0].proposal == alpha_proposal
            && proposals.proposals[0].namespace_id == alpha.memory_namespace_id(),
        "proposal list crossed namespaces"
    );

    let CommandView::EpisodicSummaries(summaries) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ListEpisodicSummaries {
            selector: alpha.profile_id().into(),
        },
    )
    .view
    else {
        panic!("summary list returned the wrong view")
    };
    assert!(
        summaries.profile == alpha.reference()
            && summaries.namespace_id == alpha.memory_namespace_id(),
        "summary list identity mismatch"
    );
    assert!(
        summaries.summaries.len() == 1 && summaries.summaries[0].summary == alpha_summary,
        "summary list crossed namespaces"
    );

    let CommandView::MemoryProposal(proposal_detail) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ShowMemoryProposal {
            proposal_id: alpha_proposal.proposal_id(),
        },
    )
    .view
    else {
        panic!("proposal detail returned the wrong view")
    };
    let CommandView::EpisodicSummary(summary_detail) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ShowEpisodicSummary {
            summary_id: alpha_summary.summary_id(),
        },
    )
    .view
    else {
        panic!("summary detail returned the wrong view")
    };
    assert!(
        proposal_detail.proposal.reference() == alpha_proposal
            && summary_detail.summary.reference() == alpha_summary,
        "exact-object detail mismatch"
    );

    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &alpha,
            MemoryPurposeScope::tagged(vec!["isolation".to_owned()]).expect("valid purpose scope"),
        )
        .expect("valid retrieval scope"),
        MemoryRetrievalBudget::default(),
    )
    .expect("valid retrieval request");
    let CommandView::MemorySnapshot(first_snapshot) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::BuildMemorySnapshot {
            request: request.clone(),
        },
    )
    .view
    else {
        panic!("first snapshot returned the wrong view")
    };
    let CommandView::MemorySnapshot(second_snapshot) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::BuildMemorySnapshot { request },
    )
    .view
    else {
        panic!("second snapshot returned the wrong view")
    };
    assert!(
        first_snapshot == second_snapshot,
        "unchanged bounded snapshots were nondeterministic"
    );
    assert!(
        first_snapshot.snapshot.entries().len() == 1
            && first_snapshot.snapshot.entries()[0].entry() == &alpha_entry,
        "snapshot entry crossed namespaces"
    );
    assert!(
        first_snapshot.snapshot.summaries().len() == 1
            && first_snapshot.snapshot.summaries()[0].summary() == &alpha_summary,
        "snapshot summary crossed namespaces"
    );

    let deliberate_alpha_views = serde_json::to_string(&(
        entries,
        proposals,
        summaries,
        proposal_detail,
        summary_detail,
        first_snapshot,
    ))
    .expect("serialize deliberate alpha views");
    for foreign in [
        beta_entry.entry_id().to_string(),
        beta_proposal.proposal_id().to_string(),
        beta_summary.summary_id().to_string(),
        "beta acceptance plaintext".to_owned(),
        "beta proposal plaintext".to_owned(),
        "beta episodic plaintext".to_owned(),
    ] {
        assert!(
            !deliberate_alpha_views.contains(&foreign),
            "an alpha-scoped read contained beta memory"
        );
    }

    let CommandView::MemoryEntries(beta_entries) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ListMemoryEntries {
            selector: beta.profile_id().into(),
        },
    )
    .view
    else {
        panic!("beta entry list returned the wrong view")
    };
    let CommandView::MemoryProposals(beta_proposals) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ListMemoryProposals {
            selector: beta.profile_id().into(),
            filter: MemoryProposalFilter::All,
        },
    )
    .view
    else {
        panic!("beta proposal list returned the wrong view")
    };
    let CommandView::EpisodicSummaries(beta_summaries) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ListEpisodicSummaries {
            selector: beta.profile_id().into(),
        },
    )
    .view
    else {
        panic!("beta summary list returned the wrong view")
    };
    let beta_request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &beta,
            MemoryPurposeScope::tagged(vec!["isolation".to_owned()])
                .expect("valid beta purpose scope"),
        )
        .expect("valid beta retrieval scope"),
        MemoryRetrievalBudget::default(),
    )
    .expect("valid beta retrieval request");
    let CommandView::MemorySnapshot(beta_snapshot) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::BuildMemorySnapshot {
            request: beta_request,
        },
    )
    .view
    else {
        panic!("beta snapshot returned the wrong view")
    };
    assert!(
        beta_entries.profile == beta.reference()
            && beta_entries.namespace_id == beta.memory_namespace_id()
            && beta_entries.entries.len() == 1
            && beta_entries.entries[0].entry == beta_entry,
        "beta entry list crossed namespaces"
    );
    assert!(
        beta_proposals.profile == beta.reference()
            && beta_proposals.namespace_id == beta.memory_namespace_id()
            && beta_proposals.proposals.len() == 1
            && beta_proposals.proposals[0].proposal == beta_proposal,
        "beta proposal list crossed namespaces"
    );
    assert!(
        beta_summaries.profile == beta.reference()
            && beta_summaries.namespace_id == beta.memory_namespace_id()
            && beta_summaries.summaries.len() == 1
            && beta_summaries.summaries[0].summary == beta_summary,
        "beta summary list crossed namespaces"
    );
    assert!(
        beta_snapshot.snapshot.entries().len() == 1
            && beta_snapshot.snapshot.entries()[0].entry() == &beta_entry
            && beta_snapshot.snapshot.summaries().len() == 1
            && beta_snapshot.snapshot.summaries()[0].summary() == &beta_summary,
        "beta snapshot crossed namespaces"
    );
    let deliberate_beta_views =
        serde_json::to_string(&(beta_entries, beta_proposals, beta_summaries, beta_snapshot))
            .expect("serialize deliberate beta views");
    for foreign in [
        alpha_entry.entry_id().to_string(),
        alpha_proposal.proposal_id().to_string(),
        alpha_summary.summary_id().to_string(),
        "alpha acceptance plaintext".to_owned(),
        "alpha proposal plaintext".to_owned(),
        "alpha episodic plaintext".to_owned(),
    ] {
        assert!(
            !deliberate_beta_views.contains(&foreign),
            "a beta-scoped read contained alpha memory"
        );
    }
}

#[test]
fn copied_non_memory_profile_fields_create_a_fresh_empty_namespace() {
    let mut app = support::app();
    let source = create_acceptance_profile(&mut app, 10_100_000, "Acceptance copy source");
    let (_, source_event) = set_acceptance_entry(
        &mut app,
        &source,
        10_100_001,
        "Copy source entry",
        "copy-source entry plaintext",
    );
    propose_acceptance_entry(
        &mut app,
        &source,
        10_100_002,
        "Copy source proposal",
        "copy-source proposal plaintext",
    );
    app.record_test_episodic_summary(
        source.reference(),
        "Copy source summary".to_owned(),
        "copy-source episodic plaintext".to_owned(),
        vec!["isolation".to_owned()],
        vec![source_event],
    )
    .unwrap_or_else(|error| panic!("copy source summary failed: {}", error.code()));

    let mut copied_draft = source.to_draft();
    copied_draft.display_name = "Acceptance copied profile".to_owned();
    let expected_non_memory_fields = copied_draft.clone();
    let created = app
        .execute(acceptance_envelope(
            10_100_003,
            Actor::Human,
            ApplicationCommand::CreateAgentProfile {
                draft: copied_draft,
                template_provenance: None,
            },
        ))
        .unwrap_or_else(|error| panic!("copied profile failed: {}", error.code()));
    let CommandView::AgentProfileCreated(created) = created.view else {
        panic!("copied profile returned the wrong view")
    };
    let copy = app
        .projection()
        .agent_profiles
        .active_profile(created.profile_id)
        .cloned()
        .unwrap_or_else(|| panic!("copied profile was not projected"));
    assert!(
        copy.to_draft() == expected_non_memory_fields,
        "copied non-memory fields changed"
    );
    assert!(
        copy.memory_namespace_id() != source.memory_namespace_id(),
        "copied profile reused the source namespace"
    );

    let mut command_id = 10_101_000;
    let CommandView::MemoryEntries(entries) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ListMemoryEntries {
            selector: copy.profile_id().into(),
        },
    )
    .view
    else {
        panic!("copied entry list returned the wrong view")
    };
    let CommandView::MemoryProposals(proposals) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ListMemoryProposals {
            selector: copy.profile_id().into(),
            filter: MemoryProposalFilter::All,
        },
    )
    .view
    else {
        panic!("copied proposal list returned the wrong view")
    };
    let CommandView::EpisodicSummaries(summaries) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ListEpisodicSummaries {
            selector: copy.profile_id().into(),
        },
    )
    .view
    else {
        panic!("copied summary list returned the wrong view")
    };
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &copy,
            MemoryPurposeScope::tagged(vec!["isolation".to_owned()])
                .expect("valid copied purpose scope"),
        )
        .expect("valid copied retrieval scope"),
        MemoryRetrievalBudget::default(),
    )
    .expect("valid copied retrieval request");
    let CommandView::MemorySnapshot(snapshot) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::BuildMemorySnapshot { request },
    )
    .view
    else {
        panic!("copied snapshot returned the wrong view")
    };
    assert!(
        entries.profile == copy.reference()
            && proposals.profile == copy.reference()
            && summaries.profile == copy.reference(),
        "copied list identity mismatch"
    );
    assert!(
        entries.entries.is_empty()
            && proposals.proposals.is_empty()
            && summaries.summaries.is_empty()
            && snapshot.snapshot.entries().is_empty()
            && snapshot.snapshot.summaries().is_empty(),
        "copied profile inherited memory"
    );
}

#[test]
fn source_linked_episodic_summary_is_read_only_and_visibly_qualified() {
    let mut app = support::app();
    let profile = create_acceptance_profile(&mut app, 10_200_000, "Episodic acceptance");
    let (_, source_event) = set_acceptance_entry(
        &mut app,
        &profile,
        10_200_001,
        "Episodic source",
        "source-linked entry plaintext",
    );
    let summary = app
        .record_test_episodic_summary(
            profile.reference(),
            "Qualified acceptance summary".to_owned(),
            "source-linked summary plaintext".to_owned(),
            vec!["isolation".to_owned()],
            vec![source_event],
        )
        .unwrap_or_else(|error| panic!("episodic summary failed: {}", error.code()));
    let summary_rows = app.count_rows("episodic_summaries");
    let source_rows = app.count_rows("episodic_summary_sources");

    let mut command_id = 10_201_000;
    let CommandView::EpisodicSummary(first) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ShowEpisodicSummary {
            summary_id: summary.summary_id(),
        },
    )
    .view
    else {
        panic!("episodic detail returned the wrong view")
    };
    let CommandView::EpisodicSummary(second) = acceptance_read(
        &mut app,
        &mut command_id,
        ApplicationCommand::ShowEpisodicSummary {
            summary_id: summary.summary_id(),
        },
    )
    .view
    else {
        panic!("repeated episodic detail returned the wrong view")
    };

    assert!(first == second, "episodic detail changed between reads");
    assert!(
        first.qualification == EpisodicQualification::SummaryVerifySources
            && first.qualification.label() == "Summary — verify sources",
        "episodic detail lacked its source qualification"
    );
    assert!(
        first.summary.sources().len() == 1 && first.summary.sources()[0].event_id() == source_event,
        "episodic source reference changed"
    );
    let rendered = render_acceptance_view(&CommandView::EpisodicSummary(first.clone()));
    assert!(
        rendered.contains("Summary — verify sources")
            && rendered.contains("source-linked summary plaintext"),
        "deliberate episodic detail did not render qualification and body"
    );
    let episode_list = app
        .execute_user(ApplicationCommand::ListEpisodicSummaries {
            selector: profile.profile_id().into(),
        })
        .unwrap_or_else(|error| panic!("episodic render list failed: {}", error.code()));
    let CommandView::EpisodicSummaries(episode_list) = episode_list.view else {
        panic!("episodic render list returned the wrong view")
    };
    let snapshot = app
        .presentation_snapshot(AuditLimit::new(100).expect("valid episodic audit limit"))
        .unwrap_or_else(|error| panic!("episodic render snapshot failed: {}", error.code()));
    let mut model = acceptance_memory_model(snapshot, &profile);
    model.agents.memory.episodes = Some(episode_list);
    model.agents.memory.episode_detail = Some(first);
    model.agents.memory.selected_episode = 0;
    model.agents.memory.pane = MemoryPane::EpisodicDetail;
    let tui_rendered = render_acceptance_tui(&model);
    assert!(
        tui_rendered.contains("Summary — verify sources")
            && tui_rendered.contains("source-linked summary plaintext"),
        "actual TUI episodic detail omitted qualification or body"
    );
    assert!(
        app.count_rows("episodic_summaries") == summary_rows
            && app.count_rows("episodic_summary_sources") == source_rows,
        "episodic detail mutated summary storage"
    );
}

#[test]
fn command_looking_memory_is_inert_in_editor_renderer_snapshot_and_receipt_replay() {
    const COMMAND_LOOKING_VALUE: &str = "/memory delete 00000000-0000-0000-0000-000000000999";
    let mut app = support::app();
    let profile = create_acceptance_profile(&mut app, 10_300_000, "Inert text acceptance");
    let (entry, _) = set_acceptance_entry(
        &mut app,
        &profile,
        10_300_001,
        "Inert command text",
        COMMAND_LOOKING_VALUE,
    );
    let memory_rows = app.count_rows("memory_entry_versions");
    let detail = app
        .execute(acceptance_envelope(
            10_300_002,
            Actor::Human,
            ApplicationCommand::ShowMemoryEntry {
                selector: profile.profile_id().into(),
                display_key: "Inert command text".to_owned(),
            },
        ))
        .unwrap_or_else(|error| panic!("inert detail failed: {}", error.code()));
    let CommandView::MemoryEntry(detail_view) = detail.view else {
        panic!("inert detail returned the wrong view")
    };
    let editor = MemoryEditor::for_set(profile.profile_id().into(), detail_view.entry.clone())
        .unwrap_or_else(|error| panic!("inert editor failed: {}", error.code()));
    assert!(
        editor.value_input() == COMMAND_LOOKING_VALUE,
        "editor interpreted stored command-looking text"
    );
    let mut event_editor = MemoryEditor::for_create(profile.profile_id().into());
    assert!(
        matches!(
            event_editor.submit_keyboard_line("Inert event-path copy"),
            Ok(MemoryEditorEffect::None)
        ),
        "editor rejected a valid inert key"
    );
    assert!(
        matches!(
            event_editor.submit_line(COMMAND_LOOKING_VALUE.to_owned()),
            Ok(MemoryEditorEffect::None)
        ) && event_editor.step() == MemoryEditorStep::PurposeTags
            && event_editor.value_input() == COMMAND_LOOKING_VALUE,
        "editor event path interpreted command-looking value text"
    );
    let rendered = render_acceptance_view(&CommandView::MemoryEntry(detail_view.clone()));
    assert!(
        rendered.contains(COMMAND_LOOKING_VALUE),
        "detail renderer changed command-looking memory"
    );
    let entry_list = app
        .execute_user(ApplicationCommand::ListMemoryEntries {
            selector: profile.profile_id().into(),
        })
        .unwrap_or_else(|error| panic!("inert render list failed: {}", error.code()));
    let CommandView::MemoryEntries(entry_list) = entry_list.view else {
        panic!("inert render list returned the wrong view")
    };
    let presentation = app
        .presentation_snapshot(AuditLimit::new(100).expect("valid inert audit limit"))
        .unwrap_or_else(|error| panic!("inert render snapshot failed: {}", error.code()));
    let mut model = acceptance_memory_model(presentation, &profile);
    model.agents.memory.entries = Some(entry_list);
    model.agents.memory.entry_detail = Some(detail_view);
    model.agents.memory.selected_entry = 0;
    model.agents.memory.pane = MemoryPane::EntryDetail;
    assert!(
        render_acceptance_tui(&model).contains(COMMAND_LOOKING_VALUE),
        "actual TUI entry detail changed command-looking memory"
    );

    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &profile,
            MemoryPurposeScope::tagged(vec!["isolation".to_owned()])
                .expect("valid inert purpose scope"),
        )
        .expect("valid inert retrieval scope"),
        MemoryRetrievalBudget::default(),
    )
    .expect("valid inert retrieval request");
    let replay_envelope = acceptance_envelope(
        10_300_100,
        Actor::Human,
        ApplicationCommand::BuildMemorySnapshot { request },
    );
    let first = app
        .execute(replay_envelope.clone())
        .unwrap_or_else(|error| panic!("inert snapshot failed: {}", error.code()));
    let event_count = app.count_rows("event_stream");
    let replay = app
        .execute(replay_envelope)
        .unwrap_or_else(|error| panic!("inert snapshot replay failed: {}", error.code()));
    assert!(first == replay, "receipt replay changed the inert snapshot");
    assert!(
        app.count_rows("event_stream") == event_count,
        "receipt replay appended a second event"
    );
    let CommandView::MemorySnapshot(snapshot) = first.view else {
        panic!("inert snapshot returned the wrong view")
    };
    assert!(
        snapshot.snapshot.entries().len() == 1
            && snapshot.snapshot.entries()[0].entry() == &entry
            && snapshot.snapshot.entries()[0].value() == COMMAND_LOOKING_VALUE,
        "snapshot interpreted command-looking memory"
    );
    assert!(
        app.count_rows("memory_entry_versions") == memory_rows,
        "command-looking memory caused a mutation"
    );
}

#[test]
fn routine_views_redact_memory_prose_and_internal_producer_routes_remain_absent() {
    const ENTRY_KEY: &str = "routine-entry-key-acceptance";
    const ENTRY_VALUE: &str = "routine-entry-value-acceptance";
    const ENTRY_TAG: &str = "routine-entry-tag-acceptance";
    const PROPOSAL_KEY: &str = "routine-proposal-key-acceptance";
    const PROPOSAL_VALUE: &str = "routine-proposal-value-acceptance";
    const PROPOSAL_TAG: &str = "routine-proposal-tag-acceptance";
    const PROPOSAL_RATIONALE: &str = "routine-proposal-rationale-acceptance";
    const SUMMARY_LABEL: &str = "routine-summary-label-acceptance";
    const SUMMARY_BODY: &str = "routine-summary-body-acceptance";
    const SUMMARY_TAG: &str = "routine-summary-tag-acceptance";
    const ERROR_KEY: &str = "routine-failing-command-acceptance";
    let mut app = support::app();
    let profile = create_acceptance_profile(&mut app, 10_400_000, "Redaction acceptance");
    let (_, source_event) = set_acceptance_entry_with_tag(
        &mut app,
        &profile,
        10_400_001,
        ENTRY_KEY,
        ENTRY_VALUE,
        ENTRY_TAG,
    );
    propose_acceptance_entry_with_metadata(
        &mut app,
        &profile,
        10_400_002,
        PROPOSAL_KEY,
        PROPOSAL_VALUE,
        PROPOSAL_TAG,
        PROPOSAL_RATIONALE,
    );
    app.record_test_episodic_summary(
        profile.reference(),
        SUMMARY_LABEL.to_owned(),
        SUMMARY_BODY.to_owned(),
        vec![SUMMARY_TAG.to_owned()],
        vec![source_event],
    )
    .unwrap_or_else(|error| panic!("redaction summary failed: {}", error.code()));

    let mut routine_text = String::new();
    for command in [
        ApplicationCommand::ShowHelp,
        ApplicationCommand::ShowStatus,
        ApplicationCommand::ShowSetupStatus,
        ApplicationCommand::ShowAuditTail {
            limit: AuditLimit::new(100).expect("valid audit limit"),
        },
    ] {
        let outcome = app
            .execute_user(command)
            .unwrap_or_else(|error| panic!("routine view failed: {}", error.code()));
        routine_text.push_str(&render_acceptance_view(&outcome.view));
    }
    let failing_memory_command = app
        .execute_user(ApplicationCommand::ShowMemoryEntry {
            selector: profile.profile_id().into(),
            display_key: ERROR_KEY.to_owned(),
        })
        .err()
        .unwrap_or_else(|| panic!("sentinel-bearing memory command unexpectedly succeeded"));
    assert!(
        failing_memory_command == AppError::MemoryEntryNotFound,
        "sentinel-bearing memory command returned an unexpected error"
    );
    routine_text.push_str(&render_acceptance_error(&RuntimeError::Application(
        failing_memory_command,
    )));
    for sentinel in [
        ENTRY_KEY,
        ENTRY_VALUE,
        ENTRY_TAG,
        PROPOSAL_KEY,
        PROPOSAL_VALUE,
        PROPOSAL_TAG,
        PROPOSAL_RATIONALE,
        SUMMARY_LABEL,
        SUMMARY_BODY,
        SUMMARY_TAG,
        ERROR_KEY,
    ] {
        assert!(
            !routine_text.contains(sentinel),
            "routine output disclosed memory prose"
        );
    }

    let entries = app
        .execute_user(ApplicationCommand::ListMemoryEntries {
            selector: profile.profile_id().into(),
        })
        .unwrap_or_else(|error| panic!("redaction entry list failed: {}", error.code()));
    let proposals = app
        .execute_user(ApplicationCommand::ListMemoryProposals {
            selector: profile.profile_id().into(),
            filter: MemoryProposalFilter::All,
        })
        .unwrap_or_else(|error| panic!("redaction proposal list failed: {}", error.code()));
    let summaries = app
        .execute_user(ApplicationCommand::ListEpisodicSummaries {
            selector: profile.profile_id().into(),
        })
        .unwrap_or_else(|error| panic!("redaction summary list failed: {}", error.code()));
    let entry_list = render_acceptance_view(&entries.view);
    let proposal_list = render_acceptance_view(&proposals.view);
    let summary_list = render_acceptance_view(&summaries.view);
    assert!(
        entry_list.contains(ENTRY_KEY)
            && entry_list.contains(ENTRY_TAG)
            && !entry_list.contains(ENTRY_VALUE),
        "entry metadata list exposed a value or omitted declared metadata"
    );
    assert!(
        proposal_list.contains(PROPOSAL_KEY)
            && !proposal_list.contains(PROPOSAL_VALUE)
            && !proposal_list.contains(PROPOSAL_TAG)
            && !proposal_list.contains(PROPOSAL_RATIONALE),
        "proposal metadata list exposed deliberate proposal prose"
    );
    assert!(
        summary_list.contains(SUMMARY_LABEL)
            && summary_list.contains(SUMMARY_TAG)
            && !summary_list.contains(SUMMARY_BODY),
        "summary metadata list exposed a body or omitted declared metadata"
    );

    for unsupported in [
        b"/memory propose synthetic".as_slice(),
        b"/memory summarize synthetic".as_slice(),
        b"/memory snapshot synthetic".as_slice(),
    ] {
        assert!(
            matches!(
                parse_fallback_line(unsupported),
                FallbackParsedLine::Command(ApplicationCommand::RejectInput(_))
            ),
            "fallback exposed an internal memory producer"
        );
        assert!(
            matches!(
                parse_line(unsupported),
                ParsedLine::Command(ApplicationCommand::RejectInput(_))
            ),
            "TUI parser exposed an internal memory producer"
        );
    }
    let help = render_acceptance_view(&CommandView::Help(ai_stock_forum::app::HelpView));
    const HELP_MEMORY_COMMANDS: [&str; 11] = [
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
    ];
    let rendered_help_commands = help
        .lines()
        .skip_while(|line| *line != "Memory commands:")
        .skip(1)
        .take_while(|line| line.trim() != "/quit")
        .map(str::trim)
        .collect::<Vec<_>>();
    assert!(
        rendered_help_commands == HELP_MEMORY_COMMANDS,
        "rendered help did not contain the exact supported Memory grammar"
    );
    assert!(
        help.contains(
            "Memory internal producers are unavailable: proposal creation, summary mutation, and snapshot building."
        ),
        "help omitted the internal-producer boundary"
    );

    let usage_command = match parse_fallback_line(b"/memory") {
        FallbackParsedLine::Command(command @ ApplicationCommand::RejectInput(_)) => command,
        _ => panic!("bare Memory command did not produce usage rejection"),
    };
    let usage = app
        .execute_user(usage_command)
        .unwrap_or_else(|error| panic!("Memory usage render failed: {}", error.code()));
    let usage = render_acceptance_view(&usage.view);
    const USAGE_MEMORY_COMMANDS: [&str; 11] = [
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
    ];
    let rendered_usage_commands = usage
        .lines()
        .skip_while(|line| *line != "Usage:")
        .skip(1)
        .map(str::trim)
        .collect::<Vec<_>>();
    assert!(
        rendered_usage_commands == USAGE_MEMORY_COMMANDS,
        "rendered usage did not contain the exact supported Memory grammar"
    );
}

#[test]
fn restart_preserves_history_proposals_approvals_and_deterministic_fresh_retrieval() {
    const RESTART_COMMAND_VALUE: &str = "/memory delete 00000000-0000-0000-0000-000000000777";
    let mut fixture = support::persistent_fixture();
    let mut service = fixture.service();
    let profile = create_acceptance_profile(&mut service, 10_500_000, "Restart acceptance");
    let (tombstone_present, tombstone_source_event) = set_acceptance_entry(
        &mut service,
        &profile,
        10_500_001,
        "Restart tombstone",
        "restart tombstone plaintext",
    );
    let (inert_entry, inert_source_event) = set_acceptance_entry(
        &mut service,
        &profile,
        10_500_002,
        "Restart inert command",
        RESTART_COMMAND_VALUE,
    );
    let deleted_entry =
        delete_acceptance_entry(&mut service, &profile, 10_500_003, "Restart tombstone");

    let accepted_one = propose_acceptance_entry(
        &mut service,
        &profile,
        10_500_010,
        "Restart accepted one",
        "restart accepted one plaintext",
    );
    let accepted_two = propose_acceptance_entry(
        &mut service,
        &profile,
        10_500_011,
        "Restart accepted two",
        "restart accepted two plaintext",
    );
    let rejected_one = propose_acceptance_entry(
        &mut service,
        &profile,
        10_500_012,
        "Restart rejected one",
        "restart rejected one plaintext",
    );
    let rejected_two = propose_acceptance_entry(
        &mut service,
        &profile,
        10_500_013,
        "Restart rejected two",
        "restart rejected two plaintext",
    );
    let still_pending = propose_acceptance_entry(
        &mut service,
        &profile,
        10_500_014,
        "Restart still pending",
        "restart pending plaintext",
    );
    let pending_before_restart = service
        .execute(acceptance_envelope(
            10_500_015,
            Actor::Human,
            ApplicationCommand::ShowMemoryProposal {
                proposal_id: still_pending.proposal_id(),
            },
        ))
        .unwrap_or_else(|error| panic!("pending proposal read failed: {}", error.code()));
    let CommandView::MemoryProposal(pending_before_restart) = pending_before_restart.view else {
        panic!("pending proposal read returned the wrong view")
    };
    assert!(
        pending_before_restart.proposal.reference() == still_pending
            && pending_before_restart.status == MemoryProposalStatus::Pending
            && pending_before_restart.resolution.is_none(),
        "pending proposal was not exact before restart"
    );
    let pending_approval_id = pending_before_restart.proposal.approval_id();
    let accepted_one_approval = resolve_acceptance_proposal(
        &mut service,
        10_500_020,
        accepted_one.clone(),
        MemoryResolutionAction::Approve,
    );
    let accepted_two_approval = resolve_acceptance_proposal(
        &mut service,
        10_500_021,
        accepted_two.clone(),
        MemoryResolutionAction::Approve,
    );
    let rejected_one_approval = resolve_acceptance_proposal(
        &mut service,
        10_500_022,
        rejected_one.clone(),
        MemoryResolutionAction::Reject,
    );
    let rejected_two_approval = resolve_acceptance_proposal(
        &mut service,
        10_500_023,
        rejected_two.clone(),
        MemoryResolutionAction::Reject,
    );
    let summary = support::record_test_episodic_summary(
        &mut fixture,
        profile.reference(),
        "Restart source-linked summary".to_owned(),
        "restart summary plaintext".to_owned(),
        vec!["isolation".to_owned()],
        vec![tombstone_source_event, inert_source_event],
    )
    .unwrap_or_else(|error| panic!("restart summary failed: {}", error.code()));

    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(
            &profile,
            MemoryPurposeScope::tagged(vec!["isolation".to_owned()])
                .expect("valid restart purpose scope"),
        )
        .expect("valid restart retrieval scope"),
        MemoryRetrievalBudget::default(),
    )
    .expect("valid restart retrieval request");
    let original_envelope = acceptance_envelope(
        10_500_100,
        Actor::Human,
        ApplicationCommand::BuildMemorySnapshot {
            request: request.clone(),
        },
    );
    let original = service
        .execute(original_envelope.clone())
        .unwrap_or_else(|error| panic!("original restart snapshot failed: {}", error.code()));
    service
        .finish(ShutdownReason::InputClosed)
        .unwrap_or_else(|error| panic!("first restart service finish failed: {}", error.code()));
    drop(service);

    let mut reopened = fixture.service();
    let inert_detail = reopened
        .execute(acceptance_envelope(
            10_501_000,
            Actor::Human,
            ApplicationCommand::ShowMemoryEntry {
                selector: profile.profile_id().into(),
                display_key: "Restart inert command".to_owned(),
            },
        ))
        .unwrap_or_else(|error| panic!("restarted inert detail failed: {}", error.code()));
    let CommandView::MemoryEntry(inert_detail) = inert_detail.view else {
        panic!("restarted inert detail returned the wrong view")
    };
    assert!(
        inert_detail.entry.reference() == inert_entry
            && inert_detail.entry.value() == Some(RESTART_COMMAND_VALUE),
        "command-looking memory changed across restart"
    );

    let history = reopened
        .execute(acceptance_envelope(
            10_501_001,
            Actor::Human,
            ApplicationCommand::ShowMemoryEntryHistory {
                selector: profile.profile_id().into(),
                display_key: "Restart tombstone".to_owned(),
            },
        ))
        .unwrap_or_else(|error| panic!("restarted history failed: {}", error.code()));
    let CommandView::MemoryEntryHistory(history) = history.view else {
        panic!("restarted history returned the wrong view")
    };
    let exact_versions = matches!(
        history.versions.as_slice(),
        [deleted, present]
            if deleted.entry == deleted_entry
                && deleted.entry.state() == MemoryEntryState::Deleted
                && present.entry == tombstone_present
                && present.entry.state() == MemoryEntryState::Present
    );
    assert!(
        history.current == deleted_entry
            && exact_versions
            && history.total_count == 2
            && history.returned_count == 2
            && history.omitted_count == 0,
        "tombstone history changed across restart"
    );

    let listed = reopened
        .execute(acceptance_envelope(
            10_501_002,
            Actor::Human,
            ApplicationCommand::ListMemoryProposals {
                selector: profile.profile_id().into(),
                filter: MemoryProposalFilter::All,
            },
        ))
        .unwrap_or_else(|error| panic!("restarted proposal list failed: {}", error.code()));
    let CommandView::MemoryProposals(listed) = listed.view else {
        panic!("restarted proposal list returned the wrong view")
    };
    assert!(
        listed.total_count == 5
            && listed.returned_count == 5
            && listed.omitted_count == 0
            && listed.proposals.len() == 5,
        "proposal count changed across restart"
    );
    let statuses = listed
        .proposals
        .iter()
        .map(|proposal| (proposal.proposal.proposal_id(), proposal.status))
        .collect::<std::collections::BTreeMap<_, _>>();
    for proposal in [&accepted_one, &accepted_two] {
        assert!(
            statuses.get(&proposal.proposal_id()) == Some(&MemoryProposalStatus::Accepted),
            "accepted proposal status changed across restart"
        );
    }
    for proposal in [&rejected_one, &rejected_two] {
        assert!(
            statuses.get(&proposal.proposal_id()) == Some(&MemoryProposalStatus::Rejected),
            "rejected proposal status changed across restart"
        );
    }
    assert!(
        statuses.get(&still_pending.proposal_id()) == Some(&MemoryProposalStatus::Pending),
        "pending proposal status changed across restart"
    );

    for (proposal_detail_command_id, (proposal, status, approval_id)) in (10_501_010..).zip([
        (
            accepted_one,
            MemoryProposalStatus::Accepted,
            accepted_one_approval,
        ),
        (
            accepted_two,
            MemoryProposalStatus::Accepted,
            accepted_two_approval,
        ),
        (
            rejected_one,
            MemoryProposalStatus::Rejected,
            rejected_one_approval,
        ),
        (
            rejected_two,
            MemoryProposalStatus::Rejected,
            rejected_two_approval,
        ),
    ]) {
        let detail = reopened
            .execute(acceptance_envelope(
                proposal_detail_command_id,
                Actor::Human,
                ApplicationCommand::ShowMemoryProposal {
                    proposal_id: proposal.proposal_id(),
                },
            ))
            .unwrap_or_else(|error| panic!("restarted proposal detail failed: {}", error.code()));
        let CommandView::MemoryProposal(detail) = detail.view else {
            panic!("restarted proposal detail returned the wrong view")
        };
        let resolution = detail
            .resolution
            .as_ref()
            .unwrap_or_else(|| panic!("terminal proposal lost its resolution"));
        assert!(
            detail.status == status
                && detail.proposal.reference() == proposal
                && detail.proposal.approval_id() == approval_id
                && resolution.approval_id() == approval_id
                && *resolution.resolved_by() == Actor::Human
                && detail.proposer_identity.profile == profile.reference()
                && detail.namespace_owner_identity.profile == profile.reference(),
            "terminal proposal identity changed across restart"
        );
    }
    let pending_detail = reopened
        .execute(acceptance_envelope(
            10_501_020,
            Actor::Human,
            ApplicationCommand::ShowMemoryProposal {
                proposal_id: still_pending.proposal_id(),
            },
        ))
        .unwrap_or_else(|error| panic!("pending proposal detail failed: {}", error.code()));
    let CommandView::MemoryProposal(pending_detail) = pending_detail.view else {
        panic!("pending proposal detail returned the wrong view")
    };
    assert!(
        pending_detail.status == MemoryProposalStatus::Pending
            && pending_detail.resolution.is_none()
            && pending_detail.proposal.reference() == still_pending
            && pending_detail.proposal.approval_id() == pending_approval_id,
        "pending proposal became terminal across restart"
    );

    let fresh_one = reopened
        .execute(acceptance_envelope(
            10_501_100,
            Actor::Human,
            ApplicationCommand::BuildMemorySnapshot {
                request: request.clone(),
            },
        ))
        .unwrap_or_else(|error| panic!("first fresh snapshot failed: {}", error.code()));
    let fresh_two = reopened
        .execute(acceptance_envelope(
            10_501_101,
            Actor::Human,
            ApplicationCommand::BuildMemorySnapshot { request },
        ))
        .unwrap_or_else(|error| panic!("second fresh snapshot failed: {}", error.code()));
    let (CommandView::MemorySnapshot(fresh_one_view), CommandView::MemorySnapshot(fresh_two_view)) =
        (&fresh_one.view, &fresh_two.view)
    else {
        panic!("fresh retrieval returned the wrong view")
    };
    assert!(
        fresh_one.command_id != fresh_two.command_id
            && fresh_one_view == fresh_two_view
            && fresh_one_view.snapshot.snapshot_digest()
                == fresh_two_view.snapshot.snapshot_digest(),
        "fresh distinct-ID retrieval was nondeterministic"
    );
    assert!(
        fresh_one_view
            .snapshot
            .entries()
            .iter()
            .any(|item| item.entry() == &inert_entry && item.value() == RESTART_COMMAND_VALUE)
            && fresh_one_view
                .snapshot
                .summaries()
                .iter()
                .any(|item| item.summary() == &summary),
        "fresh retrieval lost recovered inert or episodic memory"
    );
    let CommandView::MemorySnapshot(original_view) = &original.view else {
        panic!("original snapshot returned the wrong view")
    };
    assert!(
        original_view == fresh_one_view,
        "fresh recovered snapshot differs from pre-restart content"
    );

    let before_replay = fixture.events().len();
    let replay = reopened
        .execute(original_envelope)
        .unwrap_or_else(|error| panic!("original snapshot replay failed: {}", error.code()));
    assert!(
        original == replay,
        "original command receipt did not replay exactly"
    );
    assert!(
        fixture.events().len() == before_replay,
        "original command replay appended an event"
    );
    reopened
        .finish(ShutdownReason::InputClosed)
        .unwrap_or_else(|error| panic!("reopened restart service finish failed: {}", error.code()));
    drop(reopened);
}

#[test]
#[ignore = "creates a disposable local state directory for manual Hybrid Memory acceptance"]
fn seed_manual_acceptance_state() {
    let discovered = AppPaths::discover()
        .unwrap_or_else(|error| panic!("manual acceptance discovery failed: {}", error.code()));
    let requested = env::var_os("AI_STOCK_FORUM_MEMORY_ACCEPTANCE_STATE_DIR")
        .map(std::path::PathBuf::from)
        .expect("AI_STOCK_FORUM_MEMORY_ACCEPTANCE_STATE_DIR is required");
    let target_exists = classify_target_metadata(fs::symlink_metadata(&requested))
        .unwrap_or_else(|message| panic!("{message}"));
    validate_manual_acceptance_target(&requested, discovered.state_dir(), target_exists)
        .unwrap_or_else(|message| panic!("{message}"));

    let seed = support::seed_manual_memory_acceptance(&discovered)
        .unwrap_or_else(|error| panic!("manual acceptance seed failed: {}", error.code()));
    for line in manual_acceptance_output(&seed) {
        println!("{line}");
    }
}
