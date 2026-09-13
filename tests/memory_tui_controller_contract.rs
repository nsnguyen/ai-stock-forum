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
        MemoryEditReview, MemoryEntryDraft, MemoryEntryVersion, MemoryField, MemoryFieldDiff,
        MemoryFieldValue, MemoryMutationKind, MemoryNoChange, MemoryPlaintextAcknowledgement,
        MemoryProposal, MemoryProposalFilter, MemoryProposalOperation, MemoryProposalOperationKind,
        MemoryProposalResolution, MemoryProposalStatus, MemoryResolutionAction,
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
    let candidate =
        MemoryEntryDraft::new(key.to_owned(), format!("review value {seed}"), Vec::new())
            .expect("review draft");
    MemoryEditReview {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        expected: ExpectedMemoryEntryState::Absent,
        operation: MemoryMutationKind::Set,
        candidate: Some(candidate.clone()),
        diff: absent_set_diff(&candidate),
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
        diff: delete_diff(current),
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed)),
        review_digest: sha256(format!("delete review {seed}").as_bytes()),
    }
}

fn absent_set_diff(candidate: &MemoryEntryDraft) -> Vec<MemoryFieldDiff> {
    vec![
        MemoryFieldDiff {
            field: MemoryField::DisplayKey,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Text(candidate.display_key().to_owned()),
        },
        MemoryFieldDiff {
            field: MemoryField::State,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Present),
        },
        MemoryFieldDiff {
            field: MemoryField::Value,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Text(candidate.value().to_owned()),
        },
        MemoryFieldDiff {
            field: MemoryField::PurposeTags,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
        },
    ]
}

fn delete_diff(current: &MemoryEntryVersion) -> Vec<MemoryFieldDiff> {
    vec![
        MemoryFieldDiff {
            field: MemoryField::State,
            before: MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Present),
            after: MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Deleted),
        },
        MemoryFieldDiff {
            field: MemoryField::Value,
            before: MemoryFieldValue::Text(current.value().expect("present entry").to_owned()),
            after: MemoryFieldValue::Missing,
        },
        MemoryFieldDiff {
            field: MemoryField::PurposeTags,
            before: MemoryFieldValue::Tags(current.purpose_tags().to_vec()),
            after: MemoryFieldValue::Missing,
        },
    ]
}

fn present_set_diff(
    current: &MemoryEntryVersion,
    candidate: &MemoryEntryDraft,
) -> Vec<MemoryFieldDiff> {
    let before = [
        MemoryFieldValue::Text(current.display_key().to_owned()),
        MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Present),
        MemoryFieldValue::Text(current.value().expect("present entry").to_owned()),
        MemoryFieldValue::Tags(current.purpose_tags().to_vec()),
    ];
    let after = [
        MemoryFieldValue::Text(candidate.display_key().to_owned()),
        MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Present),
        MemoryFieldValue::Text(candidate.value().to_owned()),
        MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
    ];
    [
        MemoryField::DisplayKey,
        MemoryField::State,
        MemoryField::Value,
        MemoryField::PurposeTags,
    ]
    .into_iter()
    .zip(before)
    .zip(after)
    .filter_map(|((field, before), after)| {
        (before != after).then_some(MemoryFieldDiff {
            field,
            before,
            after,
        })
    })
    .collect()
}

fn deleted_set_diff(candidate: &MemoryEntryDraft) -> Vec<MemoryFieldDiff> {
    vec![
        MemoryFieldDiff {
            field: MemoryField::State,
            before: MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Deleted),
            after: MemoryFieldValue::State(ai_stock_forum::memory::MemoryEntryState::Present),
        },
        MemoryFieldDiff {
            field: MemoryField::Value,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Text(candidate.value().to_owned()),
        },
        MemoryFieldDiff {
            field: MemoryField::PurposeTags,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
        },
    ]
}

fn deleted_set_diff_with_prior_key(
    candidate: &MemoryEntryDraft,
    prior_key: &str,
) -> Vec<MemoryFieldDiff> {
    let mut diff = deleted_set_diff(candidate);
    diff.insert(
        0,
        MemoryFieldDiff {
            field: MemoryField::DisplayKey,
            before: MemoryFieldValue::Text(prior_key.to_owned()),
            after: MemoryFieldValue::Text(candidate.display_key().to_owned()),
        },
    );
    diff
}

fn set_review_for(
    profile: &AgentProfileVersion,
    seed: u128,
    expected: ExpectedMemoryEntryState,
    candidate: MemoryEntryDraft,
    diff: Vec<MemoryFieldDiff>,
) -> MemoryEditReview {
    MemoryEditReview {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        expected,
        operation: MemoryMutationKind::Set,
        candidate: Some(candidate),
        diff,
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed)),
        review_digest: sha256(format!("set review {seed}").as_bytes()),
    }
}

