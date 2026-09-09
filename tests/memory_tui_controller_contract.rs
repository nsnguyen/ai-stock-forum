use ai_stock_forum::{
    agents::{AgentProfileVersion, AgentReadiness, builtin_profile_templates},
    app::{
        AgentProfileHistoryView, AgentProfileSelector, AgentProfileSummary,
        AgentProfileVersionView, AgentProfileView, AgentProfilesView, ApplicationCommand,
        CommandView, DatabaseReadiness, EpisodicSummariesView, EpisodicSummaryListItem,
        EpisodicSummaryView, HelpView, MAX_INPUT_BYTES, MemoryEditPreview, MemoryEntriesView,
        MemoryEntryHistorySummary, MemoryEntryHistoryView, MemoryEntryMutationView,
        MemoryEntrySummary, MemoryEntryVersionView, MemoryEntryView, MemoryProfileIdentityView,
        MemoryProposalResolutionReview, MemoryProposalResolutionView, MemoryProposalSummary,
        MemoryProposalView, MemoryProposalsView, PresentationSnapshot, ProcessGuardOwnership,
    },
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, DomainError, EpisodicSummaryId,
        EventId, InstallationId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId,
        MemoryProposalId, MemoryReviewToken, ObjectVersion, SessionId, sha256,
    },
    memory::{
        EpisodicQualification, EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState,
        MemoryEditReview, MemoryEntryDraft, MemoryEntryVersion, MemoryMutationKind, MemoryNoChange,
        MemoryPlaintextAcknowledgement, MemoryProposal, MemoryProposalFilter,
        MemoryProposalOperation, MemoryProposalOperationKind, MemoryProposalResolution,
        MemoryProposalStatus, MemoryResolutionAction,
    },
    policy::ApprovalStatus,
    setup::SetupStatus,
    ui::tui::{
        ControllerEffect, MemoryConfirmation, MemoryEditorOrigin, MemoryEntryDetailAction,
        MemoryOutcomeIntent, MemoryPane, MemoryProposalDetailAction, MemoryResultOrigin,
        MemoryViewState, TuiEvent, handle_event,
        model::{AgentDetailAction, AgentsPane, Focus, TuiModel, View},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use uuid::Uuid;

fn model() -> TuiModel {
    TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            database_readiness: DatabaseReadiness::Ready,
            process_guard_ownership: ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: Vec::new(),
            agent_profiles: AgentProfilesView {
                profiles: Vec::new(),
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

fn profile(seed: u128) -> AgentProfileVersion {
    let template = &builtin_profile_templates()[0];
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(seed)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(seed + 2)),
        1_800_000_000_000,
        template.copy_to_draft().expect("builtin profile draft"),
        Some(template.provenance()),
    )
    .expect("profile")
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn repeated_key(code: KeyCode) -> TuiEvent {
    let mut key = KeyEvent::new(code, KeyModifiers::NONE);
    key.kind = crossterm::event::KeyEventKind::Repeat;
    TuiEvent::Key(key)
}

fn memory_identity(profile: &AgentProfileVersion) -> MemoryProfileIdentityView {
    MemoryProfileIdentityView {
        profile: profile.reference(),
        display_name: profile.display_name().to_owned(),
    }
}

fn profile_summary(profile: &AgentProfileVersion) -> AgentProfileSummary {
    AgentProfileSummary {
        profile_id: profile.profile_id(),
        profile_version_id: profile.profile_version_id(),
        version: profile.version(),
        display_name: profile.display_name().to_owned(),
        role: profile.role(),
        primary_specialty: profile.primary_specialty().to_owned(),
        readiness: AgentReadiness::Unbound,
        content_digest: profile.content_digest().clone(),
    }
}

fn profiles_view(profiles: &[&AgentProfileVersion]) -> AgentProfilesView {
    AgentProfilesView {
        profiles: profiles
            .iter()
            .map(|profile| profile_summary(profile))
            .collect(),
        total_count: u32::try_from(profiles.len()).expect("fixture count"),
        returned_count: u32::try_from(profiles.len()).expect("fixture count"),
        truncated: false,
    }
}

fn successor(profile: &AgentProfileVersion, seed: u128) -> AgentProfileVersion {
    let mut draft = profile.to_draft();
    draft.description = format!("successor {seed}");
    AgentProfileVersion::next_version(
        profile,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(seed)),
        1_800_000_000_001,
        draft,
    )
    .expect("profile successor")
}

fn set_review(profile: &AgentProfileVersion, seed: u128, key: &str) -> MemoryEditReview {
    MemoryEditReview {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        expected: ExpectedMemoryEntryState::Absent,
        operation: MemoryMutationKind::Set,
        candidate: Some(
            MemoryEntryDraft::new(key.to_owned(), format!("review value {seed}"), Vec::new())
                .expect("review draft"),
        ),
        diff: Vec::new(),
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed)),
        review_digest: sha256(format!("review {seed}").as_bytes()),
    }
}

fn set_command(review: &MemoryEditReview) -> ApplicationCommand {
    ApplicationCommand::SetMemoryEntry {
        profile: review.profile.clone(),
        expected: review.expected.clone(),
        candidate: review.candidate.clone().expect("set candidate"),
        review_token: review.review_token,
        review_digest: review.review_digest.clone(),
    }
}

fn delete_review(
    profile: &AgentProfileVersion,
    current: &MemoryEntryVersion,
    seed: u128,
) -> MemoryEditReview {
    MemoryEditReview {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        expected: ExpectedMemoryEntryState::Present(current.reference()),
        operation: MemoryMutationKind::Delete,
        candidate: None,
        diff: Vec::new(),
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed)),
        review_digest: sha256(format!("delete review {seed}").as_bytes()),
    }
}

fn delete_command(review: &MemoryEditReview) -> ApplicationCommand {
    let expected = match &review.expected {
        ExpectedMemoryEntryState::Present(reference) => reference.clone(),
        _ => panic!("delete fixture must expect a present entry"),
    };
    ApplicationCommand::DeleteMemoryEntry {
        profile: review.profile.clone(),
        expected,
        review_token: review.review_token,
        review_digest: review.review_digest.clone(),
    }
}

fn resolution_review(
    proposer: &AgentProfileVersion,
    owner: &AgentProfileVersion,
    seed: u128,
    action: MemoryResolutionAction,
) -> MemoryProposalResolutionReview {
    let proposal = proposal(proposer, seed, "resolution key");
    MemoryProposalResolutionReview {
        action,
        approval_id: proposal.approval_id(),
        proposal,
        expected_approval_status: ApprovalStatus::Pending,
        expected_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: proposer.reference() != owner.reference(),
        proposer_identity: memory_identity(proposer),
        namespace_owner_identity: memory_identity(owner),
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed + 10)),
        review_digest: sha256(format!("resolution review {seed}").as_bytes()),
    }
}

fn resolution_command(review: &MemoryProposalResolutionReview) -> ApplicationCommand {
    let fields = (
        review.proposal.reference(),
        review.approval_id,
        review.expected_approval_status,
        review.expected_entry.clone(),
        review.review_token,
        review.review_digest.clone(),
    );
    match review.action {
        MemoryResolutionAction::Approve => ApplicationCommand::ApproveMemoryProposal {
            proposal: fields.0,
            approval_id: fields.1,
            expected_approval_status: fields.2,
            expected_entry: fields.3,
            review_token: fields.4,
            review_digest: fields.5,
        },
        MemoryResolutionAction::Reject => ApplicationCommand::RejectMemoryProposal {
            proposal: fields.0,
            approval_id: fields.1,
            expected_approval_status: fields.2,
            expected_entry: fields.3,
            review_token: fields.4,
            review_digest: fields.5,
        },
    }
}

fn entries_view(
    profile: &AgentProfileVersion,
    namespace_id: MemoryNamespaceId,
) -> MemoryEntriesView {
    MemoryEntriesView {
        profile: profile.reference(),
        namespace_id,
        entries: Vec::new(),
        total_count: 0,
        returned_count: 0,
        omitted_count: 0,
    }
}

fn entry(profile: &AgentProfileVersion, seed: u128, key: &str) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(seed)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        MemoryEntryDraft::new(key.to_owned(), format!("value {seed}"), Vec::new())
            .expect("entry draft"),
        Actor::Human,
        i64::try_from(seed).expect("fixture timestamp"),
        None,
        EventId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .expect("entry")
}

fn entry_summary(entry: &MemoryEntryVersion) -> MemoryEntrySummary {
    MemoryEntrySummary {
        entry: entry.reference(),
        display_key: entry.display_key().to_owned(),
        purpose_tags: entry.purpose_tags().to_vec(),
        value_bytes: u64::try_from(entry.value().expect("present value").len())
            .expect("fixture length"),
        created_at_ms: entry.created_at_ms(),
    }
}

fn populated_entries_view(
    profile: &AgentProfileVersion,
    entries: &[MemoryEntryVersion],
) -> MemoryEntriesView {
    MemoryEntriesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        entries: entries.iter().map(entry_summary).collect(),
        total_count: u64::try_from(entries.len()).expect("fixture count"),
        returned_count: u64::try_from(entries.len()).expect("fixture count"),
        omitted_count: 0,
    }
}

fn history_view(
    profile: &AgentProfileVersion,
    current: &MemoryEntryVersion,
    versions: &[MemoryEntryVersion],
) -> MemoryEntryHistoryView {
    MemoryEntryHistoryView {
        profile: profile.reference(),
        current: current.reference(),
        versions: versions
            .iter()
            .map(|entry| MemoryEntryHistorySummary {
                entry: entry.reference(),
                display_key: entry.display_key().to_owned(),
                created_at_ms: entry.created_at_ms(),
                accepted_proposal: entry.accepted_proposal().cloned(),
            })
            .collect(),
        total_count: u64::try_from(versions.len()).expect("fixture count"),
        returned_count: u64::try_from(versions.len()).expect("fixture count"),
        omitted_count: 0,
    }
}

