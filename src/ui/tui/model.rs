use std::collections::VecDeque;

use crate::{
    agents::ProfileTemplate,
    app::{
        AgentProfileHistoryView, AgentProfileVersionView, AgentProfileView, AgentProfilesView,
        ApplicationCommand, CommandView, DatabaseReadiness, EpisodicSummariesView,
        EpisodicSummaryView, MAX_INPUT_BYTES, MemoryEditPreview, MemoryEntriesView,
        MemoryEntryHistoryView, MemoryEntryVersionView, MemoryEntryView, MemoryProfileIdentityView,
        MemoryProposalResolutionReview, MemoryProposalView, MemoryProposalsView,
        PresentationSnapshot, ProcessGuardOwnership, SkillHistoryView, SkillView, SkillsView,
    },
    audit::AuditEntry,
    domain::{
        DomainError, EpisodicSummaryId, InstallationId, MemoryEntryId, MemoryEntryVersionId,
        MemoryNamespaceId, MemoryProposalId, ObjectVersion, SessionId,
    },
    memory::{
        EpisodicSummaryRef, ExpectedMemoryEntryState, MemoryEditReview, MemoryEntryDraft,
        MemoryEntryRef, MemoryEntryState, MemoryEntryVersion, MemoryField, MemoryFieldDiff,
        MemoryFieldValue, MemoryMutationKind, MemoryPlaintextAcknowledgement, MemoryProposalFilter,
        MemoryProposalOperation, MemoryProposalOperationKind, MemoryProposalRef,
        MemoryProposalStatus, MemoryResolutionAction,
    },
    setup::SetupStatus,
    skills::{SkillDraft, SkillVersionRef},
    ui::{
        memory_editor::{MemoryEditor, MemoryEditorStep},
        profile_editor::ProfileEditor,
        skill_editor::SkillEditor,
    },
};

