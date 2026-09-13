use ai_stock_forum::{
    agents::{AgentProfileVersion, AgentReadiness, builtin_profile_templates},
    app::{
        AgentProfileSelector, AgentProfileSummary, AgentProfileView, AgentProfilesView,
        ApplicationCommand, DatabaseReadiness, EpisodicSummariesView, EpisodicSummaryListItem,
        EpisodicSummaryView, MemoryEditPreview, MemoryEntriesView, MemoryEntryHistorySummary,
        MemoryEntryHistoryView, MemoryEntrySummary, MemoryEntryVersionView, MemoryEntryView,
        MemoryProfileIdentityView, MemoryProposalResolutionReview, MemoryProposalSummary,
        MemoryProposalView, MemoryProposalsView, PresentationSnapshot, ProcessGuardOwnership,
    },
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, ApprovalId, EpisodicSummaryId, EventId,
        InstallationId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId, MemoryProposalId,
        MemoryReviewToken, SessionId, sha256,
    },
    memory::{
        EpisodicQualification, EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState,
        MemoryEditReview, MemoryEntryDraft, MemoryEntryRef, MemoryEntryState, MemoryEntryVersion,
        MemoryField, MemoryFieldDiff, MemoryFieldValue, MemoryMutationKind, MemoryNoChange,
        MemoryPlaintextAcknowledgement, MemoryProposal, MemoryProposalFilter,
        MemoryProposalOperation, MemoryProposalOperationKind, MemoryProposalStatus,
        MemoryResolutionAction,
    },
    policy::ApprovalStatus,
    setup::SetupStatus,
    ui::memory_editor::{MEMORY_PLAINTEXT_WARNING, MemoryEditor},
    ui::tui::{
        TuiEvent, handle_event,
        layout::{
            calculate_with_input, memory_layout_mode, memory_workspace, view_geometry_for_state,
        },
        model::{
            AgentDetailAction, AgentsPane, Focus, MemoryConfirmation, MemoryPane,
            MemoryProposalDetailAction, MemoryResultOrigin, TuiModel, View,
        },
        render,
        theme::Theme,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Terminal,
    backend::TestBackend,
    layout::{Position, Rect},
};
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

fn profile() -> AgentProfileVersion {
    let template = &builtin_profile_templates()[0];
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(10)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(11)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(12)),
        1_800_000_000_000,
        template.copy_to_draft().expect("profile draft"),
        Some(template.provenance()),
    )
    .expect("profile")
}

fn successor_profile(profile: &AgentProfileVersion, seed: u128) -> AgentProfileVersion {
    let mut draft = profile.to_draft();
    draft.description = format!("render successor {seed}");
    AgentProfileVersion::next_version(
        profile,
        AgentProfileVersionId::from_uuid(Uuid::from_u128(seed)),
        1_800_000_000_001,
        draft,
    )
    .expect("profile successor")
}

fn model_with_exact_profile(profile: AgentProfileVersion) -> TuiModel {
    let mut model = model();
    model.agents.profiles = AgentProfilesView {
        profiles: vec![AgentProfileSummary {
            profile_id: profile.profile_id(),
            profile_version_id: profile.profile_version_id(),
            version: profile.version(),
            display_name: profile.display_name().to_owned(),
            role: profile.role(),
            primary_specialty: profile.primary_specialty().to_owned(),
            readiness: AgentReadiness::Unbound,
            content_digest: profile.content_digest().clone(),
        }],
        total_count: 1,
        returned_count: 1,
        truncated: false,
    };
    model.agents.detail = Some(AgentProfileView {
        profile,
        readiness: AgentReadiness::Unbound,
    });
    model
}

fn model_with_profile() -> TuiModel {
    model_with_exact_profile(profile())
}

fn memory_model(width: u16, height: u16) -> TuiModel {
    let mut model = model();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    model.set_terminal_size(width, height);
    model
}

fn bind_memory(model: &mut TuiModel) {
    let detail = model.agents.detail.as_ref().expect("profile detail");
    model
        .agents
        .memory
        .bind_profile(
            MemoryProfileIdentityView {
                profile: detail.profile.reference(),
                display_name: detail.profile.display_name().to_owned(),
            },
            detail.profile.memory_namespace_id(),
        )
        .expect("bind memory profile");
}

fn memory_entry(model: &TuiModel, seed: u128, key: &str) -> MemoryEntryVersion {
    let namespace_id = model
        .agents
        .detail
        .as_ref()
        .expect("profile detail")
        .profile
        .memory_namespace_id();
    memory_entry_in_namespace(namespace_id, seed, key)
}

fn memory_entry_in_namespace(
    namespace_id: MemoryNamespaceId,
    seed: u128,
    key: &str,
) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        namespace_id,
        MemoryEntryId::from_uuid(Uuid::from_u128(seed)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(seed + 1)),
        MemoryEntryDraft::new(
            key.to_owned(),
            format!("value {seed}"),
            vec!["risk".to_owned()],
        )
        .expect("memory draft"),
        Actor::Human,
        1_800_000_000_000 + i64::try_from(seed).expect("timestamp"),
        None,
        EventId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .expect("memory entry")
}

fn install_entries(model: &mut TuiModel, entries: Vec<MemoryEntrySummary>) {
    let detail = model.agents.detail.as_ref().expect("profile detail");
    model.agents.memory.entries = Some(MemoryEntriesView {
        profile: detail.profile.reference(),
        namespace_id: detail.profile.memory_namespace_id(),
        total_count: u64::try_from(entries.len()).expect("fixture count"),
        returned_count: u64::try_from(entries.len()).expect("fixture count"),
        omitted_count: 0,
        entries,
    });
}

fn entry_summary(
    entry: &MemoryEntryVersion,
    display_key: String,
    tags: Vec<String>,
) -> MemoryEntrySummary {
    MemoryEntrySummary {
        entry: entry.reference(),
        display_key,
        purpose_tags: tags,
        value_bytes: u64::try_from(entry.value().expect("present value").len())
            .expect("value bytes"),
        created_at_ms: entry.created_at_ms(),
    }
}

fn successor_entry(entry: &MemoryEntryVersion, seed: u128, value: &str) -> MemoryEntryVersion {
    entry
        .next_present(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(seed)),
            MemoryEntryDraft::new(
                entry.display_key().to_owned(),
                value.to_owned(),
                vec!["quality".to_owned()],
            )
            .expect("successor draft"),
            Actor::Human,
            1_800_000_000_000 + i64::try_from(seed).expect("timestamp"),
            None,
            EventId::from_uuid(Uuid::from_u128(seed + 1)),
        )
        .expect("successor entry")
}

fn install_entry_detail(model: &mut TuiModel, entry: MemoryEntryVersion) {
    let profile = model
        .agents
        .detail
        .as_ref()
        .expect("profile detail")
        .profile
        .reference();
    model.agents.memory.entry_detail = Some(MemoryEntryView { profile, entry });
}

fn install_history(
    model: &mut TuiModel,
    current: &MemoryEntryVersion,
    versions: &[MemoryEntryVersion],
) {
    let profile = model
        .agents
        .detail
        .as_ref()
        .expect("profile detail")
        .profile
        .reference();
    model.agents.memory.entry_history = Some(MemoryEntryHistoryView {
        profile,
        current: current.reference(),
        versions: versions
            .iter()
            .map(|version| MemoryEntryHistorySummary {
                entry: version.reference(),
                display_key: version.display_key().to_owned(),
                created_at_ms: version.created_at_ms(),
                accepted_proposal: version.accepted_proposal().cloned(),
            })
            .collect(),
        total_count: u64::try_from(versions.len()).expect("fixture count"),
        returned_count: u64::try_from(versions.len()).expect("fixture count"),
        omitted_count: 0,
    });
}

fn install_set_review(model: &mut TuiModel, key: &str) {
    let detail = model.agents.detail.as_ref().expect("profile detail");
    let review = MemoryEditReview {
        profile: detail.profile.reference(),
        namespace_id: detail.profile.memory_namespace_id(),
        expected: ExpectedMemoryEntryState::Absent,
        operation: MemoryMutationKind::Set,
        candidate: Some(
            MemoryEntryDraft::new(key.to_owned(), "review value".to_owned(), Vec::new())
                .expect("review candidate"),
        ),
        diff: vec![
            MemoryFieldDiff {
                field: MemoryField::DisplayKey,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::Text(key.to_owned()),
            },
            MemoryFieldDiff {
                field: MemoryField::State,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::State(MemoryEntryState::Present),
            },
            MemoryFieldDiff {
                field: MemoryField::Value,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::Text("review value".to_owned()),
            },
            MemoryFieldDiff {
                field: MemoryField::PurposeTags,
                before: MemoryFieldValue::Missing,
                after: MemoryFieldValue::Tags(Vec::new()),
            },
        ],
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(9_001)),
        review_digest: sha256(b"render review"),
    };
    let mut editor =
        MemoryEditor::for_create(AgentProfileSelector::Id(detail.profile.profile_id()));
    editor.submit_line(key.to_owned()).expect("review key");
    editor
        .submit_line("review value".to_owned())
        .expect("review value");
    editor.submit_line(String::new()).expect("review tags");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    ));
    model.agents.memory.editor = Some(editor);
    model.agents.memory.edit_review = Some(review);
    model.agents.memory.review_registered = true;
    model.agents.memory.pane = MemoryPane::MutationReview;
}

fn set_review_command(review: &MemoryEditReview) -> ApplicationCommand {
    ApplicationCommand::SetMemoryEntry {
        profile: review.profile.clone(),
        expected: review.expected.clone(),
        candidate: review.candidate.clone().expect("set candidate"),
        review_token: review.review_token,
        review_digest: review.review_digest.clone(),
    }
}

fn install_delete_review(model: &mut TuiModel, current: &MemoryEntryVersion) {
    let detail = model.agents.detail.as_ref().expect("profile detail");
    model.agents.memory.entries = Some(MemoryEntriesView {
        profile: detail.profile.reference(),
        namespace_id: detail.profile.memory_namespace_id(),
        entries: vec![entry_summary(
            current,
            current.display_key().to_owned(),
            current.purpose_tags().to_vec(),
        )],
        total_count: 1,
        returned_count: 1,
        omitted_count: 0,
    });
    model.agents.memory.selected_entry = 0;
    model.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: detail.profile.reference(),
        entry: current.clone(),
    });
    model.agents.memory.editor = None;
    model.agents.memory.edit_review = Some(MemoryEditReview {
        profile: detail.profile.reference(),
        namespace_id: detail.profile.memory_namespace_id(),
        expected: ExpectedMemoryEntryState::Present(current.reference()),
        operation: MemoryMutationKind::Delete,
        candidate: None,
        diff: vec![
            MemoryFieldDiff {
                field: MemoryField::State,
                before: MemoryFieldValue::State(MemoryEntryState::Present),
                after: MemoryFieldValue::State(MemoryEntryState::Deleted),
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
        ],
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(9_101)),
        review_digest: sha256(b"delete render review"),
    });
    model.agents.memory.review_registered = true;
    model.agents.memory.pane = MemoryPane::MutationReview;
}

fn deleted_set_diff_with_prior_key(
    candidate: &MemoryEntryDraft,
    prior_key: &str,
) -> Vec<MemoryFieldDiff> {
    vec![
        MemoryFieldDiff {
            field: MemoryField::DisplayKey,
            before: MemoryFieldValue::Text(prior_key.to_owned()),
            after: MemoryFieldValue::Text(candidate.display_key().to_owned()),
        },
        MemoryFieldDiff {
            field: MemoryField::State,
            before: MemoryFieldValue::State(MemoryEntryState::Deleted),
            after: MemoryFieldValue::State(MemoryEntryState::Present),
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

fn install_create_set_review(model: &mut TuiModel, review: &MemoryEditReview) {
    let candidate = review.candidate.as_ref().expect("set candidate");
    let profile_id = review.profile.profile_id();
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(profile_id))
        .expect("create editor");
    let editor = model.agents.memory.editor.as_mut().expect("editor");
    editor
        .submit_line(candidate.display_key().to_owned())
        .expect("candidate key");
    editor
        .submit_line(candidate.value().to_owned())
        .expect("candidate value");
    let effect = editor
        .submit_line(candidate.purpose_tags().join(","))
        .expect("candidate tags");
    assert!(matches!(
        effect,
        ai_stock_forum::ui::memory_editor::MemoryEditorEffect::Preview(_)
    ));
    model
        .agents
        .memory
        .begin_review_request()
        .expect("review generation");
    let editor = model.agents.memory.editor.as_mut().expect("editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    ));
    model.agents.memory.edit_review = Some(review.clone());
    model.agents.memory.review_registered = true;
    model.agents.memory.pane = MemoryPane::MutationReview;
}

fn synchronize_set_review_with_editor_preview(model: &mut TuiModel) {
    let review = model.agents.memory.edit_review.clone().expect("set review");
    let editor = model.agents.memory.editor.as_mut().expect("review editor");
    assert!(editor.apply_preview(
        editor.generation(),
        MemoryEditPreview::Review(review.clone()),
    ));
    assert!(matches!(
        editor.preview(),
        Some(MemoryEditPreview::Review(retained)) if retained == &review
    ));
}

fn delete_review_command(review: &MemoryEditReview) -> ApplicationCommand {
    let ExpectedMemoryEntryState::Present(expected) = &review.expected else {
        panic!("delete fixture expected present entry");
    };
    ApplicationCommand::DeleteMemoryEntry {
        profile: review.profile.clone(),
        expected: expected.clone(),
        review_token: review.review_token,
        review_digest: review.review_digest.clone(),
    }
}

fn install_editor(model: &mut TuiModel, completed_fields: usize, no_change: bool) {
    let profile_id = model
        .agents
        .detail
        .as_ref()
        .expect("profile detail")
        .profile
        .profile_id();
    let mut editor = MemoryEditor::for_create(AgentProfileSelector::Id(profile_id));
    if completed_fields >= 1 {
        editor
            .submit_keyboard_line("editor key")
            .expect("submit key");
    }
    if completed_fields >= 2 {
        editor
            .submit_line("editor value".to_owned())
            .expect("submit value");
    }
    if completed_fields >= 3 {
        editor
            .submit_keyboard_line("risk, quality")
            .expect("submit tags");
    }
    if no_change {
        assert!(editor.apply_preview(
            editor.generation(),
            MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent),
        ));
    }
    model.agents.memory.editor = Some(editor);
    model.agents.memory.pane = MemoryPane::Editor;
}

fn assert_minimum_warning_surface(model: &TuiModel, operation: &str) {
    let wide = render_text(model, 300, 40);
    assert_eq!(wide.matches(MEMORY_PLAINTEXT_WARNING).count(), 1);
    let narrow = render_rows(model, 60, 18).join(" ");
    let profile_id = model
        .agents
        .memory
        .profile
        .as_ref()
        .expect("bound memory profile")
        .profile
        .profile_id()
        .to_string();
    for required in [
        "Agents / Memory",
        profile_id.as_str(),
        operation,
        "Plaintext local memory",
        "do not store credentials",
        "is retained after overwrite or delete",
        "Enter",
        "Esc",
    ] {
        assert!(
            narrow.contains(required),
            "missing {required:?} in frame:\n{narrow}"
        );
    }
}

fn assert_maximum_key_fixed_surface(
    model: &TuiModel,
    operation: &str,
    _identity_label: &str,
    identity: &str,
) {
    let rows = render_rows(model, 60, 18);
    let text = rows
        .iter()
        .map(|row| row.trim())
        .collect::<Vec<_>>()
        .join(" ");
    let profile_id = model
        .agents
        .memory
        .profile
        .as_ref()
        .expect("bound memory profile")
        .profile
        .profile_id()
        .to_string();
    for required in [profile_id.as_str(), operation, identity, "Enter:", "Esc:"] {
        assert!(
            text.contains(required),
            "missing {required:?} in minimum frame:\n{text}"
        );
    }

    let key_row = rows
        .iter()
        .find(|row| row.contains("kkkkkkkk"))
        .expect("fixed key row");
    assert!(key_row.contains("kkkkkkkk"), "key fragment: {key_row:?}");
    assert!(key_row.contains("..."), "bounded key row: {key_row:?}");

    let mut warning_offset = 0;
    for fragment in [
        "Plaintext local memory",
        "do not store credentials",
        "history",
        "is retained after",
        "overwrite or delete",
    ] {
        let relative = text[warning_offset..]
            .find(fragment)
            .unwrap_or_else(|| panic!("missing ordered warning fragment {fragment:?}:\n{text}"));
        warning_offset = warning_offset.saturating_add(relative + fragment.len());
    }
}

fn assert_fixed_safety_surface_at_every_supported_pane_count(
    model: &TuiModel,
    operation: &str,
    stable_identity: &str,
    key_fragment: &str,
) {
    let profile_id = model
        .agents
        .memory
        .profile
        .as_ref()
        .expect("bound memory profile")
        .profile
        .profile_id()
        .to_string();
    for width in [60, 100, 140] {
        let panel = render_memory_detail_text(model, width, 18);
        let normalized = normalized_panel_text(&panel);
        for required in [
            profile_id.as_str(),
            operation,
            stable_identity,
            key_fragment,
            "Enter",
            "Esc",
        ] {
            assert!(
                normalized.contains(required),
                "width={width} missing {required:?} in active panel:\n{panel}",
            );
        }
        assert_eq!(
            normalized.matches("Plaintext local memory").count(),
            1,
            "width={width} warning count:\n{panel}",
        );
        let mut offset = 0;
        for fragment in [
            "Plaintext local memory",
            "do not store",
            "credentials",
            "history",
            "is retained after",
            "overwrite or delete",
            "Enter",
            "Esc",
        ] {
            let relative = normalized[offset..]
                .find(fragment)
                .unwrap_or_else(|| panic!("width={width} missing ordered {fragment:?}:\n{panel}"));
            offset = offset.saturating_add(relative + fragment.len());
        }
    }
}

fn proposal(model: &TuiModel, seed: u128, key: &str) -> MemoryProposal {
    let profile = &model
        .agents
        .detail
        .as_ref()
        .expect("profile detail")
        .profile;
    MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(seed)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                key.to_owned(),
                format!("proposal value {seed}"),
                vec!["proposal".to_owned()],
            )
            .expect("proposal candidate"),
        },
        key.to_owned(),
        ExpectedMemoryEntryState::Absent,
        format!("proposal rationale {seed}"),
        1_800_000_000_000 + i64::try_from(seed).expect("timestamp"),
        EventId::from_uuid(Uuid::from_u128(seed + 1)),
        ApprovalId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .expect("proposal")
}

fn proposal_with(
    proposer: &AgentProfileVersion,
    seed: u128,
    operation: MemoryProposalOperation,
    key: &str,
    expected: ExpectedMemoryEntryState,
    rationale: &str,
) -> MemoryProposal {
    MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(seed)),
        proposer,
        &Actor::Agent(proposer.profile_id()),
        operation,
        key.to_owned(),
        expected,
        rationale.to_owned(),
        1_800_000_000_000 + i64::try_from(seed).expect("timestamp"),
        EventId::from_uuid(Uuid::from_u128(seed + 1)),
        ApprovalId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .expect("custom proposal")
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

fn install_proposals(model: &mut TuiModel, proposals: &[MemoryProposal]) {
    let profile = &model
        .agents
        .detail
        .as_ref()
        .expect("profile detail")
        .profile;
    model.agents.memory.proposals = Some(MemoryProposalsView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        filter: MemoryProposalFilter::Pending,
        proposals: proposals.iter().map(proposal_summary).collect(),
        total_count: u64::try_from(proposals.len()).expect("fixture count"),
        returned_count: u64::try_from(proposals.len()).expect("fixture count"),
        omitted_count: 0,
    });
}