fn proposal(profile: &AgentProfileVersion, seed: u128, key: &str) -> MemoryProposal {
    MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(seed)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                key.to_owned(),
                format!("proposal value {seed}"),
                Vec::new(),
            )
            .expect("proposal draft"),
        },
        key.to_owned(),
        ExpectedMemoryEntryState::Absent,
        "fixture rationale".to_owned(),
        i64::try_from(seed).expect("fixture timestamp"),
        EventId::from_uuid(Uuid::from_u128(seed + 1)),
        ApprovalId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .expect("proposal")
}

fn proposal_summary(proposal: &MemoryProposal) -> MemoryProposalSummary {
    MemoryProposalSummary {
        proposal: proposal.reference(),
        proposer: proposal.proposer().clone(),
        operation: MemoryProposalOperationKind::Set,
        display_key: proposal.display_key().to_owned(),
        status: MemoryProposalStatus::Pending,
        created_at_ms: proposal.created_at_ms(),
    }
}

fn proposals_view(
    profile: &AgentProfileVersion,
    proposals: &[MemoryProposal],
) -> MemoryProposalsView {
    MemoryProposalsView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        filter: MemoryProposalFilter::Pending,
        proposals: proposals.iter().map(proposal_summary).collect(),
        total_count: u64::try_from(proposals.len()).expect("fixture count"),
        returned_count: u64::try_from(proposals.len()).expect("fixture count"),
        omitted_count: 0,
    }
}

fn proposal_detail(profile: &AgentProfileVersion, proposal: &MemoryProposal) -> MemoryProposalView {
    MemoryProposalView {
        proposal: proposal.clone(),
        status: MemoryProposalStatus::Pending,
        resolution: None,
        current_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: false,
        proposer_identity: memory_identity(profile),
        namespace_owner_identity: memory_identity(profile),
    }
}

fn summary(profile: &AgentProfileVersion, seed: u128, label: &str) -> EpisodicSummary {
    let source = EpisodicSourceRef::new(
        u64::try_from(seed).expect("fixture sequence"),
        EventId::from_uuid(Uuid::from_u128(seed + 1)),
        "help_viewed".to_owned(),
        sha256(format!("source {seed}").as_bytes()),
    )
    .expect("source");
    EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::from_u128(seed)),
        profile,
        label.to_owned(),
        format!("summary body {seed}"),
        Vec::new(),
        vec![source],
        i64::try_from(seed).expect("fixture timestamp"),
        u64::try_from(seed + 1).expect("fixture sequence"),
        EventId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .expect("summary")
}

fn summaries_view(
    profile: &AgentProfileVersion,
    summaries: &[EpisodicSummary],
) -> EpisodicSummariesView {
    EpisodicSummariesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        summaries: summaries
            .iter()
            .map(|summary| EpisodicSummaryListItem {
                summary: summary.reference(),
                label: summary.label().to_owned(),
                purpose_tags: summary.purpose_tags().to_vec(),
                source_count: u64::try_from(summary.sources().len()).expect("fixture count"),
                created_at_ms: summary.created_at_ms(),
            })
            .collect(),
        total_count: u64::try_from(summaries.len()).expect("fixture count"),
        returned_count: u64::try_from(summaries.len()).expect("fixture count"),
        omitted_count: 0,
    }
}

#[test]
fn memory_is_nested_under_agents_without_changing_global_shortcuts() {
    let profile = profile(10);
    let profile_id = profile.profile_id();
    let profile_identity = memory_identity(&profile);
    let namespace_id = profile.memory_namespace_id();
    let mut model = model();
    model.select_view(View::Agents);
    model.skills.library_loaded = true;
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail = Some(AgentProfileView {
        readiness: profile.readiness(),
        profile,
    });

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Right)),
        ControllerEffect::Redraw,
    );
    assert_eq!(
        model.agents.selected_detail_action,
        AgentDetailAction::Memory,
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadAgentMemory(profile_id.into()),
    );
    assert_eq!(model.agents.pane, AgentsPane::Memory);
    assert_eq!(model.agents.memory.profile, Some(profile_identity));
    assert_eq!(model.agents.memory.namespace_id, Some(namespace_id));
    assert_eq!(model.active_view, View::Agents);
    assert!(!model.skills.active);

    let before_q = model.clone();
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('q'))),
        ControllerEffect::None,
    );
    assert_eq!(model, before_q);
}

#[test]
fn assigned_skills_opens_when_empty_and_its_open_panel_owns_left_right() {
    let profile = profile(20);
    assert!(profile.skill_refs().is_empty());
    let mut model = model();
    model.select_view(View::Agents);
    model.skills.library_loaded = true;
    model.agents.pane = AgentsPane::Detail;
    model.agents.detail = Some(AgentProfileView {
        readiness: profile.readiness(),
        profile,
    });

    assert_eq!(
        model.agents.selected_detail_action,
        AgentDetailAction::AssignedSkills
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert!(model.agents.skill_panel_open);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Right)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.selected_detail_action,
        AgentDetailAction::AssignedSkills
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Left)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.selected_detail_action,
        AgentDetailAction::AssignedSkills
    );
}

#[test]
fn memory_state_defaults_and_safe_errors_define_the_nested_workspace_contract() {
    let state = MemoryViewState::default();

    assert_eq!(state.pane, MemoryPane::EntryList);
    assert_eq!(
        state.selected_entry_detail_action,
        MemoryEntryDetailAction::Edit,
    );
    assert_eq!(
        state.selected_proposal_detail_action,
        MemoryProposalDetailAction::Approve,
    );
    assert_eq!(state.editor_origin, MemoryEditorOrigin::Create);
    assert_eq!(state.result_origin, MemoryResultOrigin::Mutation);
    assert_eq!(state.generation, 0);
    assert_eq!(state.pending_intent, None::<MemoryOutcomeIntent>);
    assert!(!state.review_registered);
    assert_eq!(
        DomainError::MemoryGenerationOverflow.code(),
        "memory_generation_overflow",
    );
    assert_eq!(
        DomainError::MemorySelectionUnavailable.code(),
        "memory_selection_unavailable",
    );

    assert_eq!(
        ai_stock_forum::ui::tui::AgentsViewState::default().selected_detail_action,
        AgentDetailAction::AssignedSkills,
    );
}

#[test]
fn opening_a_create_editor_checks_generation_before_mutating_state() {
    let selector = AgentProfileSelector::Id(AgentProfileId::from_uuid(Uuid::from_u128(30)));
    let mut state = MemoryViewState::default();

    state
        .open_create_editor(selector.clone())
        .expect("first generation");
    assert_eq!(state.generation, 1);
    assert_eq!(state.pane, MemoryPane::Editor);
    assert_eq!(state.editor_origin, MemoryEditorOrigin::Create);
    assert_eq!(
        state.editor.as_ref().map(|editor| editor.selector()),
        Some(&selector),
    );

    state.generation = u64::MAX;
    let before = state.clone();
    assert_eq!(
        state.open_create_editor(selector),
        Err(DomainError::MemoryGenerationOverflow),
    );
    assert_eq!(state, before);

    let owner = profile(31);
    let mut edit_state = MemoryViewState {
        generation: u64::MAX,
        ..MemoryViewState::default()
    };
    let before = edit_state.clone();
    assert_eq!(
        edit_state.open_edit_editor(
            AgentProfileSelector::Id(owner.profile_id()),
            entry(&owner, 32, "overflow edit"),
        ),
        Err(DomainError::MemoryGenerationOverflow),
    );
    assert_eq!(edit_state, before);
    assert_eq!(
        edit_state.begin_review_request(),
        Err(DomainError::MemoryGenerationOverflow),
    );
    assert_eq!(edit_state, before);
}

#[test]
fn pending_intents_and_empty_selection_use_checked_content_free_transitions() {
    let mut state = MemoryViewState::default();
    assert_eq!(
        state.selected_entry_id(),
        Err(DomainError::MemorySelectionUnavailable),
    );

    assert_eq!(state.begin_pending(MemoryOutcomeIntent::Entries), Ok(1));
    assert_eq!(state.generation, 1);
    assert_eq!(state.pending_intent, Some(MemoryOutcomeIntent::Entries));
    state.clear_pending();
    assert_eq!(state.pending_intent, None);

    state.generation = u64::MAX;
    let before = state.clone();
    assert_eq!(
        state.begin_pending(MemoryOutcomeIntent::Proposals),
        Err(DomainError::MemoryGenerationOverflow),
    );
    assert_eq!(state, before);
}

#[test]
fn matching_views_require_pending_intent_and_exact_profile_namespace_identity() {
    let selected = profile(40);
    let other = profile(50);
    let mut state = MemoryViewState {
        profile: Some(memory_identity(&selected)),
        namespace_id: Some(selected.memory_namespace_id()),
        ..MemoryViewState::default()
    };
    let exact = CommandView::MemoryEntries(entries_view(&selected, selected.memory_namespace_id()));

    let before_without_pending = state.clone();
    assert!(!state.apply_matching_view(&MemoryOutcomeIntent::Entries, exact.clone()));
    assert_eq!(state, before_without_pending);

    state
        .begin_pending(MemoryOutcomeIntent::Entries)
        .expect("pending list");
    let before_wrong_type = state.clone();
    assert!(
        !state.apply_matching_view(&MemoryOutcomeIntent::Entries, CommandView::Help(HelpView),)
    );
    assert_eq!(state, before_wrong_type);

    let before_wrong_profile = state.clone();
    assert!(!state.apply_matching_view(
        &MemoryOutcomeIntent::Entries,
        CommandView::MemoryEntries(entries_view(&other, selected.memory_namespace_id(),)),
    ));
    assert_eq!(state, before_wrong_profile);

    let before_wrong_namespace = state.clone();
    assert!(!state.apply_matching_view(
        &MemoryOutcomeIntent::Entries,
        CommandView::MemoryEntries(entries_view(&selected, other.memory_namespace_id(),)),
    ));
    assert_eq!(state, before_wrong_namespace);

    assert!(state.apply_matching_view(&MemoryOutcomeIntent::Entries, exact.clone()));
    let CommandView::MemoryEntries(expected) = exact else {
        unreachable!()
    };
    assert_eq!(state.entries, Some(expected));
    assert_eq!(state.pending_intent, None);
}