fn formatted_tags(tags: &[String]) -> String {
    tags.iter()
        .map(|tag| tag.replace('\\', "\\\\").replace(',', "\\,"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn install_create_set_review_state(memory: &mut MemoryViewState, review: &MemoryEditReview) {
    let candidate = review.candidate.as_ref().expect("set candidate");
    memory
        .open_create_editor(AgentProfileSelector::Id(review.profile.profile_id()))
        .expect("open create editor");
    let editor = memory.editor.as_mut().expect("editor");
    editor
        .submit_line(candidate.display_key().to_owned())
        .expect("candidate key");
    editor
        .submit_line(candidate.value().to_owned())
        .expect("candidate value");
    let effect = editor
        .submit_line(formatted_tags(candidate.purpose_tags()))
        .expect("candidate tags");
    assert!(matches!(
        effect,
        ai_stock_forum::ui::memory_editor::MemoryEditorEffect::Preview(_)
    ));
    memory.begin_review_request().expect("review generation");
    let editor = memory.editor.as_mut().expect("editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    ));
    memory.edit_review = Some(review.clone());
    memory.review_registered = true;
    memory.pane = MemoryPane::MutationReview;
}

fn synchronize_set_review_with_editor_preview(memory: &mut MemoryViewState) {
    let review = memory.edit_review.clone().expect("set review");
    let editor = memory.editor.as_mut().expect("review editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    ));
    assert!(matches!(
        editor.preview(),
        Some(MemoryEditPreview::Review(retained)) if retained == &review
    ));
}

fn install_edit_set_review_state(
    memory: &mut MemoryViewState,
    profile: &AgentProfileVersion,
    seed: &MemoryEntryVersion,
    review: &MemoryEditReview,
) {
    let candidate = review.candidate.as_ref().expect("set candidate");
    memory.entries = Some(populated_entries_view(profile, std::slice::from_ref(seed)));
    memory.selected_entry = 0;
    memory.entry_detail = Some(MemoryEntryView {
        profile: profile.reference(),
        entry: seed.clone(),
    });
    memory
        .open_edit_editor(
            AgentProfileSelector::Id(review.profile.profile_id()),
            seed.clone(),
        )
        .expect("open edit editor");
    let editor = memory.editor.as_mut().expect("editor");
    editor
        .submit_line(candidate.value().to_owned())
        .expect("candidate value");
    let effect = editor
        .submit_line(formatted_tags(candidate.purpose_tags()))
        .expect("candidate tags");
    assert!(matches!(
        effect,
        ai_stock_forum::ui::memory_editor::MemoryEditorEffect::Preview(_)
    ));
    memory.begin_review_request().expect("review generation");
    let editor = memory.editor.as_mut().expect("editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    ));
    memory.edit_review = Some(review.clone());
    memory.review_registered = true;
    memory.pane = MemoryPane::MutationReview;
}

fn install_delete_review_state(
    memory: &mut MemoryViewState,
    profile: &AgentProfileVersion,
    current: &MemoryEntryVersion,
    review: &MemoryEditReview,
) {
    memory.entries = Some(populated_entries_view(
        profile,
        std::slice::from_ref(current),
    ));
    memory.selected_entry = 0;
    memory.entry_detail = Some(MemoryEntryView {
        profile: profile.reference(),
        entry: current.clone(),
    });
    memory.editor = None;
    memory
        .begin_review_request()
        .expect("delete review generation");
    memory.edit_review = Some(review.clone());
    memory.review_registered = true;
    memory.pane = MemoryPane::MutationReview;
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

fn direct_review_command(review: &MemoryEditReview) -> ApplicationCommand {
    match review.operation {
        MemoryMutationKind::Set => set_command(review),
        MemoryMutationKind::Delete => delete_command(review),
    }
}

fn retain_exact_direct_confirmation(memory: &mut MemoryViewState) {
    let review = memory.edit_review.as_ref().expect("direct review");
    memory.confirmation = Some(MemoryConfirmation {
        command: direct_review_command(review),
        generation: memory.generation,
    });
    memory.pane = MemoryPane::Confirmation;
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

fn entry_with_identity(
    namespace_id: MemoryNamespaceId,
    entry_id: MemoryEntryId,
    seed: u128,
    draft: MemoryEntryDraft,
) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        namespace_id,
        entry_id,
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(seed)),
        draft,
        Actor::Human,
        i64::try_from(seed).expect("fixture timestamp"),
        None,
        EventId::from_uuid(Uuid::from_u128(seed + 1)),
    )
    .expect("entry with exact identity")
}

fn plausible_set_result(
    owner: &AgentProfileVersion,
    review: &MemoryEditReview,
    seed: u128,
) -> MemoryEntryVersion {
    let candidate = review.candidate.clone().expect("set candidate");
    let entry_id = match &review.expected {
        ExpectedMemoryEntryState::Absent => MemoryEntryId::from_uuid(Uuid::from_u128(seed)),
        ExpectedMemoryEntryState::Present(reference)
        | ExpectedMemoryEntryState::Deleted(reference) => reference.entry_id(),
    };
    entry_with_identity(owner.memory_namespace_id(), entry_id, seed + 1, candidate)
}

fn plausible_delete_result(
    owner: &AgentProfileVersion,
    review: &MemoryEditReview,
    seed: u128,
) -> MemoryEntryVersion {
    let ExpectedMemoryEntryState::Present(expected) = &review.expected else {
        panic!("delete review must carry a Present discriminant")
    };
    let present = entry_with_identity(
        owner.memory_namespace_id(),
        expected.entry_id(),
        seed,
        MemoryEntryDraft::new(
            expected.normalized_key().as_str().to_owned(),
            "plausible delete value".to_owned(),
            Vec::new(),
        )
        .expect("plausible delete draft"),
    );
    present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(seed + 2)),
            Actor::Human,
            i64::try_from(seed + 2).expect("fixture timestamp"),
            None,
            EventId::from_uuid(Uuid::from_u128(seed + 3)),
        )
        .expect("plausible delete result")
}

fn plausible_mutation_result(
    owner: &AgentProfileVersion,
    review: &MemoryEditReview,
    seed: u128,
) -> MemoryEntryVersion {
    match review.operation {
        MemoryMutationKind::Set => plausible_set_result(owner, review, seed),
        MemoryMutationKind::Delete => plausible_delete_result(owner, review, seed),
    }
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
        namespace_id: proposal.namespace_id(),
        proposer: proposal.proposer().clone(),
        operation: MemoryProposalOperationKind::Set,
        display_key: proposal.display_key().to_owned(),
        status: MemoryProposalStatus::Pending,
        created_at_ms: proposal.created_at_ms(),
    }
}

fn with_proposal_summary_namespace(
    summary: &MemoryProposalSummary,
    namespace_id: MemoryNamespaceId,
) -> MemoryProposalSummary {
    let mut value = serde_json::to_value(summary).expect("serialize proposal summary");
    value
        .as_object_mut()
        .expect("proposal summary object")
        .insert(
            "namespace_id".to_owned(),
            serde_json::Value::String(namespace_id.to_string()),
        );
    serde_json::from_value(value).expect("namespace-bearing proposal summary")
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
    install_create_set_review_state(&mut agents.memory, &review);
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
    let mut state = MemoryViewState::default();
    state
        .bind_profile(memory_identity(&owner), owner.memory_namespace_id())
        .expect("bind mutation state");
    install_create_set_review_state(&mut state, &review);
    state.generation = 5;
    state.confirmation = Some(MemoryConfirmation {
        command: set_command(&review),
        generation: 5,
    });
    state.pending_intent = Some(MemoryOutcomeIntent::Mutation);
    state.pane = MemoryPane::Confirmation;
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
    let mut delete_state = MemoryViewState::default();
    delete_state
        .bind_profile(memory_identity(&owner), owner.memory_namespace_id())
        .expect("bind delete state");
    install_delete_review_state(&mut delete_state, &owner, &current, &delete_review);
    delete_state.generation = 8;
    delete_state.confirmation = Some(MemoryConfirmation {
        command: delete_command(&delete_review),
        generation: 8,
    });
    delete_state.pending_intent = Some(MemoryOutcomeIntent::Mutation);
    delete_state.pane = MemoryPane::Confirmation;
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

fn invalid_editor_model(
    owner: &AgentProfileVersion,
    selector: AgentProfileSelector,
    completed_fields: usize,
) -> TuiModel {
    let mut model = memory_model(owner);
    model
        .agents
        .memory
        .open_create_editor(selector)
        .expect("open invalid editor fixture");
    let editor = model.agents.memory.editor.as_mut().expect("editor");
    if completed_fields >= 1 {
        editor
            .submit_keyboard_line("guarded key")
            .expect("submit fixture key");
    }
    if completed_fields >= 2 {
        editor
            .submit_line("guarded value".to_owned())
            .expect("submit fixture value");
    }
    if completed_fields >= 3 {
        editor
            .submit_keyboard_line("guarded tag")
            .expect("submit fixture tags");
    }
    if completed_fields >= 4 {
        let review = set_review(owner, 24_000, "guarded key");
        assert!(editor.apply_preview(
            editor.generation(),
            MemoryEditPreview::Review(review.clone()),
        ));
        model.agents.memory.edit_review = Some(review);
        model.agents.memory.review_registered = true;
    }
    model.command.ingest("guarded draft");
    model.command.move_left();
    model.agents.memory.entry_scroll = 3;
    model.agents.memory.detail_scroll = 9;
    model.agents.memory.history_scroll = 4;
    model.agents.memory.proposal_scroll = 5;
    model.agents.memory.episode_scroll = 6;
    model
}

fn install_valid_protected_scroll_owner(
    model: &mut TuiModel,
    owner: &AgentProfileVersion,
    pane: MemoryPane,
    seed: u128,
) {
    match pane {
        MemoryPane::Editor => model
            .agents
            .memory
            .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
            .expect("valid editor scroll owner"),
        MemoryPane::MutationReview => {
            let review = set_review(owner, seed, "scroll review");
            install_create_set_review_state(&mut model.agents.memory, &review);
        }
        MemoryPane::ProposalResolutionReview => {
            model.agents.memory.resolution_review = Some(resolution_review(
                owner,
                owner,
                seed,
                MemoryResolutionAction::Approve,
            ));
            model.agents.memory.review_registered = true;
            model.agents.memory.selected_proposal_detail_action =
                MemoryProposalDetailAction::Approve;
            model.agents.memory.pane = pane;
        }
        MemoryPane::Confirmation => {
            let review = set_review(owner, seed, "scroll confirmation");
            let command = set_command(&review);
            install_create_set_review_state(&mut model.agents.memory, &review);
            model.agents.memory.generation = u64::try_from(seed).expect("fixture generation");
            model.agents.memory.confirmation = Some(MemoryConfirmation {
                command,
                generation: u64::try_from(seed).expect("fixture generation"),
            });
            model.agents.memory.pane = pane;
        }
        _ => {}
    }
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
    model.agents.memory.selected_entry = 0;
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

    model.agents.memory.editor = None;
    install_create_set_review_state(&mut model.agents.memory, &review);
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
    install_create_set_review_state(&mut valid.agents.memory, &review);

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
    install_create_set_review_state(&mut review_model.agents.memory, &review);
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
    model.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&seeded),
    ));
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
    model.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&seeded),
    ));
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
fn memory_editor_field_submissions_never_enter_global_command_history() {
    let owner = profile(24_500);
    let mut model = memory_model(&owner);
    model
        .command
        .remember("/status PREEXISTING ORDINARY HISTORY".to_owned());
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
        .expect("open editor");

    for (index, input) in [
        "SYNTHETIC MEMORY KEY",
        "SYNTHETIC MEMORY VALUE",
        "SYNTHETIC MEMORY TAG ONE, SYNTHETIC MEMORY TAG TWO",
    ]
    .into_iter()
    .enumerate()
    {
        model.command.clear();
        model.command.ingest(input);
        let effect = handle_event(&mut model, key(KeyCode::Enter));
        if index == 2 {
            assert!(
                matches!(effect, ControllerEffect::RequestMemorySetPreview(_)),
                "tags should request the exact preview: {effect:?}",
            );
        } else {
            assert_eq!(effect, ControllerEffect::Redraw);
        }
        assert_eq!(model.command.history_len(), 1);
        assert_eq!(
            model.command.history_back(),
            Some("/status PREEXISTING ORDINARY HISTORY"),
        );
    }

    for _ in 0..3 {
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Esc)),
            ControllerEffect::Redraw,
        );
    }
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryList);
    assert!(model.agents.memory.editor.is_none());
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('/'))),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.focus, Focus::Command);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Up)),
        ControllerEffect::Redraw,
    );
    assert_eq!(model.command.text(), "/status PREEXISTING ORDINARY HISTORY");
    assert_eq!(model.command.history_len(), 1);
}

#[test]
fn all_nine_global_shortcuts_preserve_each_non_text_memory_review_and_confirmation_state() {
    let owner = profile(1_300);
    let edit_review = set_review(&owner, 1_310, "protected navigation");
    let mut editor_state = MemoryViewState::default();
    editor_state
        .bind_profile(memory_identity(&owner), owner.memory_namespace_id())
        .expect("bind editor state");
    install_create_set_review_state(&mut editor_state, &edit_review);

    let mutation_state = MemoryViewState {
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
        for shortcut in ['1', '2', '3', '4', '5', '6', '7', '8', '9'] {
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
                handle_event(&mut model, key(KeyCode::Char('3'))),
                ControllerEffect::Redraw | ControllerEffect::LoadAgentProfiles
            ));
            assert_eq!(model.active_view, View::Agents);
            assert_eq!(model.agents.pane, AgentsPane::Memory);
            assert_eq!(model.agents.memory, before, "restored shortcut={shortcut}");
        }
    }
}

#[test]
fn empty_memory_selections_fail_safely() {
    let owner = profile(1_400);
    let current = entry(&owner, 1_401, "empty history parent");
    for pane in [
        MemoryPane::EntryList,
        MemoryPane::EntryHistory,
        MemoryPane::Proposals,
        MemoryPane::EpisodicSummaries,
    ] {
        let mut model = memory_model(&owner);
        model.agents.memory.pane = pane;
        match pane {
            MemoryPane::EntryList => {
                model.agents.memory.entries =
                    Some(entries_view(&owner, owner.memory_namespace_id()));
            }
            MemoryPane::EntryHistory => {
                model.agents.memory.entries = Some(populated_entries_view(
                    &owner,
                    std::slice::from_ref(&current),
                ));
                model.agents.memory.entry_history = Some(history_view(&owner, &current, &[]));
            }
            MemoryPane::Proposals => {
                model.agents.memory.proposals = Some(proposals_view(&owner, &[]));
            }
            MemoryPane::EpisodicSummaries => {
                model.agents.memory.episodes = Some(summaries_view(&owner, &[]));
            }
            _ => unreachable!(),
        }
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

    for pane in [MemoryPane::EntryDetail, MemoryPane::ProposalDetail] {
        let mut model = memory_model(&owner);
        model.agents.memory.pane = pane;
        let before = model.clone();
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Enter)),
            ControllerEffect::Redraw,
            "pane={pane:?}",
        );
        assert_eq!(model, before, "pane={pane:?}");
    }
}