fn install_proposal_detail(model: &mut TuiModel, proposal: MemoryProposal) {
    let identity = model
        .agents
        .memory
        .profile
        .clone()
        .expect("memory identity");
    model.agents.memory.proposal_detail = Some(MemoryProposalView {
        proposal,
        status: MemoryProposalStatus::Pending,
        resolution: None,
        current_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: false,
        proposer_identity: identity.clone(),
        namespace_owner_identity: identity,
    });
}

fn install_resolution_review(
    model: &mut TuiModel,
    proposal: MemoryProposal,
    action: MemoryResolutionAction,
) {
    let identity = model
        .agents
        .memory
        .profile
        .clone()
        .expect("memory identity");
    model.agents.memory.selected_proposal_detail_action = match action {
        MemoryResolutionAction::Approve => MemoryProposalDetailAction::Approve,
        MemoryResolutionAction::Reject => MemoryProposalDetailAction::Reject,
    };
    model.agents.memory.resolution_review = Some(MemoryProposalResolutionReview {
        action,
        approval_id: proposal.approval_id(),
        proposal,
        expected_approval_status: ApprovalStatus::Pending,
        expected_entry: ExpectedMemoryEntryState::Absent,
        proposer_is_historical: false,
        proposer_identity: identity.clone(),
        namespace_owner_identity: identity,
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(9_201)),
        review_digest: sha256(b"resolution render review"),
    });
    model.agents.memory.review_registered = true;
    model.agents.memory.pane = MemoryPane::ProposalResolutionReview;
}

fn install_exact_resolution_review(
    model: &mut TuiModel,
    proposal: MemoryProposal,
    action: MemoryResolutionAction,
    expected_entry: ExpectedMemoryEntryState,
    identities: (MemoryProfileIdentityView, MemoryProfileIdentityView),
    proposer_is_historical: bool,
    seed: u128,
) {
    let (proposer_identity, namespace_owner_identity) = identities;
    model.agents.memory.selected_proposal_detail_action = match action {
        MemoryResolutionAction::Approve => MemoryProposalDetailAction::Approve,
        MemoryResolutionAction::Reject => MemoryProposalDetailAction::Reject,
    };
    model.agents.memory.resolution_review = Some(MemoryProposalResolutionReview {
        action,
        approval_id: proposal.approval_id(),
        proposal,
        expected_approval_status: ApprovalStatus::Pending,
        expected_entry,
        proposer_is_historical,
        proposer_identity,
        namespace_owner_identity,
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(seed)),
        review_digest: sha256(format!("exact resolution review {seed}").as_bytes()),
    });
    model.agents.memory.review_registered = true;
    model.agents.memory.pane = MemoryPane::ProposalResolutionReview;
}

fn resolution_review_command(review: &MemoryProposalResolutionReview) -> ApplicationCommand {
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

fn episodic_summary(model: &TuiModel, seed: u128, label: &str) -> EpisodicSummary {
    let profile = &model
        .agents
        .detail
        .as_ref()
        .expect("profile detail")
        .profile;
    let source = EpisodicSourceRef::new(
        u64::try_from(seed).expect("sequence"),
        EventId::from_uuid(Uuid::from_u128(seed + 1)),
        format!("thesis_reviewed_{seed}"),
        sha256(format!("episodic source {seed}").as_bytes()),
    )
    .expect("episodic source");
    EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::from_u128(seed)),
        profile,
        label.to_owned(),
        format!("episodic body {seed}"),
        vec!["reflection".to_owned()],
        vec![source],
        1_800_000_000_000 + i64::try_from(seed).expect("timestamp"),
        u64::try_from(seed + 1).expect("creation sequence"),
        EventId::from_uuid(Uuid::from_u128(seed + 2)),
    )
    .expect("episodic summary")
}

fn episodic_list_item(summary: &EpisodicSummary, label: String) -> EpisodicSummaryListItem {
    EpisodicSummaryListItem {
        summary: summary.reference(),
        label,
        purpose_tags: summary.purpose_tags().to_vec(),
        source_count: u64::try_from(summary.sources().len()).expect("source count"),
        created_at_ms: summary.created_at_ms(),
    }
}

fn install_episodes(model: &mut TuiModel, summaries: Vec<EpisodicSummaryListItem>) {
    let profile = &model
        .agents
        .detail
        .as_ref()
        .expect("profile detail")
        .profile;
    model.agents.memory.episodes = Some(EpisodicSummariesView {
        profile: profile.reference(),
        namespace_id: profile.memory_namespace_id(),
        total_count: u64::try_from(summaries.len()).expect("fixture count"),
        returned_count: u64::try_from(summaries.len()).expect("fixture count"),
        omitted_count: 0,
        summaries,
    });
}

fn install_episode_detail(model: &mut TuiModel, summary: EpisodicSummary) {
    model.agents.memory.episode_detail = Some(EpisodicSummaryView {
        summary,
        qualification: EpisodicQualification::SummaryVerifySources,
    });
}

fn render_rows(model: &TuiModel, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .expect("memory render remains total");
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}

fn render_text(model: &TuiModel, width: u16, height: u16) -> String {
    render_rows(model, width, height).join("\n")
}

fn normalized_panel_text(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.trim_matches(|character| matches!(character, '│' | '┌' | '┐' | '└' | '┘' | '─'))
                .trim()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn render_memory_detail_text(model: &TuiModel, width: u16, height: u16) -> String {
    let input_active = model.agents.memory.pane == MemoryPane::Editor
        && model.agents.memory.editor.as_ref().is_some_and(|editor| {
            matches!(
                editor.step(),
                ai_stock_forum::ui::memory_editor::MemoryEditorStep::Key
                    | ai_stock_forum::ui::memory_editor::MemoryEditorStep::Value
                    | ai_stock_forum::ui::memory_editor::MemoryEditorStep::PurposeTags
            )
        });
    let cockpit = calculate_with_input(
        Rect::new(0, 0, width, height),
        model.inspector_open,
        input_active,
    );
    let nested = memory_workspace(cockpit.workspace, memory_layout_mode(cockpit.workspace));
    let area = nested.detail.unwrap_or(nested.primary);
    render_rows(model, width, height)
        .into_iter()
        .skip(usize::from(area.y))
        .take(usize::from(area.height))
        .map(|row| {
            row.chars()
                .skip(usize::from(area.x))
                .take(usize::from(area.width))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_cursor(model: &TuiModel, width: u16, height: u16) -> Position {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .expect("memory render remains total");
    terminal
        .get_cursor_position()
        .expect("active Memory input exposes its cursor")
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn assert_rendered_entry_reference(text: &str, label: &str, reference: &MemoryEntryRef) {
    for expected in [
        format!("{label} entry ID {}", reference.entry_id()),
        format!("{label} version ID {}", reference.entry_version_id()),
        format!("{label} version {}", reference.version().get()),
        format!("{label} digest {}", reference.content_digest()),
    ] {
        assert!(text.contains(&expected), "missing {expected:?}:\n{text}");
    }
}

#[test]
fn direct_memory_layout_boundaries_use_the_actual_workspace_width() {
    let cases = [
        (59, 1, false, false),
        (60, 1, false, false),
        (79, 1, false, false),
        (80, 1, false, false),
        (99, 1, false, false),
        (100, 2, true, false),
        (119, 2, true, false),
        (120, 2, true, false),
    ];

    for (width, pane_count, has_detail, has_context) in cases {
        let area = Rect::new(7, 11, width, 10);
        let layout = memory_workspace(area, memory_layout_mode(area));
        assert_eq!(
            usize::from(layout.detail.is_some()) + usize::from(layout.context.is_some()) + 1,
            pane_count
        );
        assert_eq!(layout.detail.is_some(), has_detail, "width={width}");
        assert_eq!(layout.context.is_some(), has_context, "width={width}");
        assert_eq!(layout.primary.x, area.x);
        assert_eq!(layout.primary.y, area.y);
        assert_eq!(layout.primary.height, area.height);
        let right = layout
            .context
            .or(layout.detail)
            .unwrap_or(layout.primary)
            .right();
        assert_eq!(right, area.right(), "width={width}");
    }
}

#[test]
fn memory_cockpit_returns_the_generic_inspector_width_without_a_boundary_shrink() {
    for (width, height, expected_workspace, expected_navigation) in [
        (59, 18, 59, false),
        (60, 17, 60, false),
        (60, 18, 60, false),
        (119, 30, 119, false),
        (120, 30, 120, false),
        (140, 30, 140, false),
    ] {
        let geometry =
            view_geometry_for_state(Rect::new(0, 0, width, height), View::Agents, true, true);
        assert_eq!(geometry.cockpit.inspector, None, "{width}x{height}");
        assert_eq!(
            geometry.cockpit.workspace.width, expected_workspace,
            "{width}x{height}"
        );
        assert_eq!(
            geometry.cockpit.navigation.is_some(),
            expected_navigation,
            "{width}x{height}"
        );
    }
}

#[test]
fn rendered_memory_keeps_the_inspector_latent_and_minimum_frame_legible() {
    let mut wide = memory_model(140, 30);
    wide.inspector_open = true;
    wide.focus = Focus::Inspector;
    wide.synchronize_geometry();
    assert_eq!(wide.focus, Focus::Workspace);
    assert!(wide.inspector_open);

    let wide_text = render_text(&wide, 140, 30);
    assert!(wide_text.contains("3 Agents"));
    assert!(!wide_text.contains(" Inspector "));

    let minimum = memory_model(60, 18);
    let rows = render_rows(&minimum, 60, 18);
    assert_eq!(rows[0].trim_end(), "AI STOCK FORUM  /  Agents  /  Narrow");
    let minimum_text = rows.join("\n");
    for label in [
        "1 Home",
        "2 Chat",
        "3 Agents",
        "4 Skills",
        "5 Connections",
        "6 Activity",
        "7 Setup",
        "8 Audit",
        "9 Help",
    ] {
        assert!(minimum_text.contains(label), "missing {label:?}");
    }
    assert!(!rows.join("\n").contains("Terminal too small"));

    let too_short = render_text(&memory_model(60, 17), 60, 17);
    assert!(too_short.contains("Terminal too small"));
}

#[test]
fn memory_focus_omits_inspector_and_i_is_a_state_neutral_redraw() {
    let mut model = memory_model(140, 30);
    model.inspector_open = true;
    model.focus = Focus::Navigation;

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Tab)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::List);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Tab)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::BackTab)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::List);

    let saved_open = model.inspector_open;
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('i'))),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert_eq!(model.focus, Focus::List);
    assert_eq!(model.inspector_open, saved_open);
}

#[test]
fn entering_and_leaving_memory_synchronizes_the_shared_cockpit_geometry() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;
    model.agents.selected_detail_action = AgentDetailAction::Memory;
    model.inspector_open = true;
    model.set_terminal_size(140, 30);
    assert_eq!(model.workspace_body_width, 138);

    let effect = handle_event(&mut model, key(KeyCode::Enter));
    assert!(matches!(
        effect,
        ai_stock_forum::ui::tui::ControllerEffect::LoadAgentMemory(_)
    ));
    assert_eq!(model.agents.pane, AgentsPane::Memory);
    assert_eq!(model.workspace_body_width, 138);
    assert!(model.inspector_open);

    let effect = handle_event(&mut model, key(KeyCode::Esc));
    assert_eq!(effect, ai_stock_forum::ui::tui::ControllerEffect::Redraw);
    assert_eq!(model.agents.pane, AgentsPane::Detail);
    assert_eq!(model.workspace_body_width, 138);
    assert!(model.inspector_open);

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('i'))),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert!(model.inspector_open);
    assert_eq!(model.focus, Focus::Inspector);
    assert_eq!(model.workspace_body_width, 138);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('i'))),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert!(!model.inspector_open);
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.workspace_body_width, 138);
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Char('i'))),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert!(model.inspector_open);
    assert_eq!(model.focus, Focus::Inspector);
    assert_eq!(model.workspace_body_width, 138);
    assert!(render_text(&model, 140, 30).contains(" Inspector "));
}

#[test]
fn loaded_agent_detail_prepends_the_nested_memory_action_selector() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Detail;

    let text = render_text(&model, 160, 40);
    for label in ["Profile", "Memory", "Skills", "History"] {
        assert!(text.contains(label));
    }
    assert!(text.contains("A/D choose   Enter open"));

    model.agents.selected_detail_action = AgentDetailAction::Memory;
    let selected_text = render_text(&model, 160, 40);
    assert!(
        selected_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains("Profile Memory Skills History")
    );
    assert_eq!(selected_text.matches("A/D choose   Enter open").count(), 1);
}

#[test]
fn agents_memory_delegates_to_the_nested_workspace_without_a_global_view() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);

    let text = render_text(&model, 140, 30);
    assert!(text.contains("Agents / Memory"));
    assert!(text.contains("Memory entries"));
    assert!(!text.contains("Memory view routing is not available yet"));
    assert!(!text.contains("7 Memory"));
}

#[test]
fn memory_input_title_is_owned_only_by_active_text_entry_stages() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(
            model
                .agents
                .detail
                .as_ref()
                .expect("detail")
                .profile
                .profile_id(),
        ))
        .expect("create editor");

    let active = render_text(&model, 100, 30);
    assert!(active.contains(" Memory input "));
    assert!(!active.contains(" Profile input "));

    model.agents.memory.pane = MemoryPane::EntryList;
    let inactive_pane = render_text(&model, 100, 30);
    assert!(!inactive_pane.contains(" Command "));
    assert!(!inactive_pane.contains(" Memory input "));

    model.agents.memory.pane = MemoryPane::Editor;
    model.active_view = View::Help;
    let hidden_tab = render_text(&model, 100, 30);
    assert!(!hidden_tab.contains(" Command "));
    assert!(!hidden_tab.contains(" Memory input "));

    model.active_view = View::Agents;
    model.skills.active = true;
    let overlay = render_text(&model, 100, 30);
    assert!(!overlay.contains(" Command "));
    assert!(!overlay.contains(" Memory input "));
}

#[test]
fn invalid_memory_editors_neither_claim_the_command_bar_nor_render_their_draft() {
    for (selector, remove_namespace, label) in [
        (
            AgentProfileSelector::Id(AgentProfileId::from_uuid(Uuid::from_u128(77_001))),
            false,
            "mismatched ID selector",
        ),
        (
            AgentProfileSelector::Name("unrelated profile".to_owned()),
            false,
            "mismatched normalized-name selector",
        ),
        ({
            let model = model_with_profile();
            let profile_id = model
                .agents
                .detail
                .as_ref()
                .expect("detail")
                .profile
                .profile_id();
            (
                AgentProfileSelector::Id(profile_id),
                true,
                "missing namespace context",
            )
        }),
    ] {
        let mut model = model_with_profile();
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Memory;
        bind_memory(&mut model);
        model
            .agents
            .memory
            .open_create_editor(selector)
            .expect("open malformed editor fixture");
        if remove_namespace {
            model.agents.memory.namespace_id = None;
        }
        model.command.ingest("INVALID MEMORY EDITOR DRAFT");

        let text = render_text(&model, 100, 30);
        assert!(!text.contains(" Command "), "case={label}\n{text}");
        assert!(!text.contains(" Memory input "), "case={label}\n{text}");
        assert!(
            !text.contains("INVALID MEMORY EDITOR DRAFT"),
            "case={label}\n{text}",
        );
    }
}

#[test]
fn hostile_memory_metadata_is_visibly_escaped_instead_of_silently_replaced() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    model
        .agents
        .memory
        .profile
        .as_mut()
        .expect("identity")
        .display_name = "Owner\u{e9}\t\n\u{85}\u{1b}\u{202e}".to_owned();
    let entry = memory_entry(&model, 100, "safe key");
    install_entries(
        &mut model,
        vec![entry_summary(
            &entry,
            "Key\n\u{85}\u{1b}\u{202e}\u{e9}\t".to_owned(),
            vec!["Tag\u{85}\t".to_owned()],
        )],
    );

    let flat = render_rows(&model, 300, 60).concat();
    for escaped in ["\\n", "\\u{1b}", "\\u{202e}", "\\u{e9}", "\\t", "\\u{85}"] {
        assert!(flat.contains(escaped), "missing visible escape {escaped:?}");
    }
    for raw in ['\u{1b}', '\u{202e}', '\u{e9}', '\t', '\u{85}'] {
        assert!(!flat.contains(raw), "raw scalar leaked: {raw:?}");
    }
}

#[test]
fn inconsistent_or_overflowing_entry_pages_fail_closed_without_row_prose() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let entry = memory_entry(&model, 200, "safe key");
    install_entries(
        &mut model,
        vec![entry_summary(
            &entry,
            "DO NOT RENDER".to_owned(),
            Vec::new(),
        )],
    );

    model
        .agents
        .memory
        .entries
        .as_mut()
        .expect("entries")
        .returned_count = 2;
    let inconsistent = render_text(&model, 140, 30);
    assert!(inconsistent.contains("Memory entries unavailable"));
    assert!(!inconsistent.contains("DO NOT RENDER"));

    let page = model.agents.memory.entries.as_mut().expect("entries");
    page.returned_count = 1;
    page.omitted_count = u64::MAX;
    page.total_count = u64::MAX;
    let overflowing = render_text(&model, 140, 30);
    assert!(overflowing.contains("Memory entries unavailable"));
    assert!(!overflowing.contains("DO NOT RENDER"));
}

#[test]
fn overflowing_history_and_proposal_counts_fail_closed_without_row_prose() {
    let mut history_model = model_with_profile();
    history_model.active_view = View::Agents;
    history_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut history_model);
    let entry = memory_entry(&history_model, 250, "HISTORY MUST NOT RENDER");
    install_entries(
        &mut history_model,
        vec![entry_summary(
            &entry,
            entry.display_key().to_owned(),
            Vec::new(),
        )],
    );
    install_history(&mut history_model, &entry, std::slice::from_ref(&entry));
    history_model.agents.memory.pane = MemoryPane::EntryHistory;
    let history = history_model
        .agents
        .memory
        .entry_history
        .as_mut()
        .expect("history");
    history.omitted_count = u64::MAX;
    history.total_count = u64::MAX;
    let history_text = render_text(&history_model, 140, 30);
    assert!(history_text.contains("Entry history unavailable"));
    assert!(!history_text.contains("HISTORY MUST NOT RENDER"));

    let mut proposal_model = model_with_profile();
    proposal_model.active_view = View::Agents;
    proposal_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut proposal_model);
    let proposal = proposal(&proposal_model, 260, "PROPOSAL MUST NOT RENDER");
    install_proposals(&mut proposal_model, std::slice::from_ref(&proposal));
    proposal_model.agents.memory.pane = MemoryPane::Proposals;
    let proposals = proposal_model
        .agents
        .memory
        .proposals
        .as_mut()
        .expect("proposals");
    proposals.omitted_count = u64::MAX;
    proposals.total_count = u64::MAX;
    let proposal_text = render_text(&proposal_model, 140, 30);
    assert!(proposal_text.contains("Memory proposals unavailable"));
    assert!(!proposal_text.contains("PROPOSAL MUST NOT RENDER"));
}