#[test]
fn entry_refresh_reconciles_selection_by_stable_id_and_invalidates_removed_detail() {
    let owner = profile(60);
    let first = entry(&owner, 100, "First");
    let selected = entry(&owner, 110, "Selected");
    let prepended = entry(&owner, 120, "Prepended");
    let mut state = MemoryViewState {
        profile: Some(memory_identity(&owner)),
        namespace_id: Some(owner.memory_namespace_id()),
        entries: Some(populated_entries_view(
            &owner,
            &[first.clone(), selected.clone()],
        )),
        entry_detail: Some(MemoryEntryView {
            profile: owner.reference(),
            entry: selected.clone(),
        }),
        selected_entry: 1,
        entry_scroll: 1,
        ..MemoryViewState::default()
    };

    state
        .begin_pending(MemoryOutcomeIntent::Entries)
        .expect("prepend refresh");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::Entries,
        CommandView::MemoryEntries(populated_entries_view(
            &owner,
            &[prepended, first.clone(), selected.clone()],
        )),
    ));
    assert_eq!(state.selected_entry, 2);
    assert_eq!(state.entry_scroll, 1);
    assert_eq!(
        state.selected_entry_id(),
        Ok(selected.reference().entry_id())
    );
    assert!(state.entry_detail.is_some());

    state
        .begin_pending(MemoryOutcomeIntent::Entries)
        .expect("removal refresh");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::Entries,
        CommandView::MemoryEntries(populated_entries_view(&owner, &[first])),
    ));
    assert_eq!(state.selected_entry, 0);
    assert_eq!(state.entry_scroll, 0);
    assert_eq!(state.entry_detail, None);
}

#[test]
fn history_refresh_reconciles_versions_by_stable_id_and_clears_removed_version_detail() {
    let owner = profile(70);
    let first = entry(&owner, 200, "History");
    let second = first
        .next_present(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(210)),
            MemoryEntryDraft::new("History".to_owned(), "second".to_owned(), Vec::new())
                .expect("second draft"),
            Actor::Human,
            210,
            None,
            EventId::from_uuid(Uuid::from_u128(211)),
        )
        .expect("second version");
    let third = second
        .next_present(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(220)),
            MemoryEntryDraft::new("History".to_owned(), "third".to_owned(), Vec::new())
                .expect("third draft"),
            Actor::Human,
            220,
            None,
            EventId::from_uuid(Uuid::from_u128(221)),
        )
        .expect("third version");
    let entry_id = first.reference().entry_id();
    let mut state = MemoryViewState {
        profile: Some(memory_identity(&owner)),
        namespace_id: Some(owner.memory_namespace_id()),
        entry_history: Some(history_view(
            &owner,
            &second,
            &[second.clone(), first.clone()],
        )),
        entry_version: Some(MemoryEntryVersionView {
            profile: owner.reference(),
            entry: first.clone(),
        }),
        selected_history_version: 1,
        history_scroll: 1,
        ..MemoryViewState::default()
    };

    state
        .begin_pending(MemoryOutcomeIntent::EntryHistory(entry_id))
        .expect("prepend history");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::EntryHistory(entry_id),
        CommandView::MemoryEntryHistory(history_view(
            &owner,
            &third,
            &[third.clone(), second, first.clone()],
        )),
    ));
    assert_eq!(state.selected_history_version, 2);
    assert_eq!(state.history_scroll, 1);
    assert!(state.entry_version.is_some());

    state
        .begin_pending(MemoryOutcomeIntent::EntryHistory(entry_id))
        .expect("remove history");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::EntryHistory(entry_id),
        CommandView::MemoryEntryHistory(
            history_view(&owner, &third, std::slice::from_ref(&third),)
        ),
    ));
    assert_eq!(state.selected_history_version, 0);
    assert_eq!(state.history_scroll, 0);
    assert_eq!(state.entry_version, None);
}

#[test]
fn proposal_and_episode_refreshes_reconcile_each_selection_and_scroll_independently() {
    let owner = profile(80);
    let proposal_a = proposal(&owner, 300, "Proposal A");
    let proposal_b = proposal(&owner, 310, "Proposal B");
    let proposal_c = proposal(&owner, 320, "Proposal C");
    let episode_a = summary(&owner, 400, "Episode A");
    let episode_b = summary(&owner, 410, "Episode B");
    let episode_c = summary(&owner, 420, "Episode C");
    let mut state = MemoryViewState {
        profile: Some(memory_identity(&owner)),
        namespace_id: Some(owner.memory_namespace_id()),
        proposals: Some(proposals_view(
            &owner,
            &[proposal_a.clone(), proposal_b.clone()],
        )),
        proposal_detail: Some(proposal_detail(&owner, &proposal_b)),
        episodes: Some(summaries_view(
            &owner,
            &[episode_a.clone(), episode_b.clone()],
        )),
        episode_detail: Some(EpisodicSummaryView {
            summary: episode_b.clone(),
            qualification: EpisodicQualification::SummaryVerifySources,
        }),
        selected_proposal: 1,
        selected_episode: 1,
        proposal_scroll: 1,
        episode_scroll: 1,
        entry_scroll: 7,
        history_scroll: 8,
        detail_scroll: 9,
        ..MemoryViewState::default()
    };

    state
        .begin_pending(MemoryOutcomeIntent::Proposals)
        .expect("proposal refresh");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::Proposals,
        CommandView::MemoryProposals(proposals_view(
            &owner,
            &[proposal_c, proposal_a.clone(), proposal_b.clone()],
        )),
    ));
    assert_eq!((state.selected_proposal, state.proposal_scroll), (2, 1));
    assert_eq!((state.selected_episode, state.episode_scroll), (1, 1));
    assert_eq!(
        (
            state.entry_scroll,
            state.history_scroll,
            state.detail_scroll
        ),
        (7, 8, 9)
    );

    state
        .begin_pending(MemoryOutcomeIntent::Episodes)
        .expect("episode refresh");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::Episodes,
        CommandView::EpisodicSummaries(summaries_view(
            &owner,
            &[episode_c, episode_a.clone(), episode_b.clone()],
        )),
    ));
    assert_eq!((state.selected_episode, state.episode_scroll), (2, 1));

    state
        .begin_pending(MemoryOutcomeIntent::Proposals)
        .expect("proposal removal");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::Proposals,
        CommandView::MemoryProposals(proposals_view(&owner, &[proposal_a])),
    ));
    assert_eq!((state.selected_proposal, state.proposal_scroll), (0, 0));
    assert_eq!(state.proposal_detail, None);

    state
        .begin_pending(MemoryOutcomeIntent::Episodes)
        .expect("episode removal");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::Episodes,
        CommandView::EpisodicSummaries(summaries_view(&owner, &[episode_a])),
    ));
    assert_eq!((state.selected_episode, state.episode_scroll), (0, 0));
    assert_eq!(state.episode_detail, None);
}

#[test]
fn exact_detail_payloads_match_every_requested_stable_identity_without_navigating() {
    let owner = profile(90);
    let memory_entry = entry(&owner, 500, "Exact Key");
    let memory_proposal = proposal(&owner, 510, "Exact Proposal");
    let episodic = summary(&owner, 520, "Exact Episode");
    let entry_id = memory_entry.reference().entry_id();
    let entry_version_id = memory_entry.reference().entry_version_id();
    let proposal_id = memory_proposal.reference().proposal_id();
    let summary_id = episodic.reference().summary_id();
    let selector = AgentProfileSelector::Id(owner.profile_id());
    let mut state = MemoryViewState {
        profile: Some(memory_identity(&owner)),
        namespace_id: Some(owner.memory_namespace_id()),
        pane: MemoryPane::Confirmation,
        selected_entry: 3,
        selected_history_version: 4,
        selected_proposal: 5,
        selected_episode: 6,
        entry_scroll: 7,
        detail_scroll: 8,
        history_scroll: 9,
        proposal_scroll: 10,
        episode_scroll: 11,
        ..MemoryViewState::default()
    };

    state
        .begin_pending(MemoryOutcomeIntent::EntryDetail(entry_id))
        .expect("entry detail");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::EntryDetail(entry_id),
        CommandView::MemoryEntry(MemoryEntryView {
            profile: owner.reference(),
            entry: memory_entry.clone(),
        }),
    ));
    assert_eq!(
        state
            .entry_detail
            .as_ref()
            .map(|view| view.entry.reference()),
        Some(memory_entry.reference()),
    );

    state
        .begin_pending(MemoryOutcomeIntent::EntryVersion {
            selector: selector.clone(),
            key: "Exact Key".to_owned(),
            version: memory_entry.reference().version(),
            entry_version_id,
        })
        .expect("entry version");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::EntryVersion {
            selector: selector.clone(),
            key: "Exact Key".to_owned(),
            version: memory_entry.reference().version(),
            entry_version_id,
        },
        CommandView::MemoryEntryVersion(MemoryEntryVersionView {
            profile: owner.reference(),
            entry: memory_entry.clone(),
        }),
    ));

    state
        .begin_pending(MemoryOutcomeIntent::ProposalDetail(proposal_id))
        .expect("proposal detail");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::ProposalDetail(proposal_id),
        CommandView::MemoryProposal(proposal_detail(&owner, &memory_proposal)),
    ));

    state
        .begin_pending(MemoryOutcomeIntent::EpisodeDetail(summary_id))
        .expect("episode detail");
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::EpisodeDetail(summary_id),
        CommandView::EpisodicSummary(EpisodicSummaryView {
            summary: episodic,
            qualification: EpisodicQualification::SummaryVerifySources,
        }),
    ));

    assert_eq!(state.pane, MemoryPane::Confirmation);
    assert_eq!(
        (
            state.selected_entry,
            state.selected_history_version,
            state.selected_proposal,
            state.selected_episode,
        ),
        (3, 4, 5, 6),
    );
    assert_eq!(
        (
            state.entry_scroll,
            state.detail_scroll,
            state.history_scroll,
            state.proposal_scroll,
            state.episode_scroll,
        ),
        (7, 8, 9, 10, 11),
    );
}