#[test]
fn memory_list_scroll_keys_move_stable_selections_and_only_the_matching_offset() {
    let owner = profile(1_410);
    let entries = (0..105)
        .map(|index| entry(&owner, 1_500 + index * 10, &format!("entry {index:03}")))
        .collect::<Vec<_>>();
    let proposals = (0..105)
        .map(|index| proposal(&owner, 3_000 + index * 10, &format!("proposal {index:03}")))
        .collect::<Vec<_>>();
    let episodes = (0..105)
        .map(|index| summary(&owner, 5_000 + index * 10, &format!("episode {index:03}")))
        .collect::<Vec<_>>();

    let mut entry_model = memory_model(&owner);
    entry_model.set_terminal_size(100, 30);
    entry_model.agents.memory.entries = Some(populated_entries_view(&owner, &entries));
    entry_model.agents.memory.history_scroll = 31;
    entry_model.agents.memory.proposal_scroll = 32;
    entry_model.agents.memory.episode_scroll = 33;
    assert_eq!(
        handle_event(&mut entry_model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (
            entry_model.agents.memory.selected_entry,
            entry_model.agents.memory.entry_scroll
        ),
        (1, 0)
    );
    assert_eq!(
        handle_event(&mut entry_model, key(KeyCode::PageDown)),
        ControllerEffect::Redraw
    );
    assert!(entry_model.agents.memory.selected_entry > 1);
    assert_eq!(
        handle_event(&mut entry_model, key(KeyCode::End)),
        ControllerEffect::Redraw
    );
    assert_eq!(entry_model.agents.memory.selected_entry, 99);
    assert_eq!(
        handle_event(&mut entry_model, key(KeyCode::Home)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (
            entry_model.agents.memory.selected_entry,
            entry_model.agents.memory.entry_scroll
        ),
        (0, 0)
    );
    assert_eq!(
        (
            entry_model.agents.memory.history_scroll,
            entry_model.agents.memory.proposal_scroll,
            entry_model.agents.memory.episode_scroll
        ),
        (31, 32, 33)
    );
    assert_eq!(entry_model.workspace_scroll, 0);

    let current = entries[0].clone();
    let mut history_model = memory_model(&owner);
    history_model.set_terminal_size(100, 30);
    history_model.agents.memory.pane = MemoryPane::EntryHistory;
    history_model.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&current),
    ));
    history_model.agents.memory.entry_history =
        Some(history_view(&owner, &current, &vec![current.clone(); 105]));
    history_model.agents.memory.entry_scroll = 31;
    history_model.agents.memory.proposal_scroll = 32;
    history_model.agents.memory.episode_scroll = 33;
    assert_eq!(
        handle_event(&mut history_model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (
            history_model.agents.memory.selected_history_version,
            history_model.agents.memory.history_scroll
        ),
        (1, 0)
    );
    assert_eq!(
        handle_event(&mut history_model, key(KeyCode::PageDown)),
        ControllerEffect::Redraw
    );
    assert!(history_model.agents.memory.selected_history_version > 1);
    assert_eq!(
        handle_event(&mut history_model, key(KeyCode::End)),
        ControllerEffect::Redraw
    );
    assert_eq!(history_model.agents.memory.selected_history_version, 99);
    assert_eq!(
        handle_event(&mut history_model, key(KeyCode::Home)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (
            history_model.agents.memory.selected_history_version,
            history_model.agents.memory.history_scroll
        ),
        (0, 0)
    );
    assert_eq!(
        (
            history_model.agents.memory.entry_scroll,
            history_model.agents.memory.proposal_scroll,
            history_model.agents.memory.episode_scroll
        ),
        (31, 32, 33)
    );
    assert_eq!(history_model.workspace_scroll, 0);

    let mut proposal_model = memory_model(&owner);
    proposal_model.set_terminal_size(100, 30);
    proposal_model.agents.memory.pane = MemoryPane::Proposals;
    proposal_model.agents.memory.proposals = Some(proposals_view(&owner, &proposals));
    proposal_model.agents.memory.entry_scroll = 31;
    proposal_model.agents.memory.history_scroll = 32;
    proposal_model.agents.memory.episode_scroll = 33;
    assert_eq!(
        handle_event(&mut proposal_model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (
            proposal_model.agents.memory.selected_proposal,
            proposal_model.agents.memory.proposal_scroll
        ),
        (1, 0)
    );
    assert_eq!(
        handle_event(&mut proposal_model, key(KeyCode::PageDown)),
        ControllerEffect::Redraw
    );
    assert!(proposal_model.agents.memory.selected_proposal > 1);
    assert_eq!(
        handle_event(&mut proposal_model, key(KeyCode::End)),
        ControllerEffect::Redraw
    );
    assert_eq!(proposal_model.agents.memory.selected_proposal, 99);
    assert_eq!(
        handle_event(&mut proposal_model, key(KeyCode::Home)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (
            proposal_model.agents.memory.selected_proposal,
            proposal_model.agents.memory.proposal_scroll
        ),
        (0, 0)
    );
    assert_eq!(
        (
            proposal_model.agents.memory.entry_scroll,
            proposal_model.agents.memory.history_scroll,
            proposal_model.agents.memory.episode_scroll
        ),
        (31, 32, 33)
    );
    assert_eq!(proposal_model.workspace_scroll, 0);

    let mut episode_model = memory_model(&owner);
    episode_model.set_terminal_size(100, 30);
    episode_model.agents.memory.pane = MemoryPane::EpisodicSummaries;
    episode_model.agents.memory.episodes = Some(summaries_view(&owner, &episodes));
    episode_model.agents.memory.entry_scroll = 31;
    episode_model.agents.memory.history_scroll = 32;
    episode_model.agents.memory.proposal_scroll = 33;
    assert_eq!(
        handle_event(&mut episode_model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (
            episode_model.agents.memory.selected_episode,
            episode_model.agents.memory.episode_scroll
        ),
        (1, 0)
    );
    assert_eq!(
        handle_event(&mut episode_model, key(KeyCode::PageDown)),
        ControllerEffect::Redraw
    );
    assert!(episode_model.agents.memory.selected_episode > 1);
    assert_eq!(
        handle_event(&mut episode_model, key(KeyCode::End)),
        ControllerEffect::Redraw
    );
    assert_eq!(episode_model.agents.memory.selected_episode, 99);
    assert_eq!(
        handle_event(&mut episode_model, key(KeyCode::Home)),
        ControllerEffect::Redraw
    );
    assert_eq!(
        (
            episode_model.agents.memory.selected_episode,
            episode_model.agents.memory.episode_scroll
        ),
        (0, 0)
    );
    assert_eq!(
        (
            episode_model.agents.memory.entry_scroll,
            episode_model.agents.memory.history_scroll,
            episode_model.agents.memory.proposal_scroll
        ),
        (31, 32, 33)
    );
    assert_eq!(episode_model.workspace_scroll, 0);
}

#[test]
fn every_memory_detail_owner_uses_geometry_sized_pages_home_and_end() {
    let owner = profile(7_000);
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
        model.set_terminal_size(100, 30);
        model.agents.memory.pane = pane;
        install_valid_protected_scroll_owner(&mut model, &owner, pane, 7_050);
        match pane {
            MemoryPane::EntryDetail => {
                let selected = entry(&owner, 7_010, "scroll entry");
                model.agents.memory.entries = Some(populated_entries_view(
                    &owner,
                    std::slice::from_ref(&selected),
                ));
                model.agents.memory.entry_detail = Some(MemoryEntryView {
                    profile: owner.reference(),
                    entry: selected,
                });
            }
            MemoryPane::ProposalDetail => {
                let selected = proposal(&owner, 7_020, "scroll proposal");
                model.agents.memory.proposals =
                    Some(proposals_view(&owner, std::slice::from_ref(&selected)));
                model.agents.memory.proposal_detail = Some(proposal_detail(&owner, &selected));
            }
            MemoryPane::EpisodicDetail => {
                let selected = summary(&owner, 7_030, "scroll episode");
                model.agents.memory.episodes =
                    Some(summaries_view(&owner, std::slice::from_ref(&selected)));
                model.agents.memory.episode_detail = Some(EpisodicSummaryView {
                    summary: selected,
                    qualification: EpisodicQualification::SummaryVerifySources,
                });
            }
            _ => {}
        }
        let page = usize::from(model.workspace_body_height.max(1));
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Down)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.agents.memory.detail_scroll, 1, "pane={pane:?}");
        assert_eq!(
            handle_event(&mut model, key(KeyCode::PageDown)),
            ControllerEffect::Redraw
        );
        assert!(model.agents.memory.detail_scroll > page, "pane={pane:?}");
        assert_eq!(
            handle_event(&mut model, key(KeyCode::PageUp)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.agents.memory.detail_scroll, 1, "pane={pane:?}");
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Up)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.agents.memory.detail_scroll, 0, "pane={pane:?}");
        assert_eq!(
            handle_event(&mut model, key(KeyCode::End)),
            ControllerEffect::Redraw
        );
        assert_eq!(
            model.agents.memory.detail_scroll,
            usize::MAX,
            "pane={pane:?}"
        );
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Home)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.agents.memory.detail_scroll, 0, "pane={pane:?}");
        assert_eq!(model.workspace_scroll, 0, "pane={pane:?}");
    }

    let version = entry(&owner, 7_100, "historical");
    let mut history = memory_model(&owner);
    history.set_terminal_size(100, 30);
    history.agents.memory.pane = MemoryPane::EntryHistory;
    history.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&version),
    ));
    history.agents.memory.entry_history = Some(history_view(
        &owner,
        &version,
        std::slice::from_ref(&version),
    ));
    history.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: owner.reference(),
        entry: version,
    });
    assert_eq!(
        handle_event(&mut history, key(KeyCode::PageDown)),
        ControllerEffect::Redraw
    );
    assert!(
        history.agents.memory.detail_scroll >= usize::from(history.workspace_body_height.max(1))
    );
    assert_eq!(history.workspace_scroll, 0);

    let review = set_review(&owner, 7_200, "editor review");
    let mut editor = memory_model(&owner);
    editor.set_terminal_size(100, 30);
    install_create_set_review_state(&mut editor.agents.memory, &review);
    assert_eq!(
        handle_event(&mut editor, key(KeyCode::PageDown)),
        ControllerEffect::Redraw
    );
    assert!(editor.agents.memory.detail_scroll >= usize::from(editor.workspace_body_height.max(1)));
    assert_eq!(editor.workspace_scroll, 0);
}

