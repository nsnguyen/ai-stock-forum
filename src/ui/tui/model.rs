use std::collections::VecDeque;

use crate::{
    app::PresentationSnapshot,
    audit::AuditEntry,
    domain::{InstallationId, SessionId},
    setup::SetupStatus,
};

pub const COMMAND_HISTORY_CAPACITY: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Overview,
    Setup,
    Audit,
    Help,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandEditor {
    buffer: String,
    cursor_byte: usize,
    history: VecDeque<String>,
    history_index: Option<usize>,
}

impl Default for CommandEditor {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            cursor_byte: 0,
            history: VecDeque::new(),
            history_index: None,
        }
    }
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
        self.normalize_cursor();
        self.buffer.insert(self.cursor_byte, character);
        self.cursor_byte = self.cursor_byte.saturating_add(character.len_utf8());
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiModel {
    pub active_view: View,
    pub focus: Focus,
    pub layout_mode: LayoutMode,
    pub inspector_open: bool,
    pub command: CommandEditor,
    pub installation_id: InstallationId,
    pub session_id: SessionId,
    pub setup_status: SetupStatus,
    pub audit_entries: Vec<AuditEntry>,
    pub audit_selection: Option<usize>,
    pub workspace_scroll: u16,
    pub message: Option<UiMessage>,
    pub command_in_flight: bool,
    pub runtime_status: RuntimeStatus,
    pub previous_session_interrupted: bool,
}

impl TuiModel {
    pub fn new(snapshot: PresentationSnapshot, previous_session_interrupted: bool) -> Self {
        let PresentationSnapshot {
            installation_id,
            session_id,
            setup_status,
            recent_audit,
        } = snapshot;
        let mut model = Self {
            active_view: View::Overview,
            focus: Focus::Workspace,
            layout_mode: LayoutMode::Wide,
            inspector_open: false,
            command: CommandEditor::default(),
            installation_id,
            session_id,
            setup_status,
            audit_entries: Vec::new(),
            audit_selection: None,
            workspace_scroll: 0,
            message: None,
            command_in_flight: false,
            runtime_status: RuntimeStatus::Ready,
            previous_session_interrupted,
        };
        model.replace_audit(recent_audit);
        model
    }

    pub fn select_view(&mut self, view: View) {
        self.active_view = view;
    }

    pub fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
    }

    pub fn set_layout_mode(&mut self, layout_mode: LayoutMode) {
        self.layout_mode = layout_mode;
    }

    pub fn toggle_inspector(&mut self) {
        self.inspector_open = !self.inspector_open;
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

    pub fn replace_audit(&mut self, mut entries: Vec<AuditEntry>) {
        let first_newest = entries.len().saturating_sub(COMMAND_HISTORY_CAPACITY);
        if first_newest > 0 {
            entries = entries.split_off(first_newest);
        }
        self.audit_entries = entries;
        self.audit_selection = match (self.audit_selection, self.audit_last_index()) {
            (_, None) => None,
            (Some(selection), Some(last_index)) => Some(selection.min(last_index)),
            (None, Some(last_index)) => Some(last_index),
        };
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
        self.command_in_flight = command_in_flight;
    }

    pub fn set_runtime_status(&mut self, runtime_status: RuntimeStatus) {
        self.runtime_status = runtime_status;
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
        app::PresentationSnapshot,
        audit::AuditEntry,
        domain::{Actor, CorrelationId, InstallationId, SessionId},
        setup::SetupStatus,
    };

    fn snapshot() -> PresentationSnapshot {
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            setup_status: SetupStatus::NotStarted,
            recent_audit: vec![audit_entry(1)],
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
        model.set_layout_mode(LayoutMode::Narrow);
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