#[test]
fn detail_payload_id_and_version_request_mismatches_are_wholly_nonmutating() {
    let owner = profile(100);
    let requested = entry(&owner, 600, "Requested");
    let substituted = entry(&owner, 610, "Substituted");
    let exact = MemoryOutcomeIntent::EntryVersion {
        selector: AgentProfileSelector::Id(owner.profile_id()),
        key: requested.display_key().to_owned(),
        version: requested.reference().version(),
        entry_version_id: requested.reference().entry_version_id(),
    };
    let base = MemoryViewState {
        profile: Some(memory_identity(&owner)),
        namespace_id: Some(owner.memory_namespace_id()),
        ..MemoryViewState::default()
    };
    for mismatched_intent in [
        MemoryOutcomeIntent::EntryVersion {
            selector: AgentProfileSelector::Id(profile(101).profile_id()),
            key: requested.display_key().to_owned(),
            version: requested.reference().version(),
            entry_version_id: requested.reference().entry_version_id(),
        },
        MemoryOutcomeIntent::EntryVersion {
            selector: AgentProfileSelector::Id(owner.profile_id()),
            key: "Different".to_owned(),
            version: requested.reference().version(),
            entry_version_id: requested.reference().entry_version_id(),
        },
        MemoryOutcomeIntent::EntryVersion {
            selector: AgentProfileSelector::Id(owner.profile_id()),
            key: requested.display_key().to_owned(),
            version: ObjectVersion::new(2).expect("fixture version"),
            entry_version_id: requested.reference().entry_version_id(),
        },
        MemoryOutcomeIntent::EntryVersion {
            selector: AgentProfileSelector::Id(owner.profile_id()),
            key: requested.display_key().to_owned(),
            version: requested.reference().version(),
            entry_version_id: substituted.reference().entry_version_id(),
        },
    ] {
        let mut state = base.clone();
        state
            .begin_pending(mismatched_intent.clone())
            .expect("mismatched version request");
        let before = state.clone();
        assert!(!state.apply_matching_view(
            &mismatched_intent,
            CommandView::MemoryEntryVersion(MemoryEntryVersionView {
                profile: owner.reference(),
                entry: requested.clone(),
            }),
        ));
        assert_eq!(state, before);
    }

    let mut state = base;
    state.begin_pending(exact.clone()).expect("version request");
    let before = state.clone();
    assert!(!state.apply_matching_view(
        &exact,
        CommandView::MemoryEntryVersion(MemoryEntryVersionView {
            profile: owner.reference(),
            entry: substituted,
        }),
    ));
    assert_eq!(state, before);
}

#[test]
fn proposal_detail_accepts_an_exact_owner_with_a_historical_proposer() {
    let proposer = profile(110);
    let owner = successor(&proposer, 120);
    let proposal = proposal(&proposer, 130, "historical proposal");
    let proposal_id = proposal.reference().proposal_id();
    let mut state = MemoryViewState {
        profile: Some(memory_identity(&owner)),
        namespace_id: Some(owner.memory_namespace_id()),
        pending_intent: Some(MemoryOutcomeIntent::ProposalDetail(proposal_id)),
        ..MemoryViewState::default()
    };
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::ProposalDetail(proposal_id),
        CommandView::MemoryProposal(MemoryProposalView {
            proposal,
            status: MemoryProposalStatus::Pending,
            resolution: None,
            current_entry: ExpectedMemoryEntryState::Absent,
            proposer_is_historical: true,
            proposer_identity: memory_identity(&proposer),
            namespace_owner_identity: memory_identity(&owner),
        }),
    ));
}

#[test]
fn profile_refresh_and_selection_transitions_preserve_or_invalidate_memory_by_exact_identity() {
    let selected = profile(600);
    let other = profile(610);
    let selected_v2 = successor(&selected, 620);
    let mut agents = model().agents;
    assert!(agents.replace_profiles(profiles_view(&[&other, &selected])));
    assert!(agents.select_profile_id(selected.profile_id()));
    agents.selected_detail_action = AgentDetailAction::Memory;
    agents
        .memory
        .bind_profile(memory_identity(&selected), selected.memory_namespace_id())
        .expect("bind memory");
    agents.memory.entries = Some(populated_entries_view(
        &selected,
        &[entry(&selected, 630, "preserved")],
    ));
    agents.memory.pane = MemoryPane::EntryDetail;
    agents.memory.generation = 9;
    let preserved_memory = agents.memory.clone();

    assert!(agents.replace_profiles(profiles_view(&[&selected, &other])));
    assert_eq!(agents.selected_profile, 0);
    assert_eq!(agents.selected_detail_action, AgentDetailAction::Memory);
    assert_eq!(agents.memory, preserved_memory);

    assert!(agents.replace_profiles(profiles_view(&[&selected_v2, &other])));
    assert_eq!(agents.selected_profile, 0);
    assert_eq!(agents.selected_detail_action, AgentDetailAction::Memory);
    assert_eq!(agents.memory.profile, None);
    assert_eq!(agents.memory.namespace_id, None);
    assert_eq!(agents.memory.entries, None);
    assert_eq!(agents.memory.pane, MemoryPane::EntryList);
    assert_eq!(agents.memory.generation, 9);

    agents
        .memory
        .bind_profile(
            memory_identity(&selected_v2),
            selected_v2.memory_namespace_id(),
        )
        .expect("bind successor");
    assert!(agents.select_profile_id(other.profile_id()));
    assert_eq!(
        agents.selected_detail_action,
        AgentDetailAction::AssignedSkills
    );
    assert_eq!(agents.memory.profile, None);
    assert_eq!(agents.memory.namespace_id, None);
    assert_eq!(agents.memory.generation, 9);

    let mut direct = model().agents;
    assert!(direct.replace_profiles(profiles_view(&[&selected])));
    direct.selected_detail_action = AgentDetailAction::Memory;
    direct
        .memory
        .bind_profile(memory_identity(&selected), selected.memory_namespace_id())
        .expect("bind direct detail");
    direct.memory.generation = 13;
    assert!(direct.replace_detail(AgentProfileView {
        profile: selected_v2.clone(),
        readiness: AgentReadiness::Unbound,
    }));
    assert_eq!(direct.selected_detail_action, AgentDetailAction::Memory);
    assert_eq!(direct.memory.profile, None);
    assert_eq!(direct.memory.generation, 13);

    direct
        .memory
        .bind_profile(
            memory_identity(&selected_v2),
            selected_v2.memory_namespace_id(),
        )
        .expect("rebind direct history");
    assert!(direct.replace_history(AgentProfileHistoryView {
        profile_id: selected.profile_id(),
        active_version_id: selected.profile_version_id(),
        versions: Vec::new(),
        total_count: 0,
        returned_count: 0,
        truncated: false,
    }));
    assert_eq!(direct.selected_detail_action, AgentDetailAction::Memory);
    assert_eq!(direct.memory.profile, None);
    assert_eq!(direct.memory.generation, 13);
}

#[test]
fn registered_memory_review_blocks_profile_selection_and_removal_refresh_atomically() {
    let selected = profile(700);
    let other = profile(710);
    let mut agents = model().agents;
    assert!(agents.replace_profiles(profiles_view(&[&selected, &other])));
    agents
        .memory
        .bind_profile(memory_identity(&selected), selected.memory_namespace_id())
        .expect("bind memory");
    agents.memory.pane = MemoryPane::MutationReview;
    let review = set_review(&selected, 720, "protected");
    agents.memory.edit_review = Some(review.clone());
    agents.memory.confirmation = Some(MemoryConfirmation {
        command: set_command(&review),
        generation: 4,
    });
    agents.memory.review_registered = true;
    agents.memory.generation = 4;
    let protected = agents.clone();

    assert!(!agents.select_profile_id(other.profile_id()));
    assert_eq!(agents, protected);

    assert!(!agents.replace_profiles(profiles_view(&[&other])));
    assert_eq!(agents, protected);

    assert!(!agents.replace_detail(AgentProfileView {
        profile: successor(&selected, 721),
        readiness: AgentReadiness::Unbound,
    }));
    assert_eq!(agents, protected);

    assert!(!agents.replace_detail(AgentProfileView {
        profile: other.clone(),
        readiness: AgentReadiness::Unbound,
    }));
    assert_eq!(agents, protected);

    assert!(!agents.replace_history(AgentProfileHistoryView {
        profile_id: other.profile_id(),
        active_version_id: other.profile_version_id(),
        versions: Vec::new(),
        total_count: 0,
        returned_count: 0,
        truncated: false,
    }));
    assert_eq!(agents, protected);

    assert!(!agents.replace_version_detail(AgentProfileVersionView {
        profile: other,
        readiness: AgentReadiness::Unbound,
        predecessor_diff: Vec::new(),
    }));
    assert_eq!(agents, protected);
}