#[test]
fn invalid_selected_indices_make_all_nonempty_memory_lists_content_free() {
    let mut entries = model_with_profile();
    entries.active_view = View::Agents;
    entries.agents.pane = AgentsPane::Memory;
    bind_memory(&mut entries);
    let entry = memory_entry(&entries, 270, "ENTRY ROW MUST NOT RENDER");
    install_entries(
        &mut entries,
        vec![entry_summary(
            &entry,
            entry.display_key().to_owned(),
            Vec::new(),
        )],
    );
    entries.agents.memory.selected_entry = usize::MAX;
    let text = render_text(&entries, 140, 30);
    assert!(text.contains("Memory entries unavailable"));
    assert!(!text.contains("ENTRY ROW MUST NOT RENDER"));

    let mut history = model_with_profile();
    history.active_view = View::Agents;
    history.agents.pane = AgentsPane::Memory;
    bind_memory(&mut history);
    let entry = memory_entry(&history, 280, "HISTORY ROW MUST NOT RENDER");
    install_entries(
        &mut history,
        vec![entry_summary(
            &entry,
            entry.display_key().to_owned(),
            Vec::new(),
        )],
    );
    install_history(&mut history, &entry, std::slice::from_ref(&entry));
    history.agents.memory.pane = MemoryPane::EntryHistory;
    history.agents.memory.selected_history_version = usize::MAX;
    let text = render_text(&history, 140, 30);
    assert!(text.contains("Entry history unavailable"));
    assert!(!text.contains("HISTORY ROW MUST NOT RENDER"));

    let mut proposals = model_with_profile();
    proposals.active_view = View::Agents;
    proposals.agents.pane = AgentsPane::Memory;
    bind_memory(&mut proposals);
    let proposal = proposal(&proposals, 290, "PROPOSAL ROW MUST NOT RENDER");
    install_proposals(&mut proposals, std::slice::from_ref(&proposal));
    proposals.agents.memory.pane = MemoryPane::Proposals;
    proposals.agents.memory.selected_proposal = usize::MAX;
    let text = render_text(&proposals, 140, 30);
    assert!(text.contains("Memory proposals unavailable"));
    assert!(!text.contains("PROPOSAL ROW MUST NOT RENDER"));

    let mut episodes = model_with_profile();
    episodes.active_view = View::Agents;
    episodes.agents.pane = AgentsPane::Memory;
    bind_memory(&mut episodes);
    let summary = episodic_summary(&episodes, 300, "EPISODE ROW MUST NOT RENDER");
    install_episodes(
        &mut episodes,
        vec![episodic_list_item(
            &summary,
            "EPISODE ROW MUST NOT RENDER".to_owned(),
        )],
    );
    episodes.agents.memory.pane = MemoryPane::EpisodicSummaries;
    episodes.agents.memory.selected_episode = usize::MAX;
    let text = render_text(&episodes, 140, 30);
    assert!(text.contains("Episodic summaries unavailable"));
    assert!(!text.contains("EPISODE ROW MUST NOT RENDER"));
}

#[test]
fn entry_rows_are_capped_at_one_hundred_and_extra_rows_become_omissions() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let entry = memory_entry(&model, 300, "safe key");
    let rows = (0..105)
        .map(|index| entry_summary(&entry, format!("Row {index:03}"), Vec::new()))
        .collect();
    install_entries(&mut model, rows);

    let text = render_text(&model, 140, 30);
    assert!(text.contains("Displayed 100"));
    assert!(text.contains("Omitted 5"));
    assert!(!text.contains("Row 104"));
}

#[test]
fn every_memory_page_keeps_exactly_one_hundred_rows_and_ignores_a_hostile_101st() {
    const HOSTILE_101: &str = "HOSTILE_101_\u{202e}";
    let foreign_namespace = MemoryNamespaceId::from_uuid(Uuid::from_u128(999_999));

    let mut entries = model_with_profile();
    entries.active_view = View::Agents;
    entries.agents.pane = AgentsPane::Memory;
    bind_memory(&mut entries);
    let entry = memory_entry(&entries, 310, "retained entry");
    let retained_entry = entry_summary(&entry, "retained entry".to_owned(), Vec::new());
    install_entries(&mut entries, vec![retained_entry; 100]);
    {
        let page = entries.agents.memory.entries.as_mut().expect("entries");
        page.omitted_count = 7;
        page.total_count = 107;
    }
    let exact = render_text(&entries, 300, 30);
    for count in ["Displayed 100", "Omitted 7", "Total 107"] {
        assert!(
            exact.contains(count),
            "entry exact boundary missing {count}"
        );
    }
    let hostile_entry = memory_entry_in_namespace(foreign_namespace, 320, "foreign entry");
    {
        let page = entries.agents.memory.entries.as_mut().expect("entries");
        page.entries.push(entry_summary(
            &hostile_entry,
            HOSTILE_101.to_owned(),
            Vec::new(),
        ));
        page.returned_count = 101;
        page.total_count = 108;
    }
    let capped = render_text(&entries, 300, 30);
    for count in ["Displayed 100", "Omitted 8", "Total 108"] {
        assert!(
            capped.contains(count),
            "entry capped boundary missing {count}"
        );
    }
    assert!(!capped.contains("Memory entries unavailable"));
    assert!(!capped.contains("HOSTILE_101"));
    assert!(!capped.contains("\\u{202e}"));

    let mut history = model_with_profile();
    history.active_view = View::Agents;
    history.agents.pane = AgentsPane::Memory;
    bind_memory(&mut history);
    let current = memory_entry(&history, 330, "retained history");
    install_entries(
        &mut history,
        vec![entry_summary(
            &current,
            current.display_key().to_owned(),
            Vec::new(),
        )],
    );
    install_history(&mut history, &current, &vec![current.clone(); 100]);
    history.agents.memory.pane = MemoryPane::EntryHistory;
    {
        let page = history
            .agents
            .memory
            .entry_history
            .as_mut()
            .expect("history");
        page.omitted_count = 7;
        page.total_count = 107;
    }
    let exact = render_text(&history, 300, 30);
    for count in ["Versions 100", "Omitted 7"] {
        assert!(
            exact.contains(count),
            "history exact boundary missing {count}"
        );
    }
    let foreign_history = MemoryEntryVersion::create_present(
        foreign_namespace,
        current.reference().entry_id(),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(341)),
        MemoryEntryDraft::new(
            "foreign history".to_owned(),
            "foreign value".to_owned(),
            Vec::new(),
        )
        .expect("foreign history draft"),
        Actor::Human,
        1_800_000_000_341,
        None,
        EventId::from_uuid(Uuid::from_u128(342)),
    )
    .expect("foreign history");
    {
        let page = history
            .agents
            .memory
            .entry_history
            .as_mut()
            .expect("history");
        page.versions.push(MemoryEntryHistorySummary {
            entry: foreign_history.reference(),
            display_key: HOSTILE_101.to_owned(),
            created_at_ms: foreign_history.created_at_ms(),
            accepted_proposal: None,
        });
        page.returned_count = 101;
        page.total_count = 108;
    }
    let capped = render_text(&history, 300, 30);
    for count in ["Versions 100", "Omitted 8"] {
        assert!(
            capped.contains(count),
            "history capped boundary missing {count}"
        );
    }
    assert!(!capped.contains("Entry history unavailable"));
    assert!(!capped.contains("HOSTILE_101"));
    assert!(!capped.contains("\\u{202e}"));

    let mut proposals = model_with_profile();
    proposals.active_view = View::Agents;
    proposals.agents.pane = AgentsPane::Memory;
    bind_memory(&mut proposals);
    let proposal = proposal(&proposals, 350, "retained proposal");
    install_proposals(&mut proposals, &vec![proposal; 100]);
    proposals.agents.memory.pane = MemoryPane::Proposals;
    {
        let page = proposals
            .agents
            .memory
            .proposals
            .as_mut()
            .expect("proposals");
        page.omitted_count = 7;
        page.total_count = 107;
    }
    let exact = render_text(&proposals, 300, 30);
    for count in ["Displayed 100", "Omitted 7", "Total 107"] {
        assert!(
            exact.contains(count),
            "proposal exact boundary missing {count}"
        );
    }
    {
        let page = proposals
            .agents
            .memory
            .proposals
            .as_mut()
            .expect("proposals");
        let mut hostile = page.proposals[0].clone();
        hostile.namespace_id = foreign_namespace;
        hostile.display_key = HOSTILE_101.to_owned();
        page.proposals.push(hostile);
        page.returned_count = 101;
        page.total_count = 108;
    }
    let capped = render_text(&proposals, 300, 30);
    for count in ["Displayed 100", "Omitted 8", "Total 108"] {
        assert!(
            capped.contains(count),
            "proposal capped boundary missing {count}"
        );
    }
    assert!(!capped.contains("Memory proposals unavailable"));
    assert!(!capped.contains("HOSTILE_101"));
    assert!(!capped.contains("\\u{202e}"));

    let mut episodes = model_with_profile();
    episodes.active_view = View::Agents;
    episodes.agents.pane = AgentsPane::Memory;
    bind_memory(&mut episodes);
    let summary = episodic_summary(&episodes, 360, "retained episode");
    let retained_episode = episodic_list_item(&summary, "retained episode".to_owned());
    install_episodes(&mut episodes, vec![retained_episode; 100]);
    episodes.agents.memory.pane = MemoryPane::EpisodicSummaries;
    {
        let page = episodes.agents.memory.episodes.as_mut().expect("episodes");
        page.omitted_count = 7;
        page.total_count = 107;
    }
    let exact = render_text(&episodes, 300, 30);
    for count in ["Displayed 100", "Omitted 7", "Total 107"] {
        assert!(
            exact.contains(count),
            "episode exact boundary missing {count}"
        );
    }
    {
        let page = episodes.agents.memory.episodes.as_mut().expect("episodes");
        let mut hostile = page.summaries[0].clone();
        hostile.label = HOSTILE_101.to_owned();
        hostile.source_count = 129;
        page.summaries.push(hostile);
        page.returned_count = 101;
        page.total_count = 108;
    }
    let capped = render_text(&episodes, 300, 30);
    for count in ["Displayed 100", "Omitted 8", "Total 108"] {
        assert!(
            capped.contains(count),
            "episode capped boundary missing {count}"
        );
    }
    assert!(!capped.contains("Episodic summaries unavailable"));
    assert!(!capped.contains("HOSTILE_101"));
    assert!(!capped.contains("\\u{202e}"));
}

#[test]
fn oversized_entry_scroll_is_clamped_without_hiding_identity_or_controls() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let entry = memory_entry(&model, 400, "visible key");
    install_entries(
        &mut model,
        vec![entry_summary(&entry, "visible key".to_owned(), Vec::new())],
    );
    model.agents.memory.entry_scroll = usize::MAX;

    let text = render_text(&model, 60, 18);
    assert!(text.contains("visible key"));
    assert!(text.contains("Enter"));
    assert!(text.contains("Esc"));
}

#[test]
fn mutation_review_keeps_fixed_identity_warning_and_controls_visible_at_minimum_size() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    install_set_review(&mut model, "thesis horizon");

    let wide = render_text(&model, 300, 40);
    assert_eq!(wide.matches(MEMORY_PLAINTEXT_WARNING).count(), 1);

    let narrow = render_rows(&model, 60, 18).join(" ");
    for required in [
        "Agents / Memory",
        "Operation",
        "Set",
        "thesis horizon",
        "Plaintext local memory",
        "do not store credentials",
        "history",
        "is retained after overwrite or delete",
        "Enter",
        "Esc",
    ] {
        assert!(
            narrow.contains(required),
            "missing {required:?} in frame:\n{narrow}"
        );
    }
}

#[test]
fn entry_list_preview_is_metadata_only_even_when_matching_detail_is_cached() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let entry = memory_entry(&model, 500, "valuation anchor");
    install_entries(
        &mut model,
        vec![entry_summary(
            &entry,
            entry.display_key().to_owned(),
            entry.purpose_tags().to_vec(),
        )],
    );
    install_entry_detail(&mut model, entry);
    model.agents.memory.pane = MemoryPane::EntryList;

    let text = render_text(&model, 140, 30);
    assert!(text.contains("Entry metadata"));
    assert!(text.contains("Value bytes"));
    assert!(!text.contains("value 500"));
}

#[test]
fn matching_entry_detail_uses_one_and_two_column_composition() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let entry = memory_entry(&model, 600, "margin of safety");
    install_entries(
        &mut model,
        vec![entry_summary(
            &entry,
            entry.display_key().to_owned(),
            entry.purpose_tags().to_vec(),
        )],
    );
    install_entry_detail(&mut model, entry);
    model.agents.memory.pane = MemoryPane::EntryDetail;

    let narrow = render_text(&model, 60, 30);
    assert!(narrow.contains("Entry detail"));
    assert!(narrow.contains("value 600"));
    assert!(!narrow.contains("Memory entries"));

    let medium = render_text(&model, 100, 30);
    assert!(medium.contains("Memory entries"));
    assert!(medium.contains("Entry detail"));
    assert!(medium.contains("value 600"));
    assert!(medium.contains("Entry context"));

    let wide = render_text(&model, 140, 30);
    for expected in [
        "Memory entries",
        "Entry detail",
        "Entry context",
        "Edit",
        "Delete",
        "History",
    ] {
        assert!(wide.contains(expected), "missing {expected:?}");
    }
}

#[test]
fn mismatched_entry_detail_fails_closed_without_cached_value_or_substitution() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let selected = memory_entry(&model, 700, "selected key");
    let stale = memory_entry(&model, 800, "stale key");
    install_entries(
        &mut model,
        vec![entry_summary(
            &selected,
            selected.display_key().to_owned(),
            Vec::new(),
        )],
    );
    install_entry_detail(&mut model, stale);
    model.agents.memory.pane = MemoryPane::EntryDetail;

    let text = render_text(&model, 100, 30);
    assert!(text.contains("Entry detail unavailable"));
    assert!(text.contains("reload"));
    assert!(!text.contains("value 800"));
    assert!(!text.contains("stale key"));
}

#[test]
fn final_memory_history_list_hint_matches_cached_version_escape_step() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let entry = memory_entry(&model, 80_900, "retained history");
    install_entries(
        &mut model,
        vec![entry_summary(
            &entry,
            entry.display_key().to_owned(),
            Vec::new(),
        )],
    );
    install_entry_detail(&mut model, entry.clone());
    install_history(&mut model, &entry, std::slice::from_ref(&entry));
    model.agents.memory.pane = MemoryPane::EntryHistory;
    model.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: model.agents.detail.as_ref().unwrap().profile.reference(),
        entry,
    });
    model.set_focus(Focus::List);
    model.set_terminal_size(60, 18);
    assert!(render_text(&model, 60, 18).contains("Esc: clear cached version"));
    handle_event(
        &mut model,
        TuiEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
    );
    assert_eq!(model.agents.memory.pane, MemoryPane::EntryHistory);
    assert!(model.agents.memory.entry_version.is_none());
    assert!(render_text(&model, 60, 18).contains("Esc: entry detail"));
}

#[test]
fn entry_history_without_version_detail_renders_the_history_layer() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let v1 = memory_entry(&model, 900, "risk budget");
    let v2 = successor_entry(&v1, 910, "revised risk budget");
    install_entries(
        &mut model,
        vec![entry_summary(&v2, v2.display_key().to_owned(), Vec::new())],
    );
    install_history(&mut model, &v2, &[v2.clone(), v1.clone()]);
    model.agents.memory.pane = MemoryPane::EntryHistory;
    model.agents.memory.selected_history_version = 1;

    let narrow = render_text(&model, 60, 30);
    assert!(narrow.contains("Entry history"));
    assert!(narrow.contains("v1"));
    assert!(narrow.contains("Enter: load exact version"));
    assert!(!narrow.contains("value 900"));

    let wide = render_text(&model, 140, 30);
    assert!(wide.contains("History version"));
    assert!(wide.contains("Parent entry"));
}

#[test]
fn matching_history_version_is_the_current_narrow_layer_and_mismatch_fails_closed() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let v1 = memory_entry(&model, 1_000, "quality threshold");
    let v2 = successor_entry(&v1, 1_010, "new threshold");
    install_entries(
        &mut model,
        vec![entry_summary(&v2, v2.display_key().to_owned(), Vec::new())],
    );
    install_history(&mut model, &v2, &[v2.clone(), v1.clone()]);
    model.agents.memory.selected_history_version = 1;
    model.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: model
            .agents
            .detail
            .as_ref()
            .expect("profile")
            .profile
            .reference(),
        entry: v1,
    });
    model.agents.memory.pane = MemoryPane::EntryHistory;

    let matching = render_text(&model, 60, 30);
    assert!(matching.contains("Historical entry version"));
    assert!(matching.contains("value 1000"));
    assert!(!matching.contains("Memory entries"));

    model.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: model
            .agents
            .detail
            .as_ref()
            .expect("profile")
            .profile
            .reference(),
        entry: v2,
    });
    let mismatch = render_text(&model, 60, 30);
    assert!(mismatch.contains("Historical version unavailable"));
    assert!(!mismatch.contains("new threshold"));
    assert!(!mismatch.contains("revised risk budget"));
}

#[test]
fn every_editor_stage_and_no_change_review_keeps_warning_identity_and_controls_visible() {
    for (completed_fields, stage) in [(0, "Key"), (1, "Value"), (2, "Purpose tags")] {
        let mut model = model_with_profile();
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Memory;
        bind_memory(&mut model);
        install_editor(&mut model, completed_fields, false);

        assert_minimum_warning_surface(&model, "Set");
        let text = render_text(&model, 100, 30);
        assert!(text.contains(stage), "stage={stage}");
        assert!(text.contains(" Memory input "));
    }

    let mut no_change = model_with_profile();
    no_change.active_view = View::Agents;
    no_change.agents.pane = AgentsPane::Memory;
    bind_memory(&mut no_change);
    install_editor(&mut no_change, 3, true);
    assert_minimum_warning_surface(&no_change, "Set");
    let text = render_text(&no_change, 100, 30);
    assert!(text.contains("No memory change"));
    assert!(text.contains("Identical content"));
    assert!(!text.contains(" Memory input "));
}

#[test]
fn edit_editor_fixed_safety_prefix_uses_its_retained_entry_identity_at_every_stage() {
    let key = format!("edit-key-{}", "x".repeat(87));
    for (completed_fields, stage) in [(0, "Value"), (1, "Purpose tags"), (2, "Review")] {
        let mut model = model_with_profile();
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Memory;
        bind_memory(&mut model);
        let entry = memory_entry(&model, 77_100, &key);
        let entry_id = entry.reference().entry_id().to_string();
        install_entry_detail(&mut model, entry.clone());
        model
            .agents
            .memory
            .open_edit_editor(
                AgentProfileSelector::Id(
                    model
                        .agents
                        .memory
                        .profile
                        .as_ref()
                        .expect("profile identity")
                        .profile
                        .profile_id(),
                ),
                entry,
            )
            .expect("open edit editor");
        let editor = model.agents.memory.editor.as_mut().expect("edit editor");
        if completed_fields >= 1 {
            editor
                .submit_line("replacement edit value".to_owned())
                .expect("submit edit value");
        }
        if completed_fields >= 2 {
            editor
                .submit_keyboard_line("risk, quality")
                .expect("submit edit tags");
            assert!(editor.apply_preview(
                editor.generation(),
                MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent),
            ));
        }

        assert_fixed_safety_surface_at_every_supported_pane_count(
            &model,
            &format!("Set/{stage}"),
            &entry_id,
            "edit-key-",
        );
        for width in [60, 100, 140] {
            let panel = render_memory_detail_text(&model, width, 18);
            assert!(
                !panel.contains("New entry"),
                "width={width} stage={stage} mislabeled an edit:\n{panel}",
            );
        }
    }
}