#[test]
fn every_memory_editor_text_stage_routes_vertical_and_page_keys_to_detail_scroll() {
    let owner = profile(7_250);
    for completed_fields in 0..=2 {
        for height in [18, 30] {
            let mut model = memory_model(&owner);
            model.set_terminal_size(100, height);
            model.command.remember("GLOBAL COMMAND HISTORY".to_owned());
            model
                .agents
                .memory
                .open_create_editor(AgentProfileSelector::Id(owner.profile_id()))
                .expect("editor");
            let editor = model.agents.memory.editor.as_mut().expect("editor");
            if completed_fields >= 1 {
                editor.submit_line("saved key".to_owned()).expect("key");
            }
            if completed_fields >= 2 {
                editor.submit_line("saved value".to_owned()).expect("value");
            }
            model.command.clear();
            model.command.ingest("draft");
            let editor_before = model.agents.memory.editor.clone();
            let page = usize::from(model.workspace_body_height.max(1));

            for (code, expected) in [
                (KeyCode::Down, 1),
                (KeyCode::PageDown, page.saturating_add(1)),
                (KeyCode::PageUp, 1),
                (KeyCode::Up, 0),
            ] {
                assert_eq!(
                    handle_event(&mut model, key(code)),
                    ControllerEffect::Redraw,
                    "stage={completed_fields} height={height} key={code:?}"
                );
                assert_eq!(
                    model.agents.memory.detail_scroll, expected,
                    "stage={completed_fields} height={height} key={code:?}"
                );
                assert_eq!(model.command.text(), "draft");
                assert_eq!(model.agents.memory.editor, editor_before);
                assert_eq!(model.workspace_scroll, 0);
            }

            model.agents.memory.detail_scroll = 5;
            assert_eq!(
                handle_event(&mut model, key(KeyCode::Home)),
                ControllerEffect::Redraw
            );
            assert_eq!(
                handle_event(&mut model, key(KeyCode::Char('x'))),
                ControllerEffect::Redraw
            );
            assert_eq!(
                handle_event(&mut model, key(KeyCode::End)),
                ControllerEffect::Redraw
            );
            assert_eq!(
                handle_event(&mut model, key(KeyCode::Char('y'))),
                ControllerEffect::Redraw
            );
            assert_eq!(model.command.text(), "xdrafty");
            assert_eq!(model.agents.memory.detail_scroll, 5);
            assert_eq!(model.workspace_scroll, 0);
        }
    }
}

#[test]
fn every_memory_detail_scroll_owner_uses_geometry_pages_at_both_supported_heights() {
    let owner = profile(7_260);
    for height in [18, 30] {
        for pane in [
            MemoryPane::EntryDetail,
            MemoryPane::EntryHistory,
            MemoryPane::Editor,
            MemoryPane::MutationReview,
            MemoryPane::Confirmation,
            MemoryPane::ProposalDetail,
            MemoryPane::ProposalResolutionReview,
            MemoryPane::EpisodicDetail,
            MemoryPane::Result,
        ] {
            let mut model = memory_model(&owner);
            model.set_terminal_size(100, height);
            model.agents.memory.pane = pane;
            install_valid_protected_scroll_owner(&mut model, &owner, pane, 7_310);
            match pane {
                MemoryPane::EntryDetail => {
                    let selected = entry(&owner, 7_270, "owner entry");
                    model.agents.memory.entries = Some(populated_entries_view(
                        &owner,
                        std::slice::from_ref(&selected),
                    ));
                    model.agents.memory.entry_detail = Some(MemoryEntryView {
                        profile: owner.reference(),
                        entry: selected,
                    });
                }
                MemoryPane::EntryHistory => {
                    let selected = entry(&owner, 7_280, "owner history");
                    model.agents.memory.entries = Some(populated_entries_view(
                        &owner,
                        std::slice::from_ref(&selected),
                    ));
                    model.agents.memory.entry_history = Some(history_view(
                        &owner,
                        &selected,
                        std::slice::from_ref(&selected),
                    ));
                    model.agents.memory.entry_version = Some(MemoryEntryVersionView {
                        profile: owner.reference(),
                        entry: selected,
                    });
                }
                MemoryPane::Editor => {}
                MemoryPane::ProposalDetail => {
                    let selected = proposal(&owner, 7_290, "owner proposal");
                    model.agents.memory.proposals =
                        Some(proposals_view(&owner, std::slice::from_ref(&selected)));
                    model.agents.memory.proposal_detail = Some(proposal_detail(&owner, &selected));
                }
                MemoryPane::EpisodicDetail => {
                    let selected = summary(&owner, 7_300, "owner episode");
                    model.agents.memory.episodes =
                        Some(summaries_view(&owner, std::slice::from_ref(&selected)));
                    model.agents.memory.episode_detail = Some(EpisodicSummaryView {
                        summary: selected,
                        qualification: EpisodicQualification::SummaryVerifySources,
                    });
                }
                MemoryPane::MutationReview
                | MemoryPane::Confirmation
                | MemoryPane::ProposalResolutionReview
                | MemoryPane::Result => {}
                MemoryPane::EntryList | MemoryPane::Proposals | MemoryPane::EpisodicSummaries => {
                    unreachable!()
                }
            }
            let page = usize::from(model.workspace_body_height.max(1));
            for (code, expected) in [
                (KeyCode::Down, 1),
                (KeyCode::PageDown, page.saturating_add(1)),
                (KeyCode::PageUp, 1),
                (KeyCode::Up, 0),
            ] {
                assert_eq!(
                    handle_event(&mut model, key(code)),
                    ControllerEffect::Redraw,
                    "pane={pane:?} height={height} key={code:?}"
                );
                assert_eq!(
                    model.agents.memory.detail_scroll, expected,
                    "pane={pane:?} height={height} key={code:?}"
                );
                assert_eq!(model.workspace_scroll, 0);
            }
        }
    }
}

#[test]
fn detail_scroll_resets_on_memory_identity_changes_but_survives_redraw_resize_and_tabs() {
    let owner = profile(7_300);
    let first = entry(&owner, 7_310, "first");
    let second = entry(&owner, 7_320, "second");
    let mut model = memory_model(&owner);
    model.set_terminal_size(100, 30);
    model.agents.memory.entries = Some(populated_entries_view(&owner, &[first, second]));
    model.agents.memory.detail_scroll = 9;

    assert_eq!(
        handle_event(&mut model, TuiEvent::Resize(120, 30)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.detail_scroll, 9);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('i'))),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.detail_scroll, 9);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('9'))),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.detail_scroll, 9);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('3'))),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.detail_scroll, 9);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Down)),
        ControllerEffect::Redraw
    );
    assert_eq!(model.agents.memory.detail_scroll, 0);
    model.agents.memory.detail_scroll = 11;
    assert!(matches!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::LoadMemoryEntry { .. }
    ));
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryDetail);
    assert_eq!(model.agents.memory.detail_scroll, 0);
}

#[test]
fn memory_reviews_and_confirmations_require_exact_registered_bindings_and_press_only() {
    let owner = profile(1_500);
    let review = set_review(&owner, 1_510, "review key");
    let command = set_command(&review);
    let mut model = memory_model(&owner);
    install_create_set_review_state(&mut model.agents.memory, &review);
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
    assert_eq!(model.agents.memory.pane, MemoryPane::Editor);
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
    install_delete_review_state(&mut delete_model.agents.memory, &owner, &current, &delete);
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
    install_create_set_review_state(&mut set_model.agents.memory, &set_review);
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
    install_delete_review_state(
        &mut delete_model.agents.memory,
        &owner,
        &current,
        &delete_review,
    );
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
    model.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&first_v2),
    ));
    model.agents.memory.selected_entry = 0;
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
    assert!(model.agents.memory.entry_version.is_some());
    assert_eq!(model.agents.memory.detail_scroll, 1);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Esc)),
        ControllerEffect::Redraw,
    );
    assert!(model.agents.memory.entry_version.is_none());
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
fn memory_keeps_global_help_and_cycles_focus_without_the_latent_inspector() {
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
    assert_eq!(focus.focus, Focus::Navigation);
}