#[test]
fn mutation_results_require_the_installed_review_exact_confirmation_and_result_identity() {
    let owner = profile(800);
    let review = set_review(&owner, 810, "mutation key");
    let result = entry(&owner, 820, "mutation key");
    let mut state = MemoryViewState {
        profile: Some(memory_identity(&owner)),
        namespace_id: Some(owner.memory_namespace_id()),
        edit_review: Some(review.clone()),
        confirmation: Some(MemoryConfirmation {
            command: set_command(&review),
            generation: 5,
        }),
        review_registered: true,
        generation: 5,
        pending_intent: Some(MemoryOutcomeIntent::Mutation),
        pane: MemoryPane::Confirmation,
        ..MemoryViewState::default()
    };
    let expected_after = {
        let mut expected = state.clone();
        expected.pending_intent = None;
        expected
    };
    assert!(state.apply_matching_view(
        &MemoryOutcomeIntent::Mutation,
        CommandView::MemoryEntryMutation(MemoryEntryMutationView {
            entry: result.reference(),
            expired_proposals: Vec::new(),
        }),
    ));
    assert_eq!(state, expected_after);

    let base = MemoryViewState {
        pending_intent: Some(MemoryOutcomeIntent::Mutation),
        ..expected_after
    };
    let wrong_key = entry(&owner, 830, "wrong key");
    let deleted = result
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(831)),
            Actor::Human,
            831,
            None,
            EventId::from_uuid(Uuid::from_u128(832)),
        )
        .expect("deleted result");
    for (mut mismatched, entry) in [
        (
            {
                let mut value = base.clone();
                value.review_registered = false;
                value
            },
            result.reference(),
        ),
        (
            {
                let mut value = base.clone();
                value
                    .confirmation
                    .as_mut()
                    .expect("confirmation")
                    .generation = 4;
                value
            },
            result.reference(),
        ),
        (
            {
                let mut value = base.clone();
                value.confirmation.as_mut().expect("confirmation").command =
                    ApplicationCommand::RequestShutdown;
                value
            },
            result.reference(),
        ),
        (base.clone(), wrong_key.reference()),
        (base.clone(), deleted.reference()),
    ] {
        let before = mismatched.clone();
        assert!(!mismatched.apply_matching_view(
            &MemoryOutcomeIntent::Mutation,
            CommandView::MemoryEntryMutation(MemoryEntryMutationView {
                entry,
                expired_proposals: Vec::new(),
            }),
        ));
        assert_eq!(mismatched, before);
    }

    let current = entry(&owner, 840, "delete key");
    let delete_review = delete_review(&owner, &current, 850);
    let deleted = current
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(851)),
            Actor::Human,
            851,
            None,
            EventId::from_uuid(Uuid::from_u128(852)),
        )
        .expect("delete result");
    let mut delete_state = MemoryViewState {
        profile: Some(memory_identity(&owner)),
        namespace_id: Some(owner.memory_namespace_id()),
        edit_review: Some(delete_review.clone()),
        confirmation: Some(MemoryConfirmation {
            command: delete_command(&delete_review),
            generation: 8,
        }),
        review_registered: true,
        generation: 8,
        pending_intent: Some(MemoryOutcomeIntent::Mutation),
        ..MemoryViewState::default()
    };
    assert!(delete_state.apply_matching_view(
        &MemoryOutcomeIntent::Mutation,
        CommandView::MemoryEntryMutation(MemoryEntryMutationView {
            entry: deleted.reference(),
            expired_proposals: Vec::new(),
        }),
    ));

    let other = entry(&owner, 860, "delete key");
    delete_state.pending_intent = Some(MemoryOutcomeIntent::Mutation);
    let before = delete_state.clone();
    assert!(!delete_state.apply_matching_view(
        &MemoryOutcomeIntent::Mutation,
        CommandView::MemoryEntryMutation(MemoryEntryMutationView {
            entry: other.reference(),
            expired_proposals: Vec::new(),
        }),
    ));
    assert_eq!(delete_state, before);

    let mut malformed = before;
    malformed
        .edit_review
        .as_mut()
        .expect("edit review")
        .expected = ExpectedMemoryEntryState::Deleted(current.reference());
    malformed.confirmation = Some(MemoryConfirmation {
        command: delete_command(&delete_review),
        generation: malformed.generation,
    });
    let before = malformed.clone();
    assert!(!malformed.apply_matching_view(
        &MemoryOutcomeIntent::Mutation,
        CommandView::MemoryEntryMutation(MemoryEntryMutationView {
            entry: deleted.reference(),
            expired_proposals: Vec::new(),
        }),
    ));
    assert_eq!(malformed, before);
}

#[test]
fn resolution_results_require_owner_proposal_action_status_and_exact_confirmation() {
    let proposer = profile(900);
    let owner = successor(&proposer, 910);
    for (offset, action, status) in [
        (
            0_u128,
            MemoryResolutionAction::Approve,
            MemoryProposalStatus::Accepted,
        ),
        (
            20,
            MemoryResolutionAction::Reject,
            MemoryProposalStatus::Rejected,
        ),
    ] {
        let review = resolution_review(&proposer, &owner, 920 + offset, action);
        let resolution = MemoryProposalResolution::new(
            review.proposal.reference(),
            status,
            review.approval_id,
            Actor::Human,
            930,
            EventId::from_uuid(Uuid::from_u128(930 + offset)),
        )
        .expect("resolution");
        let mut state = MemoryViewState {
            profile: Some(memory_identity(&owner)),
            namespace_id: Some(owner.memory_namespace_id()),
            resolution_review: Some(review.clone()),
            confirmation: Some(MemoryConfirmation {
                command: resolution_command(&review),
                generation: 12,
            }),
            review_registered: true,
            generation: 12,
            pending_intent: Some(MemoryOutcomeIntent::Resolution),
            pane: MemoryPane::Confirmation,
            selected_proposal_detail_action: match action {
                MemoryResolutionAction::Approve => MemoryProposalDetailAction::Approve,
                MemoryResolutionAction::Reject => MemoryProposalDetailAction::Reject,
            },
            ..MemoryViewState::default()
        };
        let expected_after = {
            let mut expected = state.clone();
            expected.pending_intent = None;
            expected
        };
        assert!(state.apply_matching_view(
            &MemoryOutcomeIntent::Resolution,
            CommandView::MemoryProposalResolution(MemoryProposalResolutionView {
                resolution: resolution.clone(),
                entry: None,
                expired_proposals: Vec::new(),
            }),
        ));
        assert_eq!(state, expected_after);

        let mut mismatched = MemoryViewState {
            pending_intent: Some(MemoryOutcomeIntent::Resolution),
            ..expected_after
        };
        let wrong_status = match status {
            MemoryProposalStatus::Accepted => MemoryProposalStatus::Rejected,
            MemoryProposalStatus::Rejected => MemoryProposalStatus::Accepted,
            _ => unreachable!(),
        };
        let wrong_resolution = MemoryProposalResolution::new(
            review.proposal.reference(),
            wrong_status,
            review.approval_id,
            Actor::Human,
            931,
            EventId::from_uuid(Uuid::from_u128(931 + offset)),
        )
        .expect("wrong resolution");
        let before = mismatched.clone();
        assert!(!mismatched.apply_matching_view(
            &MemoryOutcomeIntent::Resolution,
            CommandView::MemoryProposalResolution(MemoryProposalResolutionView {
                resolution: wrong_resolution,
                entry: None,
                expired_proposals: Vec::new(),
            }),
        ));
        assert_eq!(mismatched, before);

        for wrong_resolution in [
            MemoryProposalResolution::new(
                proposal(&proposer, 980 + offset, "other resolution").reference(),
                status,
                review.approval_id,
                Actor::Human,
                932,
                EventId::from_uuid(Uuid::from_u128(932 + offset)),
            )
            .expect("wrong proposal resolution"),
            MemoryProposalResolution::new(
                review.proposal.reference(),
                status,
                ApprovalId::from_uuid(Uuid::from_u128(990 + offset)),
                Actor::Human,
                933,
                EventId::from_uuid(Uuid::from_u128(933 + offset)),
            )
            .expect("wrong approval resolution"),
        ] {
            let mut mismatched = MemoryViewState {
                pending_intent: Some(MemoryOutcomeIntent::Resolution),
                ..state.clone()
            };
            let before = mismatched.clone();
            assert!(!mismatched.apply_matching_view(
                &MemoryOutcomeIntent::Resolution,
                CommandView::MemoryProposalResolution(MemoryProposalResolutionView {
                    resolution: wrong_resolution,
                    entry: None,
                    expired_proposals: Vec::new(),
                }),
            ));
            assert_eq!(mismatched, before);
        }

        mismatched = MemoryViewState {
            pending_intent: Some(MemoryOutcomeIntent::Resolution),
            ..state.clone()
        };
        mismatched.selected_proposal_detail_action =
            match mismatched.selected_proposal_detail_action {
                MemoryProposalDetailAction::Approve => MemoryProposalDetailAction::Reject,
                MemoryProposalDetailAction::Reject => MemoryProposalDetailAction::Approve,
            };
        let before = mismatched.clone();
        assert!(!mismatched.apply_matching_view(
            &MemoryOutcomeIntent::Resolution,
            CommandView::MemoryProposalResolution(MemoryProposalResolutionView {
                resolution: resolution.clone(),
                entry: None,
                expired_proposals: Vec::new(),
            }),
        ));
        assert_eq!(mismatched, before);

        mismatched = MemoryViewState {
            pending_intent: Some(MemoryOutcomeIntent::Resolution),
            ..state.clone()
        };
        mismatched
            .confirmation
            .as_mut()
            .expect("confirmation")
            .command = ApplicationCommand::RequestShutdown;
        let before = mismatched.clone();
        assert!(!mismatched.apply_matching_view(
            &MemoryOutcomeIntent::Resolution,
            CommandView::MemoryProposalResolution(MemoryProposalResolutionView {
                resolution: resolution.clone(),
                entry: None,
                expired_proposals: Vec::new(),
            }),
        ));
        assert_eq!(mismatched, before);

        mismatched = MemoryViewState {
            pending_intent: Some(MemoryOutcomeIntent::Resolution),
            ..state.clone()
        };
        mismatched
            .confirmation
            .as_mut()
            .expect("confirmation")
            .generation = 11;
        let before = mismatched.clone();
        assert!(!mismatched.apply_matching_view(
            &MemoryOutcomeIntent::Resolution,
            CommandView::MemoryProposalResolution(MemoryProposalResolutionView {
                resolution,
                entry: None,
                expired_proposals: Vec::new(),
            }),
        ));
        assert_eq!(mismatched, before);
    }
}

fn memory_model(profile: &AgentProfileVersion) -> TuiModel {
    let mut model = model();
    model.select_view(View::Agents);
    model.skills.library_loaded = true;
    model.agents.pane = AgentsPane::Memory;
    model
        .agents
        .memory
        .bind_profile(memory_identity(profile), profile.memory_namespace_id())
        .expect("bind memory model");
    model
}