#[test]
fn malformed_edit_editor_origin_seed_and_namespace_states_render_content_free() {
    let mut cases = Vec::new();

    let mut create_with_seed = model_with_profile();
    create_with_seed.active_view = View::Agents;
    create_with_seed.agents.pane = AgentsPane::Memory;
    bind_memory(&mut create_with_seed);
    let seeded = memory_entry(&create_with_seed, 77_200, "CREATE_WITH_SEED_PROSE");
    create_with_seed
        .agents
        .memory
        .open_edit_editor(
            AgentProfileSelector::Id(
                create_with_seed
                    .agents
                    .memory
                    .profile
                    .as_ref()
                    .expect("profile identity")
                    .profile
                    .profile_id(),
            ),
            seeded,
        )
        .expect("seeded editor");
    create_with_seed.agents.memory.editor_origin =
        ai_stock_forum::ui::tui::model::MemoryEditorOrigin::Create;
    cases.push((create_with_seed, "create origin with retained seed"));

    let mut edit_without_seed = model_with_profile();
    edit_without_seed.active_view = View::Agents;
    edit_without_seed.agents.pane = AgentsPane::Memory;
    bind_memory(&mut edit_without_seed);
    edit_without_seed
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(
            edit_without_seed
                .agents
                .memory
                .profile
                .as_ref()
                .expect("profile identity")
                .profile
                .profile_id(),
        ))
        .expect("seedless editor");
    edit_without_seed.agents.memory.editor_origin =
        ai_stock_forum::ui::tui::model::MemoryEditorOrigin::Edit;
    cases.push((edit_without_seed, "edit origin without retained seed"));

    let mut foreign_namespace = model_with_profile();
    foreign_namespace.active_view = View::Agents;
    foreign_namespace.agents.pane = AgentsPane::Memory;
    bind_memory(&mut foreign_namespace);
    let foreign_seed = memory_entry_in_namespace(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(77_300)),
        77_301,
        "FOREIGN_NAMESPACE_SEED_PROSE",
    );
    foreign_namespace
        .agents
        .memory
        .open_edit_editor(
            AgentProfileSelector::Id(
                foreign_namespace
                    .agents
                    .memory
                    .profile
                    .as_ref()
                    .expect("profile identity")
                    .profile
                    .profile_id(),
            ),
            foreign_seed,
        )
        .expect("foreign namespace editor fixture");
    cases.push((foreign_namespace, "foreign namespace retained seed"));

    let mut tombstone_cache = model_with_profile();
    tombstone_cache.active_view = View::Agents;
    tombstone_cache.agents.pane = AgentsPane::Memory;
    bind_memory(&mut tombstone_cache);
    let present = memory_entry(&tombstone_cache, 77_400, "TOMBSTONE_SEED_PROSE");
    let tombstone = present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(77_403)),
            Actor::Human,
            1_800_000_077_402,
            None,
            EventId::from_uuid(Uuid::from_u128(77_402)),
        )
        .expect("tombstone cache");
    tombstone_cache
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(
            tombstone_cache
                .agents
                .memory
                .profile
                .as_ref()
                .expect("profile identity")
                .profile
                .profile_id(),
        ))
        .expect("seedless tombstone editor");
    tombstone_cache.agents.memory.editor_origin =
        ai_stock_forum::ui::tui::model::MemoryEditorOrigin::Edit;
    install_entry_detail(&mut tombstone_cache, tombstone);
    cases.push((
        tombstone_cache,
        "edit origin without a retained seed despite a tombstone cache",
    ));

    let mut mismatched_cached_key = model_with_profile();
    mismatched_cached_key.active_view = View::Agents;
    mismatched_cached_key.agents.pane = AgentsPane::Memory;
    bind_memory(&mut mismatched_cached_key);
    mismatched_cached_key
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(
            mismatched_cached_key
                .agents
                .memory
                .profile
                .as_ref()
                .expect("profile identity")
                .profile
                .profile_id(),
        ))
        .expect("seedless mismatched-key editor");
    mismatched_cached_key
        .agents
        .memory
        .editor
        .as_mut()
        .expect("editor")
        .submit_keyboard_line("EDITOR_KEY_PROSE")
        .expect("editor key");
    mismatched_cached_key.agents.memory.editor_origin =
        ai_stock_forum::ui::tui::model::MemoryEditorOrigin::Edit;
    let cached = memory_entry(&mismatched_cached_key, 77_500, "CACHED_OTHER_KEY_PROSE");
    install_entry_detail(&mut mismatched_cached_key, cached);
    cases.push((
        mismatched_cached_key,
        "edit origin without a retained seed despite an unrelated cache",
    ));

    for (mut model, label) in cases {
        model.command.ingest("INVALID_EDIT_EDITOR_DRAFT_PROSE");
        let text = render_text(&model, 100, 30);
        assert!(!text.contains(" Command "), "case={label}\n{text}");
        assert!(!text.contains(" Memory input "), "case={label}\n{text}");
        for forbidden in [
            "INVALID_EDIT_EDITOR_DRAFT_PROSE",
            "CREATE_WITH_SEED_PROSE",
            "FOREIGN_NAMESPACE_SEED_PROSE",
            "TOMBSTONE_SEED_PROSE",
            "EDITOR_KEY_PROSE",
            "CACHED_OTHER_KEY_PROSE",
        ] {
            assert!(
                !text.contains(forbidden),
                "case={label} leaked {forbidden:?}:\n{text}",
            );
        }
    }
}

#[test]
fn set_and_delete_mutation_reviews_render_exact_authenticated_identity() {
    let mut set = model_with_profile();
    set.active_view = View::Agents;
    set.agents.pane = AgentsPane::Memory;
    bind_memory(&mut set);
    install_set_review(&mut set, "capital allocation");
    assert_minimum_warning_surface(&set, "Set");
    let set_text = render_text(&set, 140, 30);
    assert!(set_text.contains("capital allocation"));
    assert!(set_text.contains("continue"));

    let mut delete = model_with_profile();
    delete.active_view = View::Agents;
    delete.agents.pane = AgentsPane::Memory;
    bind_memory(&mut delete);
    let entry = memory_entry(&delete, 1_100, "obsolete thesis");
    install_delete_review(&mut delete, &entry);
    assert_minimum_warning_surface(&delete, "Delete");
    let delete_text = render_text(&delete, 140, 30);
    assert!(delete_text.contains(&entry.reference().entry_id().to_string()));
    assert!(delete_text.contains("obsolete thesis"));
    assert!(!delete_text.contains("review value"));
}

#[test]
fn both_mutation_confirmations_require_the_exact_reconstructed_command() {
    let mut set = model_with_profile();
    set.active_view = View::Agents;
    set.agents.pane = AgentsPane::Memory;
    bind_memory(&mut set);
    install_set_review(&mut set, "confirmation key");
    set.agents.memory.generation = 21;
    let command = set_review_command(set.agents.memory.edit_review.as_ref().expect("set review"));
    set.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: 21,
    });
    set.agents.memory.pane = MemoryPane::Confirmation;
    assert_minimum_warning_surface(&set, "Set");
    assert!(render_text(&set, 140, 30).contains("Confirm memory change"));

    let mut delete = model_with_profile();
    delete.active_view = View::Agents;
    delete.agents.pane = AgentsPane::Memory;
    bind_memory(&mut delete);
    let entry = memory_entry(&delete, 1_200, "delete confirmation");
    install_delete_review(&mut delete, &entry);
    delete.agents.memory.generation = 22;
    let command = delete_review_command(
        delete
            .agents
            .memory
            .edit_review
            .as_ref()
            .expect("delete review"),
    );
    delete.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: 22,
    });
    delete.agents.memory.pane = MemoryPane::Confirmation;
    assert_minimum_warning_surface(&delete, "Delete");
    let delete_text = render_text(&delete, 140, 30);
    assert!(delete_text.contains(&entry.reference().entry_id().to_string()));

    set.agents
        .memory
        .confirmation
        .as_mut()
        .expect("confirmation")
        .generation = 20;
    let malformed = render_text(&set, 140, 30);
    assert!(malformed.contains("Memory confirmation unavailable"));
    assert!(!malformed.contains("confirmation key"));
    assert!(!malformed.contains(MEMORY_PLAINTEXT_WARNING));
}

#[test]
fn mutation_result_is_generic_and_does_not_reconstruct_discarded_identifiers() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    model.agents.memory.pane = MemoryPane::Result;
    model.agents.memory.result_origin = MemoryResultOrigin::Mutation;

    let text = render_text(&model, 60, 18);
    assert!(text.contains("Memory change completed."));
    assert!(text.contains("Enter/Esc: return"));
    assert!(!text.contains(MEMORY_PLAINTEXT_WARNING));
    assert!(!text.contains("review value"));
}

#[test]
fn proposal_list_and_adjacent_preview_are_metadata_only() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let proposal = proposal(&model, 1_300, "durable moat");
    install_proposals(&mut model, std::slice::from_ref(&proposal));
    install_proposal_detail(&mut model, proposal);
    model.agents.memory.pane = MemoryPane::Proposals;

    let text = render_text(&model, 140, 30);
    assert!(text.contains("Memory proposals"));
    assert!(text.contains("Proposal metadata"));
    assert!(text.contains("durable moat"));
    assert!(text.contains("Pending"));
    assert!(!text.contains("proposal value 1300"));
    assert!(!text.contains("proposal rationale 1300"));
}

#[test]
fn exact_proposal_detail_uses_adaptive_composition_and_mismatch_fails_closed() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let selected = proposal(&model, 1_400, "cash conversion");
    install_proposals(&mut model, std::slice::from_ref(&selected));
    install_proposal_detail(&mut model, selected);
    model.agents.memory.pane = MemoryPane::ProposalDetail;

    let narrow = render_text(&model, 60, 30);
    assert!(narrow.contains("Proposal detail"));
    assert!(narrow.contains("proposal value 1400"));
    assert!(narrow.contains("proposal rationale 1400"));
    assert!(!narrow.contains("Memory proposals"));

    let medium = render_text(&model, 100, 30);
    assert!(medium.contains("Memory proposals"));
    assert!(medium.contains("Proposal detail"));

    let wide = render_text(&model, 140, 30);
    for expected in ["Proposal context", "Approve", "Reject", "cash conversion"] {
        assert!(wide.contains(expected), "missing {expected:?}");
    }

    let stale = proposal(&model, 1_500, "stale proposal");
    install_proposal_detail(&mut model, stale);
    let mismatch = render_text(&model, 100, 30);
    assert!(mismatch.contains("Proposal detail unavailable"));
    assert!(!mismatch.contains("proposal value 1500"));
    assert!(!mismatch.contains("proposal rationale 1500"));
}

#[test]
fn approve_and_reject_reviews_keep_exact_identity_warning_and_controls() {
    for (seed, action, label) in [
        (1_600, MemoryResolutionAction::Approve, "Approve"),
        (1_700, MemoryResolutionAction::Reject, "Reject"),
    ] {
        let mut model = model_with_profile();
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Memory;
        bind_memory(&mut model);
        let proposal = proposal(&model, seed, "resolution key");
        let proposal_id = proposal.reference().proposal_id();
        install_proposals(&mut model, std::slice::from_ref(&proposal));
        install_resolution_review(&mut model, proposal, action);

        assert_minimum_warning_surface(&model, label);
        let text = render_text(&model, 140, 30);
        assert!(text.contains(&proposal_id.to_string()));
        assert!(text.contains("resolution key"));
        model.agents.memory.detail_scroll = usize::MAX;
        let tail = render_text(&model, 140, 30);
        assert!(tail.contains("proposal rationale"));
    }
}

#[test]
fn resolution_review_end_reaches_the_exact_wrapped_approval_tail_without_scrolling_safety() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    model.set_terminal_size(100, 30);
    bind_memory(&mut model);
    let wrap_word = "abcdefghijklmnopqrstuvw";
    let value = std::iter::repeat_n(wrap_word, 170)
        .collect::<Vec<_>>()
        .join(" ");
    let rationale = std::iter::repeat_n("reasonablylongwordhere", 22)
        .collect::<Vec<_>>()
        .join(" ");
    let proposal = proposal_with(
        model
            .agents
            .detail
            .as_ref()
            .map(|detail| &detail.profile)
            .expect("profile"),
        79_450,
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new("wrapped approval tail".to_owned(), value, Vec::new())
                .expect("space-separated candidate"),
        },
        "wrapped approval tail",
        ExpectedMemoryEntryState::Absent,
        &rationale,
    );
    let proposal_id = proposal.reference().proposal_id().to_string();
    install_resolution_review(&mut model, proposal, MemoryResolutionAction::Approve);
    let review = model
        .agents
        .memory
        .resolution_review
        .as_ref()
        .expect("resolution review");
    let approval_id = review.approval_id.to_string();
    let review_digest = review.review_digest.to_string();

    assert_eq!(
        handle_event(&mut model, key(KeyCode::End)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw,
    );
    assert_eq!(model.agents.memory.detail_scroll, usize::MAX);
    let panel = render_memory_detail_text(&model, 100, 30);
    let normalized = normalized_panel_text(&panel);
    for fixed in [
        model
            .agents
            .memory
            .profile
            .as_ref()
            .expect("profile identity")
            .profile
            .profile_id()
            .to_string(),
        "Approve".to_owned(),
        proposal_id,
        "Plaintext local memory".to_owned(),
        "do not store".to_owned(),
        "credentials".to_owned(),
        "history is retained after".to_owned(),
        "overwrite or delete".to_owned(),
        "Enter: continue".to_owned(),
        "Esc: cancel review".to_owned(),
    ] {
        assert!(
            normalized.contains(&fixed),
            "missing fixed {fixed:?}:\n{panel}",
        );
    }
    for tail in [
        "Approval ID".to_owned(),
        approval_id,
        "Expected approval Pending".to_owned(),
        "Review digest".to_owned(),
    ] {
        assert!(panel.contains(&tail), "missing tail {tail:?}:\n{panel}");
    }
    let unwrapped = panel
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '│')
        .collect::<String>();
    assert!(
        unwrapped.contains(&review_digest),
        "missing exact wrapped review digest {review_digest:?}:\n{panel}",
    );
}

#[test]
fn proposal_resolution_review_renders_the_complete_exact_set_delete_and_state_contract() {
    let historical = profile();
    let owner = successor_profile(&historical, 78_000);
    let namespace_id = owner.memory_namespace_id();

    let present_target = memory_entry_in_namespace(namespace_id, 78_100, "present target");
    let present_target_tombstone = present_target
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(78_103)),
            Actor::Human,
            1_800_000_078_103,
            None,
            EventId::from_uuid(Uuid::from_u128(78_104)),
        )
        .expect("present-target tombstone");
    let deleted_target_source = memory_entry_in_namespace(namespace_id, 78_200, "deleted target");
    let deleted_target = deleted_target_source
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(78_203)),
            Actor::Human,
            1_800_000_078_203,
            None,
            EventId::from_uuid(Uuid::from_u128(78_204)),
        )
        .expect("deleted target");
    let delete_target = memory_entry_in_namespace(namespace_id, 78_300, "delete target");

    let cases = vec![
        (
            proposal_with(
                &owner,
                78_400,
                MemoryProposalOperation::Set {
                    candidate: MemoryEntryDraft::new(
                        "absent target".to_owned(),
                        "SET_ABSENT_VALUE".to_owned(),
                        vec!["absent-tag".to_owned()],
                    )
                    .expect("absent candidate"),
                },
                "absent target",
                ExpectedMemoryEntryState::Absent,
                "RATIONALE_ABSENT",
            ),
            MemoryResolutionAction::Approve,
            ExpectedMemoryEntryState::Absent,
            MemoryProfileIdentityView {
                profile: owner.reference(),
                display_name: "CURRENT_PROPOSER".to_owned(),
            },
            false,
        ),
        (
            proposal_with(
                &owner,
                78_500,
                MemoryProposalOperation::Set {
                    candidate: MemoryEntryDraft::new(
                        "present target".to_owned(),
                        "SET_PRESENT_VALUE".to_owned(),
                        vec!["present-tag".to_owned()],
                    )
                    .expect("present candidate"),
                },
                "present target",
                ExpectedMemoryEntryState::Present(present_target.reference()),
                "RATIONALE_PRESENT",
            ),
            MemoryResolutionAction::Reject,
            ExpectedMemoryEntryState::Deleted(present_target_tombstone.reference()),
            MemoryProfileIdentityView {
                profile: owner.reference(),
                display_name: "CURRENT_PROPOSER".to_owned(),
            },
            false,
        ),
        (
            proposal_with(
                &historical,
                78_600,
                MemoryProposalOperation::Set {
                    candidate: MemoryEntryDraft::new(
                        "deleted target".to_owned(),
                        "SET_DELETED_VALUE".to_owned(),
                        vec!["deleted-tag".to_owned()],
                    )
                    .expect("deleted candidate"),
                },
                "deleted target",
                ExpectedMemoryEntryState::Deleted(deleted_target.reference()),
                "RATIONALE_DELETED",
            ),
            MemoryResolutionAction::Approve,
            ExpectedMemoryEntryState::Present(deleted_target_source.reference()),
            MemoryProfileIdentityView {
                profile: historical.reference(),
                display_name: "HISTORICAL_PROPOSER".to_owned(),
            },
            true,
        ),
        (
            proposal_with(
                &owner,
                78_700,
                MemoryProposalOperation::Delete,
                "delete target",
                ExpectedMemoryEntryState::Present(delete_target.reference()),
                "RATIONALE_DELETE",
            ),
            MemoryResolutionAction::Reject,
            ExpectedMemoryEntryState::Absent,
            MemoryProfileIdentityView {
                profile: owner.reference(),
                display_name: "CURRENT_PROPOSER".to_owned(),
            },
            false,
        ),
    ];

    for (index, (proposal, action, current, proposer_identity, historical_flag)) in
        cases.into_iter().enumerate()
    {
        let proposal_expected = proposal.expected().clone();
        let proposal_reference = proposal.reference();
        let proposal_operation = match proposal.operation() {
            MemoryProposalOperation::Set { .. } => "Set",
            MemoryProposalOperation::Delete => "Delete",
        };
        let action_name = match action {
            MemoryResolutionAction::Approve => "Approve",
            MemoryResolutionAction::Reject => "Reject",
        };
        let rationale = proposal.rationale().to_owned();
        let owner_identity = MemoryProfileIdentityView {
            profile: owner.reference(),
            display_name: "CURRENT_OWNER".to_owned(),
        };
        let mut model = model_with_exact_profile(owner.clone());
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Memory;
        bind_memory(&mut model);
        install_exact_resolution_review(
            &mut model,
            proposal,
            action,
            current.clone(),
            (proposer_identity.clone(), owner_identity.clone()),
            historical_flag,
            78_800 + u128::try_from(index).expect("case index"),
        );

        let review = model
            .agents
            .memory
            .resolution_review
            .as_ref()
            .expect("resolution review");
        let detail = render_memory_detail_text(&model, 320, 120);
        for expected in [
            format!("Action          {action_name}"),
            format!("Owner name      {}", owner_identity.display_name),
            format!("Owner profile ID {}", owner_identity.profile.profile_id()),
            format!(
                "Owner version ID {}",
                owner_identity.profile.profile_version_id()
            ),
            format!("Owner version {}", owner_identity.profile.version().get()),
            format!("Owner digest {}", owner_identity.profile.content_digest()),
            format!("Namespace ID    {namespace_id}"),
            format!("Proposer name   {}", proposer_identity.display_name),
            format!(
                "Proposer profile ID {}",
                proposer_identity.profile.profile_id()
            ),
            format!(
                "Proposer version ID {}",
                proposer_identity.profile.profile_version_id()
            ),
            format!(
                "Proposer version {}",
                proposer_identity.profile.version().get()
            ),
            format!(
                "Proposer digest {}",
                proposer_identity.profile.content_digest()
            ),
            format!("Proposal ID     {}", proposal_reference.proposal_id()),
            format!("Proposal version {}", proposal_reference.version().get()),
            format!("Proposal digest {}", proposal_reference.content_digest()),
            format!("Underlying operation {proposal_operation}"),
            format!("Approval ID     {}", review.approval_id),
            "Expected approval Pending".to_owned(),
            format!("Review digest   {}", review.review_digest),
            MEMORY_PLAINTEXT_WARNING.to_owned(),
            "Enter: continue | Esc: cancel review".to_owned(),
        ] {
            assert!(
                detail.contains(&expected),
                "case={index} missing {expected:?}:\n{detail}",
            );
        }
        match &proposal_expected {
            ExpectedMemoryEntryState::Absent => {
                assert!(detail.contains("Proposal expected Absent"));
            }
            ExpectedMemoryEntryState::Present(reference) => {
                assert!(detail.contains("Proposal expected Present"));
                assert_rendered_entry_reference(&detail, "Proposal expected", reference);
            }
            ExpectedMemoryEntryState::Deleted(reference) => {
                assert!(detail.contains("Proposal expected Deleted"));
                assert_rendered_entry_reference(&detail, "Proposal expected", reference);
            }
        }
        match &current {
            ExpectedMemoryEntryState::Absent => assert!(detail.contains("Review current Absent")),
            ExpectedMemoryEntryState::Present(reference) => {
                assert!(detail.contains("Review current Present"));
                assert_rendered_entry_reference(&detail, "Review current", reference);
            }
            ExpectedMemoryEntryState::Deleted(reference) => {
                assert!(detail.contains("Review current Deleted"));
                assert_rendered_entry_reference(&detail, "Review current", reference);
            }
        }
        assert_eq!(detail.matches(&rationale).count(), 1, "case={index}");
        if historical_flag {
            assert_eq!(
                detail.matches("proposer version is historical").count(),
                1,
                "case={index}",
            );
        } else {
            assert!(!detail.contains("proposer version is historical"));
        }
        match &review.proposal.operation() {
            MemoryProposalOperation::Set { candidate } => {
                assert!(detail.contains(candidate.value()));
                for tag in candidate.purpose_tags() {
                    assert!(detail.contains(tag));
                }
            }
            MemoryProposalOperation::Delete => {
                assert!(!detail.contains("Proposed value"));
                assert!(!detail.contains("Purpose tag "));
            }
        }
    }
}