#[test]
fn ordinary_command_input_keeps_i_while_memory_is_open_and_submits_quit_exactly() {
    let owner = profile(1_599_100);
    let mut model = memory_model(&owner);
    model.inspector_open = true;

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('/'))),
        ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Command);
    for character in ['q', 'u', 'i', 't'] {
        assert_eq!(
            handle_event(&mut model, key(KeyCode::Char(character))),
            ControllerEffect::Redraw
        );
    }
    assert_eq!(model.command.text(), "/quit");
    assert!(model.inspector_open);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ControllerEffect::Submit(ApplicationCommand::RequestShutdown)
    );
    assert_eq!(model.command.history_back(), Some("/quit"));
    assert!(model.inspector_open);
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

    let action_proposal = proposal(&owner, 1_620, "layered proposal");
    model.agents.memory.proposals = Some(proposals_view(
        &owner,
        std::slice::from_ref(&action_proposal),
    ));
    model.agents.memory.proposal_detail = Some(proposal_detail(&owner, &action_proposal));
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

fn assert_memory_local_keys_are_closed(model: &TuiModel, keys: &[KeyCode]) {
    for code in keys {
        let mut attempted = model.clone();
        let before = attempted.clone();
        assert_eq!(
            handle_event(&mut attempted, key(*code)),
            ControllerEffect::Redraw,
            "pane={:?} code={code:?}",
            before.agents.memory.pane,
        );
        assert_eq!(
            attempted, before,
            "malformed or stale cache changed protected state for pane={:?} code={code:?}",
            before.agents.memory.pane,
        );
    }
}

fn assert_invalid_protected_workflow_is_closed(model: &TuiModel, label: &str) {
    for event in [
        key(KeyCode::Char('x')),
        key(KeyCode::Enter),
        key(KeyCode::Up),
        key(KeyCode::Down),
        key(KeyCode::PageUp),
        key(KeyCode::PageDown),
        key(KeyCode::Home),
        key(KeyCode::End),
    ] {
        let mut attempted = model.clone();
        let before = attempted.clone();
        assert_eq!(
            handle_event(&mut attempted, event),
            ControllerEffect::Redraw,
            "invalid workflow accepted input: {label}",
        );
        assert_eq!(
            attempted, before,
            "invalid workflow mutated protected state: {label}",
        );
    }
}

#[test]
fn malformed_memory_editors_cannot_type_paste_submit_recall_or_scroll() {
    let owner = profile(24_100);
    let other = profile(24_200);
    let cases = [
        (
            invalid_editor_model(&owner, AgentProfileSelector::Id(other.profile_id()), 0),
            "key with mismatched ID selector",
        ),
        (
            invalid_editor_model(&owner, AgentProfileSelector::Id(other.profile_id()), 1),
            "value with mismatched ID selector",
        ),
        (
            invalid_editor_model(
                &owner,
                AgentProfileSelector::Name("unrelated profile".to_owned()),
                2,
            ),
            "tags with mismatched normalized-name selector",
        ),
        (
            invalid_editor_model(
                &owner,
                AgentProfileSelector::Name("unrelated profile".to_owned()),
                4,
            ),
            "review with mismatched normalized-name selector",
        ),
        ({
            let mut missing_namespace =
                invalid_editor_model(&owner, AgentProfileSelector::Id(owner.profile_id()), 0);
            missing_namespace.agents.memory.namespace_id = None;
            (missing_namespace, "key without namespace context")
        }),
    ];

    for (invalid, label) in cases {
        assert_invalid_protected_workflow_is_closed(&invalid, label);
        let mut pasted = invalid.clone();
        let before = pasted.clone();
        assert_eq!(
            handle_event(
                &mut pasted,
                TuiEvent::Paste("PASTED INVALID MEMORY DRAFT".to_owned()),
            ),
            ControllerEffect::Redraw,
            "invalid editor accepted paste: {label}",
        );
        assert_eq!(pasted, before, "invalid editor mutated on paste: {label}");
    }

    let mut escaped = invalid_editor_model(&owner, AgentProfileSelector::Id(other.profile_id()), 0);
    assert_eq!(
        handle_event(&mut escaped, key(KeyCode::Esc)),
        ControllerEffect::Redraw,
    );
    assert_eq!(escaped.agents.memory.pane, MemoryPane::EntryList);
    assert!(escaped.agents.memory.editor.is_none());
}

#[test]
fn malformed_edit_editor_origin_seed_and_namespace_states_are_closed() {
    let owner = profile(24_210);
    let foreign = profile(24_220);
    let selector = AgentProfileSelector::Id(owner.profile_id());
    let mut cases = Vec::new();

    let mut create_with_seed = memory_model(&owner);
    create_with_seed
        .agents
        .memory
        .open_edit_editor(
            selector.clone(),
            entry(&owner, 24_211, "CREATE_WITH_SEED_PROSE"),
        )
        .expect("seeded editor");
    create_with_seed.agents.memory.editor_origin = MemoryEditorOrigin::Create;
    cases.push((create_with_seed, "create origin with retained seed"));

    let mut edit_without_seed = memory_model(&owner);
    edit_without_seed
        .agents
        .memory
        .open_create_editor(selector.clone())
        .expect("seedless editor");
    edit_without_seed.agents.memory.editor_origin = MemoryEditorOrigin::Edit;
    cases.push((edit_without_seed, "edit origin without retained seed"));

    let mut foreign_namespace_seed = memory_model(&owner);
    foreign_namespace_seed
        .agents
        .memory
        .open_edit_editor(
            selector.clone(),
            entry(&foreign, 24_221, "FOREIGN_NAMESPACE_SEED_PROSE"),
        )
        .expect("foreign namespace editor fixture");
    cases.push((foreign_namespace_seed, "foreign namespace retained seed"));

    let present = entry(&owner, 24_230, "TOMBSTONE_SEED_PROSE");
    let tombstone = present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(24_233)),
            Actor::Human,
            24_232,
            None,
            EventId::from_uuid(Uuid::from_u128(24_232)),
        )
        .expect("tombstone");
    let mut rejected_tombstone = memory_model(&owner);
    let before_rejection = rejected_tombstone.agents.memory.clone();
    assert_eq!(
        rejected_tombstone
            .agents
            .memory
            .open_edit_editor(selector.clone(), tombstone.clone()),
        Err(DomainError::InvalidMemoryEditorSeed),
    );
    assert_eq!(rejected_tombstone.agents.memory, before_rejection);

    let mut seedless_edit_with_tombstone_cache = memory_model(&owner);
    seedless_edit_with_tombstone_cache
        .agents
        .memory
        .open_create_editor(selector.clone())
        .expect("seedless tombstone editor");
    seedless_edit_with_tombstone_cache
        .agents
        .memory
        .editor_origin = MemoryEditorOrigin::Edit;
    seedless_edit_with_tombstone_cache
        .agents
        .memory
        .entry_detail = Some(MemoryEntryView {
        profile: owner.reference(),
        entry: tombstone,
    });
    cases.push((
        seedless_edit_with_tombstone_cache,
        "edit origin without a retained seed despite a tombstone cache",
    ));

    let mut seedless_edit_with_unrelated_cache = memory_model(&owner);
    seedless_edit_with_unrelated_cache
        .agents
        .memory
        .open_create_editor(selector)
        .expect("seedless mismatched-key editor");
    seedless_edit_with_unrelated_cache
        .agents
        .memory
        .editor
        .as_mut()
        .expect("editor")
        .submit_keyboard_line("EDITOR_KEY_PROSE")
        .expect("editor key");
    seedless_edit_with_unrelated_cache
        .agents
        .memory
        .editor_origin = MemoryEditorOrigin::Edit;
    seedless_edit_with_unrelated_cache
        .agents
        .memory
        .entry_detail = Some(MemoryEntryView {
        profile: owner.reference(),
        entry: entry(&owner, 24_240, "CACHED_OTHER_KEY_PROSE"),
    });
    cases.push((
        seedless_edit_with_unrelated_cache,
        "edit origin without a retained seed despite an unrelated cache",
    ));

    for (mut invalid, label) in cases {
        invalid.command.ingest("INVALID_EDIT_EDITOR_DRAFT_PROSE");
        invalid.agents.memory.detail_scroll = 9;
        assert_invalid_protected_workflow_is_closed(&invalid, label);

        let mut pasted = invalid.clone();
        let before_paste = pasted.clone();
        assert_eq!(
            handle_event(
                &mut pasted,
                TuiEvent::Paste("PASTED_INVALID_EDIT_PROSE".to_owned()),
            ),
            ControllerEffect::Redraw,
            "invalid editor accepted paste: {label}",
        );
        assert_eq!(
            pasted, before_paste,
            "invalid editor mutated on paste: {label}"
        );

        let mut escaped = invalid;
        assert_eq!(
            handle_event(&mut escaped, key(KeyCode::Esc)),
            ControllerEffect::Redraw,
            "invalid editor did not preserve Esc: {label}",
        );
        if escaped.agents.memory.editor.is_some() {
            assert_eq!(
                handle_event(&mut escaped, key(KeyCode::Esc)),
                ControllerEffect::Redraw,
                "invalid editor did not finish its established Esc path: {label}",
            );
        }
        assert!(escaped.agents.memory.editor.is_none(), "case={label}");
    }
}

#[test]
fn malformed_memory_reviews_and_confirmation_close_every_local_action() {
    let owner = profile(24_300);
    let other = profile(24_400);

    let mut mutation = memory_model(&owner);
    mutation.agents.memory.pane = MemoryPane::MutationReview;
    mutation.agents.memory.edit_review = Some(set_review(&owner, 24_310, "invalid mutation"));
    mutation.agents.memory.review_registered = false;
    mutation.agents.memory.detail_scroll = 9;
    assert_invalid_protected_workflow_is_closed(&mutation, "unregistered mutation review");

    let mut resolution = memory_model(&owner);
    resolution.agents.memory.pane = MemoryPane::ProposalResolutionReview;
    resolution.agents.memory.resolution_review = Some(resolution_review(
        &owner,
        &owner,
        24_320,
        MemoryResolutionAction::Approve,
    ));
    resolution
        .agents
        .memory
        .resolution_review
        .as_mut()
        .expect("resolution review")
        .proposer_identity = memory_identity(&other);
    resolution.agents.memory.review_registered = true;
    resolution.agents.memory.selected_proposal_detail_action = MemoryProposalDetailAction::Approve;
    resolution.agents.memory.detail_scroll = 9;
    assert_invalid_protected_workflow_is_closed(
        &resolution,
        "resolution review with foreign proposer identity",
    );

    let review = set_review(&owner, 24_330, "invalid confirmation");
    let mut confirmation = memory_model(&owner);
    confirmation.agents.memory.pane = MemoryPane::Confirmation;
    confirmation.agents.memory.edit_review = Some(review);
    confirmation.agents.memory.review_registered = true;
    confirmation.agents.memory.generation = 12;
    confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
        command: ApplicationCommand::RequestShutdown,
        generation: 12,
    });
    confirmation.agents.memory.detail_scroll = 9;
    assert_invalid_protected_workflow_is_closed(
        &confirmation,
        "confirmation with substituted command",
    );
}