#[test]
fn memory_controller_routes_exact_list_detail_history_proposal_and_episode_effects() {
    let owner = profile(1_000);
    let first = entry(&owner, 1_010, "First Key");
    let second = entry(&owner, 1_020, "Second Key");
    let selector = AgentProfileSelector::Id(owner.profile_id());
    let mut model = memory_model(&owner);
    model.agents.memory.entries = Some(populated_entries_view(
        &owner,
        &[first.clone(), second.clone()],
    ));

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.selected_entry, 1);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadMemoryEntry {
            selector: selector.clone(),
            key: second.display_key().to_owned(),
        },
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryDetail);

    model.agents.memory.pane = MemoryPane::EntryDetail;
    model.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: owner.reference(),
        entry: second.clone(),
    });
    model.agents.memory.selected_entry_detail_action = MemoryEntryDetailAction::Delete;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::RequestMemoryDeletePreview {
            selector: selector.clone(),
            key: second.display_key().to_owned(),
            generation: 1,
        },
    );
    assert_eq!(model.agents.memory.generation, 1);

    model.agents.memory.selected_entry_detail_action = MemoryEntryDetailAction::History;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadMemoryEntryHistory {
            selector: selector.clone(),
            key: second.display_key().to_owned(),
        },
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryHistory);

    let first_v2 = first
        .next_present(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_025)),
            MemoryEntryDraft::new("First Key".to_owned(), "updated".to_owned(), Vec::new())
                .expect("successor draft"),
            Actor::Human,
            1_025,
            None,
            EventId::from_uuid(Uuid::from_u128(1_026)),
        )
        .expect("entry successor");
    model.agents.memory.pane = MemoryPane::EntryHistory;
    model.agents.memory.entry_history = Some(history_view(
        &owner,
        &first_v2,
        std::slice::from_ref(&first),
    ));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadMemoryEntryVersion {
            selector: selector.clone(),
            key: first.display_key().to_owned(),
            version: first.reference().version(),
            expected_entry_version_id: first.reference().entry_version_id(),
        },
    );

    let proposal = proposal(&owner, 1_030, "Proposal Key");
    model.agents.memory.pane = MemoryPane::Proposals;
    model.agents.memory.proposals = Some(proposals_view(&owner, std::slice::from_ref(&proposal)));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadMemoryProposal(proposal.reference().proposal_id()),
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::ProposalDetail);
    model.agents.memory.pane = MemoryPane::ProposalDetail;
    model.agents.memory.proposal_detail = Some(proposal_detail(&owner, &proposal));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::RequestMemoryProposalResolutionPreview {
            proposal: proposal.reference(),
            action: MemoryResolutionAction::Approve,
            generation: 2,
        },
    );

    let episode = summary(&owner, 1_040, "Episode");
    model.agents.memory.pane = MemoryPane::EpisodicSummaries;
    model.agents.memory.episodes = Some(summaries_view(&owner, std::slice::from_ref(&episode)));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadEpisodicSummary(episode.reference().summary_id()),
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EpisodicDetail);

    model.agents.memory.pane = MemoryPane::EntryList;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('p'))),
        ControllerEffect::LoadMemoryProposals {
            selector: selector.clone(),
            filter: MemoryProposalFilter::Pending,
        },
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::Proposals);
    model.agents.memory.pane = MemoryPane::EntryList;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('e'))),
        ControllerEffect::LoadEpisodicSummaries(selector),
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EpisodicSummaries);
}

#[test]
fn memory_editor_owns_text_paste_slash_and_cancel_before_clear_for_registered_review() {
    let owner = profile(1_100);
    let review = set_review(&owner, 1_110, "editor key");
    let mut model = memory_model(&owner);
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");

    for character in "1234as/q".chars() {
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Char(character))),
            ControllerEffect::Redraw,
        );
    }
    assert_eq!(model.command.text(), "1234as/q");
    assert_eq!(model.active_view, View::Agents);
    assert_eq!(model.agents.pane, AgentsPane::Memory);
    assert_eq!(model.agents.memory.pane, MemoryPane::Editor);

    assert_eq!(
        handle_event(&mut model, TuiEvent::Paste(" pasted".to_owned())),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "1234as/q pasted");

    let editor = model.agents.memory.editor.as_mut().expect("editor");
    editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    );
    model.agents.memory.edit_review = Some(review);
    model.agents.memory.review_registered = true;
    model.command.clear();
    let protected = model.clone();
    assert_eq!(
        handle_event(&mut model, TuiEvent::Paste("ignored".to_owned())),
        ControllerEffect::None,
    );
    assert_eq!(model, protected);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::CancelMemoryReview,
    );
    assert_eq!(model, protected);

    assert_eq!(
        handle_event(&mut model, repeated_key(KeyCode::Enter)),
        ControllerEffect::None,
    );
    assert_eq!(model, protected);
}

#[test]
fn memory_editor_confirmation_authenticates_the_exact_registered_outer_review() {
    let owner = profile(1_115);
    let review = set_review(&owner, 1_116, "authenticated editor review");
    let mut valid = memory_model(&owner);
    valid
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");
    let editor = valid.agents.memory.editor.as_mut().expect("editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    ));
    valid.agents.memory.edit_review = Some(review.clone());
    valid.agents.memory.review_registered = true;

    let mut accepted = valid.clone();
    assert_eq!(
        handle_event(&mut accepted, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(accepted.agents.memory.pane, MemoryPane::Confirmation);
    assert_eq!(
        accepted
            .agents
            .memory
            .confirmation
            .as_ref()
            .map(|confirmation| &confirmation.command),
        Some(&set_command(&review)),
    );

    let another_review = set_review(&owner, 1_117, "authenticated editor review");
    let another_owner = profile(1_118);
    for mut invalid in [
        ({
            let mut value = valid.clone();
            value.agents.memory.review_registered = false;
            value
        }),
        ({
            let mut value = valid.clone();
            value.agents.memory.edit_review = Some(another_review.clone());
            value
        }),
        ({
            let mut value = valid.clone();
            value.agents.memory.profile = Some(memory_identity(&another_owner));
            value
        }),
        ({
            let mut value = valid.clone();
            value.agents.memory.namespace_id = Some(another_owner.memory_namespace_id());
            value
        }),
    ] {
        let memory_before = invalid.agents.memory.clone();
        let input_before = invalid.command.clone();
        assert_eq!(
            handle_event(&mut invalid, key(KeyCode::Enter)),
            ControllerEffect::Redraw,
        );
        assert_eq!(invalid.agents.memory, memory_before);
        assert_eq!(invalid.command, input_before);
    }
}

#[test]
fn review_slash_uses_command_capture_but_too_small_text_editor_keeps_every_character_and_paste() {
    let owner = profile(1_120);
    let review = set_review(&owner, 1_121, "review slash");
    let mut review_model = memory_model(&owner);
    review_model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");
    let editor = review_model.agents.memory.editor.as_mut().expect("editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    ));
    review_model.agents.memory.edit_review = Some(review);
    review_model.agents.memory.review_registered = true;
    assert_eq!(
        handle_event(&mut review_model, key(KeyCode::Char('/'))),
        ControllerEffect::Redraw,
    );
    assert_eq!(review_model.focus, Focus::Command);
    assert_eq!(review_model.command.text(), "/");

    let mut tiny = memory_model(&owner);
    tiny.agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");
    assert_eq!(
        handle_event(&mut tiny, TuiEvent::Resize(1, 1)),
        ControllerEffect::Redraw,
    );
    for character in "1234asq/".chars() {
        assert_eq!(
            handle_event(&mut tiny, key(KeyCode::Char(character))),
            ControllerEffect::Redraw,
        );
    }
    assert_eq!(
        handle_event(&mut tiny, TuiEvent::Paste(" pasted".to_owned())),
        ControllerEffect::Redraw,
    );
    assert_eq!(tiny.command.text(), "1234asq/ pasted");
    assert_eq!(tiny.focus, Focus::Workspace);
    assert_eq!(tiny.agents.memory.pane, MemoryPane::Editor);
}

#[test]
fn memory_editor_synchronizes_seeded_fields_and_back_navigation() {
    let owner = profile(1_125);
    let seeded = MemoryEntryVersion::create_present(
        owner.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(1_126)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_127)),
        MemoryEntryDraft::new(
            "Seeded Key".to_owned(),
            "Seeded Value".to_owned(),
            vec!["alpha".to_owned(), "beta".to_owned()],
        )
        .expect("seeded draft"),
        Actor::Human,
        1_800_000_000_100,
        None,
        EventId::from_uuid(Uuid::from_u128(1_128)),
    )
    .expect("seeded entry");
    let mut model = memory_model(&owner);
    model.agents.memory.pane = MemoryPane::EntryDetail;
    model.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: owner.reference(),
        entry: seeded,
    });
    model.agents.memory.selected_entry_detail_action = MemoryEntryDetailAction::Edit;

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "Seeded Value");
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "alpha, beta");
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "Seeded Value");
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "alpha, beta");

    let editor = model.agents.memory.editor.as_mut().expect("editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent),
    ));
    model.command.clear();
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "alpha, beta");
}

#[test]
fn memory_value_input_preserves_multiline_seed_and_paste_exactly() {
    let owner = profile(1_129);
    let seeded = MemoryEntryVersion::create_present(
        owner.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(1_131)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_132)),
        MemoryEntryDraft::new(
            "Multiline Key".to_owned(),
            "first line\nsecond line".to_owned(),
            vec!["multiline".to_owned()],
        )
        .expect("multiline draft"),
        Actor::Human,
        1_800_000_000_110,
        None,
        EventId::from_uuid(Uuid::from_u128(1_133)),
    )
    .expect("multiline entry");
    let mut model = memory_model(&owner);
    model.agents.memory.pane = MemoryPane::EntryDetail;
    model.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: owner.reference(),
        entry: seeded,
    });
    model.agents.memory.selected_entry_detail_action = MemoryEntryDetailAction::Edit;

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "first line\nsecond line");
    assert_eq!(
        handle_event(&mut model, TuiEvent::Paste("\nthird line".to_owned())),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "first line\nsecond line\nthird line",);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "multiline");
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "first line\nsecond line\nthird line",);
}

#[test]
fn memory_value_paste_normalizes_lone_cr_and_crlf_to_one_lf_each() {
    let owner = profile(1_134);
    let mut model = memory_model(&owner);
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");
    model.command.ingest("Line Ending Key");
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(
        handle_event(
            &mut model,
            TuiEvent::Paste("first\rsecond\r\nthird".to_owned()),
        ),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "first\nsecond\nthird");

    let mut large = memory_model(&owner);
    large
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");
    large.command.ingest("Large Paste Key");
    assert_eq!(
        handle_event(&mut large, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(
        handle_event(&mut large, TuiEvent::Paste("\r".repeat(100_000))),
        ControllerEffect::Redraw,
    );
    assert_eq!(large.command.text().len(), MAX_INPUT_BYTES);
    assert!(
        large
            .command
            .text()
            .chars()
            .all(|character| character == '\n')
    );
}

#[test]
fn final_tags_preview_is_atomic_when_outer_generation_is_exhausted() {
    let owner = profile(1_130);
    let mut model = memory_model(&owner);
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");
    for input in ["Overflow Key", "Overflow Value"] {
        model.command.ingest(input);
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Enter)),
            ControllerEffect::Redraw,
        );
    }
    model.agents.memory.generation = u64::MAX;
    model.command.ingest("overflow-tag");
    let memory_before = model.agents.memory.clone();
    let input_before = model.command.clone();
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.agents.memory, memory_before);
    assert_eq!(model.command, input_before);
}