#[test]
fn exact_resolution_review_bounds_hostile_maximum_fields_and_keeps_them_off_adjacent_surfaces() {
    let historical = profile();
    let owner = successor_profile(&historical, 79_000);
    let key = "é".repeat(48);
    let present = memory_entry_in_namespace(owner.memory_namespace_id(), 79_100, &key);
    let deleted = present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(79_103)),
            Actor::Human,
            1_800_000_079_103,
            None,
            EventId::from_uuid(Uuid::from_u128(79_104)),
        )
        .expect("maximum-key tombstone");
    let value = format!("MAX_VALUE_{}", "V".repeat(4_086));
    let tags = (0..8)
        .map(|index| format!("tag-{index}-{}", "t".repeat(26)))
        .collect::<Vec<_>>();
    let rationale = format!("MAX_RATIONALE_{}", "R".repeat(498));
    let proposal = proposal_with(
        &historical,
        79_200,
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(key.clone(), value.clone(), tags.clone())
                .expect("maximum candidate"),
        },
        &key,
        ExpectedMemoryEntryState::Deleted(deleted.reference()),
        &rationale,
    );
    let proposal_for_list = proposal.clone();
    let owner_label = format!("OWNER{}\u{202e}\u{e9}", "\n".repeat(100));
    let proposer_label = format!("PROPOSER{}\u{1b}\u{e9}", "\n".repeat(100));
    let mut model = model_with_exact_profile(owner.clone());
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    model
        .agents
        .memory
        .profile
        .as_mut()
        .expect("bound identity")
        .display_name = owner_label.clone();
    install_exact_resolution_review(
        &mut model,
        proposal,
        MemoryResolutionAction::Reject,
        ExpectedMemoryEntryState::Present(present.reference()),
        (
            MemoryProfileIdentityView {
                profile: historical.reference(),
                display_name: proposer_label,
            },
            MemoryProfileIdentityView {
                profile: owner.reference(),
                display_name: owner_label,
            },
        ),
        true,
        79_300,
    );

    let detail = render_memory_detail_text(&model, 320, 180);
    for escaped in ["\\n", "\\u{202e}", "\\u{e9}", "\\u{1b}"] {
        assert!(detail.contains(escaped), "missing {escaped:?}:\n{detail}");
    }
    for raw in ['\u{202e}', '\u{e9}', '\u{1b}'] {
        assert!(
            !detail.contains(raw),
            "raw scalar {raw:?} leaked:\n{detail}"
        );
    }
    assert!(
        detail.contains("..."),
        "maximum escaped fields were not bounded"
    );
    assert!(detail.contains("MAX_VALUE_"));
    assert!(detail.contains("MAX_RATIONALE_"));
    for (index, tag) in tags.iter().enumerate() {
        assert!(
            detail.contains(&format!("Purpose tag {} {tag}", index + 1)),
            "missing tag {}",
            index + 1,
        );
    }
    assert!(!detail.contains("Purpose tag 9"));
    assert_eq!(
        render_text(&model, 320, 180)
            .matches("MAX_RATIONALE_")
            .count(),
        1,
        "rationale must appear only on the exact active review",
    );

    let mut adjacent = model.clone();
    install_proposals(&mut adjacent, std::slice::from_ref(&proposal_for_list));
    for pane in [
        MemoryPane::Proposals,
        MemoryPane::Confirmation,
        MemoryPane::Result,
    ] {
        adjacent.agents.memory.pane = pane;
        if pane == MemoryPane::Confirmation {
            adjacent.agents.memory.generation = 79_400;
            let command = resolution_review_command(
                adjacent
                    .agents
                    .memory
                    .resolution_review
                    .as_ref()
                    .expect("resolution review"),
            );
            adjacent.agents.memory.confirmation = Some(MemoryConfirmation {
                command,
                generation: 79_400,
            });
        }
        let text = render_text(&adjacent, 320, 80);
        assert!(!text.contains("MAX_VALUE_"), "pane={pane:?}");
        assert!(!text.contains("MAX_RATIONALE_"), "pane={pane:?}");
    }
}

#[test]
fn forged_expected_state_discriminants_render_content_free_in_detail_review_and_confirmation() {
    let mut base = model_with_profile();
    base.active_view = View::Agents;
    base.agents.pane = AgentsPane::Memory;
    bind_memory(&mut base);
    let present = memory_entry(&base, 79_500, "forged discriminant key");
    let deleted = present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(79_503)),
            Actor::Human,
            1_800_000_079_503,
            None,
            EventId::from_uuid(Uuid::from_u128(79_504)),
        )
        .expect("deleted discriminant entry");
    let forged_states = [
        (
            ExpectedMemoryEntryState::Present(deleted.reference()),
            "Present carrying a deleted reference",
        ),
        (
            ExpectedMemoryEntryState::Deleted(present.reference()),
            "Deleted carrying a present reference",
        ),
    ];

    for (index, (forged, label)) in forged_states.into_iter().enumerate() {
        let seed = 79_600 + u128::try_from(index).expect("case index") * 10;
        let proposal = proposal(&base, seed, "forged discriminant key");
        let candidate_prose = format!("proposal value {seed}");
        let rationale_prose = format!("proposal rationale {seed}");
        let reference_prose = match &forged {
            ExpectedMemoryEntryState::Present(reference)
            | ExpectedMemoryEntryState::Deleted(reference) => reference.entry_id().to_string(),
            ExpectedMemoryEntryState::Absent => unreachable!("forged fixture is referenced"),
        };

        let mut detail = base.clone();
        install_proposals(&mut detail, std::slice::from_ref(&proposal));
        install_proposal_detail(&mut detail, proposal.clone());
        detail
            .agents
            .memory
            .proposal_detail
            .as_mut()
            .expect("proposal detail")
            .current_entry = forged.clone();
        detail.agents.memory.pane = MemoryPane::ProposalDetail;
        let detail_text = render_memory_detail_text(&detail, 140, 40);
        assert!(
            detail_text.contains("Proposal detail unavailable."),
            "case={label}:\n{detail_text}",
        );
        for forbidden in [&candidate_prose, &rationale_prose, &reference_prose] {
            assert!(
                !detail_text.contains(forbidden),
                "case={label} leaked {forbidden:?}:\n{detail_text}",
            );
        }

        let mut review = base.clone();
        install_resolution_review(
            &mut review,
            proposal.clone(),
            MemoryResolutionAction::Approve,
        );
        review
            .agents
            .memory
            .resolution_review
            .as_mut()
            .expect("resolution review")
            .expected_entry = forged;
        review.agents.memory.detail_scroll = usize::MAX;
        let review_text = render_memory_detail_text(&review, 140, 40);
        assert!(
            review_text.contains("Proposal resolution review unavailable."),
            "case={label}:\n{review_text}",
        );
        for forbidden in [&candidate_prose, &rationale_prose, &reference_prose] {
            assert!(
                !review_text.contains(forbidden),
                "case={label} leaked {forbidden:?}:\n{review_text}",
            );
        }

        let command = resolution_review_command(
            review
                .agents
                .memory
                .resolution_review
                .as_ref()
                .expect("resolution review"),
        );
        review.agents.memory.generation = 79_700 + u64::try_from(index).expect("case index");
        review.agents.memory.confirmation = Some(MemoryConfirmation {
            command,
            generation: review.agents.memory.generation,
        });
        review.agents.memory.pane = MemoryPane::Confirmation;
        let confirmation_text = render_memory_detail_text(&review, 140, 40);
        assert!(
            confirmation_text.contains("Memory confirmation unavailable."),
            "case={label}:\n{confirmation_text}",
        );
        for forbidden in [&candidate_prose, &rationale_prose, &reference_prose] {
            assert!(
                !confirmation_text.contains(forbidden),
                "case={label} leaked {forbidden:?}:\n{confirmation_text}",
            );
        }
    }
}

#[test]
fn malformed_direct_reviews_and_confirmations_render_only_content_free_unavailable_panels() {
    let mut base_set = model_with_profile();
    base_set.active_view = View::Agents;
    base_set.agents.pane = AgentsPane::Memory;
    base_set.set_terminal_size(100, 30);
    bind_memory(&mut base_set);
    install_set_review(&mut base_set, "direct render auth");

    let present = memory_entry(&base_set, 79_700, "direct render auth");
    let deleted = present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(79_703)),
            Actor::Human,
            79_703,
            None,
            EventId::from_uuid(Uuid::from_u128(79_704)),
        )
        .expect("deleted reference");
    let foreign = memory_entry_in_namespace(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(79_705)),
        79_706,
        "direct render auth",
    );
    let different_key = memory_entry(&base_set, 79_710, "different render key");
    let mut cases = Vec::new();
    for (label, expected) in [
        (
            "Set Present carrying deleted reference",
            ExpectedMemoryEntryState::Present(deleted.reference()),
        ),
        (
            "Set Deleted carrying present reference",
            ExpectedMemoryEntryState::Deleted(present.reference()),
        ),
        (
            "Set foreign namespace reference",
            ExpectedMemoryEntryState::Present(foreign.reference()),
        ),
        (
            "Set mismatched normalized key",
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
        synchronize_set_review_with_editor_preview(&mut model);
        cases.push((label, model));
    }

    let mut detached_set = base_set.clone();
    detached_set.agents.memory.editor = None;
    cases.push(("Set detached editor", detached_set));

    let mut other_preview = model_with_profile();
    bind_memory(&mut other_preview);
    install_set_review(&mut other_preview, "other editor preview");
    let mut different_candidate = base_set.clone();
    let forged_candidate = MemoryEntryDraft::new(
        "direct render auth".to_owned(),
        "FORGED_SET_CANDIDATE_PROSE".to_owned(),
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
    forged_review.diff = vec![
        MemoryFieldDiff {
            field: MemoryField::DisplayKey,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Text(forged_candidate.display_key().to_owned()),
        },
        MemoryFieldDiff {
            field: MemoryField::State,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::State(MemoryEntryState::Present),
        },
        MemoryFieldDiff {
            field: MemoryField::Value,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Text(forged_candidate.value().to_owned()),
        },
        MemoryFieldDiff {
            field: MemoryField::PurposeTags,
            before: MemoryFieldValue::Missing,
            after: MemoryFieldValue::Tags(forged_candidate.purpose_tags().to_vec()),
        },
    ];
    synchronize_set_review_with_editor_preview(&mut different_candidate);
    cases.push((
        "Set candidate differs from editor preview request",
        different_candidate,
    ));

    let mut mismatched_preview = base_set.clone();
    mismatched_preview.agents.memory.edit_review = other_preview.agents.memory.edit_review;
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
    synchronize_set_review_with_editor_preview(&mut empty_set_diff);
    cases.push(("Set empty diff", empty_set_diff));
    let mut falsified_set_diff = base_set.clone();
    falsified_set_diff
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("set review")
        .diff[2]
        .after = MemoryFieldValue::Text("FORGED_SET_DIFF_PROSE".to_owned());
    synchronize_set_review_with_editor_preview(&mut falsified_set_diff);
    cases.push(("Set falsified diff", falsified_set_diff));
    let mut noncanonical_set_diff = base_set;
    noncanonical_set_diff
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("set review")
        .diff
        .swap(0, 1);
    synchronize_set_review_with_editor_preview(&mut noncanonical_set_diff);
    cases.push(("Set noncanonical diff", noncanonical_set_diff));

    let mut base_delete = model_with_profile();
    base_delete.active_view = View::Agents;
    base_delete.agents.pane = AgentsPane::Memory;
    base_delete.set_terminal_size(100, 30);
    bind_memory(&mut base_delete);
    let delete_entry = memory_entry(&base_delete, 79_720, "delete render auth");
    install_delete_review(&mut base_delete, &delete_entry);
    let deleted_expected = delete_entry
        .next_deleted(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(79_723)),
            Actor::Human,
            79_723,
            None,
            EventId::from_uuid(Uuid::from_u128(79_724)),
        )
        .expect("deleted expected");
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
    let foreign_delete = memory_entry_in_namespace(
        MemoryNamespaceId::from_uuid(Uuid::from_u128(79_725)),
        79_726,
        "delete render auth",
    );
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
    cases.push(("Delete detached detail", detached_delete));
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
        .before = MemoryFieldValue::Text("FORGED_DELETE_DIFF_PROSE".to_owned());
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

    for (label, mut model) in cases {
        model.agents.memory.pane = MemoryPane::MutationReview;
        model.agents.memory.confirmation = None;
        let review_panel = render_memory_detail_text(&model, 100, 30);
        assert!(
            review_panel.contains("Memory review unavailable."),
            "case={label}:\n{review_panel}",
        );
        for forbidden in [
            "review value",
            "direct render auth",
            "delete render auth",
            "different render key",
            "FORGED_SET_CANDIDATE_PROSE",
            "FORGED_SET_DIFF_PROSE",
            "FORGED_DELETE_DIFF_PROSE",
            "MUTATION DIFF",
            MEMORY_PLAINTEXT_WARNING,
            "Enter:",
        ] {
            assert!(
                !review_panel.contains(forbidden),
                "review leaked {forbidden:?}, case={label}:\n{review_panel}",
            );
        }

        let review = model.agents.memory.edit_review.as_ref().expect("review");
        let command = match review.operation {
            MemoryMutationKind::Set => set_review_command(review),
            MemoryMutationKind::Delete => delete_review_command(review),
        };
        model.agents.memory.confirmation = Some(MemoryConfirmation {
            command,
            generation: model.agents.memory.generation,
        });
        model.agents.memory.pane = MemoryPane::Confirmation;
        let confirmation_panel = render_memory_detail_text(&model, 100, 30);
        assert!(
            confirmation_panel.contains("Memory confirmation unavailable."),
            "case={label}:\n{confirmation_panel}",
        );
        for forbidden in [
            "review value",
            "direct render auth",
            "delete render auth",
            "different render key",
            "FORGED_SET_CANDIDATE_PROSE",
            "FORGED_SET_DIFF_PROSE",
            "FORGED_DELETE_DIFF_PROSE",
            MEMORY_PLAINTEXT_WARNING,
            "Enter:",
        ] {
            assert!(
                !confirmation_panel.contains(forbidden),
                "confirmation leaked {forbidden:?}, case={label}:\n{confirmation_panel}",
            );
        }
    }
}

#[test]
fn deleted_recreation_prior_display_key_is_bounded_or_fails_closed_at_every_width() {
    for (index, (label, prior_key, valid)) in [
        ("canonical same-normalized prior key", "Recreated Key", true),
        ("foreign normalized prior key", "FOREIGN_PRIOR_KEY", false),
        ("unsafe prior key", "UNSAFE_PRIOR\u{202e}_KEY", false),
        ("noncanonical prior key", "recreated  key", false),
    ]
    .into_iter()
    .enumerate()
    {
        let mut model = model_with_profile();
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Memory;
        bind_memory(&mut model);
        let profile = model
            .agents
            .detail
            .as_ref()
            .expect("profile detail")
            .profile
            .clone();
        let source = memory_entry(&model, 79_750, "Recreated Key");
        let tombstone = source
            .next_deleted(
                MemoryEntryVersionId::from_uuid(Uuid::from_u128(79_753)),
                Actor::Human,
                79_753,
                None,
                EventId::from_uuid(Uuid::from_u128(79_754)),
            )
            .expect("deleted seed");
        let candidate = MemoryEntryDraft::new(
            "recreated key".to_owned(),
            "R5_CANDIDATE_VALUE".to_owned(),
            vec!["recreated".to_owned()],
        )
        .expect("recreated candidate");
        let review = MemoryEditReview {
            profile: profile.reference(),
            namespace_id: profile.memory_namespace_id(),
            expected: ExpectedMemoryEntryState::Deleted(tombstone.reference()),
            operation: MemoryMutationKind::Set,
            candidate: Some(candidate.clone()),
            diff: deleted_set_diff_with_prior_key(&candidate, prior_key),
            plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            review_token: MemoryReviewToken::from_uuid(Uuid::from_u128(
                79_760 + u128::try_from(index).expect("case index"),
            )),
            review_digest: sha256(format!("R5 prior-key review {index}").as_bytes()),
        };
        install_create_set_review(&mut model, &review);

        for width in [60, 100, 140] {
            model.agents.memory.pane = MemoryPane::MutationReview;
            model.agents.memory.confirmation = None;
            let review_text = render_memory_detail_text(&model, width, 18);
            if valid {
                let normalized = normalized_panel_text(&review_text);
                assert!(
                    !review_text.contains("Memory review unavailable."),
                    "width={width}, case={label}:\n{review_text}",
                );
                for required in [
                    "Plaintext local memory",
                    "do not store",
                    "credentials",
                    "is retained after",
                    "overwrite or delete",
                    "Enter: continue",
                ] {
                    assert!(
                        normalized.contains(required),
                        "width={width} missing {required:?}:\n{review_text}",
                    );
                }
            } else {
                assert!(
                    review_text.contains("Memory review unavailable."),
                    "width={width}, case={label}:\n{review_text}",
                );
                for forbidden in [
                    "FOREIGN_PRIOR_KEY",
                    "UNSAFE_PRIOR",
                    "recreated  key",
                    "R5_CANDIDATE_VALUE",
                    "MUTATION DIFF",
                    MEMORY_PLAINTEXT_WARNING,
                    "Plaintext local memory",
                    "do not store",
                    "credentials",
                    "is retained after",
                    "overwrite or delete",
                    "Enter: continue",
                    &tombstone.reference().entry_id().to_string(),
                ] {
                    assert!(
                        !review_text.contains(forbidden),
                        "width={width}, case={label} leaked {forbidden:?}:\n{review_text}",
                    );
                }
            }

            model.agents.memory.confirmation = Some(MemoryConfirmation {
                command: set_review_command(&review),
                generation: model.agents.memory.generation,
            });
            model.agents.memory.pane = MemoryPane::Confirmation;
            let confirmation_text = render_memory_detail_text(&model, width, 18);
            if valid {
                let normalized = normalized_panel_text(&confirmation_text);
                assert!(
                    !confirmation_text.contains("Memory confirmation unavailable."),
                    "width={width}, case={label}:\n{confirmation_text}",
                );
                for required in [
                    "Plaintext local memory",
                    "do not store",
                    "credentials",
                    "is retained after",
                    "overwrite or delete",
                    "Enter: confirm",
                ] {
                    assert!(
                        normalized.contains(required),
                        "width={width} missing {required:?}:\n{confirmation_text}",
                    );
                }
            } else {
                assert!(
                    confirmation_text.contains("Memory confirmation unavailable."),
                    "width={width}, case={label}:\n{confirmation_text}",
                );
                for forbidden in [
                    "FOREIGN_PRIOR_KEY",
                    "UNSAFE_PRIOR",
                    "recreated  key",
                    "R5_CANDIDATE_VALUE",
                    "MUTATION DIFF",
                    MEMORY_PLAINTEXT_WARNING,
                    "Plaintext local memory",
                    "do not store",
                    "credentials",
                    "is retained after",
                    "overwrite or delete",
                    "Enter: confirm",
                    &tombstone.reference().entry_id().to_string(),
                ] {
                    assert!(
                        !confirmation_text.contains(forbidden),
                        "width={width}, case={label} leaked {forbidden:?}:\n{confirmation_text}",
                    );
                }
            }
        }
    }
}