#[test]
fn direct_reviews_confirmations_and_results_require_exact_provenance_state_and_diffs() {
    let owner = profile(24_410);
    let foreign = profile(24_420);
    let base_review = set_review(&owner, 24_430, "direct auth key");
    let mut base_set = memory_model(&owner);
    install_create_set_review_state(&mut base_set.agents.memory, &base_review);

    let current = entry(&owner, 24_440, "direct auth key");
    let deleted = current
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(24_443)),
            Actor::Human,
            24_443,
            None,
            EventId::from_uuid(Uuid::from_u128(24_444)),
        )
        .expect("deleted reference");
    let foreign_current = entry(&foreign, 24_450, "direct auth key");
    let different_key = entry(&owner, 24_460, "different normalized key");

    let mut cases = Vec::new();
    for (label, expected) in [
        (
            "Set Present carrying deleted reference",
            ExpectedMemoryEntryState::Present(deleted.reference()),
        ),
        (
            "Set Deleted carrying present reference",
            ExpectedMemoryEntryState::Deleted(current.reference()),
        ),
        (
            "Set referenced foreign namespace",
            ExpectedMemoryEntryState::Present(foreign_current.reference()),
        ),
        (
            "Set referenced normalized key differs from candidate",
            ExpectedMemoryEntryState::Present(different_key.reference()),
        ),
    ] {
        let mut model = base_set.clone();
        model
            .agents
            .memory
            .edit_review
            .as_mut()
            .expect("set review")
            .expected = expected;
        synchronize_set_review_with_editor_preview(&mut model.agents.memory);
        cases.push((label, model));
    }

    let mut detached_editor = base_set.clone();
    detached_editor.agents.memory.editor = None;
    cases.push(("Set detached from editor", detached_editor));

    let mut different_candidate = base_set.clone();
    let forged_candidate = MemoryEntryDraft::new(
        "direct auth key".to_owned(),
        "different editor candidate value".to_owned(),
        Vec::new(),
    )
    .expect("different candidate");
    let forged_review = different_candidate
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("set review");
    forged_review.candidate = Some(forged_candidate.clone());
    forged_review.diff = absent_set_diff(&forged_candidate);
    synchronize_set_review_with_editor_preview(&mut different_candidate.agents.memory);
    cases.push((
        "Set candidate differs from editor preview request",
        different_candidate,
    ));

    let mut mismatched_preview = base_set.clone();
    mismatched_preview.agents.memory.edit_review =
        Some(set_review(&owner, 24_431, "different editor candidate"));
    cases.push((
        "Set outer review differs from retained editor preview",
        mismatched_preview,
    ));

    let mut empty_set_diff = base_set.clone();
    empty_set_diff
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("set review")
        .diff
        .clear();
    synchronize_set_review_with_editor_preview(&mut empty_set_diff.agents.memory);
    cases.push(("Set empty diff", empty_set_diff));

    let mut falsified_set_diff = base_set.clone();
    falsified_set_diff
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("set review")
        .diff[2]
        .after = MemoryFieldValue::Text("falsified candidate value".to_owned());
    synchronize_set_review_with_editor_preview(&mut falsified_set_diff.agents.memory);
    cases.push(("Set falsified diff", falsified_set_diff));

    let mut noncanonical_set_diff = base_set.clone();
    noncanonical_set_diff
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("set review")
        .diff
        .swap(0, 1);
    synchronize_set_review_with_editor_preview(&mut noncanonical_set_diff.agents.memory);
    cases.push(("Set noncanonical diff", noncanonical_set_diff));

    let delete_current = entry(&owner, 24_470, "delete auth key");
    let delete = delete_review(&owner, &delete_current, 24_480);
    let mut base_delete = memory_model(&owner);
    install_delete_review_state(
        &mut base_delete.agents.memory,
        &owner,
        &delete_current,
        &delete,
    );
    let deleted_expected = delete_current
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(24_483)),
            Actor::Human,
            24_483,
            None,
            EventId::from_uuid(Uuid::from_u128(24_484)),
        )
        .expect("deleted expected reference");
    let mut deleted_as_present = base_delete.clone();
    deleted_as_present
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("delete review")
        .expected = ExpectedMemoryEntryState::Present(deleted_expected.reference());
    cases.push((
        "Delete Present carrying deleted reference",
        deleted_as_present,
    ));

    let foreign_delete = entry(&foreign, 24_490, "delete auth key");
    let mut foreign_delete_state = base_delete.clone();
    foreign_delete_state
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("delete review")
        .expected = ExpectedMemoryEntryState::Present(foreign_delete.reference());
    cases.push(("Delete foreign namespace reference", foreign_delete_state));

    let mut detached_delete = base_delete.clone();
    detached_delete.agents.memory.entry_detail = None;
    cases.push(("Delete detached from entry detail", detached_delete));

    let mut empty_delete_diff = base_delete.clone();
    empty_delete_diff
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("delete review")
        .diff
        .clear();
    cases.push(("Delete empty diff", empty_delete_diff));

    let mut falsified_delete_diff = base_delete.clone();
    falsified_delete_diff
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("delete review")
        .diff[1]
        .before = MemoryFieldValue::Text("falsified selected value".to_owned());
    cases.push(("Delete falsified diff", falsified_delete_diff));

    let mut noncanonical_delete_diff = base_delete;
    noncanonical_delete_diff
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("delete review")
        .diff
        .swap(0, 1);
    cases.push(("Delete noncanonical diff", noncanonical_delete_diff));

    for (index, (label, malformed)) in cases.into_iter().enumerate() {
        let mut review_layer = malformed.clone();
        review_layer.agents.memory.pane = MemoryPane::MutationReview;
        review_layer.agents.memory.confirmation = None;
        review_layer.agents.memory.detail_scroll = 9;
        assert_invalid_protected_workflow_is_closed(&review_layer, label);
        let before_escape = review_layer.clone();
        assert_eq!(
            handle_event(&mut review_layer, key(KeyCode::Esc)),
            ControllerEffect::CancelMemoryReview,
            "missing explicit review cancellation: {label}",
        );
        assert_eq!(review_layer, before_escape, "review Esc mutated: {label}");

        let mut confirmation_layer = malformed;
        retain_exact_direct_confirmation(&mut confirmation_layer.agents.memory);
        confirmation_layer.agents.memory.detail_scroll = 9;
        assert_invalid_protected_workflow_is_closed(&confirmation_layer, label);
        let before_result = confirmation_layer.agents.memory.clone();
        let review = before_result.edit_review.as_ref().expect("direct review");
        let result = plausible_mutation_result(
            &owner,
            review,
            25_000 + u128::try_from(index).expect("case index") * 10,
        );
        confirmation_layer.agents.memory.pending_intent = Some(MemoryOutcomeIntent::Mutation);
        let before_result = confirmation_layer.agents.memory.clone();
        assert!(
            !confirmation_layer.agents.memory.apply_matching_view(
                &MemoryOutcomeIntent::Mutation,
                CommandView::MemoryEntryMutation(MemoryEntryMutationView {
                    entry: result.reference(),
                    expired_proposals: Vec::new(),
                }),
            ),
            "malformed review accepted plausible mutation result: {label}",
        );
        assert_eq!(
            confirmation_layer.agents.memory, before_result,
            "malformed result mutated state: {label}",
        );
    }
}