#[test]
fn memory_editor_routes_a_set_preview_while_its_draft_is_the_active_protected_owner() {
    let owner = profile(1_200);
    let mut model = memory_model(&owner);
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");

    for input in ["Preview Key", "Preview Value", "tag"] {
        model.command.ingest(input);
        let effect = handle_event(&mut model, key(KeyCode::Enter));
        if input == "tag" {
            let ControllerEffect::RequestMemorySetPreview(request) = effect else {
                panic!("expected set preview, got {effect:?}");
            };
            assert_eq!(
                request.selector,
                AgentProfileSelector::Id(owner.profile_id())
            );
            assert_eq!(request.candidate.display_key(), "Preview Key");
            assert_eq!(request.candidate.value(), "Preview Value");
            assert_eq!(request.candidate.purpose_tags(), &["tag"]);
            assert_eq!(request.generation, 3);
        } else {
            assert_eq!(effect, ControllerEffect::Redraw);
        }
    }
    assert_eq!(model.agents.memory.generation, 2);
    assert_eq!(model.agents.memory.pane, MemoryPane::Editor);
}

#[test]
fn all_six_global_shortcuts_preserve_each_non_text_memory_review_and_confirmation_state() {
    let owner = profile(1_300);
    let edit_review = set_review(&owner, 1_310, "protected navigation");
    let mut editor_state = MemoryViewState::default();
    editor_state
        .bind_profile(memory_identity(&owner), owner.memory_namespace_id())
        .expect("bind editor state");
    editor_state
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");
    let editor = editor_state.editor.as_mut().expect("editor");
    editor
        .submit_line("protected navigation".to_owned())
        .expect("key");
    editor.submit_line("value".to_owned()).expect("value");
    editor.submit_line(String::new()).expect("tags");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(edit_review.clone()),
    ));
    editor_state.edit_review = Some(edit_review.clone());
    editor_state.review_registered = true;

    let mutation_state = MemoryViewState {
        editor: None,
        pane: MemoryPane::MutationReview,
        ..editor_state.clone()
    };
    let confirmation_state = MemoryViewState {
        confirmation: Some(MemoryConfirmation {
            command: set_command(&edit_review),
            generation: editor_state.generation,
        }),
        pane: MemoryPane::Confirmation,
        ..mutation_state.clone()
    };
    let resolution = resolution_review(&owner, &owner, 1_320, MemoryResolutionAction::Approve);
    let resolution_state = MemoryViewState {
        profile: Some(memory_identity(&owner)),
        namespace_id: Some(owner.memory_namespace_id()),
        pane: MemoryPane::ProposalResolutionReview,
        edit_review: None,
        resolution_review: Some(resolution),
        confirmation: None,
        review_registered: true,
        generation: 7,
        ..MemoryViewState::default()
    };

    for state in [
        editor_state,
        mutation_state,
        resolution_state,
        confirmation_state,
    ] {
        for shortcut in ['1', '2', '3', '4', 'a', 's'] {
            let mut model = memory_model(&owner);
            model.agents.memory = state.clone();
            let before = model.agents.memory.clone();
            assert!(matches!(
                handle_event(&mut model, key(KeyCode::Char(shortcut))),
                ControllerEffect::Redraw
                    | ControllerEffect::LoadAgentProfiles
                    | ControllerEffect::LoadSkills
            ));
            assert_eq!(model.agents.memory, before, "shortcut={shortcut}");
            assert!(matches!(
                handle_event(&mut model, key(KeyCode::Char('a'))),
                ControllerEffect::Redraw | ControllerEffect::LoadAgentProfiles
            ));
            assert_eq!(model.active_view, View::Agents);
            assert_eq!(model.agents.pane, AgentsPane::Memory);
            assert_eq!(model.agents.memory, before, "restored shortcut={shortcut}");
        }
    }
}

#[test]
fn empty_memory_selections_fail_safely_and_detail_panes_own_their_scroll() {
    let owner = profile(1_400);
    for pane in [
        MemoryPane::EntryList,
        MemoryPane::EntryDetail,
        MemoryPane::EntryHistory,
        MemoryPane::Proposals,
        MemoryPane::ProposalDetail,
        MemoryPane::EpisodicSummaries,
    ] {
        let mut model = memory_model(&owner);
        model.agents.memory.pane = pane;
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Enter)),
            ControllerEffect::Redraw,
            "pane={pane:?}",
        );
        assert_eq!(
            model.message.as_ref().map(|message| message.text.as_str()),
            Some("memory_selection_unavailable"),
            "pane={pane:?}",
        );
    }

    for pane in [
        MemoryPane::EntryDetail,
        MemoryPane::ProposalDetail,
        MemoryPane::EpisodicDetail,
        MemoryPane::MutationReview,
        MemoryPane::ProposalResolutionReview,
        MemoryPane::Confirmation,
        MemoryPane::Result,
    ] {
        let mut model = memory_model(&owner);
        model.agents.memory.pane = pane;
        model.agents.memory.entry_scroll = 2;
        model.agents.memory.history_scroll = 3;
        model.agents.memory.proposal_scroll = 4;
        model.agents.memory.episode_scroll = 5;
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Down)),
            ControllerEffect::Redraw
        );
        assert_eq!(
            handle_event(&mut model, key(KeyCode::PageDown)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.agents.memory.detail_scroll, 11, "pane={pane:?}");
        assert_eq!(
            (
                model.agents.memory.entry_scroll,
                model.agents.memory.history_scroll,
                model.agents.memory.proposal_scroll,
                model.agents.memory.episode_scroll,
            ),
            (2, 3, 4, 5),
            "pane={pane:?}",
        );
        assert_eq!(
            handle_event(&mut model, key(KeyCode::PageUp)),
            ControllerEffect::Redraw
        );
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Up)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.agents.memory.detail_scroll, 0, "pane={pane:?}");
    }
}

