use std::collections::VecDeque;

use crate::{
    agents::ProfileTemplate,
    app::{
        AgentProfileHistoryView, AgentProfileVersionView, AgentProfileView, AgentProfilesView,
        ApplicationCommand, DatabaseReadiness, MAX_INPUT_BYTES, PresentationSnapshot,
        ProcessGuardOwnership, SkillHistoryView, SkillView, SkillsView,
    },
    audit::AuditEntry,
    domain::{InstallationId, SessionId},
    setup::SetupStatus,
    skills::{SkillDraft, SkillVersionRef},
    ui::{profile_editor::ProfileEditor, skill_editor::SkillEditor},
};

pub const COMMAND_HISTORY_CAPACITY: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Overview,
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
    AgentSkills { profile_id: crate::domain::AgentProfileId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillWorkspaceOrigin {
    Cockpit(View),
    AgentSkills { profile_id: crate::domain::AgentProfileId },
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
        let selected_id = self.selected_summary().map(|summary| summary.skill_ref.skill_id());
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
        actions[self.selected_action_index.min(actions.len().saturating_sub(1))]
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
                    self.detail.as_ref().map(|detail| detail.skill_ref.skill_id())
                        == Some(version.skill_ref.skill_id())
                })
                .map(|version| &version.skill_ref)
                .or_else(|| self.detail
                .as_ref()
                .map(|detail| &detail.skill_ref)
                .or_else(|| self.selected_summary().map(|summary| &summary.skill_ref)))
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
    pub fn select_profile_id(&mut self, profile_id: crate::domain::AgentProfileId) {
        if let Some(index) = self
            .profiles
            .profiles
            .iter()
            .position(|summary| summary.profile_id == profile_id)
        {
            self.select_profile_index(index);
        }
    }

    pub fn selected_skill_action(&self) -> AgentSkillAction {
        match self.selected_skill_action_index.min(2) {
            0 => AgentSkillAction::View,
            1 => AgentSkillAction::Upgrade,
            _ => AgentSkillAction::Unassign,
        }
    }

    pub fn selected_assigned_skill_ref(&self) -> Option<&SkillVersionRef> {
        self.detail
            .as_ref()
            .and_then(|detail| detail.profile.skill_refs().get(self.selected_assigned_skill))
    }

    pub fn selected_summary(&self) -> Option<&crate::app::AgentProfileSummary> {
        self.profiles.profiles.get(self.selected_profile)
    }

    fn select_profile_index(&mut self, index: usize) {
        self.selected_profile = index.min(self.profiles.profiles.len().saturating_sub(1));
        self.list_scroll = self.list_scroll.min(self.selected_profile);
    }

    pub fn replace_profiles(&mut self, profiles: AgentProfilesView) {
        let selected_id = self.selected_summary().map(|summary| summary.profile_id);
        self.profiles = profiles;
        if self.profiles.profiles.is_empty() {
            self.selected_profile = 0;
            self.list_scroll = 0;
            self.detail = None;
            self.history = None;
            self.version_detail = None;
            self.skill_panel_open = false;
            return;
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
        self.select_profile_index(selected_profile);
        let selected_id = self.selected_summary().map(|summary| summary.profile_id);
        if self
            .detail
            .as_ref()
            .map(|detail| detail.profile.profile_id())
            != selected_id
        {
            self.detail = None;
            self.history = None;
            self.version_detail = None;
        }
    }

    pub fn replace_detail(&mut self, detail: AgentProfileView) {
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
        self.selected_assigned_skill = self
            .selected_assigned_skill
            .min(self.detail.as_ref().map(|detail| detail.profile.skill_refs().len().saturating_sub(1)).unwrap_or(0));
    }

    pub fn replace_history(&mut self, history: AgentProfileHistoryView) {
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
    }

    pub fn replace_version_detail(&mut self, version: AgentProfileVersionView) {
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
    Workspace,
    Inspector,
    Command,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NavigationTab {
    Overview,
    Setup,
    Audit,
    Help,
    Agents,
    Skills,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingOutcomeNavigation {
    SelectIfUnchanged {
        tab: NavigationTab,
        generation: u64,
    },
    PreserveCurrent,
}

impl NavigationTab {
    const fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Setup => 1,
            Self::Audit => 2,
            Self::Help => 3,
            Self::Agents => 4,
            Self::Skills => 5,
        }
    }

    pub(super) const fn for_view(view: View) -> Self {
        match view {
            View::Overview => Self::Overview,
            View::Setup => Self::Setup,
            View::Audit => Self::Audit,
            View::Help => Self::Help,
            View::Agents => Self::Agents,
        }
    }

    pub(super) const fn adjacent(self, forward: bool) -> Self {
        match (self, forward) {
            (Self::Overview, true) | (Self::Audit, false) => Self::Setup,
            (Self::Setup, true) | (Self::Help, false) => Self::Audit,
            (Self::Audit, true) | (Self::Agents, false) => Self::Help,
            (Self::Help, true) | (Self::Skills, false) => Self::Agents,
            (Self::Agents, true) | (Self::Overview, false) => Self::Skills,
            (Self::Skills, true) | (Self::Setup, false) => Self::Overview,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TabState {
    focus: Focus,
    inspector_open: bool,
    workspace_scroll: u16,
    command_draft: CommandDraft,
}

impl Default for TabState {
    fn default() -> Self {
        Self {
            focus: Focus::Workspace,
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
    inspector_open: bool,
    command: CommandEditor,
    workspace_scroll: u16,
    message: Option<UiMessage>,
    pending_agent_outcome: Option<AgentOutcomeIntent>,
    navigation_generation: u64,
    pending_outcome_navigation: Option<PendingOutcomeNavigation>,
    tab_states: [TabState; 6],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiModel {
    pub active_view: View,
    pub agents: AgentsViewState,
    pub skills: SkillsViewState,
    pub pending_agent_outcome: Option<AgentOutcomeIntent>,
    pub focus: Focus,
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
    tab_states: [TabState; 6],
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
            View::Setup => NavigationTab::Setup,
            View::Audit => NavigationTab::Audit,
            View::Help => NavigationTab::Help,
            View::Agents => NavigationTab::Agents,
        }
    }

    fn swap_tab_state(&mut self, tab: NavigationTab) {
        let state = &mut self.tab_states[tab.index()];
        std::mem::swap(&mut self.focus, &mut state.focus);
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
        let area = ratatui::layout::Rect::new(
            0,
            0,
            self.terminal_width,
            self.terminal_height,
        );
        let geometry = if self.skills.active {
            super::layout::skill_geometry(area, self.inspector_open)
        } else {
            super::layout::view_geometry(area, self.active_view, self.inspector_open)
        };
        self.layout_mode = geometry.cockpit.mode;
        self.workspace_body_width = geometry.workspace_body_width;
        self.workspace_body_height = geometry.workspace_body_height;
    }

    fn normalize_visible_focus(&mut self) {
        if self.focus == Focus::Command {
            return;
        }
        let visible = match self.layout_mode {
            LayoutMode::Wide => true,
            LayoutMode::Medium => self.focus != Focus::Inspector || self.inspector_open,
            LayoutMode::Narrow => {
                self.focus != Focus::Navigation
                    && (self.focus != Focus::Inspector || self.inspector_open)
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
            self.pending_outcome_navigation = Some(
                PendingOutcomeNavigation::SelectIfUnchanged {
                    tab: self.active_navigation_tab(),
                    generation: self.navigation_generation,
                },
            );
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
                tab == self.active_navigation_tab()
                    && generation == self.navigation_generation
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
        const STANDARD: &[AgentSkillAction] =
            &[AgentSkillAction::View, AgentSkillAction::Unassign];
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
}