#[test]
fn valid_absent_present_deleted_set_and_present_delete_reviews_remain_executable() {
    let owner = profile(25_200);

    let absent = set_review(&owner, 25_210, "absent set");
    let mut absent_model = memory_model(&owner);
    install_create_set_review_state(&mut absent_model.agents.memory, &absent);
    assert_eq!(
        handle_event(&mut absent_model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(absent_model.agents.memory.pane, MemoryPane::Confirmation);

    let present_seed = entry(&owner, 25_220, "present set");
    let present_candidate = MemoryEntryDraft::new(
        "present set".to_owned(),
        "changed present value".to_owned(),
        Vec::new(),
    )
    .expect("present candidate");
    let present = set_review_for(
        &owner,
        25_230,
        ExpectedMemoryEntryState::Present(present_seed.reference()),
        present_candidate.clone(),
        present_set_diff(&present_seed, &present_candidate),
    );
    let mut present_model = memory_model(&owner);
    install_edit_set_review_state(
        &mut present_model.agents.memory,
        &owner,
        &present_seed,
        &present,
    );
    assert_eq!(
        handle_event(&mut present_model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(present_model.agents.memory.pane, MemoryPane::Confirmation);

    let deleted_source = entry(&owner, 25_240, "deleted set");
    let deleted_seed = deleted_source
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(25_243)),
            Actor::Human,
            25_243,
            None,
            EventId::from_uuid(Uuid::from_u128(25_244)),
        )
        .expect("deleted seed");
    let deleted_candidate = MemoryEntryDraft::new(
        "deleted set".to_owned(),
        "recreated value".to_owned(),
        Vec::new(),
    )
    .expect("deleted candidate");
    let deleted = set_review_for(
        &owner,
        25_250,
        ExpectedMemoryEntryState::Deleted(deleted_seed.reference()),
        deleted_candidate.clone(),
        deleted_set_diff(&deleted_candidate),
    );
    let mut deleted_model = memory_model(&owner);
    install_create_set_review_state(&mut deleted_model.agents.memory, &deleted);
    assert_eq!(
        handle_event(&mut deleted_model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(deleted_model.agents.memory.pane, MemoryPane::Confirmation);

    let delete_current = entry(&owner, 25_260, "present delete");
    let delete = delete_review(&owner, &delete_current, 25_270);
    let mut delete_model = memory_model(&owner);
    install_delete_review_state(
        &mut delete_model.agents.memory,
        &owner,
        &delete_current,
        &delete,
    );
    assert_eq!(
        handle_event(&mut delete_model, key(KeyCode::Enter)),
        ControllerEffect::Redraw,
    );
    assert_eq!(delete_model.agents.memory.pane, MemoryPane::Confirmation);
}

#[test]
fn deleted_recreation_prior_display_key_is_bound_through_review_confirmation_and_result() {
    let owner = profile(25_300);
    let source = entry(&owner, 25_310, "Recreated Key");
    let tombstone = source
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(25_313)),
            Actor::Human,
            25_313,
            None,
            EventId::from_uuid(Uuid::from_u128(25_314)),
        )
        .expect("deleted seed");
    let candidate = MemoryEntryDraft::new(
        "recreated key".to_owned(),
        "recreated candidate value".to_owned(),
        vec!["recreated".to_owned()],
    )
    .expect("recreated candidate");

    for (index, (label, prior_key, valid)) in [
        ("canonical same-normalized prior key", "Recreated Key", true),
        ("foreign normalized prior key", "FOREIGN_PRIOR_KEY", false),
        ("unsafe prior key", "UNSAFE_PRIOR\u{202e}_KEY", false),
        ("noncanonical prior key", "recreated  key", false),
    ]
    .into_iter()
    .enumerate()
    {
        let review = set_review_for(
            &owner,
            25_320 + u128::try_from(index).expect("case index"),
            ExpectedMemoryEntryState::Deleted(tombstone.reference()),
            candidate.clone(),
            deleted_set_diff_with_prior_key(&candidate, prior_key),
        );
        let mut model = memory_model(&owner);
        install_create_set_review_state(&mut model.agents.memory, &review);

        if valid {
            assert_eq!(
                handle_event(&mut model, key(KeyCode::Enter)),
                ControllerEffect::Redraw,
                "case={label}",
            );
            assert_eq!(model.agents.memory.pane, MemoryPane::Confirmation);
            assert_eq!(
                handle_event(&mut model, key(KeyCode::Enter)),
                ControllerEffect::ExecuteMemory(set_command(&review)),
                "case={label}",
            );
            let result = plausible_mutation_result(&owner, &review, 25_350);
            model.agents.memory.pending_intent = Some(MemoryOutcomeIntent::Mutation);
            assert!(
                model.agents.memory.apply_matching_view(
                    &MemoryOutcomeIntent::Mutation,
                    CommandView::MemoryEntryMutation(MemoryEntryMutationView {
                        entry: result.reference(),
                        expired_proposals: Vec::new(),
                    }),
                ),
                "case={label}",
            );
            assert!(model.agents.memory.pending_intent.is_none());
            continue;
        }

        assert_invalid_protected_workflow_is_closed(&model, label);
        let mut escaped = model.clone();
        let before_escape = escaped.clone();
        assert_eq!(
            handle_event(&mut escaped, key(KeyCode::Esc)),
            ControllerEffect::CancelMemoryReview,
            "case={label}",
        );
        assert_eq!(escaped, before_escape, "review Esc mutated: {label}");

        retain_exact_direct_confirmation(&mut model.agents.memory);
        assert_invalid_protected_workflow_is_closed(&model, label);
        let result = plausible_mutation_result(
            &owner,
            &review,
            25_360 + u128::try_from(index).expect("case index") * 10,
        );
        model.agents.memory.pending_intent = Some(MemoryOutcomeIntent::Mutation);
        let before_result = model.agents.memory.clone();
        assert!(
            !model.agents.memory.apply_matching_view(
                &MemoryOutcomeIntent::Mutation,
                CommandView::MemoryEntryMutation(MemoryEntryMutationView {
                    entry: result.reference(),
                    expired_proposals: Vec::new(),
                }),
            ),
            "invalid prior key accepted plausible result: {label}",
        );
        assert_eq!(model.agents.memory, before_result, "case={label}");
    }
}

#[test]
fn forged_expected_state_discriminants_close_proposal_detail_review_and_confirmation_actions() {
    let owner = profile(24_500);
    let current = entry(&owner, 24_510, "resolution key");
    let deleted = current
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(24_513)),
            Actor::Human,
            24_514,
            None,
            EventId::from_uuid(Uuid::from_u128(24_514)),
        )
        .expect("deleted current entry");
    let forged_states = [
        (
            ExpectedMemoryEntryState::Present(deleted.reference()),
            "Present carrying a deleted reference",
        ),
        (
            ExpectedMemoryEntryState::Deleted(current.reference()),
            "Deleted carrying a present reference",
        ),
    ];

    for (index, (forged, label)) in forged_states.into_iter().enumerate() {
        let proposal = proposal(
            &owner,
            24_520 + u128::try_from(index).expect("case index") * 10,
            "resolution key",
        );

        let mut detail = memory_model(&owner);
        detail.agents.memory.proposals =
            Some(proposals_view(&owner, std::slice::from_ref(&proposal)));
        let mut forged_detail = proposal_detail(&owner, &proposal);
        forged_detail.current_entry = forged.clone();
        detail.agents.memory.proposal_detail = Some(forged_detail);
        detail.agents.memory.pane = MemoryPane::ProposalDetail;
        detail.agents.memory.detail_scroll = 9;
        assert_memory_local_keys_are_closed(
            &detail,
            &[
                KeyCode::Left,
                KeyCode::Right,
                KeyCode::Enter,
                KeyCode::Up,
                KeyCode::Down,
                KeyCode::PageUp,
                KeyCode::PageDown,
                KeyCode::Home,
                KeyCode::End,
            ],
        );

        let mut review_model = memory_model(&owner);
        let mut review = resolution_review(
            &owner,
            &owner,
            24_550 + u128::try_from(index).expect("case index") * 10,
            MemoryResolutionAction::Approve,
        );
        review.expected_entry = forged.clone();
        review_model.agents.memory.resolution_review = Some(review.clone());
        review_model.agents.memory.review_registered = true;
        review_model.agents.memory.selected_proposal_detail_action =
            MemoryProposalDetailAction::Approve;
        review_model.agents.memory.pane = MemoryPane::ProposalResolutionReview;
        review_model.agents.memory.detail_scroll = 9;
        assert_invalid_protected_workflow_is_closed(&review_model, label);

        let mut confirmation = review_model;
        confirmation.agents.memory.generation = 24_580 + u64::try_from(index).expect("case index");
        confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
            command: resolution_command(&review),
            generation: confirmation.agents.memory.generation,
        });
        confirmation.agents.memory.pane = MemoryPane::Confirmation;
        assert_invalid_protected_workflow_is_closed(&confirmation, label);

        let resolution = MemoryProposalResolution::new(
            review.proposal.reference(),
            MemoryProposalStatus::Accepted,
            review.approval_id,
            Actor::Human,
            24_590 + i64::try_from(index).expect("case index"),
            EventId::from_uuid(Uuid::from_u128(
                24_590 + u128::try_from(index).expect("case index"),
            )),
        )
        .expect("plausible forged-state resolution result");
        confirmation.agents.memory.pending_intent = Some(MemoryOutcomeIntent::Resolution);
        let before = confirmation.agents.memory.clone();
        assert!(
            !confirmation.agents.memory.apply_matching_view(
                &MemoryOutcomeIntent::Resolution,
                CommandView::MemoryProposalResolution(MemoryProposalResolutionView {
                    resolution,
                    entry: None,
                    expired_proposals: Vec::new(),
                }),
            ),
            "forged resolution review accepted result: {label}",
        );
        assert_eq!(confirmation.agents.memory, before, "case={label}");
    }
}