#[test]
fn approve_and_reject_confirmations_require_exact_resolution_commands() {
    for (seed, action, label) in [
        (1_800, MemoryResolutionAction::Approve, "Approve"),
        (1_900, MemoryResolutionAction::Reject, "Reject"),
    ] {
        let mut model = model_with_profile();
        model.active_view = View::Agents;
        model.agents.pane = AgentsPane::Memory;
        bind_memory(&mut model);
        let proposal = proposal(&model, seed, "confirmed proposal");
        install_resolution_review(&mut model, proposal, action);
        model.agents.memory.generation = u64::try_from(seed).expect("generation");
        let command = resolution_review_command(
            model
                .agents
                .memory
                .resolution_review
                .as_ref()
                .expect("resolution review"),
        );
        model.agents.memory.confirmation = Some(MemoryConfirmation {
            command,
            generation: u64::try_from(seed).expect("generation"),
        });
        model.agents.memory.pane = MemoryPane::Confirmation;

        assert_minimum_warning_surface(&model, label);
        let text = render_text(&model, 140, 30);
        assert!(text.contains("Confirm proposal resolution"));
        assert!(text.contains("confirmed proposal"));
    }
}

#[test]
fn maximum_valid_keys_keep_fixed_review_and_confirmation_safety_rows_visible_at_minimum_size() {
    let maximum_key = "k".repeat(96);

    let mut set_review = model_with_profile();
    set_review.active_view = View::Agents;
    set_review.agents.pane = AgentsPane::Memory;
    bind_memory(&mut set_review);
    install_set_review(&mut set_review, &maximum_key);
    assert_maximum_key_fixed_surface(&set_review, "Set", "Entry ID", "New entry");

    let mut set_confirmation = set_review.clone();
    set_confirmation.agents.memory.generation = 31;
    let command = set_review_command(
        set_confirmation
            .agents
            .memory
            .edit_review
            .as_ref()
            .expect("set review"),
    );
    set_confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: 31,
    });
    set_confirmation.agents.memory.pane = MemoryPane::Confirmation;
    assert_maximum_key_fixed_surface(&set_confirmation, "Set", "Entry ID", "New entry");

    let mut delete_review = model_with_profile();
    delete_review.active_view = View::Agents;
    delete_review.agents.pane = AgentsPane::Memory;
    bind_memory(&mut delete_review);
    let entry = memory_entry(&delete_review, 1_950, &maximum_key);
    let entry_id = entry.reference().entry_id().to_string();
    install_delete_review(&mut delete_review, &entry);
    assert_maximum_key_fixed_surface(&delete_review, "Delete", "Entry ID", &entry_id);

    let mut delete_confirmation = delete_review.clone();
    delete_confirmation.agents.memory.generation = 32;
    let command = delete_review_command(
        delete_confirmation
            .agents
            .memory
            .edit_review
            .as_ref()
            .expect("delete review"),
    );
    delete_confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: 32,
    });
    delete_confirmation.agents.memory.pane = MemoryPane::Confirmation;
    assert_maximum_key_fixed_surface(&delete_confirmation, "Delete", "Entry ID", &entry_id);

    for (seed, action, operation) in [
        (1_960, MemoryResolutionAction::Approve, "Approve"),
        (1_970, MemoryResolutionAction::Reject, "Reject"),
    ] {
        let mut review = model_with_profile();
        review.active_view = View::Agents;
        review.agents.pane = AgentsPane::Memory;
        bind_memory(&mut review);
        let proposal = proposal(&review, seed, &maximum_key);
        let proposal_id = proposal.reference().proposal_id().to_string();
        install_resolution_review(&mut review, proposal, action);
        assert_maximum_key_fixed_surface(&review, operation, "Proposal ID", &proposal_id);

        let mut confirmation = review.clone();
        confirmation.agents.memory.generation = u64::try_from(seed).expect("generation");
        let command = resolution_review_command(
            confirmation
                .agents
                .memory
                .resolution_review
                .as_ref()
                .expect("resolution review"),
        );
        confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
            command,
            generation: u64::try_from(seed).expect("generation"),
        });
        confirmation.agents.memory.pane = MemoryPane::Confirmation;
        assert_maximum_key_fixed_surface(&confirmation, operation, "Proposal ID", &proposal_id);
    }
}

#[test]
fn every_memory_safety_surface_keeps_fixed_identity_warning_and_controls_at_eighteen_rows() {
    let maximum_key = "k".repeat(96);

    for completed_fields in 0..=3 {
        let mut editor_model = model_with_profile();
        editor_model.active_view = View::Agents;
        editor_model.agents.pane = AgentsPane::Memory;
        bind_memory(&mut editor_model);
        let profile_id = editor_model
            .agents
            .detail
            .as_ref()
            .expect("profile")
            .profile
            .profile_id();
        editor_model
            .agents
            .memory
            .open_create_editor(AgentProfileSelector::Id(profile_id))
            .expect("editor");
        let editor = editor_model.agents.memory.editor.as_mut().expect("editor");
        if completed_fields >= 1 {
            editor
                .submit_keyboard_line(&maximum_key)
                .expect("maximum key");
        }
        if completed_fields >= 2 {
            editor
                .submit_line("editor value".to_owned())
                .expect("editor value");
        }
        if completed_fields >= 3 {
            editor
                .submit_keyboard_line("risk, quality")
                .expect("editor tags");
            assert!(editor.apply_preview(
                editor.generation(),
                MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent),
            ));
        }
        assert_fixed_safety_surface_at_every_supported_pane_count(
            &editor_model,
            "Set",
            "New entry",
            if completed_fields == 0 {
                "New entry"
            } else {
                "kkkkkkkk"
            },
        );
    }

    let mut set_review = model_with_profile();
    set_review.active_view = View::Agents;
    set_review.agents.pane = AgentsPane::Memory;
    bind_memory(&mut set_review);
    install_set_review(&mut set_review, &maximum_key);
    assert_fixed_safety_surface_at_every_supported_pane_count(
        &set_review,
        "Set",
        "New entry",
        "kkkkkkkk",
    );

    let mut set_confirmation = set_review.clone();
    set_confirmation.agents.memory.generation = 80_000;
    let command = set_review_command(
        set_confirmation
            .agents
            .memory
            .edit_review
            .as_ref()
            .expect("set review"),
    );
    set_confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: 80_000,
    });
    set_confirmation.agents.memory.pane = MemoryPane::Confirmation;
    assert_fixed_safety_surface_at_every_supported_pane_count(
        &set_confirmation,
        "Set",
        "New entry",
        "kkkkkkkk",
    );

    let mut delete_review = model_with_profile();
    delete_review.active_view = View::Agents;
    delete_review.agents.pane = AgentsPane::Memory;
    bind_memory(&mut delete_review);
    let delete_entry = memory_entry(&delete_review, 80_100, &maximum_key);
    let delete_entry_id = delete_entry.reference().entry_id().to_string();
    install_delete_review(&mut delete_review, &delete_entry);
    assert_fixed_safety_surface_at_every_supported_pane_count(
        &delete_review,
        "Delete",
        &delete_entry_id,
        "kkkkkkkk",
    );

    let mut delete_confirmation = delete_review.clone();
    delete_confirmation.agents.memory.generation = 80_200;
    let command = delete_review_command(
        delete_confirmation
            .agents
            .memory
            .edit_review
            .as_ref()
            .expect("delete review"),
    );
    delete_confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: 80_200,
    });
    delete_confirmation.agents.memory.pane = MemoryPane::Confirmation;
    assert_fixed_safety_surface_at_every_supported_pane_count(
        &delete_confirmation,
        "Delete",
        &delete_entry_id,
        "kkkkkkkk",
    );

    for (seed, action, operation) in [
        (80_300, MemoryResolutionAction::Approve, "Approve"),
        (80_400, MemoryResolutionAction::Reject, "Reject"),
    ] {
        let mut review = model_with_profile();
        review.active_view = View::Agents;
        review.agents.pane = AgentsPane::Memory;
        bind_memory(&mut review);
        let proposal = proposal(&review, seed, &maximum_key);
        let proposal_id = proposal.reference().proposal_id().to_string();
        install_resolution_review(&mut review, proposal, action);
        assert_fixed_safety_surface_at_every_supported_pane_count(
            &review,
            operation,
            &proposal_id,
            "kkkkkkkk",
        );

        let mut confirmation = review.clone();
        confirmation.agents.memory.generation = u64::try_from(seed).expect("generation");
        let command = resolution_review_command(
            confirmation
                .agents
                .memory
                .resolution_review
                .as_ref()
                .expect("resolution review"),
        );
        confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
            command,
            generation: u64::try_from(seed).expect("generation"),
        });
        confirmation.agents.memory.pane = MemoryPane::Confirmation;
        assert_fixed_safety_surface_at_every_supported_pane_count(
            &confirmation,
            operation,
            &proposal_id,
            "kkkkkkkk",
        );
    }
}

#[test]
fn maximum_valid_keys_and_labels_keep_other_fixed_identity_surfaces_single_line() {
    let maximum_key = "k".repeat(96);

    let mut entry_model = model_with_profile();
    entry_model.active_view = View::Agents;
    entry_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut entry_model);
    let entry = memory_entry(&entry_model, 1_980, &maximum_key);
    let entry_id = entry.reference().entry_id().to_string();
    install_entries(
        &mut entry_model,
        vec![entry_summary(&entry, maximum_key.clone(), Vec::new())],
    );
    install_entry_detail(&mut entry_model, entry.clone());
    entry_model.agents.memory.pane = MemoryPane::EntryDetail;
    let rows = render_rows(&entry_model, 60, 18);
    let text = rows.join(" ");
    assert!(text.contains(&entry_id));
    assert!(text.contains("Esc: entries"));
    assert!(
        rows.iter()
            .find(|row| row.contains("Key            "))
            .expect("entry key row")
            .contains("...")
    );

    install_history(&mut entry_model, &entry, std::slice::from_ref(&entry));
    entry_model.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: entry_model
            .agents
            .detail
            .as_ref()
            .expect("profile")
            .profile
            .reference(),
        entry,
    });
    entry_model.agents.memory.pane = MemoryPane::EntryHistory;
    let rows = render_rows(&entry_model, 60, 18);
    let text = rows.join(" ");
    assert!(text.contains(&entry_id));
    assert!(text.contains("Esc: history"));
    assert!(
        rows.iter()
            .find(|row| row.contains("Key            "))
            .expect("history key row")
            .contains("...")
    );

    let mut proposal_model = model_with_profile();
    proposal_model.active_view = View::Agents;
    proposal_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut proposal_model);
    let proposal = proposal(&proposal_model, 1_990, &maximum_key);
    let proposal_id = proposal.reference().proposal_id().to_string();
    install_proposals(&mut proposal_model, std::slice::from_ref(&proposal));
    install_proposal_detail(&mut proposal_model, proposal);
    proposal_model.agents.memory.pane = MemoryPane::ProposalDetail;
    let rows = render_rows(&proposal_model, 60, 18);
    let text = rows.join(" ");
    assert!(text.contains(&proposal_id));
    assert!(text.contains("Esc: proposals"));
    assert!(
        rows.iter()
            .find(|row| row.contains("Key            "))
            .expect("proposal key row")
            .contains("...")
    );

    let mut episodic_model = model_with_profile();
    episodic_model.active_view = View::Agents;
    episodic_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut episodic_model);
    let maximum_label = "l".repeat(128);
    let summary = episodic_summary(&episodic_model, 1_995, &maximum_label);
    let summary_id = summary.reference().summary_id().to_string();
    install_episodes(
        &mut episodic_model,
        vec![episodic_list_item(&summary, maximum_label)],
    );
    install_episode_detail(&mut episodic_model, summary);
    episodic_model.agents.memory.pane = MemoryPane::EpisodicDetail;
    let rows = render_rows(&episodic_model, 60, 18);
    let text = rows.join(" ");
    assert!(text.contains(&summary_id));
    assert!(text.contains("Read-only | Esc: episodic summaries"));
    assert!(
        rows.iter()
            .find(|row| row.contains("Label          "))
            .expect("episodic label row")
            .contains("...")
    );
}

#[test]
fn maximum_key_is_display_bounded_in_the_editor_context_column() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let maximum_key = "k".repeat(96);
    let entry = memory_entry(&model, 1_999, &maximum_key);
    install_entries(
        &mut model,
        vec![entry_summary(&entry, maximum_key.clone(), Vec::new())],
    );
    install_entry_detail(&mut model, entry.clone());
    model
        .agents
        .memory
        .open_edit_editor(
            AgentProfileSelector::Id(
                model
                    .agents
                    .detail
                    .as_ref()
                    .expect("profile")
                    .profile
                    .profile_id(),
            ),
            entry,
        )
        .expect("edit editor");

    let rows = render_rows(&model, 100, 30);
    let context_key_row = rows
        .iter()
        .find(|row| row.contains("Key kkkkk"))
        .unwrap_or_else(|| panic!("editor context key must share one row with its bounded value"));
    assert!(
        context_key_row.contains("..."),
        "context key must truncate inside one display row: {context_key_row:?}"
    );
}

#[test]
fn proposal_resolution_result_is_generic_and_origin_specific() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    model.agents.memory.pane = MemoryPane::Result;
    model.agents.memory.result_origin = MemoryResultOrigin::Resolution;

    let text = render_text(&model, 60, 18);
    assert!(text.contains("Proposal resolution completed."));
    assert!(text.contains("Enter/Esc: return"));
    assert!(!text.contains(MEMORY_PLAINTEXT_WARNING));
    assert!(!text.contains("confirmed proposal"));
}

#[test]
fn episodic_list_and_adjacent_preview_are_metadata_only() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let summary = episodic_summary(&model, 2_000, "Quarterly reflection");
    install_episodes(
        &mut model,
        vec![episodic_list_item(
            &summary,
            "Quarterly reflection".to_owned(),
        )],
    );
    install_episode_detail(&mut model, summary);
    model.agents.memory.pane = MemoryPane::EpisodicSummaries;

    let text = render_text(&model, 140, 30);
    assert!(text.contains("Episodic summaries"));
    assert!(text.contains("Summary metadata"));
    assert!(text.contains("Quarterly reflection"));
    assert!(text.contains("Sources 1"));
    assert!(!text.contains("episodic body 2000"));
    assert!(!text.contains("thesis_reviewed_2000"));
}

#[test]
fn exact_episodic_detail_is_source_linked_read_only_and_adaptive() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let summary = episodic_summary(&model, 2_100, "Source-linked reflection");
    let source = summary.sources()[0].clone();
    install_episodes(
        &mut model,
        vec![episodic_list_item(
            &summary,
            "Source-linked reflection".to_owned(),
        )],
    );
    install_episode_detail(&mut model, summary);
    model.agents.memory.pane = MemoryPane::EpisodicDetail;

    let narrow = render_text(&model, 60, 30);
    for expected in [
        "Episodic detail",
        "Summary — verify sources",
        "episodic body 2100",
        "Read-only",
        "Esc",
    ] {
        assert!(narrow.contains(expected), "missing {expected:?}");
    }
    assert!(!narrow.contains("Episodic summaries"));

    let medium = render_text(&model, 100, 30);
    assert!(medium.contains("Episodic summaries"));
    assert!(medium.contains("Episodic detail"));

    let wide = render_text(&model, 300, 40);
    assert!(wide.contains("Episodic sources"));
    assert!(wide.contains("thesis_reviewed_2100"));
    assert!(wide.contains(&source.event_id().to_string()));
    assert!(wide.contains(&source.event_digest().to_string()));
}

#[test]
fn episodic_detail_never_exposes_mutation_or_resolution_controls() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let summary = episodic_summary(&model, 2_150, "read only boundary");
    install_episodes(
        &mut model,
        vec![episodic_list_item(&summary, summary.label().to_owned())],
    );
    install_episode_detail(&mut model, summary);
    model.agents.memory.pane = MemoryPane::EpisodicDetail;

    for (width, height) in [(60, 30), (100, 30), (300, 40)] {
        let text = render_text(&model, width, height);
        assert!(text.contains("Read-only"), "{width}x{height}");
        for forbidden in ["Edit", "Delete", "Approve", "Reject"] {
            assert!(
                !text.contains(forbidden),
                "{width}x{height} exposed {forbidden:?} on episodic detail"
            );
        }
    }
}

#[test]
fn mismatched_episodic_detail_fails_closed_without_body_or_sources() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let selected = episodic_summary(&model, 2_200, "Selected summary");
    let stale = episodic_summary(&model, 2_300, "Stale summary");
    install_episodes(
        &mut model,
        vec![episodic_list_item(&selected, "Selected summary".to_owned())],
    );
    install_episode_detail(&mut model, stale);
    model.agents.memory.pane = MemoryPane::EpisodicDetail;

    let text = render_text(&model, 100, 30);
    assert!(text.contains("Episodic detail unavailable"));
    assert!(!text.contains("episodic body 2300"));
    assert!(!text.contains("thesis_reviewed_2300"));
}

#[test]
fn episodic_rows_are_capped_before_rendering_and_extra_rows_are_omitted() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let summary = episodic_summary(&model, 2_400, "Reusable summary");
    let rows = (0..105)
        .map(|index| episodic_list_item(&summary, format!("Episode {index:03}")))
        .collect();
    install_episodes(&mut model, rows);
    model.agents.memory.pane = MemoryPane::EpisodicSummaries;

    let text = render_text(&model, 140, 30);
    assert!(text.contains("Displayed 100"));
    assert!(text.contains("Omitted 5"));
    assert!(!text.contains("Episode 104"));
}