pub const COMMAND_HISTORY_CAPACITY: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Overview,
    Chat,
    Connections,
    Activity,
    Setup,
    Audit,
    Help,
    Agents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentsPane {
    List,
    Detail,
    History,
    Editor,
    Confirmation,
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentDetailAction {
    AssignedSkills,
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPane {
    EntryList,
    EntryDetail,
    EntryHistory,
    Editor,
    MutationReview,
    Confirmation,
    Proposals,
    ProposalDetail,
    ProposalResolutionReview,
    EpisodicSummaries,
    EpisodicDetail,
    Result,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryEntryDetailAction {
    Edit,
    Delete,
    History,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryProposalDetailAction {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryEditorOrigin {
    Create,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryResultOrigin {
    Mutation,
    Resolution,
}

pub(crate) const MEMORY_RETAINED_ROW_CAP: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MemoryPageCounts {
    pub displayed: usize,
    pub omitted: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryConfirmation {
    pub command: ApplicationCommand,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryOutcomeIntent {
    Entries,
    EntryDetail(MemoryEntryId),
    EntryHistory(MemoryEntryId),
    EntryVersion {
        selector: crate::app::AgentProfileSelector,
        key: String,
        version: ObjectVersion,
        entry_version_id: MemoryEntryVersionId,
    },
    Proposals,
    ProposalDetail(MemoryProposalId),
    Episodes,
    EpisodeDetail(EpisodicSummaryId),
    Mutation,
    Resolution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryViewState {
    pub profile: Option<MemoryProfileIdentityView>,
    pub namespace_id: Option<MemoryNamespaceId>,
    pub pane: MemoryPane,
    pub entries: Option<MemoryEntriesView>,
    pub entry_detail: Option<MemoryEntryView>,
    pub entry_history: Option<MemoryEntryHistoryView>,
    pub entry_version: Option<MemoryEntryVersionView>,
    pub proposals: Option<MemoryProposalsView>,
    pub proposal_detail: Option<MemoryProposalView>,
    pub episodes: Option<EpisodicSummariesView>,
    pub episode_detail: Option<EpisodicSummaryView>,
    pub selected_entry: usize,
    pub selected_history_version: usize,
    pub selected_proposal: usize,
    pub selected_episode: usize,
    pub selected_entry_detail_action: MemoryEntryDetailAction,
    pub selected_proposal_detail_action: MemoryProposalDetailAction,
    pub entry_scroll: usize,
    pub detail_scroll: usize,
    pub history_scroll: usize,
    pub proposal_scroll: usize,
    pub episode_scroll: usize,
    pub editor: Option<MemoryEditor>,
    pub editor_origin: MemoryEditorOrigin,
    pub edit_review: Option<MemoryEditReview>,
    pub resolution_review: Option<MemoryProposalResolutionReview>,
    pub confirmation: Option<MemoryConfirmation>,
    pub review_registered: bool,
    pub result_origin: MemoryResultOrigin,
    pub generation: u64,
    pub pending_intent: Option<MemoryOutcomeIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MemoryExactDetailIdentity {
    Entry(MemoryEntryRef),
    HistoricalEntry(MemoryEntryRef),
    Proposal(MemoryProposalRef),
    Episodic(EpisodicSummaryRef),
}

impl Default for MemoryViewState {
    fn default() -> Self {
        Self {
            profile: None,
            namespace_id: None,
            pane: MemoryPane::EntryList,
            entries: None,
            entry_detail: None,
            entry_history: None,
            entry_version: None,
            proposals: None,
            proposal_detail: None,
            episodes: None,
            episode_detail: None,
            selected_entry: 0,
            selected_history_version: 0,
            selected_proposal: 0,
            selected_episode: 0,
            selected_entry_detail_action: MemoryEntryDetailAction::Edit,
            selected_proposal_detail_action: MemoryProposalDetailAction::Approve,
            entry_scroll: 0,
            detail_scroll: 0,
            history_scroll: 0,
            proposal_scroll: 0,
            episode_scroll: 0,
            editor: None,
            editor_origin: MemoryEditorOrigin::Create,
            edit_review: None,
            resolution_review: None,
            confirmation: None,
            review_registered: false,
            result_origin: MemoryResultOrigin::Mutation,
            generation: 0,
            pending_intent: None,
        }
    }
}

impl MemoryViewState {
    pub(crate) fn transition_to(&mut self, pane: MemoryPane) {
        if self.pane != pane {
            self.pane = pane;
            self.detail_scroll = 0;
        }
    }

    fn exact_detail_identity(&self) -> Option<MemoryExactDetailIdentity> {
        match self.pane {
            MemoryPane::EntryDetail => self
                .entry_detail
                .as_ref()
                .map(|detail| MemoryExactDetailIdentity::Entry(detail.entry.reference())),
            MemoryPane::EntryHistory => self
                .entry_version
                .as_ref()
                .map(|detail| MemoryExactDetailIdentity::HistoricalEntry(detail.entry.reference())),
            MemoryPane::ProposalDetail => self
                .proposal_detail
                .as_ref()
                .map(|detail| MemoryExactDetailIdentity::Proposal(detail.proposal.reference())),
            MemoryPane::EpisodicDetail => self
                .episode_detail
                .as_ref()
                .map(|detail| MemoryExactDetailIdentity::Episodic(detail.summary.reference())),
            MemoryPane::EntryList
            | MemoryPane::Editor
            | MemoryPane::MutationReview
            | MemoryPane::Confirmation
            | MemoryPane::Proposals
            | MemoryPane::ProposalResolutionReview
            | MemoryPane::EpisodicSummaries
            | MemoryPane::Result => None,
        }
    }

    pub fn has_protected_workflow(&self) -> bool {
        self.editor.is_some()
            || self.edit_review.is_some()
            || self.resolution_review.is_some()
            || self.confirmation.is_some()
            || self.review_registered
    }

    pub fn bind_profile(
        &mut self,
        profile: MemoryProfileIdentityView,
        namespace_id: MemoryNamespaceId,
    ) -> Result<(), DomainError> {
        let unchanged =
            self.profile.as_ref() == Some(&profile) && self.namespace_id == Some(namespace_id);
        if unchanged {
            return Ok(());
        }
        if self.has_protected_workflow() {
            return Err(DomainError::MemoryReviewUnavailable);
        }
        self.invalidate_profile_context();
        self.profile = Some(profile);
        self.namespace_id = Some(namespace_id);
        Ok(())
    }

    fn invalidate_profile_context(&mut self) {
        let generation = self.generation;
        *self = Self {
            generation,
            ..Self::default()
        };
    }

    pub fn open_create_editor(
        &mut self,
        selector: crate::app::AgentProfileSelector,
    ) -> Result<(), DomainError> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(DomainError::MemoryGenerationOverflow)?;
        if self.has_protected_workflow() {
            return Err(DomainError::MemoryReviewUnavailable);
        }
        self.generation = generation;
        self.editor = Some(MemoryEditor::for_create(selector));
        self.editor_origin = MemoryEditorOrigin::Create;
        self.edit_review = None;
        self.resolution_review = None;
        self.confirmation = None;
        self.review_registered = false;
        self.pending_intent = None;
        self.transition_to(MemoryPane::Editor);
        Ok(())
    }

    pub fn open_edit_editor(
        &mut self,
        selector: crate::app::AgentProfileSelector,
        entry: crate::memory::MemoryEntryVersion,
    ) -> Result<(), DomainError> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(DomainError::MemoryGenerationOverflow)?;
        if self.has_protected_workflow() {
            return Err(DomainError::MemoryReviewUnavailable);
        }
        let editor = MemoryEditor::for_set(selector, entry)?;
        self.generation = generation;
        self.editor = Some(editor);
        self.editor_origin = MemoryEditorOrigin::Edit;
        self.edit_review = None;
        self.resolution_review = None;
        self.confirmation = None;
        self.review_registered = false;
        self.pending_intent = None;
        self.transition_to(MemoryPane::Editor);
        Ok(())
    }

    pub fn begin_review_request(&mut self) -> Result<u64, DomainError> {
        let generation = self.next_review_request_generation()?;
        self.generation = generation;
        self.pending_intent = None;
        Ok(generation)
    }

    pub(crate) fn next_review_request_generation(&self) -> Result<u64, DomainError> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(DomainError::MemoryGenerationOverflow)?;
        if self.edit_review.is_some()
            || self.resolution_review.is_some()
            || self.confirmation.is_some()
            || self.review_registered
        {
            return Err(DomainError::MemoryReviewUnavailable);
        }
        Ok(generation)
    }

    pub(crate) fn profile_selector(&self) -> Result<crate::app::AgentProfileSelector, DomainError> {
        self.profile
            .as_ref()
            .map(|identity| identity.profile.profile_id().into())
            .ok_or(DomainError::MemorySelectionUnavailable)
    }

    pub(crate) fn authenticated_edit_review(&self) -> Option<&MemoryEditReview> {
        let review = self.edit_review.as_ref()?;
        if !self.review_registered
            || self.resolution_review.is_some()
            || self.profile.as_ref().map(|identity| &identity.profile) != Some(&review.profile)
            || self.namespace_id != Some(review.namespace_id)
            || review.plaintext_acknowledgement
                != MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1
            || !canonical_memory_edit_diff(&review.diff)
        {
            return None;
        }

        match (&review.operation, &review.expected, &review.candidate) {
            (MemoryMutationKind::Set, expected, Some(candidate)) => {
                if !expected_state_matches_namespace_key(
                    expected,
                    review.namespace_id,
                    &candidate.normalized_key(),
                ) || !self.editor_is_authenticated()
                {
                    return None;
                }
                let editor = self.editor.as_ref()?;
                if editor.step() != MemoryEditorStep::Review
                    || !matches!(
                        editor.preview(),
                        Some(MemoryEditPreview::Review(retained)) if retained == review
                    )
                {
                    return None;
                }
                let request = editor.preview_request().ok()?;
                if request.selector != *editor.selector()
                    || !self.matches_profile_selector(&request.selector)
                    || request.candidate != *candidate
                {
                    return None;
                }
                let exact_diff = match (self.editor_origin, editor.seed(), expected) {
                    (MemoryEditorOrigin::Create, None, ExpectedMemoryEntryState::Absent) => {
                        absent_set_diff_matches(&review.diff, candidate)
                    }
                    (MemoryEditorOrigin::Create, None, ExpectedMemoryEntryState::Deleted(_)) => {
                        deleted_set_diff_matches(&review.diff, candidate)
                    }
                    (
                        MemoryEditorOrigin::Edit,
                        Some(seed),
                        ExpectedMemoryEntryState::Present(reference),
                    ) if reference == &seed.reference() => seeded_present_set_diff(seed, candidate)
                        .is_some_and(|expected_diff| review.diff == expected_diff),
                    _ => false,
                };
                exact_diff.then_some(review)
            }
            (MemoryMutationKind::Delete, ExpectedMemoryEntryState::Present(expected), None) => {
                if self.editor.is_some()
                    || !expected_state_matches_namespace_key(
                        &review.expected,
                        review.namespace_id,
                        expected.normalized_key(),
                    )
                {
                    return None;
                }
                let detail = self.authenticated_entry_detail()?;
                (*expected == detail.entry.reference()
                    && delete_diff_matches(&review.diff, &detail.entry))
                .then_some(review)
            }
            _ => None,
        }
    }

    pub(crate) fn edit_review_command(&self) -> Option<ApplicationCommand> {
        let review = self.authenticated_edit_review()?;
        match (&review.operation, &review.expected, &review.candidate) {
            (MemoryMutationKind::Set, expected, Some(candidate)) => {
                Some(ApplicationCommand::SetMemoryEntry {
                    profile: review.profile.clone(),
                    expected: expected.clone(),
                    candidate: candidate.clone(),
                    review_token: review.review_token,
                    review_digest: review.review_digest.clone(),
                })
            }
            (MemoryMutationKind::Delete, ExpectedMemoryEntryState::Present(expected), None) => {
                Some(ApplicationCommand::DeleteMemoryEntry {
                    profile: review.profile.clone(),
                    expected: expected.clone(),
                    review_token: review.review_token,
                    review_digest: review.review_digest.clone(),
                })
            }
            _ => None,
        }
    }

    fn reconstructed_resolution_review_command(&self) -> Option<ApplicationCommand> {
        let review = self.resolution_review.as_ref()?;
        let expected_action = match review.action {
            MemoryResolutionAction::Approve => MemoryProposalDetailAction::Approve,
            MemoryResolutionAction::Reject => MemoryProposalDetailAction::Reject,
        };
        if !self.review_registered
            || self.edit_review.is_some()
            || self.selected_proposal_detail_action != expected_action
            || !self.matches_profile_namespace(
                &review.namespace_owner_identity.profile,
                review.proposal.namespace_id(),
            )
        {
            return None;
        }
        let fields = (
            review.proposal.reference(),
            review.approval_id,
            review.expected_approval_status,
            review.expected_entry.clone(),
            review.review_token,
            review.review_digest.clone(),
        );
        Some(match review.action {
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
        })
    }

    pub(crate) fn authenticated_resolution_review(
        &self,
    ) -> Option<&MemoryProposalResolutionReview> {
        let command = self.reconstructed_resolution_review_command()?;
        let review = self.resolution_review.as_ref()?;
        let expected_proposal = match command {
            ApplicationCommand::ApproveMemoryProposal { proposal, .. }
                if review.action == MemoryResolutionAction::Approve =>
            {
                proposal
            }
            ApplicationCommand::RejectMemoryProposal { proposal, .. }
                if review.action == MemoryResolutionAction::Reject =>
            {
                proposal
            }
            _ => return None,
        };
        let owner = self.profile.as_ref()?;
        (review.proposal.reference() == expected_proposal
            && review.proposal.proposer() == &review.proposer_identity.profile
            && review.namespace_owner_identity.profile == owner.profile
            && review.proposal.namespace_id() == self.namespace_id?
            && review.proposal.proposer().profile_id()
                == review.namespace_owner_identity.profile.profile_id()
            && review.proposer_is_historical
                == (review.proposer_identity.profile != review.namespace_owner_identity.profile)
            && review.approval_id == review.proposal.approval_id()
            && review.expected_approval_status == crate::policy::ApprovalStatus::Pending
            && review.plaintext_acknowledgement
                == MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1
            && expected_state_matches_namespace_key(
                &review.expected_entry,
                review.proposal.namespace_id(),
                review.proposal.normalized_key(),
            ))
        .then_some(review)
    }

    pub(crate) fn resolution_review_command(&self) -> Option<ApplicationCommand> {
        self.authenticated_resolution_review()?;
        self.reconstructed_resolution_review_command()
    }

    pub(crate) fn confirmed_command(&self) -> Option<ApplicationCommand> {
        let confirmation = self.confirmation.as_ref()?;
        if confirmation.generation != self.generation {
            return None;
        }
        let reconstructed = if self.edit_review.is_some() {
            self.edit_review_command()
        } else {
            self.resolution_review_command()
        }?;
        (confirmation.command == reconstructed).then_some(reconstructed)
    }

    pub(crate) fn editor_is_authenticated(&self) -> bool {
        let Some(namespace_id) = self.namespace_id else {
            return false;
        };
        let Some(editor) = self.editor.as_ref() else {
            return false;
        };
        if !self.matches_profile_selector(editor.selector()) {
            return false;
        }
        match (self.editor_origin, editor.seed()) {
            (MemoryEditorOrigin::Create, None) => true,
            (MemoryEditorOrigin::Edit, Some(seed)) => {
                let reference = seed.reference();
                reference.state() == MemoryEntryState::Present
                    && seed.value().is_some()
                    && reference.namespace_id() == namespace_id
                    && crate::memory::NormalizedMemoryKey::new(editor.key_input())
                        .is_ok_and(|key| &key == reference.normalized_key())
            }
            (MemoryEditorOrigin::Create, Some(_)) | (MemoryEditorOrigin::Edit, None) => false,
        }
    }

    pub fn begin_pending(&mut self, intent: MemoryOutcomeIntent) -> Result<u64, DomainError> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(DomainError::MemoryGenerationOverflow)?;
        self.generation = generation;
        self.pending_intent = Some(intent);
        Ok(generation)
    }

    pub fn clear_pending(&mut self) {
        self.pending_intent = None;
    }

    pub(crate) fn clear_pending_if(
        &mut self,
        intent: &MemoryOutcomeIntent,
        generation: u64,
    ) -> bool {
        if self.generation != generation || self.pending_intent.as_ref() != Some(intent) {
            return false;
        }
        self.pending_intent = None;
        true
    }

    pub fn selected_entry_id(&self) -> Result<MemoryEntryId, DomainError> {
        self.authenticated_selected_entry()
            .map(|summary| summary.entry.entry_id())
            .ok_or(DomainError::MemorySelectionUnavailable)
    }

    pub(crate) fn authenticated_entries(&self) -> Option<(&MemoryEntriesView, MemoryPageCounts)> {
        let page = self.entries.as_ref()?;
        if !self.matches_profile_namespace(&page.profile, page.namespace_id)
            || u64::try_from(page.entries.len()).ok()? != page.returned_count
            || page.returned_count.checked_add(page.omitted_count) != Some(page.total_count)
        {
            return None;
        }
        let displayed = page.entries.len().min(MEMORY_RETAINED_ROW_CAP);
        if page.entries.iter().take(displayed).any(|summary| {
            summary.entry.namespace_id() != page.namespace_id
                || summary.entry.state() != MemoryEntryState::Present
        }) {
            return None;
        }
        Some((
            page,
            page_counts(
                page.entries.len(),
                displayed,
                page.omitted_count,
                page.total_count,
            ),
        ))
    }

    pub(crate) fn authenticated_selected_entry(&self) -> Option<&crate::app::MemoryEntrySummary> {
        let (page, counts) = self.authenticated_entries()?;
        page.entries
            .get(self.selected_entry)
            .filter(|_| self.selected_entry < counts.displayed)
    }

    pub(crate) fn authenticated_entry_detail(&self) -> Option<&MemoryEntryView> {
        let selected = self.authenticated_selected_entry()?;
        let detail = self.entry_detail.as_ref()?;
        let reference = detail.entry.reference();
        (self.profile.as_ref().map(|identity| &identity.profile) == Some(&detail.profile)
            && self.namespace_id == Some(reference.namespace_id())
            && reference.entry_id() == selected.entry.entry_id())
        .then_some(detail)
    }

    pub(crate) fn authenticated_history(
        &self,
    ) -> Option<(&MemoryEntryHistoryView, MemoryPageCounts)> {
        let selected_entry = self.authenticated_selected_entry()?;
        let history = self.entry_history.as_ref()?;
        if self.profile.as_ref().map(|identity| &identity.profile) != Some(&history.profile)
            || self.namespace_id != Some(history.current.namespace_id())
            || history.current.entry_id() != selected_entry.entry.entry_id()
            || u64::try_from(history.versions.len()).ok()? != history.returned_count
            || history.returned_count.checked_add(history.omitted_count)
                != Some(history.total_count)
        {
            return None;
        }
        let displayed = history.versions.len().min(MEMORY_RETAINED_ROW_CAP);
        if history.versions.iter().take(displayed).any(|version| {
            version.entry.namespace_id() != history.current.namespace_id()
                || version.entry.entry_id() != history.current.entry_id()
        }) {
            return None;
        }
        Some((
            history,
            page_counts(
                history.versions.len(),
                displayed,
                history.omitted_count,
                history.total_count,
            ),
        ))
    }

    pub(crate) fn authenticated_selected_history(
        &self,
    ) -> Option<&crate::app::MemoryEntryHistorySummary> {
        let (history, counts) = self.authenticated_history()?;
        history
            .versions
            .get(self.selected_history_version)
            .filter(|_| self.selected_history_version < counts.displayed)
    }

    pub(crate) fn authenticated_history_version(
        &self,
    ) -> Result<Option<&MemoryEntryVersionView>, ()> {
        let Some(detail) = self.entry_version.as_ref() else {
            return Ok(None);
        };
        let (history, _) = self.authenticated_history().ok_or(())?;
        let selected = self.authenticated_selected_history().ok_or(())?;
        let reference = detail.entry.reference();
        if self.profile.as_ref().map(|identity| &identity.profile) != Some(&detail.profile)
            || detail.profile != history.profile
            || self.namespace_id != Some(reference.namespace_id())
            || reference.namespace_id() != history.current.namespace_id()
            || reference.entry_id() != history.current.entry_id()
            || reference.entry_version_id() != selected.entry.entry_version_id()
        {
            return Err(());
        }
        Ok(Some(detail))
    }

    pub(crate) fn authenticated_proposals(
        &self,
    ) -> Option<(&MemoryProposalsView, MemoryPageCounts)> {
        let page = self.proposals.as_ref()?;
        if !self.matches_profile_namespace(&page.profile, page.namespace_id)
            || u64::try_from(page.proposals.len()).ok()? != page.returned_count
            || page.returned_count.checked_add(page.omitted_count) != Some(page.total_count)
        {
            return None;
        }
        let displayed = page.proposals.len().min(MEMORY_RETAINED_ROW_CAP);
        if page.proposals.iter().take(displayed).any(|summary| {
            summary.namespace_id != page.namespace_id
                || summary.proposer.profile_id() != page.profile.profile_id()
                || (page.filter == MemoryProposalFilter::Pending
                    && summary.status != MemoryProposalStatus::Pending)
        }) {
            return None;
        }
        Some((
            page,
            page_counts(
                page.proposals.len(),
                displayed,
                page.omitted_count,
                page.total_count,
            ),
        ))
    }

    pub(crate) fn authenticated_selected_proposal(
        &self,
    ) -> Option<&crate::app::MemoryProposalSummary> {
        let (page, counts) = self.authenticated_proposals()?;
        page.proposals
            .get(self.selected_proposal)
            .filter(|_| self.selected_proposal < counts.displayed)
    }

    pub(crate) fn authenticated_proposal_detail(&self) -> Option<&MemoryProposalView> {
        let summary = self.authenticated_selected_proposal()?;
        let detail = self.proposal_detail.as_ref()?;
        let proposal = &detail.proposal;
        let reference = proposal.reference();
        if reference != summary.proposal
            || self.namespace_id != Some(proposal.namespace_id())
            || proposal.proposer() != &detail.proposer_identity.profile
            || self.profile.as_ref().map(|identity| &identity.profile)
                != Some(&detail.namespace_owner_identity.profile)
            || proposal.proposer().profile_id()
                != detail.namespace_owner_identity.profile.profile_id()
            || detail.status != summary.status
            || proposal_operation_kind(proposal.operation()) != summary.operation
            || !expected_state_matches_namespace_key(
                &detail.current_entry,
                proposal.namespace_id(),
                proposal.normalized_key(),
            )
            || detail
                .resolution
                .as_ref()
                .is_some_and(|resolution| resolution.proposal() != &reference)
        {
            return None;
        }
        Some(detail)
    }

    pub(crate) fn authenticated_episodes(
        &self,
    ) -> Option<(&EpisodicSummariesView, MemoryPageCounts)> {
        let page = self.episodes.as_ref()?;
        if !self.matches_profile_namespace(&page.profile, page.namespace_id)
            || u64::try_from(page.summaries.len()).ok()? != page.returned_count
            || page.returned_count.checked_add(page.omitted_count) != Some(page.total_count)
        {
            return None;
        }
        let displayed = page.summaries.len().min(MEMORY_RETAINED_ROW_CAP);
        if page.summaries.iter().take(displayed).any(|summary| {
            summary.summary.namespace_id() != page.namespace_id
                || summary.summary.profile() != &page.profile
                || summary.source_count > 128
        }) {
            return None;
        }
        Some((
            page,
            page_counts(
                page.summaries.len(),
                displayed,
                page.omitted_count,
                page.total_count,
            ),
        ))
    }

    pub(crate) fn authenticated_selected_episode(
        &self,
    ) -> Option<&crate::app::EpisodicSummaryListItem> {
        let (page, counts) = self.authenticated_episodes()?;
        page.summaries
            .get(self.selected_episode)
            .filter(|_| self.selected_episode < counts.displayed)
    }

    pub(crate) fn authenticated_episode_detail(&self) -> Option<&EpisodicSummaryView> {
        let selected = self.authenticated_selected_episode()?;
        let detail = self.episode_detail.as_ref()?;
        let reference = detail.summary.reference();
        (reference == selected.summary
            && self.namespace_id == Some(reference.namespace_id())
            && self.profile.as_ref().map(|identity| &identity.profile) == Some(reference.profile())
            && detail.summary.sources().len() <= 128
            && u64::try_from(detail.summary.sources().len()).ok()? == selected.source_count)
            .then_some(detail)
    }

    pub(crate) fn local_layer_cache_is_authenticated(&self) -> bool {
        match self.pane {
            MemoryPane::EntryList => self.authenticated_entries().is_some_and(|(_, counts)| {
                counts.displayed == 0 || self.authenticated_selected_entry().is_some()
            }),
            MemoryPane::EntryDetail => self.authenticated_entry_detail().is_some(),
            MemoryPane::EntryHistory => self.authenticated_history().is_some_and(|(_, counts)| {
                (counts.displayed == 0 || self.authenticated_selected_history().is_some())
                    && match self.entry_version {
                        None => true,
                        Some(_) => matches!(self.authenticated_history_version(), Ok(Some(_))),
                    }
            }),
            MemoryPane::Proposals => self.authenticated_proposals().is_some_and(|(_, counts)| {
                counts.displayed == 0 || self.authenticated_selected_proposal().is_some()
            }),
            MemoryPane::ProposalDetail => self.authenticated_proposal_detail().is_some(),
            MemoryPane::EpisodicSummaries => {
                self.authenticated_episodes().is_some_and(|(_, counts)| {
                    counts.displayed == 0 || self.authenticated_selected_episode().is_some()
                })
            }
            MemoryPane::EpisodicDetail => self.authenticated_episode_detail().is_some(),
            MemoryPane::Editor => self.editor_is_authenticated(),
            MemoryPane::MutationReview => self.edit_review_command().is_some(),
            MemoryPane::Confirmation => self.confirmed_command().is_some(),
            MemoryPane::ProposalResolutionReview => {
                self.authenticated_resolution_review().is_some()
            }
            MemoryPane::Result => true,
        }
    }

    pub fn apply_matching_view(&mut self, intent: &MemoryOutcomeIntent, view: CommandView) -> bool {
        if self.pending_intent.as_ref() != Some(intent) {
            return false;
        }
        let previous_detail_identity = self.exact_detail_identity();
        let matched = match (intent, view) {
            (MemoryOutcomeIntent::Entries, CommandView::MemoryEntries(value))
                if self.matches_profile_namespace(&value.profile, value.namespace_id) =>
            {
                self.replace_entries(value);
                true
            }
            (MemoryOutcomeIntent::EntryDetail(expected), CommandView::MemoryEntry(value))
                if value.entry.reference().entry_id() == *expected
                    && self.matches_profile_namespace(
                        &value.profile,
                        value.entry.reference().namespace_id(),
                    ) =>
            {
                self.entry_detail = Some(value);
                true
            }
            (
                MemoryOutcomeIntent::EntryHistory(expected),
                CommandView::MemoryEntryHistory(value),
            ) if value.current.entry_id() == *expected
                && self.matches_profile_namespace(&value.profile, value.current.namespace_id()) =>
            {
                self.replace_entry_history(value);
                true
            }
            (
                MemoryOutcomeIntent::EntryVersion {
                    selector,
                    key,
                    version,
                    entry_version_id,
                },
                CommandView::MemoryEntryVersion(value),
            ) if self.matches_profile_selector(selector)
                && self.matches_profile_namespace(
                    &value.profile,
                    value.entry.reference().namespace_id(),
                )
                && crate::memory::normalize_memory_key(key).ok().as_ref()
                    == Some(value.entry.reference().normalized_key())
                && value.entry.reference().version() == *version
                && value.entry.reference().entry_version_id() == *entry_version_id =>
            {
                self.entry_version = Some(value);
                true
            }
            (MemoryOutcomeIntent::Proposals, CommandView::MemoryProposals(value))
                if self.matches_profile_namespace(&value.profile, value.namespace_id) =>
            {
                self.replace_proposals(value);
                true
            }
            (MemoryOutcomeIntent::ProposalDetail(expected), CommandView::MemoryProposal(value))
                if value.proposal.reference().proposal_id() == *expected
                    && self.matches_profile_namespace(
                        &value.namespace_owner_identity.profile,
                        value.proposal.namespace_id(),
                    ) =>
            {
                self.proposal_detail = Some(value);
                true
            }
            (MemoryOutcomeIntent::Episodes, CommandView::EpisodicSummaries(value))
                if self.matches_profile_namespace(&value.profile, value.namespace_id) =>
            {
                self.replace_episodes(value);
                true
            }
            (MemoryOutcomeIntent::EpisodeDetail(expected), CommandView::EpisodicSummary(value))
                if value.summary.reference().summary_id() == *expected
                    && self.matches_profile_namespace(
                        value.summary.reference().profile(),
                        value.summary.reference().namespace_id(),
                    ) =>
            {
                self.episode_detail = Some(value);
                true
            }
            (MemoryOutcomeIntent::Mutation, CommandView::MemoryEntryMutation(value))
                if self.matches_mutation_result(&value) =>
            {
                true
            }
            (MemoryOutcomeIntent::Resolution, CommandView::MemoryProposalResolution(value))
                if self.matches_resolution_result(&value) =>
            {
                true
            }
            _ => false,
        };
        if matched {
            self.pending_intent = None;
            if self.exact_detail_identity() != previous_detail_identity {
                self.detail_scroll = 0;
            }
        }
        matched
    }

    fn matches_profile_namespace(
        &self,
        profile: &crate::agents::AgentProfileVersionRef,
        namespace_id: MemoryNamespaceId,
    ) -> bool {
        self.profile
            .as_ref()
            .is_some_and(|identity| &identity.profile == profile)
            && self.namespace_id == Some(namespace_id)
    }

    fn matches_profile_selector(&self, selector: &crate::app::AgentProfileSelector) -> bool {
        let Some(identity) = self.profile.as_ref() else {
            return false;
        };
        match selector {
            crate::app::AgentProfileSelector::Id(profile_id) => {
                identity.profile.profile_id() == *profile_id
            }
            crate::app::AgentProfileSelector::Name(_) => {
                selector.normalized_name()
                    == crate::app::AgentProfileSelector::Name(identity.display_name.clone())
                        .normalized_name()
            }
        }
    }

    fn matches_mutation_result(&self, value: &crate::app::MemoryEntryMutationView) -> bool {
        let Some(command) = self.confirmed_command() else {
            return false;
        };
        let Some(review) = self.authenticated_edit_review() else {
            return false;
        };
        if !matches!(
            (&review.operation, command),
            (
                MemoryMutationKind::Set,
                ApplicationCommand::SetMemoryEntry { .. }
            ) | (
                MemoryMutationKind::Delete,
                ApplicationCommand::DeleteMemoryEntry { .. }
            )
        ) {
            return false;
        }
        let result = &value.entry;
        if result.namespace_id() != review.namespace_id {
            return false;
        }
        let (expected_key, expected_state) = match (&review.operation, &review.candidate) {
            (MemoryMutationKind::Set, Some(candidate)) => {
                (candidate.normalized_key(), MemoryEntryState::Present)
            }
            (MemoryMutationKind::Delete, None) => {
                let expected = match &review.expected {
                    ExpectedMemoryEntryState::Present(expected) => expected,
                    ExpectedMemoryEntryState::Absent | ExpectedMemoryEntryState::Deleted(_) => {
                        return false;
                    }
                };
                (expected.normalized_key().clone(), MemoryEntryState::Deleted)
            }
            _ => return false,
        };
        if result.normalized_key() != &expected_key || result.state() != expected_state {
            return false;
        }
        match &review.expected {
            ExpectedMemoryEntryState::Absent => true,
            ExpectedMemoryEntryState::Present(expected)
            | ExpectedMemoryEntryState::Deleted(expected) => {
                result.entry_id() == expected.entry_id()
            }
        }
    }

    fn matches_resolution_result(&self, value: &crate::app::MemoryProposalResolutionView) -> bool {
        let Some(command) = self.confirmed_command() else {
            return false;
        };
        let Some(review) = self.authenticated_resolution_review() else {
            return false;
        };
        if !matches!(
            (review.action, command),
            (
                MemoryResolutionAction::Approve,
                ApplicationCommand::ApproveMemoryProposal { .. }
            ) | (
                MemoryResolutionAction::Reject,
                ApplicationCommand::RejectMemoryProposal { .. }
            )
        ) || value.resolution.proposal() != &review.proposal.reference()
            || value.resolution.approval_id() != review.approval_id
        {
            return false;
        }
        value.resolution.status()
            == match review.action {
                MemoryResolutionAction::Approve => MemoryProposalStatus::Accepted,
                MemoryResolutionAction::Reject => MemoryProposalStatus::Rejected,
            }
    }

    fn replace_entries(&mut self, entries: MemoryEntriesView) {
        let selected_id = self
            .entries
            .as_ref()
            .and_then(|view| view.entries.get(self.selected_entry))
            .map(|summary| summary.entry.entry_id());
        self.entries = Some(entries);
        let values = &self.entries.as_ref().expect("entries installed").entries;
        self.selected_entry = selected_id
            .and_then(|id| {
                values
                    .iter()
                    .position(|summary| summary.entry.entry_id() == id)
            })
            .unwrap_or_else(|| self.selected_entry.min(values.len().saturating_sub(1)));
        self.entry_scroll = self.entry_scroll.min(self.selected_entry);
        let current_id = values
            .get(self.selected_entry)
            .map(|summary| summary.entry.entry_id());
        if self
            .entry_detail
            .as_ref()
            .map(|detail| detail.entry.reference().entry_id())
            != current_id
        {
            self.entry_detail = None;
            self.entry_history = None;
            self.entry_version = None;
        }
    }

    fn replace_entry_history(&mut self, history: MemoryEntryHistoryView) {
        let selected_id = self
            .entry_history
            .as_ref()
            .and_then(|view| view.versions.get(self.selected_history_version))
            .map(|summary| summary.entry.entry_version_id());
        self.entry_history = Some(history);
        let versions = &self
            .entry_history
            .as_ref()
            .expect("entry history installed")
            .versions;
        self.selected_history_version = selected_id
            .and_then(|id| {
                versions
                    .iter()
                    .position(|summary| summary.entry.entry_version_id() == id)
            })
            .unwrap_or_else(|| {
                self.selected_history_version
                    .min(versions.len().saturating_sub(1))
            });
        self.history_scroll = self.history_scroll.min(self.selected_history_version);
        let current_id = versions
            .get(self.selected_history_version)
            .map(|summary| summary.entry.entry_version_id());
        if self
            .entry_version
            .as_ref()
            .map(|detail| detail.entry.reference().entry_version_id())
            != current_id
        {
            self.entry_version = None;
        }
    }

    fn replace_proposals(&mut self, proposals: MemoryProposalsView) {
        let selected_id = self
            .proposals
            .as_ref()
            .and_then(|view| view.proposals.get(self.selected_proposal))
            .map(|summary| summary.proposal.proposal_id());
        self.proposals = Some(proposals);
        let values = &self
            .proposals
            .as_ref()
            .expect("proposals installed")
            .proposals;
        self.selected_proposal = selected_id
            .and_then(|id| {
                values
                    .iter()
                    .position(|summary| summary.proposal.proposal_id() == id)
            })
            .unwrap_or_else(|| self.selected_proposal.min(values.len().saturating_sub(1)));
        self.proposal_scroll = self.proposal_scroll.min(self.selected_proposal);
        let current_id = values
            .get(self.selected_proposal)
            .map(|summary| summary.proposal.proposal_id());
        if self
            .proposal_detail
            .as_ref()
            .map(|detail| detail.proposal.reference().proposal_id())
            != current_id
        {
            self.proposal_detail = None;
        }
    }

    fn replace_episodes(&mut self, episodes: EpisodicSummariesView) {
        let selected_id = self
            .episodes
            .as_ref()
            .and_then(|view| view.summaries.get(self.selected_episode))
            .map(|summary| summary.summary.summary_id());
        self.episodes = Some(episodes);
        let values = &self
            .episodes
            .as_ref()
            .expect("episodes installed")
            .summaries;
        self.selected_episode = selected_id
            .and_then(|id| {
                values
                    .iter()
                    .position(|summary| summary.summary.summary_id() == id)
            })
            .unwrap_or_else(|| self.selected_episode.min(values.len().saturating_sub(1)));
        self.episode_scroll = self.episode_scroll.min(self.selected_episode);
        let current_id = values
            .get(self.selected_episode)
            .map(|summary| summary.summary.summary_id());
        if self
            .episode_detail
            .as_ref()
            .map(|detail| detail.summary.reference().summary_id())
            != current_id
        {
            self.episode_detail = None;
        }
    }
}

fn page_counts(
    raw_len: usize,
    displayed: usize,
    server_omitted: u64,
    total: u64,
) -> MemoryPageCounts {
    let locally_omitted = u64::try_from(raw_len.saturating_sub(displayed)).unwrap_or(u64::MAX);
    MemoryPageCounts {
        displayed,
        omitted: server_omitted.saturating_add(locally_omitted),
        total,
    }
}

fn proposal_operation_kind(operation: &MemoryProposalOperation) -> MemoryProposalOperationKind {
    match operation {
        MemoryProposalOperation::Set { .. } => MemoryProposalOperationKind::Set,
        MemoryProposalOperation::Delete => MemoryProposalOperationKind::Delete,
    }
}

pub(super) fn canonical_memory_edit_diff(diff: &[MemoryFieldDiff]) -> bool {
    if diff.is_empty() || diff.len() > 4 {
        return false;
    }
    diff.iter().all(|item| {
        item.before != item.after
            && memory_field_value_matches(item.field, &item.before)
            && memory_field_value_matches(item.field, &item.after)
    }) && diff
        .windows(2)
        .all(|items| memory_field_rank(items[0].field) < memory_field_rank(items[1].field))
}

fn memory_field_rank(field: MemoryField) -> u8 {
    match field {
        MemoryField::DisplayKey => 0,
        MemoryField::State => 1,
        MemoryField::Value => 2,
        MemoryField::PurposeTags => 3,
    }
}

fn memory_field_value_matches(field: MemoryField, value: &MemoryFieldValue) -> bool {
    matches!(
        (field, value),
        (
            MemoryField::DisplayKey | MemoryField::Value,
            MemoryFieldValue::Missing
        ) | (
            MemoryField::DisplayKey | MemoryField::Value,
            MemoryFieldValue::Text(_)
        ) | (MemoryField::State, MemoryFieldValue::Missing)
            | (MemoryField::State, MemoryFieldValue::State(_))
            | (MemoryField::PurposeTags, MemoryFieldValue::Missing)
            | (MemoryField::PurposeTags, MemoryFieldValue::Tags(_))
    )
}

pub(super) fn absent_set_diff_matches(
    diff: &[MemoryFieldDiff],
    candidate: &MemoryEntryDraft,
) -> bool {
    diff.len() == 4
        && diff[0].field == MemoryField::DisplayKey
        && diff[0].before == MemoryFieldValue::Missing
        && text_value_matches(&diff[0].after, candidate.display_key())
        && diff[1].field == MemoryField::State
        && diff[1].before == MemoryFieldValue::Missing
        && diff[1].after == MemoryFieldValue::State(MemoryEntryState::Present)
        && diff[2].field == MemoryField::Value
        && diff[2].before == MemoryFieldValue::Missing
        && text_value_matches(&diff[2].after, candidate.value())
        && diff[3].field == MemoryField::PurposeTags
        && diff[3].before == MemoryFieldValue::Missing
        && tags_value_matches(&diff[3].after, candidate.purpose_tags())
}

pub(super) fn seeded_present_set_diff(
    seed: &MemoryEntryVersion,
    candidate: &MemoryEntryDraft,
) -> Option<Vec<MemoryFieldDiff>> {
    if seed.reference().state() != MemoryEntryState::Present
        || seed.reference().normalized_key() != &candidate.normalized_key()
    {
        return None;
    }
    let before = [
        MemoryFieldValue::Text(seed.display_key().to_owned()),
        MemoryFieldValue::State(MemoryEntryState::Present),
        MemoryFieldValue::Text(seed.value()?.to_owned()),
        MemoryFieldValue::Tags(seed.purpose_tags().to_vec()),
    ];
    let after = [
        MemoryFieldValue::Text(candidate.display_key().to_owned()),
        MemoryFieldValue::State(MemoryEntryState::Present),
        MemoryFieldValue::Text(candidate.value().to_owned()),
        MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
    ];
    Some(
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
        .collect(),
    )
}

pub(super) fn deleted_set_diff_matches(
    diff: &[MemoryFieldDiff],
    candidate: &MemoryEntryDraft,
) -> bool {
    let required = if diff.first().is_some_and(|item| {
        item.field == MemoryField::DisplayKey
            && matches!(&item.before, MemoryFieldValue::Text(prior_key) if {
                MemoryEntryDraft::new(
                    prior_key.clone(),
                    candidate.value().to_owned(),
                    candidate.purpose_tags().to_vec(),
                )
                .is_ok_and(|canonical| {
                    canonical.display_key() == prior_key
                        && canonical.normalized_key() == candidate.normalized_key()
                })
            })
            && text_value_matches(&item.after, candidate.display_key())
    }) {
        &diff[1..]
    } else {
        diff
    };
    required.len() == 3
        && required[0].field == MemoryField::State
        && required[0].before == MemoryFieldValue::State(MemoryEntryState::Deleted)
        && required[0].after == MemoryFieldValue::State(MemoryEntryState::Present)
        && required[1].field == MemoryField::Value
        && required[1].before == MemoryFieldValue::Missing
        && text_value_matches(&required[1].after, candidate.value())
        && required[2].field == MemoryField::PurposeTags
        && required[2].before == MemoryFieldValue::Missing
        && tags_value_matches(&required[2].after, candidate.purpose_tags())
}

pub(super) fn delete_diff_matches(
    diff: &[MemoryFieldDiff],
    requested_entry: &MemoryEntryVersion,
) -> bool {
    diff.len() == 3
        && diff[0].field == MemoryField::State
        && diff[0].before == MemoryFieldValue::State(MemoryEntryState::Present)
        && diff[0].after == MemoryFieldValue::State(MemoryEntryState::Deleted)
        && diff[1].field == MemoryField::Value
        && requested_entry
            .value()
            .is_some_and(|value| text_value_matches(&diff[1].before, value))
        && diff[1].after == MemoryFieldValue::Missing
        && diff[2].field == MemoryField::PurposeTags
        && tags_value_matches(&diff[2].before, requested_entry.purpose_tags())
        && diff[2].after == MemoryFieldValue::Missing
}

fn text_value_matches(value: &MemoryFieldValue, expected: &str) -> bool {
    matches!(value, MemoryFieldValue::Text(actual) if actual == expected)
}

fn tags_value_matches(value: &MemoryFieldValue, expected: &[String]) -> bool {
    matches!(value, MemoryFieldValue::Tags(actual) if actual == expected)
}

pub(super) fn expected_state_matches_namespace_key(
    expected: &ExpectedMemoryEntryState,
    namespace_id: MemoryNamespaceId,
    key: &crate::memory::NormalizedMemoryKey,
) -> bool {
    match expected {
        ExpectedMemoryEntryState::Absent => true,
        ExpectedMemoryEntryState::Present(reference) => {
            reference.state() == MemoryEntryState::Present
                && reference.namespace_id() == namespace_id
                && reference.normalized_key() == key
        }
        ExpectedMemoryEntryState::Deleted(reference) => {
            reference.state() == MemoryEntryState::Deleted
                && reference.namespace_id() == namespace_id
                && reference.normalized_key() == key
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSkillAction {
    View,
    Upgrade,
    Unassign,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSkillUpgradeAvailability {
    Unknown,
    Current,
    Available(SkillVersionRef),
    Inconsistent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileConfirmation {
    pub command: ApplicationCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillsPane {
    List,
    CreateSource,
    Detail,
    History,
    Editor,
    AgentPicker,
    AssignmentReview,
    Confirmation,
    Result,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillDetailAction {
    Assign,
    CreateVersion,
    History,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignmentKind {
    Add,
    Upgrade { expected: SkillVersionRef },
    Reassign { expected: SkillVersionRef },
    AlreadyAssigned,
    Unassign { expected: SkillVersionRef },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentOutcomeIntent {
    AgentsWorkspace,
    SkillAssignment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillOperationOrigin {
    Skills(SkillsPane),
    AgentSkills {
        profile_id: crate::domain::AgentProfileId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillWorkspaceOrigin {
    Cockpit(View),
    AgentSkills {
        profile_id: crate::domain::AgentProfileId,
    },
}

impl AssignmentKind {
    pub fn classify(target: &SkillVersionRef, current: Option<&SkillVersionRef>) -> Self {
        match current {
            None => Self::Add,
            Some(current) if current == target => Self::AlreadyAssigned,
            Some(current) if target.version().get() > current.version().get() => Self::Upgrade {
                expected: current.clone(),
            },
            Some(current) => Self::Reassign {
                expected: current.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillConfirmation {
    pub command: ApplicationCommand,
    pub origin: SkillOperationOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillsViewState {
    pub active: bool,
    pub library_loaded: bool,
    pub workspace_origin: Option<SkillWorkspaceOrigin>,
    pub pane: SkillsPane,
    pub selected_skill: usize,
    pub selected_action_index: usize,
    pub selected_create_source: usize,
    pub selected_history_version: usize,
    pub selected_agent: usize,
    pub library: SkillsView,
    pub detail: Option<SkillView>,
    pub history: Option<SkillHistoryView>,
    pub version_detail: Option<SkillView>,
    pub selected_agent_detail: Option<AgentProfileView>,
    pub assignment: Option<AssignmentKind>,
    pub editor: Option<SkillEditor>,
    pub editor_origin: SkillsPane,
    pub pending_confirmation: Option<SkillConfirmation>,
    pub review_registered: bool,
    pub operation_origin: SkillOperationOrigin,
    pub pending_active_skill: Option<crate::domain::SkillId>,
}

impl Default for SkillsViewState {
    fn default() -> Self {
        Self {
            active: false,
            library_loaded: false,
            workspace_origin: None,
            pane: SkillsPane::List,
            selected_skill: 0,
            selected_action_index: 0,
            selected_create_source: 0,
            selected_history_version: 0,
            selected_agent: 0,
            library: SkillsView {
                skills: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            detail: None,
            history: None,
            version_detail: None,
            selected_agent_detail: None,
            assignment: None,
            editor: None,
            editor_origin: SkillsPane::CreateSource,
            pending_confirmation: None,
            review_registered: false,
            operation_origin: SkillOperationOrigin::Skills(SkillsPane::Detail),
            pending_active_skill: None,
        }
    }
}

impl SkillsViewState {
    pub fn replace_skills(&mut self, library: SkillsView) {
        let selected_id = self
            .selected_summary()
            .map(|summary| summary.skill_ref.skill_id());
        self.library = library;
        self.library_loaded = true;
        self.selected_skill = selected_id
            .and_then(|id| {
                self.library
                    .skills
                    .iter()
                    .position(|summary| summary.skill_ref.skill_id() == id)
            })
            .unwrap_or_else(|| {
                self.selected_skill
                    .min(self.library.skills.len().saturating_sub(1))
            });
    }

    pub fn replace_detail(&mut self, detail: SkillView) {
        let skill_id = detail.skill_ref.skill_id();
        self.synchronize_selected_skill(skill_id);
        if self.history.as_ref().map(|history| history.skill_id) != Some(skill_id) {
            self.history = None;
        }
        self.version_detail = None;
        self.detail = Some(detail);
        self.pane = SkillsPane::Detail;
    }

    pub fn replace_history(&mut self, history: SkillHistoryView) {
        self.selected_history_version = 0;
        self.version_detail = None;
        self.history = Some(history);
    }

    pub fn replace_version_detail(&mut self, detail: SkillView) {
        let skill_id = detail.skill_ref.skill_id();
        self.synchronize_selected_skill(skill_id);
        if self.history.as_ref().map(|history| history.skill_id) != Some(skill_id) {
            self.history = None;
            self.selected_history_version = 0;
        }
        self.detail = Some(detail.clone());
        self.version_detail = Some(detail);
    }

    pub fn clear_skill_context(&mut self) {
        self.detail = None;
        self.history = None;
        self.version_detail = None;
        self.selected_history_version = 0;
        self.pending_active_skill = None;
    }

    fn synchronize_selected_skill(&mut self, skill_id: crate::domain::SkillId) {
        if let Some(index) = self
            .library
            .skills
            .iter()
            .position(|summary| summary.skill_ref.skill_id() == skill_id)
        {
            self.selected_skill = index;
        }
    }

    pub fn current_skill_id(&self) -> Option<crate::domain::SkillId> {
        self.selected_skill_ref().map(SkillVersionRef::skill_id)
    }

    pub fn agent_origin_profile_id(&self) -> Option<crate::domain::AgentProfileId> {
        match self.workspace_origin {
            Some(SkillWorkspaceOrigin::AgentSkills { profile_id }) => Some(profile_id),
            _ => None,
        }
    }

    pub fn selected_summary(&self) -> Option<&crate::app::SkillSummary> {
        self.library.skills.get(self.selected_skill)
    }

    pub fn selected_action(&self) -> SkillDetailAction {
        let actions = self.available_detail_actions();
        actions[self
            .selected_action_index
            .min(actions.len().saturating_sub(1))]
    }

    pub fn available_detail_actions(&self) -> &'static [SkillDetailAction] {
        const ACTIVE: &[SkillDetailAction] = &[
            SkillDetailAction::Assign,
            SkillDetailAction::CreateVersion,
            SkillDetailAction::History,
        ];
        const HISTORICAL: &[SkillDetailAction] =
            &[SkillDetailAction::Assign, SkillDetailAction::History];
        if self.version_detail.is_some() {
            HISTORICAL
        } else {
            ACTIVE
        }
    }

    pub fn selected_skill_ref(&self) -> Option<&SkillVersionRef> {
        if self.pane == SkillsPane::History {
            self.history
                .as_ref()
                .and_then(|history| history.versions.get(self.selected_history_version))
                .map(|entry| &entry.skill_ref)
        } else {
            self.version_detail
                .as_ref()
                .filter(|version| {
                    self.detail
                        .as_ref()
                        .map(|detail| detail.skill_ref.skill_id())
                        == Some(version.skill_ref.skill_id())
                })
                .map(|version| &version.skill_ref)
                .or_else(|| {
                    self.detail
                        .as_ref()
                        .map(|detail| &detail.skill_ref)
                        .or_else(|| self.selected_summary().map(|summary| &summary.skill_ref))
                })
        }
    }

    pub fn start_create(&mut self, seed: Option<SkillDraft>) {
        self.editor = Some(SkillEditor::for_create(seed));
        self.editor_origin = SkillsPane::CreateSource;
        self.pane = SkillsPane::Editor;
        self.pending_confirmation = None;
    }

    pub fn start_version(&mut self) -> bool {
        if self.version_detail.is_some() {
            return false;
        }
        let Some(detail) = self.detail.as_ref() else {
            return false;
        };
        self.editor = Some(SkillEditor::for_version(
            detail.skill_ref.skill_id(),
            detail.skill_ref.skill_version_id(),
            detail.content.clone(),
        ));
        self.editor_origin = SkillsPane::Detail;
        self.pane = SkillsPane::Editor;
        self.pending_confirmation = None;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsViewState {
    pub selected_profile: usize,
    pub selected_template: usize,
    pub selected_detail_action: AgentDetailAction,
    pub memory: MemoryViewState,
    pub pane: AgentsPane,
    pub list_scroll: usize,
    pub detail_scroll: usize,
    pub history_scroll: usize,
    pub selected_history_version: usize,
    pub skill_panel_open: bool,
    pub selected_assigned_skill: usize,
    pub selected_skill_action_index: usize,
    pub editor: Option<ProfileEditor>,
    pub pending_confirmation: Option<ProfileConfirmation>,
    pub profiles: AgentProfilesView,
    pub detail: Option<AgentProfileView>,
    pub history: Option<AgentProfileHistoryView>,
    pub version_detail: Option<AgentProfileVersionView>,
}

impl Default for AgentsViewState {
    fn default() -> Self {
        Self {
            selected_profile: 0,
            selected_template: 0,
            selected_detail_action: AgentDetailAction::AssignedSkills,
            memory: MemoryViewState::default(),
            pane: AgentsPane::List,
            list_scroll: 0,
            detail_scroll: 0,
            history_scroll: 0,
            selected_history_version: 0,
            skill_panel_open: false,
            selected_assigned_skill: 0,
            selected_skill_action_index: 0,
            editor: None,
            pending_confirmation: None,
            profiles: AgentProfilesView {
                profiles: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            detail: None,
            history: None,
            version_detail: None,
        }
    }
}

impl AgentsViewState {
    pub fn select_profile_id(&mut self, profile_id: crate::domain::AgentProfileId) -> bool {
        if let Some(index) = self
            .profiles
            .profiles
            .iter()
            .position(|summary| summary.profile_id == profile_id)
        {
            return self.select_profile_index(index);
        }
        false
    }

    pub fn selected_skill_action(&self) -> AgentSkillAction {
        match self.selected_skill_action_index.min(2) {
            0 => AgentSkillAction::View,
            1 => AgentSkillAction::Upgrade,
            _ => AgentSkillAction::Unassign,
        }
    }

    pub fn selected_assigned_skill_ref(&self) -> Option<&SkillVersionRef> {
        self.detail.as_ref().and_then(|detail| {
            detail
                .profile
                .skill_refs()
                .get(self.selected_assigned_skill)
        })
    }

    pub fn selected_summary(&self) -> Option<&crate::app::AgentProfileSummary> {
        self.profiles.profiles.get(self.selected_profile)
    }

    pub fn select_profile_index(&mut self, index: usize) -> bool {
        let index = index.min(self.profiles.profiles.len().saturating_sub(1));
        let previous_id = self.selected_summary().map(|summary| summary.profile_id);
        let next_id = self
            .profiles
            .profiles
            .get(index)
            .map(|summary| summary.profile_id);
        if self.memory.has_protected_workflow() && previous_id != next_id {
            return false;
        }
        self.selected_profile = index;
        self.list_scroll = self.list_scroll.min(self.selected_profile);
        if previous_id != next_id {
            self.selected_detail_action = AgentDetailAction::AssignedSkills;
            self.memory.invalidate_profile_context();
        }
        true
    }

    pub fn replace_profiles(&mut self, profiles: AgentProfilesView) -> bool {
        let selected_id = self.selected_summary().map(|summary| summary.profile_id);
        if self.memory.has_protected_workflow() {
            let protected_profile = self
                .memory
                .profile
                .as_ref()
                .map(|identity| &identity.profile);
            let protected_is_preserved = selected_id
                .and_then(|profile_id| {
                    profiles
                        .profiles
                        .iter()
                        .find(|summary| summary.profile_id == profile_id)
                })
                .is_some_and(|summary| {
                    protected_profile.is_some_and(|profile| {
                        summary.profile_version_id == profile.profile_version_id()
                            && summary.version == profile.version()
                            && summary.content_digest == *profile.content_digest()
                    })
                });
            if !protected_is_preserved {
                return false;
            }
        }
        let bound_profile = self
            .memory
            .profile
            .as_ref()
            .map(|identity| identity.profile.clone());
        self.profiles = profiles;
        if self.profiles.profiles.is_empty() {
            self.selected_profile = 0;
            self.list_scroll = 0;
            self.detail = None;
            self.history = None;
            self.version_detail = None;
            self.skill_panel_open = false;
            self.selected_detail_action = AgentDetailAction::AssignedSkills;
            self.memory.invalidate_profile_context();
            return true;
        }
        let selected_profile = selected_id
            .and_then(|profile_id| {
                self.profiles
                    .profiles
                    .iter()
                    .position(|summary| summary.profile_id == profile_id)
            })
            .unwrap_or_else(|| {
                self.selected_profile
                    .min(self.profiles.profiles.len().saturating_sub(1))
            });
        self.selected_profile = selected_profile;
        self.list_scroll = self.list_scroll.min(self.selected_profile);
        let next_summary = self.selected_summary();
        let next_id = next_summary.map(|summary| summary.profile_id);
        if selected_id != next_id {
            self.selected_detail_action = AgentDetailAction::AssignedSkills;
            self.memory.invalidate_profile_context();
        } else if bound_profile.as_ref().is_some_and(|profile| {
            next_summary.is_some_and(|summary| {
                summary.profile_version_id != profile.profile_version_id()
                    || summary.version != profile.version()
                    || summary.content_digest != *profile.content_digest()
            })
        }) {
            self.memory.invalidate_profile_context();
        }
        if self
            .detail
            .as_ref()
            .map(|detail| detail.profile.profile_id())
            != next_id
        {
            self.detail = None;
            self.history = None;
            self.version_detail = None;
        }
        true
    }

    pub fn replace_detail(&mut self, detail: AgentProfileView) -> bool {
        if self.memory.has_protected_workflow()
            && self
                .memory
                .profile
                .as_ref()
                .is_none_or(|identity| identity.profile != detail.profile.reference())
        {
            return false;
        }
        if self
            .memory
            .profile
            .as_ref()
            .is_some_and(|identity| identity.profile != detail.profile.reference())
        {
            self.memory.invalidate_profile_context();
        }
        let profile_id = detail.profile.profile_id();
        if let Some(index) = self
            .profiles
            .profiles
            .iter()
            .position(|summary| summary.profile_id == profile_id)
        {
            self.select_profile_index(index);
        }
        if self.history.as_ref().map(|history| history.profile_id) != Some(profile_id) {
            self.history = None;
            self.version_detail = None;
        }
        self.detail = Some(detail);
        self.selected_assigned_skill = self.selected_assigned_skill.min(
            self.detail
                .as_ref()
                .map(|detail| detail.profile.skill_refs().len().saturating_sub(1))
                .unwrap_or(0),
        );
        true
    }

    pub fn replace_history(&mut self, history: AgentProfileHistoryView) -> bool {
        if self.memory.has_protected_workflow()
            && self.memory.profile.as_ref().is_none_or(|identity| {
                identity.profile.profile_id() != history.profile_id
                    || identity.profile.profile_version_id() != history.active_version_id
            })
        {
            return false;
        }
        if self.memory.profile.as_ref().is_some_and(|identity| {
            identity.profile.profile_id() == history.profile_id
                && identity.profile.profile_version_id() != history.active_version_id
        }) {
            self.memory.invalidate_profile_context();
        }
        if let Some(index) = self
            .profiles
            .profiles
            .iter()
            .position(|summary| summary.profile_id == history.profile_id)
        {
            self.select_profile_index(index);
        }
        self.selected_history_version = 0;
        self.history_scroll = 0;
        self.version_detail = None;
        self.history = Some(history);
        true
    }

    pub fn replace_version_detail(&mut self, version: AgentProfileVersionView) -> bool {
        if self.memory.has_protected_workflow()
            && self
                .memory
                .profile
                .as_ref()
                .is_none_or(|identity| identity.profile != version.profile.reference())
        {
            return false;
        }
        let profile_id = version.profile.profile_id();
        if self.history.as_ref().map(|history| history.profile_id) != Some(profile_id) {
            self.history = None;
            self.selected_history_version = 0;
            self.history_scroll = 0;
        }
        if let Some(index) = self
            .profiles
            .profiles
            .iter()
            .position(|summary| summary.profile_id == profile_id)
        {
            self.select_profile_index(index);
        }
        self.version_detail = Some(version);
        true
    }

    pub fn start_profile_create(
        &mut self,
        template_index: usize,
        templates: &[ProfileTemplate],
    ) -> bool {
        let Some(editor) = templates
            .get(template_index)
            .and_then(|template| ProfileEditor::for_create(template).ok())
        else {
            return false;
        };
        self.editor = Some(editor);
        self.pane = AgentsPane::Editor;
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Navigation,
    List,
    Workspace,
    Actions,
    Inspector,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Nav,
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Wide,
    Medium,
    Narrow,
    TooSmall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStatus {
    Ready,
    Stopping,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiMessage {
    pub severity: Severity,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandEditor {
    buffer: String,
    cursor_byte: usize,
    history: VecDeque<String>,
    history_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CommandDraft {
    buffer: String,
    cursor_byte: usize,
    history_index: Option<usize>,
}

impl CommandEditor {
    pub fn text(&self) -> &str {
        &self.buffer
    }

    pub fn cursor_byte(&self) -> usize {
        self.cursor_byte
    }

    pub fn prefix(&self) -> &str {
        self.buffer.get(..self.cursor_byte).unwrap_or(&self.buffer)
    }

    pub fn insert(&mut self, character: char) {
        let mut encoded = [0; 4];
        self.ingest(character.encode_utf8(&mut encoded));
    }

    pub fn ingest(&mut self, text: &str) {
        self.normalize_cursor();
        let available = MAX_INPUT_BYTES.saturating_sub(self.buffer.len());
        let accepted = bounded_safe_prefix(text, available);
        if accepted.is_empty() {
            return;
        }
        self.buffer.insert_str(self.cursor_byte, &accepted);
        self.cursor_byte = self.cursor_byte.saturating_add(accepted.len());
        self.history_index = None;
    }

    pub(super) fn ingest_memory_value(&mut self, text: &str) {
        self.normalize_cursor();
        let available = MAX_INPUT_BYTES.saturating_sub(self.buffer.len());
        let accepted = bounded_multiline_prefix(text, available);
        if accepted.is_empty() {
            return;
        }
        self.buffer.insert_str(self.cursor_byte, &accepted);
        self.cursor_byte = self.cursor_byte.saturating_add(accepted.len());
        self.history_index = None;
    }

    pub(super) fn replace_with_memory_value(&mut self, value: &str) {
        self.clear();
        self.ingest_memory_value(value);
    }

    pub fn move_left(&mut self) {
        self.normalize_cursor();
        if let Some((index, _)) = self.buffer[..self.cursor_byte].char_indices().last() {
            self.cursor_byte = index;
        }
    }

    pub fn move_right(&mut self) {
        self.normalize_cursor();
        if let Some(character) = self.buffer[self.cursor_byte..].chars().next() {
            self.cursor_byte = self.cursor_byte.saturating_add(character.len_utf8());
        }
    }

    pub fn move_home(&mut self) {
        self.cursor_byte = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_byte = self.buffer.len();
    }

    pub fn backspace(&mut self) {
        self.normalize_cursor();
        self.history_index = None;
        let Some((start, _)) = self.buffer[..self.cursor_byte].char_indices().last() else {
            return;
        };
        self.buffer.replace_range(start..self.cursor_byte, "");
        self.cursor_byte = start;
    }

    pub fn delete(&mut self) {
        self.normalize_cursor();
        self.history_index = None;
        let Some(character) = self.buffer[self.cursor_byte..].chars().next() else {
            return;
        };
        let end = self.cursor_byte.saturating_add(character.len_utf8());
        self.buffer.replace_range(self.cursor_byte..end, "");
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor_byte = 0;
        self.history_index = None;
    }

    pub fn take_text(&mut self) -> String {
        let text = std::mem::take(&mut self.buffer);
        self.cursor_byte = 0;
        self.history_index = None;
        text
    }

    pub fn remember(&mut self, entry: String) {
        let entry = bounded_safe_prefix(&entry, MAX_INPUT_BYTES);
        if entry.trim().is_empty() || self.history.back() == Some(&entry) {
            return;
        }
        if self.history.len() >= COMMAND_HISTORY_CAPACITY {
            self.history.pop_front();
        }
        self.history.push_back(entry);
    }

    pub fn history_previous(&mut self) {
        let next_index = match self.history_index {
            Some(index) => index.checked_sub(1),
            None => self.history.len().checked_sub(1),
        };
        if let Some(index) = next_index {
            self.recall(index);
        }
    }

    pub fn history_next(&mut self) {
        let Some(index) = self.history_index else {
            return;
        };
        let next_index = index.saturating_add(1);
        if next_index < self.history.len() {
            self.recall(next_index);
        } else {
            self.clear();
        }
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn history_back(&self) -> Option<&str> {
        self.history.back().map(String::as_str)
    }

    fn normalize_cursor(&mut self) {
        self.cursor_byte = self.cursor_byte.min(self.buffer.len());
        while self.cursor_byte > 0 && !self.buffer.is_char_boundary(self.cursor_byte) {
            self.cursor_byte = self.cursor_byte.saturating_sub(1);
        }
    }

    fn recall(&mut self, index: usize) {
        if let Some(entry) = self.history.get(index) {
            self.buffer.clone_from(entry);
            self.cursor_byte = self.buffer.len();
            self.history_index = Some(index);
        }
    }

    fn swap_draft(&mut self, draft: &mut CommandDraft) {
        std::mem::swap(&mut self.buffer, &mut draft.buffer);
        std::mem::swap(&mut self.cursor_byte, &mut draft.cursor_byte);
        std::mem::swap(&mut self.history_index, &mut draft.history_index);
        if let Some(index) = self.history_index
            && self.history.get(index) != Some(&self.buffer)
        {
            self.history_index = self.history.iter().rposition(|entry| entry == &self.buffer);
        }
        self.normalize_cursor();
    }
}

fn bounded_safe_prefix(input: &str, byte_limit: usize) -> String {
    let mut bounded = String::with_capacity(input.len().min(byte_limit));
    for character in input.chars() {
        if character.is_control() {
            continue;
        }
        if bounded.len().saturating_add(character.len_utf8()) > byte_limit {
            break;
        }
        bounded.push(character);
    }
    bounded
}

fn bounded_multiline_prefix(input: &str, byte_limit: usize) -> String {
    let mut bounded = String::with_capacity(input.len().min(byte_limit));
    let mut characters = input.chars().peekable();
    while let Some(mut character) = characters.next() {
        if character == '\r' {
            if characters.peek() == Some(&'\n') {
                characters.next();
            }
            character = '\n';
        }
        if character.is_control() && character != '\n' {
            continue;
        }
        if bounded.len().saturating_add(character.len_utf8()) > byte_limit {
            break;
        }
        bounded.push(character);
    }
    bounded
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NavigationTab {
    Overview,
    Chat,
    Agents,
    Skills,
    Connections,
    Activity,
    Setup,
    Audit,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingOutcomeNavigation {
    SelectIfUnchanged { tab: NavigationTab, generation: u64 },
    PreserveCurrent,
}

impl NavigationTab {
    pub(super) const ALL: [Self; 9] = [
        Self::Overview,
        Self::Chat,
        Self::Agents,
        Self::Skills,
        Self::Connections,
        Self::Activity,
        Self::Setup,
        Self::Audit,
        Self::Help,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Chat => 1,
            Self::Agents => 2,
            Self::Skills => 3,
            Self::Connections => 4,
            Self::Activity => 5,
            Self::Setup => 6,
            Self::Audit => 7,
            Self::Help => 8,
        }
    }

    pub(super) const fn key(self) -> char {
        match self {
            Self::Overview => '1',
            Self::Chat => '2',
            Self::Agents => '3',
            Self::Skills => '4',
            Self::Connections => '5',
            Self::Activity => '6',
            Self::Setup => '7',
            Self::Audit => '8',
            Self::Help => '9',
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Home",
            Self::Chat => "Chat",
            Self::Agents => "Agents",
            Self::Skills => "Skills",
            Self::Connections => "Connections",
            Self::Activity => "Activity",
            Self::Setup => "Setup",
            Self::Audit => "Audit",
            Self::Help => "Help",
        }
    }

    pub(super) fn for_number(number: char) -> Option<Self> {
        number
            .to_digit(10)
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| Self::ALL.get(index as usize))
            .copied()
    }

    pub(super) const fn for_view(view: View) -> Self {
        match view {
            View::Overview => Self::Overview,
            View::Chat => Self::Chat,
            View::Connections => Self::Connections,
            View::Activity => Self::Activity,
            View::Setup => Self::Setup,
            View::Audit => Self::Audit,
            View::Help => Self::Help,
            View::Agents => Self::Agents,
        }
    }

    pub(super) const fn adjacent(self, forward: bool) -> Self {
        let index = self.index();
        let next = if forward {
            (index + 1) % Self::ALL.len()
        } else if index == 0 {
            Self::ALL.len() - 1
        } else {
            index - 1
        };
        Self::ALL[next]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabState {
    focus: Focus,
    input_mode: InputMode,
    inspector_open: bool,
    workspace_scroll: u16,
    command_draft: CommandDraft,
}

impl Default for TabState {
    fn default() -> Self {
        Self {
            focus: Focus::Workspace,
            input_mode: InputMode::Nav,
            inspector_open: false,
            workspace_scroll: 0,
            command_draft: CommandDraft::default(),
        }
    }
}

#[derive(Clone)]
pub(super) struct NavigationStateSnapshot {
    active_view: View,
    skills_active: bool,
    skills_pane: SkillsPane,
    skills_workspace_origin: Option<SkillWorkspaceOrigin>,
    agents_pane: AgentsPane,
    focus: Focus,
    input_mode: InputMode,
    inspector_open: bool,
    command: CommandEditor,
    workspace_scroll: u16,
    message: Option<UiMessage>,
    pending_agent_outcome: Option<AgentOutcomeIntent>,
    navigation_generation: u64,
    pending_outcome_navigation: Option<PendingOutcomeNavigation>,
    tab_states: [TabState; 9],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiModel {
    pub active_view: View,
    pub agents: AgentsViewState,
    pub skills: SkillsViewState,
    pub pending_agent_outcome: Option<AgentOutcomeIntent>,
    pub focus: Focus,
    pub input_mode: InputMode,
    pub layout_mode: LayoutMode,
    pub inspector_open: bool,
    pub command: CommandEditor,
    pub installation_id: InstallationId,
    pub session_id: SessionId,
    pub database_readiness: DatabaseReadiness,
    pub process_guard_ownership: ProcessGuardOwnership,
    pub setup_status: SetupStatus,
    pub audit_entries: Vec<AuditEntry>,
    pub audit_selection: Option<usize>,
    pub workspace_scroll: u16,
    pub workspace_body_width: u16,
    pub workspace_body_height: u16,
    pub terminal_width: u16,
    pub terminal_height: u16,
    pub message: Option<UiMessage>,
    pub command_in_flight: bool,
    pub runtime_status: RuntimeStatus,
    pub previous_session_interrupted: bool,
    navigation_generation: u64,
    pending_outcome_navigation: Option<PendingOutcomeNavigation>,
    tab_states: [TabState; 9],
}

impl TuiModel {
    pub fn new(snapshot: PresentationSnapshot, previous_session_interrupted: bool) -> Self {
        let PresentationSnapshot {
            installation_id,
            session_id,
            database_readiness,
            process_guard_ownership,
            setup_status,
            recent_audit,
            agent_profiles,
            selected_agent_profile,
            selected_agent_profile_history,
        } = snapshot;
        let agents = AgentsViewState {
            profiles: agent_profiles,
            detail: selected_agent_profile,
            history: selected_agent_profile_history,
            ..AgentsViewState::default()
        };
        let mut model = Self {
            active_view: View::Overview,
            agents,
            skills: SkillsViewState::default(),
            pending_agent_outcome: None,
            focus: Focus::Workspace,
            input_mode: InputMode::Nav,
            layout_mode: LayoutMode::Wide,
            inspector_open: false,
            command: CommandEditor::default(),
            installation_id,
            session_id,
            database_readiness,
            process_guard_ownership,
            setup_status,
            audit_entries: Vec::new(),
            audit_selection: None,
            workspace_scroll: 0,
            workspace_body_width: 0,
            workspace_body_height: 0,
            terminal_width: super::layout::WIDE_WIDTH,
            terminal_height: super::layout::WIDE_HEIGHT,
            message: None,
            command_in_flight: false,
            runtime_status: RuntimeStatus::Ready,
            previous_session_interrupted,
            navigation_generation: 0,
            pending_outcome_navigation: None,
            tab_states: std::array::from_fn(|_| TabState::default()),
        };
        model.replace_audit(recent_audit);
        model.synchronize_geometry();
        model
    }

    pub fn select_view(&mut self, view: View) {
        self.switch_tab(NavigationTab::for_view(view));
    }

    pub(super) fn switch_tab(&mut self, target: NavigationTab) {
        let current = self.active_navigation_tab();
        if current == target {
            return;
        }

        self.navigation_generation = self.navigation_generation.wrapping_add(1);
        self.swap_tab_state(current);
        match target {
            NavigationTab::Overview => {
                self.skills.active = false;
                self.active_view = View::Overview;
            }
            NavigationTab::Chat => {
                self.skills.active = false;
                self.active_view = View::Chat;
            }
            NavigationTab::Connections => {
                self.skills.active = false;
                self.active_view = View::Connections;
            }
            NavigationTab::Activity => {
                self.skills.active = false;
                self.active_view = View::Activity;
            }
            NavigationTab::Setup => {
                self.skills.active = false;
                self.active_view = View::Setup;
            }
            NavigationTab::Audit => {
                self.skills.active = false;
                self.active_view = View::Audit;
            }
            NavigationTab::Help => {
                self.skills.active = false;
                self.active_view = View::Help;
            }
            NavigationTab::Agents => {
                self.skills.active = false;
                self.active_view = View::Agents;
            }
            NavigationTab::Skills => self.skills.active = true,
        }
        self.swap_tab_state(target);
        self.synchronize_geometry();
        self.normalize_visible_focus();
    }

    pub(super) fn active_navigation_tab(&self) -> NavigationTab {
        if self.skills.active {
            return NavigationTab::Skills;
        }
        match self.active_view {
            View::Overview => NavigationTab::Overview,
            View::Chat => NavigationTab::Chat,
            View::Connections => NavigationTab::Connections,
            View::Activity => NavigationTab::Activity,
            View::Setup => NavigationTab::Setup,
            View::Audit => NavigationTab::Audit,
            View::Help => NavigationTab::Help,
            View::Agents => NavigationTab::Agents,
        }
    }

    fn swap_tab_state(&mut self, tab: NavigationTab) {
        let state = &mut self.tab_states[tab.index()];
        std::mem::swap(&mut self.focus, &mut state.focus);
        std::mem::swap(&mut self.input_mode, &mut state.input_mode);
        std::mem::swap(&mut self.inspector_open, &mut state.inspector_open);
        std::mem::swap(&mut self.workspace_scroll, &mut state.workspace_scroll);
        self.command.swap_draft(&mut state.command_draft);
    }

    pub(super) fn navigation_state_snapshot(&self) -> NavigationStateSnapshot {
        NavigationStateSnapshot {
            active_view: self.active_view,
            skills_active: self.skills.active,
            skills_pane: self.skills.pane,
            skills_workspace_origin: self.skills.workspace_origin,
            agents_pane: self.agents.pane,
            focus: self.focus,
            input_mode: self.input_mode,
            inspector_open: self.inspector_open,
            command: self.command.clone(),
            workspace_scroll: self.workspace_scroll,
            message: self.message.clone(),
            pending_agent_outcome: self.pending_agent_outcome,
            navigation_generation: self.navigation_generation,
            pending_outcome_navigation: self.pending_outcome_navigation,
            tab_states: self.tab_states.clone(),
        }
    }

    pub(super) fn restore_navigation_state(&mut self, snapshot: NavigationStateSnapshot) {
        self.active_view = snapshot.active_view;
        self.skills.active = snapshot.skills_active;
        self.skills.pane = snapshot.skills_pane;
        self.skills.workspace_origin = snapshot.skills_workspace_origin;
        self.agents.pane = snapshot.agents_pane;
        self.focus = snapshot.focus;
        self.input_mode = snapshot.input_mode;
        self.inspector_open = snapshot.inspector_open;
        self.command = snapshot.command;
        self.workspace_scroll = snapshot.workspace_scroll;
        self.message = snapshot.message;
        self.pending_agent_outcome = snapshot.pending_agent_outcome;
        self.navigation_generation = snapshot.navigation_generation;
        self.pending_outcome_navigation = snapshot.pending_outcome_navigation;
        self.tab_states = snapshot.tab_states;
        self.synchronize_geometry();
        self.normalize_visible_focus();
    }

    pub fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
        self.input_mode = if focus == Focus::Command {
            InputMode::Type
        } else {
            InputMode::Nav
        };
        self.synchronize_geometry();
    }

    pub fn set_input_mode(&mut self, input_mode: InputMode) {
        self.input_mode = input_mode;
    }

    pub fn set_layout_mode(&mut self, layout_mode: LayoutMode) {
        self.layout_mode = layout_mode;
    }

    pub fn toggle_inspector(&mut self) {
        self.inspector_open = !self.inspector_open;
        self.synchronize_geometry();
    }

    pub fn scroll_up(&mut self, amount: u16) {
        self.workspace_scroll = self.workspace_scroll.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.workspace_scroll = self.workspace_scroll.saturating_add(amount);
    }

    pub fn scroll_home(&mut self) {
        self.workspace_scroll = 0;
    }

    pub fn set_workspace_body_size(&mut self, width: u16, height: u16) {
        self.workspace_body_width = width;
        self.workspace_body_height = height;
    }

    pub fn set_terminal_size(&mut self, width: u16, height: u16) {
        self.terminal_width = width;
        self.terminal_height = height;
        self.synchronize_geometry();
    }

    pub fn synchronize_geometry(&mut self) {
        let area = ratatui::layout::Rect::new(0, 0, self.terminal_width, self.terminal_height);
        let cockpit = super::layout::calculate_with_input(
            area,
            self.inspector_is_visible(),
            self.input_is_visible(),
        );
        let (workspace_body_width, workspace_body_height) = if cockpit.mode == LayoutMode::TooSmall
        {
            (0, 0)
        } else {
            (
                cockpit.workspace.width.saturating_sub(2),
                cockpit.workspace.height.saturating_sub(2),
            )
        };
        self.layout_mode = cockpit.mode;
        self.workspace_body_width = workspace_body_width;
        self.workspace_body_height = workspace_body_height;
        self.normalize_visible_focus();
    }

    pub(super) fn input_is_visible(&self) -> bool {
        self.focus == Focus::Command
            || (!self.skills.active
                && self.active_view == View::Agents
                && self.agents.pane == AgentsPane::Editor)
            || (self.skills.active && self.skills.pane == SkillsPane::Editor)
            || (!self.skills.active
                && self.active_view == View::Agents
                && self.agents.pane == AgentsPane::Memory
                && self.agents.memory.pane == MemoryPane::Editor
                && self.agents.memory.local_layer_cache_is_authenticated()
                && self.agents.memory.editor.as_ref().is_some_and(|editor| {
                    matches!(
                        editor.step(),
                        MemoryEditorStep::Key
                            | MemoryEditorStep::Value
                            | MemoryEditorStep::PurposeTags
                    )
                }))
    }

    pub(super) fn inspector_is_visible(&self) -> bool {
        self.inspector_open
            && !(!self.skills.active
                && self.active_view == View::Agents
                && self.agents.pane == AgentsPane::Memory)
    }

    fn normalize_visible_focus(&mut self) {
        if self.focus == Focus::Command {
            return;
        }
        if !self.skills.active
            && self.active_view == View::Agents
            && self.agents.pane == AgentsPane::Memory
            && self.focus == Focus::Inspector
        {
            self.focus = Focus::Workspace;
            return;
        }
        let visible = match self.layout_mode {
            LayoutMode::Wide | LayoutMode::Medium | LayoutMode::Narrow => {
                self.focus != Focus::Inspector || self.inspector_open
            }
            LayoutMode::TooSmall => self.focus == Focus::Workspace,
        };
        if !visible {
            self.focus = Focus::Workspace;
        }
    }

    pub fn replace_audit(&mut self, mut entries: Vec<AuditEntry>) {
        let selected_sequence = self
            .audit_selection
            .and_then(|selection| self.audit_entries.get(selection))
            .map(|entry| entry.sequence);
        let previous_selection = self.audit_selection;
        let first_newest = entries.len().saturating_sub(COMMAND_HISTORY_CAPACITY);
        if first_newest > 0 {
            entries = entries.split_off(first_newest);
        }
        self.audit_entries = entries;
        self.audit_selection = selected_sequence
            .and_then(|sequence| {
                self.audit_entries
                    .iter()
                    .position(|entry| entry.sequence == sequence)
            })
            .or_else(|| match (previous_selection, self.audit_last_index()) {
                (_, None) => None,
                (Some(selection), Some(last_index)) => Some(selection.min(last_index)),
                (None, Some(last_index)) => Some(last_index),
            });
    }

    pub fn select_previous_audit(&mut self) {
        self.audit_selection = match (self.audit_selection, self.audit_last_index()) {
            (_, None) => None,
            (Some(selection), Some(last_index)) => {
                Some(selection.min(last_index).saturating_sub(1))
            }
            (None, Some(last_index)) => Some(last_index),
        };
    }

    pub fn select_next_audit(&mut self) {
        self.audit_selection = match (self.audit_selection, self.audit_last_index()) {
            (_, None) => None,
            (Some(selection), Some(last_index)) => {
                Some(selection.min(last_index).saturating_add(1).min(last_index))
            }
            (None, Some(_)) => Some(0),
        };
    }

    pub fn select_first_audit(&mut self) {
        self.audit_selection = self.audit_last_index().map(|_| 0);
    }

    pub fn select_last_audit(&mut self) {
        self.audit_selection = self.audit_last_index();
    }

    pub fn set_message(&mut self, severity: Severity, text: impl Into<String>) {
        self.message = Some(UiMessage {
            severity,
            text: text.into(),
        });
    }

    pub fn clear_message(&mut self) {
        self.message = None;
    }

    pub fn set_command_in_flight(&mut self, command_in_flight: bool) {
        if command_in_flight && !self.command_in_flight {
            self.pending_outcome_navigation = Some(PendingOutcomeNavigation::SelectIfUnchanged {
                tab: self.active_navigation_tab(),
                generation: self.navigation_generation,
            });
        } else if !command_in_flight {
            self.pending_outcome_navigation = None;
        }
        self.command_in_flight = command_in_flight;
    }

    pub(super) fn set_command_in_flight_preserving_navigation(&mut self) {
        self.command_in_flight = true;
        self.pending_outcome_navigation = Some(PendingOutcomeNavigation::PreserveCurrent);
    }

    pub(super) fn should_present_pending_outcome(&self) -> bool {
        match self.pending_outcome_navigation {
            Some(PendingOutcomeNavigation::SelectIfUnchanged { tab, generation }) => {
                tab == self.active_navigation_tab() && generation == self.navigation_generation
            }
            Some(PendingOutcomeNavigation::PreserveCurrent) => false,
            None => true,
        }
    }

    pub fn set_runtime_status(&mut self, runtime_status: RuntimeStatus) {
        self.runtime_status = runtime_status;
    }

    pub fn agent_skill_upgrade_availability(&self) -> AgentSkillUpgradeAvailability {
        let Some(pinned) = self.agents.selected_assigned_skill_ref() else {
            return AgentSkillUpgradeAvailability::Unknown;
        };
        let Some(active) = self
            .skills
            .library
            .skills
            .iter()
            .find(|summary| summary.skill_ref.skill_id() == pinned.skill_id())
            .map(|summary| &summary.skill_ref)
        else {
            return AgentSkillUpgradeAvailability::Unknown;
        };
        if active == pinned {
            AgentSkillUpgradeAvailability::Current
        } else if active.version() > pinned.version() {
            AgentSkillUpgradeAvailability::Available(active.clone())
        } else {
            AgentSkillUpgradeAvailability::Inconsistent
        }
    }

    pub fn available_agent_skill_actions(&self) -> &'static [AgentSkillAction] {
        const STANDARD: &[AgentSkillAction] = &[AgentSkillAction::View, AgentSkillAction::Unassign];
        const UPGRADEABLE: &[AgentSkillAction] = &[
            AgentSkillAction::View,
            AgentSkillAction::Upgrade,
            AgentSkillAction::Unassign,
        ];
        if matches!(
            self.agent_skill_upgrade_availability(),
            AgentSkillUpgradeAvailability::Available(_)
        ) {
            UPGRADEABLE
        } else {
            STANDARD
        }
    }

    pub fn selected_available_agent_skill_action(&self) -> AgentSkillAction {
        let selected = self.agents.selected_skill_action();
        if self.available_agent_skill_actions().contains(&selected) {
            selected
        } else {
            AgentSkillAction::View
        }
    }

    pub fn select_available_agent_skill_action(&mut self, action: AgentSkillAction) {
        let action = if self.available_agent_skill_actions().contains(&action) {
            action
        } else {
            AgentSkillAction::View
        };
        self.agents.selected_skill_action_index = match action {
            AgentSkillAction::View => 0,
            AgentSkillAction::Upgrade => 1,
            AgentSkillAction::Unassign => 2,
        };
    }

    fn audit_last_index(&self) -> Option<usize> {
        self.audit_entries.len().checked_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        app::{MAX_INPUT_BYTES, PresentationSnapshot},
        audit::AuditEntry,
        domain::{Actor, CorrelationId, InstallationId, SessionId},
        setup::SetupStatus,
    };

    fn snapshot() -> PresentationSnapshot {
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            database_readiness: crate::app::DatabaseReadiness::Ready,
            process_guard_ownership: crate::app::ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: vec![audit_entry(1)],
            agent_profiles: crate::app::AgentProfilesView {
                profiles: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            selected_agent_profile: None,
            selected_agent_profile_history: None,
        }
    }

    fn audit_entry(sequence: u64) -> AuditEntry {
        AuditEntry {
            sequence,
            occurred_at_ms: 0,
            actor: Actor::System,
            kind: "help_viewed".to_owned(),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(sequence as u128 + 10)),
            summary: "help viewed".to_owned(),
        }
    }

    #[test]
    fn model_starts_on_overview_with_snapshot_data() {
        let snapshot = snapshot();
        let expected_audit = snapshot.recent_audit.clone();
        let model = TuiModel::new(snapshot, true);
        assert_eq!(model.active_view, View::Overview);
        assert_eq!(model.focus, Focus::Workspace);
        assert!(model.previous_session_interrupted);
        assert_eq!(model.audit_entries, expected_audit);
    }

    #[test]
    fn editor_inserts_and_deletes_unicode_only_at_char_boundaries() {
        let mut editor = CommandEditor::default();
        editor.insert('A');
        editor.insert('界');
        editor.move_left();
        editor.backspace();
        assert_eq!(editor.text(), "界");
        assert_eq!(editor.cursor_byte(), 0);
    }

    #[test]
    fn editor_ingestion_stops_at_the_byte_limit_without_splitting_unicode() {
        let mut editor = CommandEditor::default();
        editor.ingest(&"a".repeat(MAX_INPUT_BYTES - 1));
        assert_eq!(editor.text().len(), 4095);

        editor.ingest("界");
        assert_eq!(editor.text().len(), 4095);
        editor.ingest("b");
        assert_eq!(editor.text().len(), 4096);
        editor.ingest("c");
        editor.insert('d');

        assert_eq!(editor.text().len(), MAX_INPUT_BYTES);
        assert!(editor.text().is_char_boundary(editor.text().len()));
    }

    #[test]
    fn public_history_ingestion_is_control_safe_utf8_bounded_and_duplicate_collapsing() {
        let oversized = format!("{}\n\tignored", "界".repeat(MAX_INPUT_BYTES));
        let mut editor = CommandEditor::default();

        editor.remember(oversized.clone());
        editor.remember(oversized);

        let stored = editor.history_back().expect("bounded history entry");
        assert!(stored.len() <= MAX_INPUT_BYTES);
        assert!(stored.is_char_boundary(stored.len()));
        assert!(!stored.chars().any(char::is_control));
        assert_eq!(editor.history_len(), 1);
    }

    #[test]
    fn history_collapses_consecutive_duplicates_and_caps_at_one_hundred() {
        let mut editor = CommandEditor::default();
        editor.remember("/status".into());
        editor.remember("/status".into());
        for index in 0..110 {
            editor.remember(format!("/audit {}", index + 1));
        }
        assert_eq!(editor.history_len(), COMMAND_HISTORY_CAPACITY);
        assert_eq!(editor.history_back(), Some("/audit 110"));
    }

    #[test]
    fn audit_selection_is_clamped_after_entries_are_replaced() {
        let mut model = TuiModel::new(snapshot(), false);
        model.audit_selection = Some(8);
        model.replace_audit(vec![audit_entry(1), audit_entry(2)]);
        assert_eq!(model.audit_selection, Some(1));
    }

    #[test]
    fn audit_replacement_preserves_the_selected_event_sequence_when_its_index_moves() {
        let mut model = TuiModel::new(snapshot(), false);
        model.replace_audit((1..=100).map(audit_entry).collect());
        model.audit_selection = Some(75);

        model.replace_audit((2..=101).map(audit_entry).collect());

        assert_eq!(model.audit_selection, Some(74));
        assert_eq!(model.audit_entries[74].sequence, 76);
    }

    #[test]
    fn editor_exposes_a_prefix_and_moves_only_between_character_boundaries() {
        let mut editor = CommandEditor::default();
        editor.insert('A');
        editor.insert('界');
        editor.move_left();
        assert_eq!(editor.prefix(), "A");
        editor.move_right();
        editor.move_home();
        assert_eq!(editor.cursor_byte(), 0);
        editor.move_end();
        assert_eq!(editor.cursor_byte(), "A界".len());
    }

    #[test]
    fn editor_delete_clear_and_take_text_reset_the_editable_buffer() {
        let mut editor = CommandEditor::default();
        editor.insert('界');
        editor.insert('A');
        editor.move_home();
        editor.delete();
        assert_eq!(editor.text(), "A");
        editor.clear();
        assert_eq!(editor.text(), "");
        editor.insert('B');
        assert_eq!(editor.take_text(), "B");
        assert_eq!(editor.cursor_byte(), 0);
        assert_eq!(editor.text(), "");
    }

    #[test]
    fn history_navigation_copies_entries_and_editing_exits_recall() {
        let mut editor = CommandEditor::default();
        editor.remember("/help".into());
        editor.remember("/status".into());
        editor.history_previous();
        assert_eq!(editor.text(), "/status");
        editor.history_previous();
        assert_eq!(editor.text(), "/help");
        editor.history_next();
        assert_eq!(editor.text(), "/status");
        editor.insert('!');
        editor.history_next();
        assert_eq!(editor.text(), "/status!");
        assert_eq!(editor.history_back(), Some("/status"));
    }

    #[test]
    fn history_next_past_newest_clears_the_buffer_and_blank_entries_are_ignored() {
        let mut editor = CommandEditor::default();
        editor.remember("   ".into());
        editor.remember("/help".into());
        editor.history_previous();
        editor.history_next();
        assert_eq!(editor.text(), "");
        assert_eq!(editor.history_len(), 1);
    }

    #[test]
    fn restored_tab_drops_a_stale_history_position_but_keeps_its_exact_buffer() {
        let mut model = TuiModel::new(snapshot(), false);
        for index in 0..COMMAND_HISTORY_CAPACITY {
            model.command.remember(format!("command {index}"));
        }
        for _ in 0..COMMAND_HISTORY_CAPACITY {
            model.command.history_previous();
        }
        assert_eq!(model.command.text(), "command 0");

        model.switch_tab(NavigationTab::Setup);
        model.command.remember("new command".to_owned());
        model.switch_tab(NavigationTab::Overview);
        assert_eq!(model.command.text(), "command 0");

        model.command.history_next();
        assert_eq!(model.command.text(), "command 0");
    }

    #[test]
    fn restored_tab_keeps_the_exact_position_of_a_duplicate_history_entry() {
        let mut model = TuiModel::new(snapshot(), false);
        for command in ["A", "B", "A", "C"] {
            model.command.remember(command.to_owned());
        }
        for _ in 0..4 {
            model.command.history_previous();
        }
        assert_eq!(model.command.text(), "A");

        model.switch_tab(NavigationTab::Setup);
        model.switch_tab(NavigationTab::Overview);
        model.command.history_previous();

        assert_eq!(model.command.text(), "A");
    }

    #[test]
    fn no_op_deletions_exit_history_recall() {
        let mut editor = CommandEditor::default();
        editor.remember("/help".into());
        editor.history_previous();
        editor.move_home();
        editor.backspace();
        editor.history_next();
        assert_eq!(editor.text(), "/help");

        editor.move_end();
        editor.delete();
        editor.history_next();
        assert_eq!(editor.text(), "/help");
    }

    #[test]
    fn model_initializes_the_newest_audit_selection_and_pure_state() {
        let model = TuiModel::new(snapshot(), false);
        assert_eq!(model.layout_mode, LayoutMode::Wide);
        assert!(!model.inspector_open);
        assert_eq!(model.workspace_scroll, 0);
        assert_eq!(model.audit_selection, Some(0));
        assert_eq!(model.message, None);
        assert!(!model.command_in_flight);
    }

    #[test]
    fn model_initializes_ready_runtime_and_updates_typed_status() {
        let mut model = TuiModel::new(snapshot(), false);
        assert_eq!(model.runtime_status, RuntimeStatus::Ready);

        model.set_runtime_status(RuntimeStatus::Stopping);

        assert_eq!(model.runtime_status, RuntimeStatus::Stopping);
    }

    #[test]
    fn model_updates_navigation_layout_focus_scroll_and_messages() {
        let mut model = TuiModel::new(snapshot(), false);
        model.select_view(View::Audit);
        model.set_focus(Focus::Command);
        model.set_terminal_size(70, 20);
        model.toggle_inspector();
        model.scroll_down(u16::MAX);
        model.scroll_down(1);
        model.scroll_up(u16::MAX);
        model.scroll_home();
        model.set_message(Severity::Warning, "check setup");
        model.set_command_in_flight(true);

        assert_eq!(model.active_view, View::Audit);
        assert_eq!(model.focus, Focus::Command);
        assert_eq!(model.layout_mode, LayoutMode::Narrow);
        assert!(model.inspector_open);
        assert_eq!(model.workspace_scroll, 0);
        assert_eq!(
            model.message,
            Some(UiMessage {
                severity: Severity::Warning,
                text: "check setup".to_owned(),
            })
        );
        assert!(model.command_in_flight);
        model.clear_message();
        assert_eq!(model.message, None);
    }

    #[test]
    fn model_initialization_keeps_only_the_newest_hundred_audit_entries() {
        let mut snapshot = snapshot();
        snapshot.recent_audit = (1..=110).map(audit_entry).collect();

        let model = TuiModel::new(snapshot, false);

        assert_eq!(model.audit_entries.len(), COMMAND_HISTORY_CAPACITY);
        assert_eq!(
            model.audit_entries.first().map(|entry| entry.sequence),
            Some(11)
        );
        assert_eq!(
            model.audit_entries.last().map(|entry| entry.sequence),
            Some(110)
        );
        assert_eq!(model.audit_selection, Some(99));
    }

    #[test]
    fn audit_replacement_keeps_the_newest_hundred_entries() {
        let mut model = TuiModel::new(snapshot(), false);
        model.audit_selection = None;
        model.replace_audit((1..=110).map(audit_entry).collect());
        assert_eq!(model.audit_entries.len(), 100);
        assert_eq!(
            model.audit_entries.first().map(|entry| entry.sequence),
            Some(11)
        );
        assert_eq!(
            model.audit_entries.last().map(|entry| entry.sequence),
            Some(110)
        );
        assert_eq!(model.audit_selection, Some(99));
    }

    #[test]
    fn audit_selection_methods_are_total_and_clamped() {
        let mut model = TuiModel::new(snapshot(), false);
        model.replace_audit(Vec::new());
        model.select_previous_audit();
        model.select_next_audit();
        model.select_first_audit();
        model.select_last_audit();
        assert_eq!(model.audit_selection, None);

        model.replace_audit(vec![audit_entry(1), audit_entry(2)]);
        model.select_first_audit();
        model.select_previous_audit();
        assert_eq!(model.audit_selection, Some(0));
        model.select_next_audit();
        model.select_next_audit();
        assert_eq!(model.audit_selection, Some(1));
        model.select_last_audit();
        assert_eq!(model.audit_selection, Some(1));
    }

    #[test]
    fn memory_pending_cleanup_requires_the_exact_intent_and_generation() {
        let mut state = MemoryViewState::default();
        let generation = state
            .begin_pending(MemoryOutcomeIntent::Entries)
            .expect("pending generation");

        assert!(!state.clear_pending_if(&MemoryOutcomeIntent::Proposals, generation));
        assert!(!state.clear_pending_if(&MemoryOutcomeIntent::Entries, generation + 1));
        assert_eq!(state.pending_intent, Some(MemoryOutcomeIntent::Entries));

        assert!(state.clear_pending_if(&MemoryOutcomeIntent::Entries, generation));
        assert_eq!(state.pending_intent, None);
    }
}