#[test]
fn malformed_memory_pages_close_the_entire_local_keymap_without_mutation() {
    let owner = profile(20_000);
    let other = profile(21_000);
    let selected = entry(&owner, 20_100, "selected entry");
    let foreign = entry(&other, 21_100, "foreign entry");

    let mut entry_cases = Vec::new();
    let mut bad_count = memory_model(&owner);
    bad_count.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&selected),
    ));
    bad_count
        .agents
        .memory
        .entries
        .as_mut()
        .expect("entries")
        .returned_count = 2;
    entry_cases.push(bad_count);

    let mut bad_profile = memory_model(&owner);
    bad_profile.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&selected),
    ));
    bad_profile
        .agents
        .memory
        .entries
        .as_mut()
        .expect("entries")
        .profile = other.reference();
    entry_cases.push(bad_profile);

    let mut bad_namespace = memory_model(&owner);
    bad_namespace.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&selected),
    ));
    bad_namespace
        .agents
        .memory
        .entries
        .as_mut()
        .expect("entries")
        .namespace_id = other.memory_namespace_id();
    entry_cases.push(bad_namespace);

    let mut bad_row = memory_model(&owner);
    bad_row.agents.memory.entries = Some(MemoryEntriesView {
        profile: owner.reference(),
        namespace_id: owner.memory_namespace_id(),
        entries: vec![entry_summary(&foreign)],
        total_count: 1,
        returned_count: 1,
        omitted_count: 0,
    });
    entry_cases.push(bad_row);

    let entry_keys = [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Enter,
        KeyCode::Char('c'),
        KeyCode::Char('p'),
        KeyCode::Char('e'),
    ];
    for mut malformed in entry_cases {
        malformed.agents.memory.entry_scroll = 7;
        malformed.agents.memory.detail_scroll = 8;
        malformed.agents.memory.history_scroll = 9;
        malformed.agents.memory.proposal_scroll = 10;
        malformed.agents.memory.episode_scroll = 11;
        malformed.agents.memory.generation = 12;
        malformed.command.ingest("protected command");
        assert_memory_local_keys_are_closed(&malformed, &entry_keys);
    }

    let mut bad_history_count = memory_model(&owner);
    bad_history_count.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&selected),
    ));
    bad_history_count.agents.memory.pane = MemoryPane::EntryHistory;
    bad_history_count.agents.memory.entry_history = Some(history_view(
        &owner,
        &selected,
        std::slice::from_ref(&selected),
    ));
    bad_history_count
        .agents
        .memory
        .entry_history
        .as_mut()
        .expect("history")
        .total_count = 2;
    assert_memory_local_keys_are_closed(
        &bad_history_count,
        &[
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Enter,
        ],
    );

    let first_proposal = proposal(&owner, 20_200, "selected proposal");
    let mut bad_proposal_count = memory_model(&owner);
    bad_proposal_count.agents.memory.pane = MemoryPane::Proposals;
    bad_proposal_count.agents.memory.proposals = Some(proposals_view(
        &owner,
        std::slice::from_ref(&first_proposal),
    ));
    bad_proposal_count
        .agents
        .memory
        .proposals
        .as_mut()
        .expect("proposals")
        .omitted_count = u64::MAX;
    assert_memory_local_keys_are_closed(
        &bad_proposal_count,
        &[
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Enter,
        ],
    );

    let first_episode = summary(&owner, 20_300, "selected episode");
    let mut bad_episode_row = memory_model(&owner);
    bad_episode_row.agents.memory.pane = MemoryPane::EpisodicSummaries;
    bad_episode_row.agents.memory.episodes =
        Some(summaries_view(&owner, std::slice::from_ref(&first_episode)));
    bad_episode_row
        .agents
        .memory
        .episodes
        .as_mut()
        .expect("episodes")
        .summaries[0] = summaries_view(&other, &[summary(&other, 21_300, "foreign episode")])
        .summaries
        .remove(0);
    assert_memory_local_keys_are_closed(
        &bad_episode_row,
        &[
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Enter,
        ],
    );
}

#[test]
fn stale_memory_details_close_actions_scrolling_and_navigation_except_escape() {
    let owner = profile(22_000);
    let other = profile(23_000);
    let selected = entry(&owner, 22_100, "selected entry");
    let stale = entry(&owner, 22_200, "stale entry");

    let mut entry_detail = memory_model(&owner);
    entry_detail.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&selected),
    ));
    entry_detail.agents.memory.pane = MemoryPane::EntryDetail;
    entry_detail.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: owner.reference(),
        entry: stale.clone(),
    });
    assert_memory_local_keys_are_closed(
        &entry_detail,
        &[
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Enter,
        ],
    );
    let mut escaped = entry_detail.clone();
    assert_eq!(
        handle_event(&mut escaped, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(escaped.agents.memory.pane, MemoryPane::EntryList);

    let mut history = memory_model(&owner);
    history.agents.memory.entries = Some(populated_entries_view(
        &owner,
        std::slice::from_ref(&selected),
    ));
    history.agents.memory.pane = MemoryPane::EntryHistory;
    history.agents.memory.entry_history = Some(history_view(
        &owner,
        &selected,
        std::slice::from_ref(&selected),
    ));
    history
        .agents
        .memory
        .entry_history
        .as_mut()
        .expect("history")
        .profile = other.reference();
    history.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: other.reference(),
        entry: selected.clone(),
    });
    assert_memory_local_keys_are_closed(
        &history,
        &[
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Enter,
        ],
    );
    let mut escaped = history.clone();
    assert_eq!(
        handle_event(&mut escaped, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(escaped.agents.memory.pane, MemoryPane::EntryHistory);
    assert!(escaped.agents.memory.entry_version.is_none());

    let selected_proposal = proposal(&owner, 22_300, "selected proposal");
    let stale_proposal = proposal(&owner, 22_400, "stale proposal");
    let mut proposal_detail_model = memory_model(&owner);
    proposal_detail_model.agents.memory.pane = MemoryPane::ProposalDetail;
    proposal_detail_model.agents.memory.proposals = Some(proposals_view(
        &owner,
        std::slice::from_ref(&selected_proposal),
    ));
    proposal_detail_model.agents.memory.proposal_detail =
        Some(proposal_detail(&owner, &stale_proposal));
    assert_memory_local_keys_are_closed(
        &proposal_detail_model,
        &[
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Enter,
        ],
    );
    let mut escaped = proposal_detail_model.clone();
    assert_eq!(
        handle_event(&mut escaped, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(escaped.agents.memory.pane, MemoryPane::Proposals);

    let selected_episode = summary(&owner, 22_500, "selected episode");
    let stale_episode = summary(&owner, 22_600, "stale episode");
    let mut episode_detail = memory_model(&owner);
    episode_detail.agents.memory.pane = MemoryPane::EpisodicDetail;
    episode_detail.agents.memory.episodes = Some(summaries_view(
        &owner,
        std::slice::from_ref(&selected_episode),
    ));
    episode_detail.agents.memory.episode_detail = Some(EpisodicSummaryView {
        summary: stale_episode,
        qualification: EpisodicQualification::SummaryVerifySources,
    });
    assert_memory_local_keys_are_closed(
        &episode_detail,
        &[
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
        ],
    );
    let mut escaped = episode_detail;
    assert_eq!(
        handle_event(&mut escaped, key(KeyCode::Esc)),
        ControllerEffect::Redraw
    );
    assert_eq!(escaped.agents.memory.pane, MemoryPane::EpisodicSummaries);
}

#[test]
fn same_profile_foreign_namespace_proposal_row_closes_list_and_detail_interaction() {
    let owner = profile(22_700);
    let selected = proposal(&owner, 22_800, "foreign namespace proposal");
    let mut model = memory_model(&owner);
    model.agents.memory.proposals = Some(proposals_view(&owner, std::slice::from_ref(&selected)));
    model.agents.memory.proposal_detail = Some(proposal_detail(&owner, &selected));
    let foreign_namespace = MemoryNamespaceId::from_uuid(Uuid::from_u128(22_899));
    let summaries = &mut model
        .agents
        .memory
        .proposals
        .as_mut()
        .expect("proposal page")
        .proposals;
    summaries[0] = with_proposal_summary_namespace(&summaries[0], foreign_namespace);

    model.agents.memory.pane = MemoryPane::Proposals;
    assert_memory_local_keys_are_closed(
        &model,
        &[
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Enter,
        ],
    );

    model.agents.memory.pane = MemoryPane::ProposalDetail;
    assert_memory_local_keys_are_closed(
        &model,
        &[
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Enter,
        ],
    );
}

fn assert_exact_memory_list_bound_movements(
    model: &mut TuiModel,
    selection_and_scroll: impl Fn(&TuiModel) -> (usize, usize),
) {
    model.set_terminal_size(100, 30);
    for (key_code, expected) in [
        (KeyCode::PageDown, (8, 1)),
        (KeyCode::PageDown, (16, 9)),
        (KeyCode::PageUp, (8, 8)),
        (KeyCode::End, (19, 12)),
        (KeyCode::Home, (0, 0)),
    ] {
        assert_eq!(handle_event(model, key(key_code)), ControllerEffect::Redraw);
        assert_eq!(selection_and_scroll(model), expected, "key={key_code:?}");
    }
}

#[test]
fn all_memory_lists_use_exact_page_home_end_selection_and_offset_movements() {
    let owner = profile(23_000);
    let entry = entry(&owner, 23_100, "movement entry");

    let mut entries = memory_model(&owner);
    entries.agents.memory.entries = Some(populated_entries_view(&owner, &vec![entry.clone(); 20]));
    assert_exact_memory_list_bound_movements(&mut entries, |model| {
        (
            model.agents.memory.selected_entry,
            model.agents.memory.entry_scroll,
        )
    });

    let mut history = memory_model(&owner);
    history.agents.memory.entries =
        Some(populated_entries_view(&owner, std::slice::from_ref(&entry)));
    history.agents.memory.entry_history =
        Some(history_view(&owner, &entry, &vec![entry.clone(); 20]));
    history.agents.memory.pane = MemoryPane::EntryHistory;
    assert_exact_memory_list_bound_movements(&mut history, |model| {
        (
            model.agents.memory.selected_history_version,
            model.agents.memory.history_scroll,
        )
    });

    let proposal = proposal(&owner, 23_200, "movement proposal");
    let mut proposals = memory_model(&owner);
    proposals.agents.memory.proposals = Some(proposals_view(&owner, &vec![proposal; 20]));
    proposals.agents.memory.pane = MemoryPane::Proposals;
    assert_exact_memory_list_bound_movements(&mut proposals, |model| {
        (
            model.agents.memory.selected_proposal,
            model.agents.memory.proposal_scroll,
        )
    });

    let episode = summary(&owner, 23_300, "movement episode");
    let mut episodes = memory_model(&owner);
    episodes.agents.memory.episodes = Some(summaries_view(&owner, &vec![episode; 20]));
    episodes.agents.memory.pane = MemoryPane::EpisodicSummaries;
    assert_exact_memory_list_bound_movements(&mut episodes, |model| {
        (
            model.agents.memory.selected_episode,
            model.agents.memory.episode_scroll,
        )
    });
}