#[test]
fn exact_eight_tags_and_128_sources_are_rendered_without_a_ninth_item() {
    let tags = (0..8)
        .map(|index| format!("boundary-tag-{index}"))
        .collect::<Vec<_>>();

    let mut entry_model = model_with_profile();
    entry_model.active_view = View::Agents;
    entry_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut entry_model);
    let namespace_id = entry_model.agents.memory.namespace_id.expect("namespace");
    let entry = MemoryEntryVersion::create_present(
        namespace_id,
        MemoryEntryId::from_uuid(Uuid::from_u128(2_500)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(2_501)),
        MemoryEntryDraft::new(
            "eight tag entry".to_owned(),
            "eight tag value".to_owned(),
            tags.clone(),
        )
        .expect("eight-tag draft"),
        Actor::Human,
        1_800_000_002_500,
        None,
        EventId::from_uuid(Uuid::from_u128(2_502)),
    )
    .expect("eight-tag entry");
    install_entries(
        &mut entry_model,
        vec![entry_summary(
            &entry,
            entry.display_key().to_owned(),
            entry.purpose_tags().to_vec(),
        )],
    );
    install_entry_detail(&mut entry_model, entry);
    entry_model.agents.memory.pane = MemoryPane::EntryDetail;
    let entry_text = render_text(&entry_model, 300, 80);
    for tag in &tags {
        assert!(entry_text.contains(tag), "entry detail missing {tag}");
    }
    assert!(!entry_text.contains("boundary-tag-8"));

    let mut proposal_model = model_with_profile();
    proposal_model.active_view = View::Agents;
    proposal_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut proposal_model);
    let profile = &proposal_model
        .agents
        .detail
        .as_ref()
        .expect("profile")
        .profile;
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(2_510)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                "eight tag proposal".to_owned(),
                "eight tag candidate".to_owned(),
                tags.clone(),
            )
            .expect("eight-tag candidate"),
        },
        "eight tag proposal".to_owned(),
        ExpectedMemoryEntryState::Absent,
        "eight tag rationale".to_owned(),
        1_800_000_002_510,
        EventId::from_uuid(Uuid::from_u128(2_511)),
        ApprovalId::from_uuid(Uuid::from_u128(2_512)),
    )
    .expect("eight-tag proposal");
    install_proposals(&mut proposal_model, std::slice::from_ref(&proposal));
    install_proposal_detail(&mut proposal_model, proposal);
    proposal_model.agents.memory.pane = MemoryPane::ProposalDetail;
    let proposal_text = render_text(&proposal_model, 300, 80);
    for tag in &tags {
        assert!(proposal_text.contains(tag), "proposal detail missing {tag}");
    }
    assert!(!proposal_text.contains("boundary-tag-8"));

    let mut episode_model = model_with_profile();
    episode_model.active_view = View::Agents;
    episode_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut episode_model);
    let profile = &episode_model
        .agents
        .detail
        .as_ref()
        .expect("profile")
        .profile;
    let sources = (1_u64..=128)
        .map(|sequence| {
            EpisodicSourceRef::new(
                sequence,
                EventId::from_uuid(Uuid::from_u128(3_000 + u128::from(sequence))),
                format!("boundary_event_{sequence:03}"),
                sha256(format!("boundary digest {sequence}").as_bytes()),
            )
            .expect("boundary source")
        })
        .collect::<Vec<_>>();
    let summary = EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::from_u128(2_520)),
        profile,
        "boundary summary".to_owned(),
        "boundary body".to_owned(),
        tags.clone(),
        sources,
        1_800_000_002_520,
        129,
        EventId::from_uuid(Uuid::from_u128(3_500)),
    )
    .expect("boundary summary");
    install_episodes(
        &mut episode_model,
        vec![episodic_list_item(&summary, summary.label().to_owned())],
    );
    install_episode_detail(&mut episode_model, summary);
    episode_model.agents.memory.pane = MemoryPane::EpisodicDetail;
    let episode_text = render_text(&episode_model, 300, 80);
    assert!(
        !episode_text.contains("Episodic detail unavailable"),
        "exact 128-source detail must authenticate"
    );
    for tag in &tags {
        assert!(episode_text.contains(tag), "episodic detail missing {tag}");
    }
    assert!(!episode_text.contains("boundary-tag-8"));
    assert!(episode_text.contains("boundary_event_001"));
    assert!(!episode_text.contains("boundary_event_129"));
    episode_model.agents.memory.detail_scroll = usize::MAX;
    let last_sources = render_text(&episode_model, 300, 80);
    assert!(last_sources.contains("boundary_event_128"));
    assert!(!last_sources.contains("boundary_event_129"));
}

#[test]
fn memory_preserves_exactly_the_nine_global_navigation_labels_and_shortcuts() {
    let text = render_text(&memory_model(140, 30), 140, 30);
    for label in [
        "1 Home",
        "2 Chat",
        "3 Agents",
        "4 Skills",
        "5 Connections",
        "6 Activity",
        "7 Setup",
        "8 Audit",
        "9 Help",
    ] {
        assert_eq!(text.matches(label).count(), 1, "label={label}");
    }
    assert!(!text.contains("7 Memory"));
    assert!(!text.contains("m Memory"));

    let cases = [
        ('1', View::Overview, false),
        ('2', View::Chat, false),
        ('3', View::Agents, false),
        ('4', View::Agents, true),
        ('5', View::Connections, false),
        ('6', View::Activity, false),
        ('7', View::Setup, false),
        ('8', View::Audit, false),
        ('9', View::Help, false),
    ];
    for (shortcut, expected_view, skills_active) in cases {
        let mut model = memory_model(140, 30);
        let _ = handle_event(&mut model, key(KeyCode::Char(shortcut)));
        assert_eq!(model.active_view, expected_view, "shortcut={shortcut}");
        assert_eq!(model.skills.active, skills_active, "shortcut={shortcut}");
    }

    for code in [
        KeyCode::Char('m'),
        KeyCode::Char('a'),
        KeyCode::Char('s'),
        KeyCode::Char('q'),
        KeyCode::F(7),
    ] {
        let mut model = memory_model(140, 30);
        let before = (model.active_view, model.skills.active, model.agents.pane);
        let _ = handle_event(&mut model, key(code));
        assert_eq!(
            (model.active_view, model.skills.active, model.agents.pane),
            before
        );
    }
}

#[test]
fn memory_owned_command_text_is_visibly_escaped_and_uses_the_escaped_cursor_width() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(
            model
                .agents
                .detail
                .as_ref()
                .expect("profile")
                .profile
                .profile_id(),
        ))
        .expect("open editor");
    model.command.ingest("A\u{202e}\u{e9}\\B");

    let flat = render_rows(&model, 100, 30).concat();
    assert!(flat.contains("A\\u{202e}\\u{e9}\\\\B"));
    for raw in ['\u{202e}', '\u{e9}'] {
        assert!(!flat.contains(raw), "raw command scalar leaked: {raw:?}");
    }
    assert_eq!(render_cursor(&model, 100, 30), Position::new(21, 26));

    model.command.clear();
    model.command.ingest("Multiline Key");
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert_eq!(
        handle_event(&mut model, TuiEvent::Paste("line1\nline2".to_owned())),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    let multiline = render_rows(&model, 100, 30).concat();
    assert!(multiline.contains("line1\\nline2"));

    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    let tags = (0..8)
        .map(|index| format!("tag-{index}-{}", "x".repeat(24)))
        .collect::<Vec<_>>()
        .join(", ");
    assert!(tags.len() > 128);
    model.command.ingest(&tags);
    let aggregate = render_rows(&model, 300, 30).concat();
    assert!(aggregate.contains("tag-0-xxxxxxxx"));
    assert!(aggregate.contains("tag-7-xxxxxxxx"));
}

#[test]
fn memory_cursor_in_the_middle_uses_the_visible_escaped_prefix_width() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let profile_id = model
        .agents
        .detail
        .as_ref()
        .expect("profile")
        .profile
        .profile_id();
    model
        .agents
        .memory
        .open_create_editor(AgentProfileSelector::Id(profile_id))
        .expect("open editor");
    model.command.ingest("escaped cursor key");
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Enter)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw,
    );
    assert_eq!(
        handle_event(&mut model, TuiEvent::Paste("A\n\u{202e}\u{e9}Z".to_owned()),),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw,
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Left)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw,
    );
    assert_eq!(
        handle_event(&mut model, key(KeyCode::Left)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw,
    );
    assert_eq!(model.command.prefix(), "A\n\u{202e}");

    let flat = render_rows(&model, 100, 30).concat();
    assert!(flat.contains("A\\n\\u{202e}|\\u{e9}Z"), "{flat}");
    for raw in ['\u{202e}', '\u{e9}'] {
        assert!(!flat.contains(raw), "raw scalar leaked: {raw:?}");
    }
    assert_eq!(
        render_cursor(&model, 100, 30),
        Position::new(14, 26),
        "cursor column must be prompt width 3 plus escaped prefix width 11",
    );
}

#[test]
fn help_explains_nested_memory_without_advertising_a_global_route_or_leaking_cache_prose() {
    let mut model = model_with_profile();
    bind_memory(&mut model);
    let entry = memory_entry(&model, 8_000, "PRIVATE_MEMORY_ROW");
    install_entries(
        &mut model,
        vec![entry_summary(
            &entry,
            "PRIVATE_MEMORY_ROW".to_owned(),
            vec!["PRIVATE_MEMORY_TAG".to_owned()],
        )],
    );
    model.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: model
            .agents
            .detail
            .as_ref()
            .expect("profile")
            .profile
            .reference(),
        entry,
    });

    model.active_view = View::Help;
    model.agents.pane = AgentsPane::List;
    let help = render_text(&model, 140, 40);
    assert!(help.contains("Agents detail"));
    assert!(help.contains("Memory"));
    for forbidden in [
        "m Memory",
        "7 Memory",
        "/memory",
        "PRIVATE_MEMORY_ROW",
        "PRIVATE_MEMORY_TAG",
    ] {
        assert!(
            !help.contains(forbidden),
            "help leaked or advertised {forbidden:?}"
        );
    }

    for (view, pane, skills) in [
        (View::Overview, AgentsPane::List, false),
        (View::Setup, AgentsPane::List, false),
        (View::Audit, AgentsPane::List, false),
        (View::Agents, AgentsPane::List, false),
        (View::Agents, AgentsPane::Memory, true),
    ] {
        model.active_view = view;
        model.agents.pane = pane;
        model.skills.active = skills;
        let text = render_text(&model, 140, 40);
        assert!(!text.contains("PRIVATE_MEMORY_ROW"));
        assert!(!text.contains("PRIVATE_MEMORY_TAG"));
        assert!(!text.contains(" Memory input "));
    }
}

#[test]
fn routine_views_and_skills_overlay_never_render_cached_memory_payloads() {
    const ENTRY_VALUE: &str = "ENTRY_VALUE_SENTINEL_91";
    const PROPOSAL_CANDIDATE: &str = "PROPOSAL_CANDIDATE_SENTINEL_92";
    const PROPOSAL_RATIONALE: &str = "PROPOSAL_RATIONALE_SENTINEL_93";
    const EPISODIC_BODY: &str = "EPISODIC_BODY_SENTINEL_94";
    const EPISODIC_SOURCE: &str = "EPISODIC_SOURCE_SENTINEL_95";

    let mut model = model_with_profile();
    bind_memory(&mut model);
    let profile = &model.agents.detail.as_ref().expect("profile").profile;
    let entry = MemoryEntryVersion::create_present(
        profile.memory_namespace_id(),
        MemoryEntryId::from_uuid(Uuid::from_u128(8_100)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(8_101)),
        MemoryEntryDraft::new(
            "routine leak entry".to_owned(),
            ENTRY_VALUE.to_owned(),
            Vec::new(),
        )
        .expect("entry draft"),
        Actor::Human,
        1_800_000_008_100,
        None,
        EventId::from_uuid(Uuid::from_u128(8_102)),
    )
    .expect("entry");
    let proposal = MemoryProposal::new(
        MemoryProposalId::from_uuid(Uuid::from_u128(8_110)),
        profile,
        &Actor::Agent(profile.profile_id()),
        MemoryProposalOperation::Set {
            candidate: MemoryEntryDraft::new(
                "routine leak proposal".to_owned(),
                PROPOSAL_CANDIDATE.to_owned(),
                Vec::new(),
            )
            .expect("proposal candidate"),
        },
        "routine leak proposal".to_owned(),
        ExpectedMemoryEntryState::Absent,
        PROPOSAL_RATIONALE.to_owned(),
        1_800_000_008_110,
        EventId::from_uuid(Uuid::from_u128(8_111)),
        ApprovalId::from_uuid(Uuid::from_u128(8_112)),
    )
    .expect("proposal");
    let source = EpisodicSourceRef::new(
        1,
        EventId::from_uuid(Uuid::from_u128(8_121)),
        EPISODIC_SOURCE.to_owned(),
        sha256(b"routine leak source"),
    )
    .expect("source");
    let summary = EpisodicSummary::new(
        EpisodicSummaryId::from_uuid(Uuid::from_u128(8_120)),
        profile,
        "routine leak episode".to_owned(),
        EPISODIC_BODY.to_owned(),
        Vec::new(),
        vec![source],
        1_800_000_008_120,
        2,
        EventId::from_uuid(Uuid::from_u128(8_122)),
    )
    .expect("summary");

    install_entries(
        &mut model,
        vec![entry_summary(
            &entry,
            entry.display_key().to_owned(),
            Vec::new(),
        )],
    );
    install_entry_detail(&mut model, entry);
    install_proposals(&mut model, std::slice::from_ref(&proposal));
    install_proposal_detail(&mut model, proposal);
    install_episodes(
        &mut model,
        vec![episodic_list_item(&summary, summary.label().to_owned())],
    );
    install_episode_detail(&mut model, summary);

    let cases = [
        (View::Overview, AgentsPane::List, false),
        (View::Setup, AgentsPane::List, false),
        (View::Audit, AgentsPane::List, false),
        (View::Help, AgentsPane::List, false),
        (View::Agents, AgentsPane::List, false),
        (View::Agents, AgentsPane::Detail, false),
        (View::Agents, AgentsPane::Memory, true),
    ];
    for (view, pane, skills) in cases {
        model.active_view = view;
        model.agents.pane = pane;
        model.skills.active = skills;
        let text = render_text(&model, 300, 60);
        for forbidden in [
            ENTRY_VALUE,
            PROPOSAL_CANDIDATE,
            PROPOSAL_RATIONALE,
            EPISODIC_BODY,
            EPISODIC_SOURCE,
        ] {
            assert!(
                !text.contains(forbidden),
                "view={view:?} pane={pane:?} skills={skills} leaked {forbidden}"
            );
        }
    }
}

#[test]
fn mismatched_memory_pages_and_reviews_fail_closed_without_cached_prose() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let entry = memory_entry(&model, 8_100, "MISMATCHED_ENTRY_PROSE");
    install_entries(
        &mut model,
        vec![entry_summary(
            &entry,
            "MISMATCHED_ENTRY_PROSE".to_owned(),
            Vec::new(),
        )],
    );
    let proposal = proposal(&model, 8_200, "MISMATCHED_PROPOSAL_PROSE");
    install_proposals(&mut model, std::slice::from_ref(&proposal));
    let episode = episodic_summary(&model, 8_300, "MISMATCHED_EPISODE_PROSE");
    install_episodes(
        &mut model,
        vec![episodic_list_item(
            &episode,
            "MISMATCHED_EPISODE_PROSE".to_owned(),
        )],
    );
    model.agents.memory.namespace_id = Some(MemoryNamespaceId::from_uuid(Uuid::from_u128(99_999)));

    for (pane, prose, unavailable) in [
        (
            MemoryPane::EntryList,
            "MISMATCHED_ENTRY_PROSE",
            "Memory entries unavailable",
        ),
        (
            MemoryPane::Proposals,
            "MISMATCHED_PROPOSAL_PROSE",
            "Memory proposals unavailable",
        ),
        (
            MemoryPane::EpisodicSummaries,
            "MISMATCHED_EPISODE_PROSE",
            "Episodic summaries unavailable",
        ),
    ] {
        model.agents.memory.pane = pane;
        let text = render_text(&model, 140, 30);
        assert!(text.contains(unavailable), "pane={pane:?}");
        assert!(!text.contains(prose), "pane={pane:?}");
    }

    model.agents.memory.namespace_id = model
        .agents
        .detail
        .as_ref()
        .map(|detail| detail.profile.memory_namespace_id());
    install_set_review(&mut model, "MISMATCHED_REVIEW_PROSE");
    model
        .agents
        .memory
        .edit_review
        .as_mut()
        .expect("review")
        .namespace_id = MemoryNamespaceId::from_uuid(Uuid::from_u128(88_888));
    let review = render_text(&model, 140, 30);
    assert!(review.contains("Memory review unavailable"));
    assert!(!review.contains("MISMATCHED_REVIEW_PROSE"));
}

#[test]
fn oversized_detail_offsets_keep_authenticated_identity_and_controls_fixed() {
    let mut entry_model = model_with_profile();
    entry_model.active_view = View::Agents;
    entry_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut entry_model);
    let entry = memory_entry(&entry_model, 8_400, "fixed entry");
    let entry_id = entry.reference().entry_id().to_string();
    install_entries(
        &mut entry_model,
        vec![entry_summary(&entry, "fixed entry".to_owned(), Vec::new())],
    );
    entry_model.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: entry_model
            .agents
            .memory
            .profile
            .as_ref()
            .expect("identity")
            .profile
            .clone(),
        entry: entry.clone(),
    });
    entry_model.agents.memory.pane = MemoryPane::EntryDetail;
    entry_model.agents.memory.detail_scroll = usize::MAX;
    let entry_text = render_text(&entry_model, 60, 18);
    assert!(entry_text.contains(&entry_id));
    assert!(entry_text.contains("Esc: entries"));

    let mut history_model = entry_model.clone();
    install_history(&mut history_model, &entry, std::slice::from_ref(&entry));
    history_model.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: history_model
            .agents
            .memory
            .profile
            .as_ref()
            .expect("identity")
            .profile
            .clone(),
        entry,
    });
    history_model.agents.memory.pane = MemoryPane::EntryHistory;
    let history_text = render_text(&history_model, 60, 18);
    assert!(history_text.contains(&entry_id));
    assert!(history_text.contains("Esc: history"));

    let mut proposal_model = model_with_profile();
    proposal_model.active_view = View::Agents;
    proposal_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut proposal_model);
    let proposal = proposal(&proposal_model, 8_500, "fixed proposal");
    let proposal_id = proposal.reference().proposal_id().to_string();
    install_proposals(&mut proposal_model, std::slice::from_ref(&proposal));
    install_proposal_detail(&mut proposal_model, proposal);
    proposal_model.agents.memory.pane = MemoryPane::ProposalDetail;
    proposal_model.agents.memory.detail_scroll = usize::MAX;
    let proposal_text = render_text(&proposal_model, 60, 18);
    assert!(proposal_text.contains(&proposal_id));
    assert!(proposal_text.contains("Esc: proposals"));

    let mut episode_model = model_with_profile();
    episode_model.active_view = View::Agents;
    episode_model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut episode_model);
    let episode = episodic_summary(&episode_model, 8_600, "fixed episode");
    let episode_id = episode.reference().summary_id().to_string();
    install_episodes(
        &mut episode_model,
        vec![episodic_list_item(&episode, "fixed episode".to_owned())],
    );
    install_episode_detail(&mut episode_model, episode);
    episode_model.agents.memory.pane = MemoryPane::EpisodicDetail;
    episode_model.agents.memory.detail_scroll = usize::MAX;
    let episode_text = render_text(&episode_model, 60, 18);
    assert!(episode_text.contains(&episode_id));
    assert!(episode_text.contains("Esc: episodic summaries"));
}