#[test]
fn memory_reviews_and_confirmations_require_exact_registered_bindings_and_press_only() {
    let owner = profile(1_500);
    let review = set_review(&owner, 1_510, "review key");
    let command = set_command(&review);
    let mut model = memory_model(&owner);
    model.agents.memory.pane = MemoryPane::MutationReview;
    model.agents.memory.edit_review = Some(review.clone());
    model.agents.memory.review_registered = true;
    model.agents.memory.generation = 6;

    let before_repeat = model.clone();
    assert_eq!(
        handle_event(&mut model, repeated_key(KeyCode::Enter)),
        ControllerEffect::None,
    );
    assert_eq!(model, before_repeat);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::Confirmation);
    assert_eq!(
        model.agents.memory.confirmation,
        Some(MemoryConfirmation {
            command: command.clone(),
            generation: 6,
        }),
    );

    let confirmation = model.agents.memory.clone();
    assert_eq!(
        handle_event(&mut model, repeated_key(KeyCode::Enter)),
        ControllerEffect::None,
    );
    assert_eq!(model.agents.memory, confirmation);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::ExecuteMemory(command.clone()),
    );
    assert_eq!(model.agents.memory, confirmation);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::MutationReview);
    assert!(model.agents.memory.review_registered);
    assert_eq!(model.agents.memory.edit_review, Some(review.clone()));
    assert!(model.agents.memory.confirmation.is_some());

    for mut invalid in [
        ({
            let mut value = model.clone();
            value.agents.memory.review_registered = false;
            value
        }),
        ({
            let mut value = model.clone();
            value.agents.memory.profile = Some(memory_identity(&profile(1_520)));
            value
        }),
    ] {
        invalid.agents.memory.pane = MemoryPane::MutationReview;
        invalid.agents.memory.confirmation = None;
        let before = invalid.clone();
        assert_eq!(
            handle_event(&mut invalid, key(KeyCode::Enter)),
            ControllerEffect::Redraw
        );
        assert_eq!(invalid, before);
    }

    model.agents.memory.pane = MemoryPane::Confirmation;
    model
        .agents
        .memory
        .confirmation
        .as_mut()
        .expect("confirmation")
        .command = ApplicationCommand::RequestShutdown;
    let before = model.clone();
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model, before);

    let current = entry(&owner, 1_525, "delete review");
    let delete = delete_review(&owner, &current, 1_526);
    let expected_delete = delete_command(&delete);
    let mut delete_model = memory_model(&owner);
    delete_model.agents.memory.pane = MemoryPane::MutationReview;
    delete_model.agents.memory.edit_review = Some(delete);
    delete_model.agents.memory.review_registered = true;
    delete_model.agents.memory.generation = 8;
    assert_eq!(
        handle_event(&mut delete_model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(
        delete_model
            .agents
            .memory
            .confirmation
            .as_ref()
            .map(|confirmation| &confirmation.command),
        Some(&expected_delete),
    );
    assert_eq!(
        handle_event(&mut delete_model, key(KeyCode::Enter)),
        ControllerEffect::ExecuteMemory(expected_delete),
    );

    for (seed, action, selected) in [
        (
            1_530,
            MemoryResolutionAction::Approve,
            MemoryProposalDetailAction::Approve,
        ),
        (
            1_540,
            MemoryResolutionAction::Reject,
            MemoryProposalDetailAction::Reject,
        ),
    ] {
        let resolution = resolution_review(&owner, &owner, seed, action);
        let expected = resolution_command(&resolution);
        let mut model = memory_model(&owner);
        model.agents.memory.pane = MemoryPane::ProposalResolutionReview;
        model.agents.memory.resolution_review = Some(resolution);
        model.agents.memory.review_registered = true;
        model.agents.memory.selected_proposal_detail_action = selected;
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Enter)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.agents.memory.pane, MemoryPane::Confirmation);
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Enter)),
            ControllerEffect::ExecuteMemory(expected),
        );
    }

    let resolution = resolution_review(&owner, &owner, 1_550, MemoryResolutionAction::Approve);
    let mut opposite = memory_model(&owner);
    opposite.agents.memory.pane = MemoryPane::ProposalResolutionReview;
    opposite.agents.memory.resolution_review = Some(resolution);
    opposite.agents.memory.review_registered = true;
    opposite.agents.memory.selected_proposal_detail_action = MemoryProposalDetailAction::Reject;
    let before = opposite.clone();
    assert_eq!(
        handle_event(&mut opposite, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(opposite, before);
}

#[test]
fn confirmation_escape_returns_to_the_exact_review_origin() {
    let owner = profile(1_560);
    let set_review = set_review(&owner, 1_561, "set editor origin");
    let mut set_model = memory_model(&owner);
    set_model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open set editor");
    let editor = set_model.agents.memory.editor.as_mut().expect("editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(set_review.clone()),
    ));
    set_model.agents.memory.edit_review = Some(set_review);
    set_model.agents.memory.review_registered = true;
    assert_eq!(
        handle_event(&mut set_model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    let confirmation = set_model.agents.memory.confirmation.clone();
    assert_eq!(
        handle_event(&mut set_model, key(KeyCode::Esc)),
        ControllerEffect::Redraw,
    );
    assert_eq!(set_model.agents.memory.pane, MemoryPane::Editor);
    assert_eq!(
        set_model
            .agents
            .memory
            .editor
            .as_ref()
            .map(|editor| editor.step()),
        Some(ai_stock_forum::ui::memory_editor::MemoryEditorStep::Review),
    );
    assert_eq!(set_model.agents.memory.confirmation, confirmation);

    let current = entry(&owner, 1_562, "delete origin");
    let delete_review = delete_review(&owner, &current, 1_563);
    let mut delete_model = memory_model(&owner);
    delete_model.agents.memory.pane = MemoryPane::MutationReview;
    delete_model.agents.memory.edit_review = Some(delete_review);
    delete_model.agents.memory.review_registered = true;
    assert_eq!(
        handle_event(&mut delete_model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(
        handle_event(&mut delete_model, key(KeyCode::Esc)),
        ControllerEffect::Redraw,
    );
    assert_eq!(delete_model.agents.memory.pane, MemoryPane::MutationReview);

    let resolution = resolution_review(&owner, &owner, 1_564, MemoryResolutionAction::Approve);
    let mut resolution_model = memory_model(&owner);
    resolution_model.agents.memory.pane = MemoryPane::ProposalResolutionReview;
    resolution_model.agents.memory.resolution_review = Some(resolution);
    resolution_model.agents.memory.review_registered = true;
    assert_eq!(
        handle_event(&mut resolution_model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(
        handle_event(&mut resolution_model, key(KeyCode::Esc)),
        ControllerEffect::Redraw,
    );
    assert_eq!(
        resolution_model.agents.memory.pane,
        MemoryPane::ProposalResolutionReview,
    );
}

#[test]
fn memory_selection_navigation_never_retains_a_detail_for_another_stable_id() {
    let owner = profile(1_570);
    let first = entry(&owner, 1_571, "first cache");
    let second = entry(&owner, 1_575, "second cache");
    let mut model = memory_model(&owner);
    model.agents.memory.entries = Some(populated_entries_view(
        &owner,
        &[first.clone(), second.clone()],
    ));
    model.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: owner.reference(),
        entry: first.clone(),
    });
    model.agents.memory.entry_history =
        Some(history_view(&owner, &first, std::slice::from_ref(&first)));
    model.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: owner.reference(),
        entry: first.clone(),
    });
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw,
    );
    assert!(model.agents.memory.entry_detail.is_none());
    assert!(model.agents.memory.entry_history.is_none());
    assert!(model.agents.memory.entry_version.is_none());

    let first_v2 = first
        .next_present(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(1_579)),
            MemoryEntryDraft::new(
                "first cache".to_owned(),
                "second version".to_owned(),
                Vec::new(),
            )
            .expect("successor draft"),
            Actor::Human,
            1_800_000_000_200,
            None,
            EventId::from_uuid(Uuid::from_u128(1_580)),
        )
        .expect("successor");
    model.agents.memory.pane = MemoryPane::EntryHistory;
    model.agents.memory.selected_history_version = 0;
    model.agents.memory.entry_history = Some(history_view(
        &owner,
        &first_v2,
        &[first.clone(), first_v2.clone()],
    ));
    model.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: owner.reference(),
        entry: first,
    });
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw,
    );
    assert!(model.agents.memory.entry_version.is_none());

    let first_proposal = proposal(&owner, 1_581, "first proposal cache");
    let second_proposal = proposal(&owner, 1_585, "second proposal cache");
    model.agents.memory.pane = MemoryPane::Proposals;
    model.agents.memory.selected_proposal = 0;
    model.agents.memory.proposals = Some(proposals_view(
        &owner,
        &[first_proposal.clone(), second_proposal],
    ));
    model.agents.memory.proposal_detail = Some(proposal_detail(&owner, &first_proposal));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw,
    );
    assert!(model.agents.memory.proposal_detail.is_none());

    let first_episode = summary(&owner, 1_590, "first episode cache");
    let second_episode = summary(&owner, 1_595, "second episode cache");
    model.agents.memory.pane = MemoryPane::EpisodicSummaries;
    model.agents.memory.selected_episode = 0;
    model.agents.memory.episodes = Some(summaries_view(
        &owner,
        &[first_episode.clone(), second_episode],
    ));
    model.agents.memory.episode_detail = Some(EpisodicSummaryView {
        summary: first_episode,
        qualification: EpisodicQualification::SummaryVerifySources,
    });
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw,
    );
    assert!(model.agents.memory.episode_detail.is_none());
}

#[test]
fn unhandled_memory_keys_fall_through_to_existing_generic_controls() {
    let owner = profile(1_599);
    let mut help = memory_model(&owner);
    assert_eq!(
        handle_event(&mut help, key(KeyCode::Char('?'))),
        ControllerEffect::Redraw,
    );
    assert_eq!(help.active_view, View::Help);

    let mut focus = memory_model(&owner);
    focus.set_focus(Focus::Workspace);
    assert_eq!(
        handle_event(&mut focus, key(KeyCode::Tab)),
        ControllerEffect::Redraw,
    );
    assert_eq!(focus.focus, Focus::Inspector);
}

#[test]
fn memory_local_actions_and_escape_unwind_exactly_one_layer() {
    let owner = profile(1_600);
    let entry = entry(&owner, 1_610, "layered");
    let mut model = memory_model(&owner);
    model.agents.memory.entries =
        Some(populated_entries_view(&owner, std::slice::from_ref(&entry)));

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('c'))),
        ControllerEffect::Redraw,
    );
    assert_eq!(
        model.agents.memory.editor_origin,
        MemoryEditorOrigin::Create
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::Editor);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryList);

    model.agents.memory.pane = MemoryPane::EntryDetail;
    model.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: owner.reference(),
        entry: entry.clone(),
    });
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.editor_origin, MemoryEditorOrigin::Edit);
    assert_eq!(model.agents.memory.pane, MemoryPane::Editor);
    assert_eq!(model.command.text(), entry.value().expect("present value"));
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryDetail);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Right)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.memory.selected_entry_detail_action,
        MemoryEntryDetailAction::Delete
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Right)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.memory.selected_entry_detail_action,
        MemoryEntryDetailAction::History
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Left)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.memory.selected_entry_detail_action,
        MemoryEntryDetailAction::Delete
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryList);

    model.agents.memory.pane = MemoryPane::EntryHistory;
    model.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: owner.reference(),
        entry: entry.clone(),
    });
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryHistory);
    assert_eq!(model.agents.memory.entry_version, None);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryDetail);

    for (pane, parent) in [
        (MemoryPane::ProposalDetail, MemoryPane::Proposals),
        (MemoryPane::Proposals, MemoryPane::EntryList),
        (MemoryPane::EpisodicDetail, MemoryPane::EpisodicSummaries),
        (MemoryPane::EpisodicSummaries, MemoryPane::EntryList),
    ] {
        model.agents.memory.pane = pane;
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Esc)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.agents.memory.pane, parent, "pane={pane:?}");
    }

    model.agents.memory.pane = MemoryPane::ProposalDetail;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Right)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.memory.selected_proposal_detail_action,
        MemoryProposalDetailAction::Reject,
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Left)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        model.agents.memory.selected_proposal_detail_action,
        MemoryProposalDetailAction::Approve,
    );

    for (origin, parent) in [
        (MemoryResultOrigin::Mutation, MemoryPane::EntryList),
        (MemoryResultOrigin::Resolution, MemoryPane::Proposals),
    ] {
        model.agents.memory.pane = MemoryPane::Result;
        model.agents.memory.result_origin = origin;
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Enter)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.agents.memory.pane, parent);
    }

    model.agents.memory.pane = MemoryPane::EntryList;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.pane, AgentsPane::Detail);
}

#[test]
fn modified_memory_local_keys_are_inert() {
    let owner = profile(1_700);
    for (pane, code) in [
        (MemoryPane::EntryList, KeyCode::Down),
        (MemoryPane::EntryList, KeyCode::Char('c')),
        (MemoryPane::EntryDetail, KeyCode::Right),
        (MemoryPane::Proposals, KeyCode::Enter),
        (MemoryPane::ProposalDetail, KeyCode::Left),
        (MemoryPane::EpisodicSummaries, KeyCode::Enter),
        (MemoryPane::Result, KeyCode::Esc),
    ] {
        let mut model = memory_model(&owner);
        model.agents.memory.pane = pane;
        let before = model.clone();
        assert_eq!(
            handle_event(
                &mut model,
                TuiEvent::Key(KeyEvent::new(code, KeyModifiers::ALT)),
            ),
            ControllerEffect::None,
            "pane={pane:?} code={code:?}",
        );
        assert_eq!(model, before);
    }
}