#[test]
fn invalid_selected_indices_fail_closed_without_highlighting_or_substituting_the_last_row() {
    let mut entries = model_with_profile();
    entries.active_view = View::Agents;
    entries.agents.pane = AgentsPane::Memory;
    bind_memory(&mut entries);
    let entry = memory_entry(&entries, 8_700, "INVALID_INDEX_ENTRY");
    install_entries(
        &mut entries,
        vec![entry_summary(
            &entry,
            "INVALID_INDEX_ENTRY".to_owned(),
            Vec::new(),
        )],
    );
    entries.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: entries
            .agents
            .memory
            .profile
            .as_ref()
            .expect("identity")
            .profile
            .clone(),
        entry: entry.clone(),
    });
    entries.agents.memory.selected_entry = usize::MAX;
    let entry_list = render_text(&entries, 100, 30);
    assert!(entry_list.contains("Entry metadata unavailable"));
    assert!(!entry_list.contains("> INVALID_INDEX_ENTRY"));
    entries.agents.memory.pane = MemoryPane::EntryDetail;
    let entry_detail = render_text(&entries, 60, 30);
    assert!(entry_detail.contains("Entry detail unavailable"));
    assert!(!entry_detail.contains("value 8700"));

    let mut history = model_with_profile();
    history.active_view = View::Agents;
    history.agents.pane = AgentsPane::Memory;
    bind_memory(&mut history);
    let historical = memory_entry(&history, 8_800, "INVALID_INDEX_HISTORY");
    install_entries(
        &mut history,
        vec![entry_summary(
            &historical,
            "INVALID_INDEX_HISTORY".to_owned(),
            Vec::new(),
        )],
    );
    install_history(&mut history, &historical, std::slice::from_ref(&historical));
    history.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: history
            .agents
            .memory
            .profile
            .as_ref()
            .expect("identity")
            .profile
            .clone(),
        entry: historical,
    });
    history.agents.memory.pane = MemoryPane::EntryHistory;
    history.agents.memory.selected_history_version = usize::MAX;
    let history_text = render_text(&history, 100, 30);
    assert!(history_text.contains("Historical version unavailable"));
    assert!(!history_text.contains("> v1 INVALID_INDEX_HISTORY"));

    let mut proposals = model_with_profile();
    proposals.active_view = View::Agents;
    proposals.agents.pane = AgentsPane::Memory;
    bind_memory(&mut proposals);
    let proposal = proposal(&proposals, 8_900, "INVALID_INDEX_PROPOSAL");
    install_proposals(&mut proposals, std::slice::from_ref(&proposal));
    install_proposal_detail(&mut proposals, proposal);
    proposals.agents.memory.pane = MemoryPane::Proposals;
    proposals.agents.memory.selected_proposal = usize::MAX;
    let proposal_list = render_text(&proposals, 100, 30);
    assert!(proposal_list.contains("Proposal metadata unavailable"));
    assert!(!proposal_list.contains("> INVALID_INDEX_PROPOSAL"));
    proposals.agents.memory.pane = MemoryPane::ProposalDetail;
    let proposal_detail = render_text(&proposals, 60, 30);
    assert!(proposal_detail.contains("Proposal detail unavailable"));
    assert!(!proposal_detail.contains("proposal rationale 8900"));

    let mut episodes = model_with_profile();
    episodes.active_view = View::Agents;
    episodes.agents.pane = AgentsPane::Memory;
    bind_memory(&mut episodes);
    let episode = episodic_summary(&episodes, 9_000, "INVALID_INDEX_EPISODE");
    install_episodes(
        &mut episodes,
        vec![episodic_list_item(
            &episode,
            "INVALID_INDEX_EPISODE".to_owned(),
        )],
    );
    install_episode_detail(&mut episodes, episode);
    episodes.agents.memory.pane = MemoryPane::EpisodicSummaries;
    episodes.agents.memory.selected_episode = usize::MAX;
    let episode_list = render_text(&episodes, 100, 30);
    assert!(episode_list.contains("Summary metadata unavailable"));
    assert!(!episode_list.contains("> INVALID_INDEX_EPISODE"));
    episodes.agents.memory.pane = MemoryPane::EpisodicDetail;
    let episode_detail = render_text(&episodes, 60, 30);
    assert!(episode_detail.contains("Episodic detail unavailable"));
    assert!(!episode_detail.contains("episodic body 9000"));
}

#[test]
fn oversized_list_offsets_fill_the_available_bounded_viewport() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let entry = memory_entry(&model, 9_100, "template");
    install_entries(
        &mut model,
        (0..10)
            .map(|index| entry_summary(&entry, format!("FILL_ROW_{index:02}"), Vec::new()))
            .collect(),
    );
    model.agents.memory.selected_entry = 9;
    model.agents.memory.entry_scroll = usize::MAX;

    let text = render_text(&model, 100, 30);
    assert!(text.contains("FILL_ROW_03"));
    assert!(text.contains("FILL_ROW_09"));
}

#[test]
fn wide_editor_review_confirmation_and_result_contexts_follow_the_pane_matrix() {
    let mut editor = model_with_profile();
    editor.active_view = View::Agents;
    editor.agents.pane = AgentsPane::Memory;
    bind_memory(&mut editor);
    let edited_entry = memory_entry(&editor, 9_200, "matrix editor entry");
    let edited_entry_id = edited_entry.reference().entry_id().to_string();
    install_entries(
        &mut editor,
        vec![entry_summary(
            &edited_entry,
            "matrix editor entry".to_owned(),
            Vec::new(),
        )],
    );
    editor.agents.memory.entry_detail = Some(MemoryEntryView {
        profile: editor
            .agents
            .memory
            .profile
            .as_ref()
            .expect("identity")
            .profile
            .clone(),
        entry: edited_entry.clone(),
    });
    editor
        .agents
        .memory
        .open_edit_editor(
            AgentProfileSelector::Id(
                editor
                    .agents
                    .detail
                    .as_ref()
                    .expect("profile")
                    .profile
                    .profile_id(),
            ),
            edited_entry,
        )
        .expect("edit editor");
    let editor_text = render_text(&editor, 300, 30);
    assert!(editor_text.contains("Editor context"));
    assert!(editor_text.contains("Origin Edit"));
    assert!(editor_text.contains(&edited_entry_id));

    let mut review = model_with_profile();
    review.active_view = View::Agents;
    review.agents.pane = AgentsPane::Memory;
    bind_memory(&mut review);
    install_set_review(&mut review, "matrix review key");
    review.agents.memory.detail_scroll = usize::MAX;
    let review_text = render_text(&review, 300, 30);
    assert!(review_text.contains("Mutation diff"));
    assert!(review_text.contains("Display key: missing -> matrix review key"));
    assert!(review_text.contains("Value: missing -> review value"));

    review.agents.memory.generation = 44;
    let command = set_review_command(review.agents.memory.edit_review.as_ref().expect("review"));
    review.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: 44,
    });
    review.agents.memory.pane = MemoryPane::Confirmation;
    let confirmation_text = render_text(&review, 300, 30);
    assert!(confirmation_text.contains("Retained mutation review"));
    assert!(confirmation_text.contains("Displayed diff rows 4"));
    assert!(confirmation_text.contains("Omitted diff rows 0"));

    let mut resolution = model_with_profile();
    resolution.active_view = View::Agents;
    resolution.agents.pane = AgentsPane::Memory;
    bind_memory(&mut resolution);
    let proposal = proposal(&resolution, 9_300, "matrix resolution key");
    let approval_id = proposal.approval_id().to_string();
    install_proposals(&mut resolution, std::slice::from_ref(&proposal));
    install_resolution_review(&mut resolution, proposal, MemoryResolutionAction::Approve);
    resolution.agents.memory.detail_scroll = usize::MAX;
    let resolution_text = render_text(&resolution, 300, 30);
    assert!(resolution_text.contains("Resolution target"));
    assert!(resolution_text.contains(&approval_id));
    assert!(resolution_text.contains("Expected entry Absent"));

    resolution.agents.memory.generation = 45;
    let command = resolution_review_command(
        resolution
            .agents
            .memory
            .resolution_review
            .as_ref()
            .expect("resolution review"),
    );
    resolution.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: 45,
    });
    resolution.agents.memory.pane = MemoryPane::Confirmation;
    let confirmation_text = render_text(&resolution, 300, 30);
    assert!(confirmation_text.contains("Retained resolution review"));
    assert!(confirmation_text.contains(&approval_id));

    for (origin, guidance) in [
        (MemoryResultOrigin::Mutation, "Reload memory entries"),
        (MemoryResultOrigin::Resolution, "Reload memory proposals"),
    ] {
        resolution.agents.memory.pane = MemoryPane::Result;
        resolution.agents.memory.result_origin = origin;
        let result = render_text(&resolution, 300, 30);
        assert!(result.contains("Result context"));
        assert!(result.contains(guidance));
        assert!(!result.contains("proposal value 9300"));
        assert!(!result.contains("proposal rationale 9300"));
    }
}

#[test]
fn same_profile_foreign_namespace_proposal_row_fails_closed_in_list_and_detail_rendering() {
    let mut model = model_with_profile();
    model.active_view = View::Agents;
    model.agents.pane = AgentsPane::Memory;
    bind_memory(&mut model);
    let proposal = proposal(&model, 9_500, "FOREIGN_NAMESPACE_PROPOSAL");
    install_proposals(&mut model, std::slice::from_ref(&proposal));
    install_proposal_detail(&mut model, proposal);
    let foreign_namespace = MemoryNamespaceId::from_uuid(Uuid::from_u128(9_599));
    let summaries = &mut model
        .agents
        .memory
        .proposals
        .as_mut()
        .expect("proposal page")
        .proposals;
    summaries[0] = with_proposal_summary_namespace(&summaries[0], foreign_namespace);

    model.agents.memory.pane = MemoryPane::Proposals;
    let list = render_text(&model, 100, 30);
    assert!(list.contains("Proposal metadata unavailable"));
    assert!(!list.contains("FOREIGN_NAMESPACE_PROPOSAL"));

    model.agents.memory.pane = MemoryPane::ProposalDetail;
    let detail = render_text(&model, 100, 30);
    assert!(detail.contains("Proposal detail unavailable"));
    assert!(!detail.contains("proposal rationale 9500"));
}

fn assert_exact_memory_panel_titles(
    case: &str,
    model: &TuiModel,
    narrow: &str,
    primary: &str,
    detail: &str,
    _merged_context: &str,
) {
    const TITLES: &[&str] = &[
        "Memory entries",
        "Entry metadata",
        "Entry detail",
        "Entry context",
        "Entry history",
        "History version",
        "Historical entry version",
        "Parent entry",
        "Memory context",
        "Memory proposals",
        "Proposal metadata",
        "Proposal context",
        "Proposal detail",
        "Proposal resolution review",
        "Resolution target",
        "Episodic summaries",
        "Summary metadata",
        "Episodic context",
        "Episodic detail",
        "Episodic sources",
        "Memory editor",
        "Editor context",
        "Mutation review",
        "Mutation diff",
        "Confirm memory change",
        "Retained mutation review",
        "Confirm proposal resolution",
        "Retained resolution review",
        "Memory result",
        "Result context",
    ];
    for (width, expected) in [
        (60, vec![narrow]),
        (100, vec![primary, detail]),
        (140, vec![primary, detail]),
    ] {
        let rendered = render_text(model, width, 30);
        for title in TITLES {
            assert_eq!(
                rendered.contains(&format!(" {title} ")),
                expected.contains(title),
                "case={case} width={width} title={title}\n{rendered}"
            );
        }
    }
}

#[test]
fn every_memory_state_uses_the_exact_narrow_medium_and_wide_pane_matrix() {
    let mut base = model_with_profile();
    base.active_view = View::Agents;
    base.agents.pane = AgentsPane::Memory;
    bind_memory(&mut base);

    let entry = memory_entry(&base, 9_600, "matrix entry");
    install_entries(
        &mut base,
        vec![entry_summary(
            &entry,
            entry.display_key().to_owned(),
            entry.purpose_tags().to_vec(),
        )],
    );
    install_entry_detail(&mut base, entry.clone());
    install_history(&mut base, &entry, std::slice::from_ref(&entry));

    let proposal = proposal(&base, 9_700, "matrix proposal");
    install_proposals(&mut base, std::slice::from_ref(&proposal));
    install_proposal_detail(&mut base, proposal.clone());

    let episode = episodic_summary(&base, 9_800, "matrix episode");
    install_episodes(
        &mut base,
        vec![episodic_list_item(&episode, episode.label().to_owned())],
    );
    install_episode_detail(&mut base, episode);

    let mut entry_list = base.clone();
    entry_list.agents.memory.pane = MemoryPane::EntryList;
    assert_exact_memory_panel_titles(
        "entry list",
        &entry_list,
        "Memory entries",
        "Memory entries",
        "Entry metadata",
        "Memory context",
    );

    let mut entry_detail = base.clone();
    entry_detail.agents.memory.pane = MemoryPane::EntryDetail;
    assert_exact_memory_panel_titles(
        "entry detail",
        &entry_detail,
        "Entry detail",
        "Memory entries",
        "Entry detail",
        "Entry context",
    );

    let mut history = base.clone();
    history.agents.memory.pane = MemoryPane::EntryHistory;
    history.agents.memory.entry_version = None;
    assert_exact_memory_panel_titles(
        "history list",
        &history,
        "Entry history",
        "Entry history",
        "History version",
        "Parent entry",
    );
    history.agents.memory.entry_version = Some(MemoryEntryVersionView {
        profile: history
            .agents
            .memory
            .profile
            .as_ref()
            .expect("profile identity")
            .profile
            .clone(),
        entry: entry.clone(),
    });
    assert_exact_memory_panel_titles(
        "history version",
        &history,
        "Historical entry version",
        "Entry history",
        "Historical entry version",
        "Parent entry",
    );

    let mut editor = base.clone();
    install_editor(&mut editor, 0, false);
    assert_exact_memory_panel_titles(
        "editor",
        &editor,
        "Memory editor",
        "Memory entries",
        "Memory editor",
        "Editor context",
    );

    let mut mutation_review = base.clone();
    install_set_review(&mut mutation_review, "matrix mutation");
    assert_exact_memory_panel_titles(
        "mutation review",
        &mutation_review,
        "Mutation review",
        "Memory entries",
        "Mutation review",
        "Mutation diff",
    );

    let mut mutation_confirmation = mutation_review.clone();
    let command = set_review_command(
        mutation_confirmation
            .agents
            .memory
            .edit_review
            .as_ref()
            .expect("mutation review"),
    );
    mutation_confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: mutation_confirmation.agents.memory.generation,
    });
    mutation_confirmation.agents.memory.pane = MemoryPane::Confirmation;
    assert_exact_memory_panel_titles(
        "mutation confirmation",
        &mutation_confirmation,
        "Confirm memory change",
        "Memory entries",
        "Confirm memory change",
        "Retained mutation review",
    );

    let mut mutation_result = base.clone();
    mutation_result.agents.memory.pane = MemoryPane::Result;
    mutation_result.agents.memory.result_origin = MemoryResultOrigin::Mutation;
    assert_exact_memory_panel_titles(
        "mutation result",
        &mutation_result,
        "Memory result",
        "Memory entries",
        "Memory result",
        "Result context",
    );

    let mut proposals = base.clone();
    proposals.agents.memory.pane = MemoryPane::Proposals;
    assert_exact_memory_panel_titles(
        "proposal list",
        &proposals,
        "Memory proposals",
        "Memory proposals",
        "Proposal metadata",
        "Proposal context",
    );

    let mut proposal_detail = base.clone();
    proposal_detail.agents.memory.pane = MemoryPane::ProposalDetail;
    assert_exact_memory_panel_titles(
        "proposal detail",
        &proposal_detail,
        "Proposal detail",
        "Memory proposals",
        "Proposal detail",
        "Proposal context",
    );

    let mut resolution_review = base.clone();
    install_resolution_review(
        &mut resolution_review,
        proposal,
        MemoryResolutionAction::Approve,
    );
    assert_exact_memory_panel_titles(
        "resolution review",
        &resolution_review,
        "Proposal resolution review",
        "Memory proposals",
        "Proposal resolution review",
        "Resolution target",
    );

    let mut resolution_confirmation = resolution_review.clone();
    let command = resolution_review_command(
        resolution_confirmation
            .agents
            .memory
            .resolution_review
            .as_ref()
            .expect("resolution review"),
    );
    resolution_confirmation.agents.memory.confirmation = Some(MemoryConfirmation {
        command,
        generation: resolution_confirmation.agents.memory.generation,
    });
    resolution_confirmation.agents.memory.pane = MemoryPane::Confirmation;
    assert_exact_memory_panel_titles(
        "resolution confirmation",
        &resolution_confirmation,
        "Confirm proposal resolution",
        "Memory proposals",
        "Confirm proposal resolution",
        "Retained resolution review",
    );

    let mut resolution_result = base.clone();
    resolution_result.agents.memory.pane = MemoryPane::Result;
    resolution_result.agents.memory.result_origin = MemoryResultOrigin::Resolution;
    assert_exact_memory_panel_titles(
        "resolution result",
        &resolution_result,
        "Memory result",
        "Memory proposals",
        "Memory result",
        "Result context",
    );

    let mut episodes = base.clone();
    episodes.agents.memory.pane = MemoryPane::EpisodicSummaries;
    assert_exact_memory_panel_titles(
        "episodic list",
        &episodes,
        "Episodic summaries",
        "Episodic summaries",
        "Summary metadata",
        "Episodic context",
    );

    let mut episode_detail = base;
    episode_detail.agents.memory.pane = MemoryPane::EpisodicDetail;
    assert_exact_memory_panel_titles(
        "episodic detail",
        &episode_detail,
        "Episodic detail",
        "Episodic summaries",
        "Episodic detail",
        "Episodic sources",
    );
}

fn assert_selected_marker_is_inside_primary_panel(model: &TuiModel) {
    let width = 100;
    let height = 30;
    let geometry = view_geometry_for_state(
        Rect::new(0, 0, width, height),
        View::Agents,
        model.inspector_open,
        true,
    );
    let nested = memory_workspace(
        geometry.cockpit.workspace,
        memory_layout_mode(geometry.cockpit.workspace),
    );
    let rows = render_rows(model, width, height);
    let marker_visible = rows.iter().any(|row| {
        row.chars()
            .skip(usize::from(nested.primary.x))
            .take(usize::from(nested.primary.width))
            .collect::<String>()
            .contains("│> ")
    });
    assert!(
        marker_visible,
        "selected marker escaped its reserved rows:\n{}",
        rows.join("\n")
    );
}

#[test]
fn long_escaped_list_labels_tags_and_profile_identity_keep_end_selection_visible() {
    let long = format!("{}{}", "\u{202e}".repeat(32), "X".repeat(512));
    let mut base = model_with_profile();
    base.active_view = View::Agents;
    base.agents.pane = AgentsPane::Memory;
    bind_memory(&mut base);
    base.agents
        .memory
        .profile
        .as_mut()
        .expect("memory identity")
        .display_name = long.clone();
    base.set_terminal_size(100, 30);
    let entry = memory_entry(&base, 9_900, "list template");

    let mut entries = base.clone();
    install_entries(
        &mut entries,
        (0..20)
            .map(|_| entry_summary(&entry, long.clone(), vec![long.clone(); 8]))
            .collect(),
    );
    entries.agents.memory.pane = MemoryPane::EntryList;
    assert_eq!(
        handle_event(&mut entries, key(KeyCode::End)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert_selected_marker_is_inside_primary_panel(&entries);

    let mut history = base.clone();
    install_entries(
        &mut history,
        vec![entry_summary(&entry, long.clone(), vec![long.clone(); 8])],
    );
    install_history(&mut history, &entry, &vec![entry.clone(); 20]);
    for version in &mut history
        .agents
        .memory
        .entry_history
        .as_mut()
        .expect("history")
        .versions
    {
        version.display_key = long.clone();
    }
    history.agents.memory.pane = MemoryPane::EntryHistory;
    assert_eq!(
        handle_event(&mut history, key(KeyCode::End)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert_selected_marker_is_inside_primary_panel(&history);

    let proposal = proposal(&base, 9_910, "proposal template");
    let mut proposals = base.clone();
    install_proposals(&mut proposals, &vec![proposal; 20]);
    for summary in &mut proposals
        .agents
        .memory
        .proposals
        .as_mut()
        .expect("proposals")
        .proposals
    {
        summary.display_key = long.clone();
    }
    proposals.agents.memory.pane = MemoryPane::Proposals;
    assert_eq!(
        handle_event(&mut proposals, key(KeyCode::End)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert_selected_marker_is_inside_primary_panel(&proposals);

    let episode = episodic_summary(&base, 9_920, "episode template");
    let mut episodes = base;
    install_episodes(
        &mut episodes,
        (0..20)
            .map(|_| {
                let mut item = episodic_list_item(&episode, long.clone());
                item.purpose_tags = vec![long.clone(); 8];
                item
            })
            .collect(),
    );
    episodes.agents.memory.pane = MemoryPane::EpisodicSummaries;
    assert_eq!(
        handle_event(&mut episodes, key(KeyCode::End)),
        ai_stock_forum::ui::tui::ControllerEffect::Redraw
    );
    assert_selected_marker_is_inside_primary_panel(&episodes);
}
