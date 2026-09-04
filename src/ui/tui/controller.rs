use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use super::{
    TuiEvent,
    layout::{layout_mode, workspace_body_size},
    model::{Focus, LayoutMode, RuntimeStatus, Severity, TuiModel, View},
    views,
};
use crate::{
    app::{ApplicationCommand, CommandOutcome, CommandView, ShutdownDisposition, ShutdownReason},
    audit::AuditEntry,
    ui::command::{ParsedLine, parse_line},
};

const COMMAND_IN_FLIGHT_MESSAGE: &str = "A command is already running.";
const COMMAND_REJECTED_MESSAGE: &str = "Command rejected. Check the command and try again.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerEffect {
    None,
    Redraw,
    Submit(ApplicationCommand),
    RequestShutdown(ShutdownReason),
}

pub fn handle_event(model: &mut TuiModel, event: TuiEvent) -> ControllerEffect {
    let effect = match event {
        TuiEvent::Interrupt => ControllerEffect::RequestShutdown(ShutdownReason::Interrupted),
        TuiEvent::Resize(width, height) => {
            let area = Rect::new(0, 0, width, height);
            model.set_layout_mode(layout_mode(area));
            let (body_width, body_height) = workspace_body_size(area, model.inspector_open);
            model.set_workspace_body_size(body_width, body_height);
            normalize_focus(model);
            ControllerEffect::Redraw
        }
        TuiEvent::Paste(text) => handle_paste(model, &text),
        TuiEvent::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            handle_key(model, key)
        }
        TuiEvent::Key(_) => ControllerEffect::None,
    };
    clamp_workspace_scroll(model);
    effect
}

pub fn apply_outcome(model: &mut TuiModel, outcome: CommandOutcome) -> ControllerEffect {
    model.set_command_in_flight(false);

    let CommandOutcome {
        committed_events,
        view,
        shutdown,
        ..
    } = outcome;
    let committed_audit = committed_events
        .iter()
        .map(AuditEntry::from_event)
        .collect::<Vec<_>>();

    let view_shutdown = match view {
        CommandView::Help(_) => {
            select_workspace_view(model, View::Help);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::Status(status) => {
            model.installation_id = status.installation_id;
            model.session_id = status.session_id;
            select_workspace_view(model, View::Overview);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::SetupStatus(setup) => {
            model.setup_status = setup.status;
            select_workspace_view(model, View::Setup);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::AuditTail(audit) => {
            model.replace_audit(audit.entries);
            select_workspace_view(model, View::Audit);
            model.clear_message();
            ShutdownDisposition::Continue
        }
        CommandView::InputRejected(_) => {
            model.set_focus(Focus::Command);
            model.set_message(Severity::Error, COMMAND_REJECTED_MESSAGE);
            ShutdownDisposition::Continue
        }
        CommandView::Shutdown(shutdown) => {
            model.set_runtime_status(RuntimeStatus::Stopping);
            shutdown.disposition
        }
    };

    merge_committed_audit(model, committed_audit);
    clamp_workspace_scroll(model);

    if requests_shutdown(shutdown) || requests_shutdown(view_shutdown) {
        model.set_runtime_status(RuntimeStatus::Stopping);
        ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
    } else {
        ControllerEffect::Redraw
    }
}

fn handle_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    if is_ctrl_c(key) {
        return ControllerEffect::RequestShutdown(ShutdownReason::Interrupted);
    }

    if model.layout_mode == LayoutMode::TooSmall {
        return if is_plain_char(key, 'q') {
            ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
        } else {
            ControllerEffect::None
        };
    }

    if model.focus == Focus::Command {
        return handle_command_key(model, key);
    }

    match key.code {
        KeyCode::Char('1') if no_modifiers(key.modifiers) => select_view(model, View::Overview),
        KeyCode::Char('2') if no_modifiers(key.modifiers) => select_view(model, View::Setup),
        KeyCode::Char('3') if no_modifiers(key.modifiers) => select_view(model, View::Audit),
        KeyCode::Char('4') if no_modifiers(key.modifiers) => select_view(model, View::Help),
        KeyCode::Char('?') if text_modifiers(key.modifiers) => select_view(model, View::Help),
        KeyCode::Char('/') if no_modifiers(key.modifiers) => {
            model.command.clear();
            model.command.insert('/');
            model.set_focus(Focus::Command);
            ControllerEffect::Redraw
        }
        KeyCode::Char('i') if no_modifiers(key.modifiers) => toggle_or_focus_inspector(model),
        KeyCode::Char('q') if no_modifiers(key.modifiers) => {
            ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
        }
        KeyCode::Tab if no_modifiers(key.modifiers) => cycle_focus(model, true),
        KeyCode::BackTab if backtab_modifiers(key.modifiers) => cycle_focus(model, false),
        KeyCode::Esc if no_modifiers(key.modifiers) => dismiss(model),
        KeyCode::Up | KeyCode::Left if no_modifiers(key.modifiers) => {
            move_focused(model, false, false)
        }
        KeyCode::Down | KeyCode::Right if no_modifiers(key.modifiers) => {
            move_focused(model, true, false)
        }
        KeyCode::PageUp if no_modifiers(key.modifiers) => move_focused(model, false, true),
        KeyCode::PageDown if no_modifiers(key.modifiers) => move_focused(model, true, true),
        KeyCode::Home if no_modifiers(key.modifiers) => move_to_bound(model, false),
        KeyCode::End if no_modifiers(key.modifiers) => move_to_bound(model, true),
        _ => ControllerEffect::None,
    }
}

fn handle_command_key(model: &mut TuiModel, key: KeyEvent) -> ControllerEffect {
    match key.code {
        KeyCode::Esc if no_modifiers(key.modifiers) => {
            model.command.clear();
            model.set_focus(Focus::Workspace);
            ControllerEffect::Redraw
        }
        KeyCode::Enter if no_modifiers(key.modifiers) => submit_command(model),
        KeyCode::Left if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_left())
        }
        KeyCode::Right if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_right())
        }
        KeyCode::Home if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_home())
        }
        KeyCode::End if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.move_end())
        }
        KeyCode::Backspace if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.backspace())
        }
        KeyCode::Delete if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.delete())
        }
        KeyCode::Up if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.history_previous())
        }
        KeyCode::Down if no_modifiers(key.modifiers) => {
            edit(model, |model| model.command.history_next())
        }
        KeyCode::Tab if no_modifiers(key.modifiers) => cycle_focus(model, true),
        KeyCode::BackTab if backtab_modifiers(key.modifiers) => cycle_focus(model, false),
        KeyCode::Char(character) if text_modifiers(key.modifiers) => {
            model.command.insert(character);
            ControllerEffect::Redraw
        }
        _ => ControllerEffect::None,
    }
}

fn submit_command(model: &mut TuiModel) -> ControllerEffect {
    if model.command_in_flight {
        model.set_message(Severity::Warning, COMMAND_IN_FLIGHT_MESSAGE);
        return ControllerEffect::None;
    }

    let input = model.command.take_text();
    match parse_line(input.as_bytes()) {
        ParsedLine::Ignored => ControllerEffect::None,
        ParsedLine::Command(command) => {
            if !matches!(command, ApplicationCommand::RejectInput(_)) {
                model.command.remember(input);
            }
            model.clear_message();
            model.set_command_in_flight(true);
            ControllerEffect::Submit(command)
        }
    }
}

fn handle_paste(model: &mut TuiModel, text: &str) -> ControllerEffect {
    if model.layout_mode == LayoutMode::TooSmall || model.focus != Focus::Command {
        return ControllerEffect::None;
    }
    let before = model.command.text().len();
    model.command.ingest(text);
    if model.command.text().len() == before {
        ControllerEffect::None
    } else {
        ControllerEffect::Redraw
    }
}

fn select_view(model: &mut TuiModel, view: View) -> ControllerEffect {
    select_workspace_view(model, view);
    ControllerEffect::Redraw
}

fn select_workspace_view(model: &mut TuiModel, view: View) {
    model.select_view(view);
    model.set_focus(Focus::Workspace);
    model.scroll_home();
}

fn toggle_or_focus_inspector(model: &mut TuiModel) -> ControllerEffect {
    if model.focus == Focus::Inspector {
        model.inspector_open = false;
        model.set_focus(Focus::Workspace);
    } else {
        model.inspector_open = true;
        model.set_focus(Focus::Inspector);
    }
    ControllerEffect::Redraw
}

fn dismiss(model: &mut TuiModel) -> ControllerEffect {
    if model.focus == Focus::Inspector || model.inspector_open {
        model.inspector_open = false;
        model.set_focus(Focus::Workspace);
        ControllerEffect::Redraw
    } else if model.message.is_some() {
        model.clear_message();
        ControllerEffect::Redraw
    } else {
        ControllerEffect::None
    }
}

fn cycle_focus(model: &mut TuiModel, forward: bool) -> ControllerEffect {
    let order = visible_focus_order(model);
    let current = order
        .iter()
        .position(|focus| *focus == model.focus)
        .unwrap_or(0);
    let next = if forward {
        current.saturating_add(1) % order.len()
    } else if current == 0 {
        order.len().saturating_sub(1)
    } else {
        current - 1
    };
    model.set_focus(order[next]);
    ControllerEffect::Redraw
}

fn visible_focus_order(model: &TuiModel) -> &'static [Focus] {
    const WIDE: &[Focus] = &[
        Focus::Navigation,
        Focus::Workspace,
        Focus::Inspector,
        Focus::Command,
    ];
    const MEDIUM: &[Focus] = &[Focus::Navigation, Focus::Workspace, Focus::Command];
    const MEDIUM_INSPECTOR: &[Focus] = &[
        Focus::Navigation,
        Focus::Workspace,
        Focus::Inspector,
        Focus::Command,
    ];
    const NARROW: &[Focus] = &[Focus::Workspace, Focus::Command];
    const NARROW_INSPECTOR: &[Focus] = &[Focus::Workspace, Focus::Inspector, Focus::Command];

    match (model.layout_mode, model.inspector_open) {
        (LayoutMode::Wide, _) => WIDE,
        (LayoutMode::Medium, true) => MEDIUM_INSPECTOR,
        (LayoutMode::Medium, false) => MEDIUM,
        (LayoutMode::Narrow, true) => NARROW_INSPECTOR,
        (LayoutMode::Narrow, false) => NARROW,
        (LayoutMode::TooSmall, _) => &[Focus::Workspace],
    }
}

fn normalize_focus(model: &mut TuiModel) {
    if !visible_focus_order(model).contains(&model.focus) {
        model.set_focus(Focus::Workspace);
    }
}

fn move_focused(model: &mut TuiModel, forward: bool, page: bool) -> ControllerEffect {
    if model.focus == Focus::Navigation {
        model.select_view(adjacent_view(model.active_view, forward));
        model.scroll_home();
        return ControllerEffect::Redraw;
    }

    if model.active_view == View::Audit {
        let count = if page {
            usize::from(model.workspace_body_height.max(1))
        } else {
            1
        };
        for _ in 0..count {
            if forward {
                model.select_next_audit();
            } else {
                model.select_previous_audit();
            }
        }
    } else if forward {
        model.scroll_down(if page {
            model.workspace_body_height.max(1)
        } else {
            1
        });
    } else {
        model.scroll_up(if page {
            model.workspace_body_height.max(1)
        } else {
            1
        });
    }
    ControllerEffect::Redraw
}

fn move_to_bound(model: &mut TuiModel, end: bool) -> ControllerEffect {
    if model.focus == Focus::Navigation {
        model.select_view(if end { View::Help } else { View::Overview });
    } else if model.active_view == View::Audit {
        if end {
            model.select_last_audit();
        } else {
            model.select_first_audit();
        }
    } else if end {
        model.workspace_scroll = workspace_max_scroll(model);
    } else {
        model.scroll_home();
    }
    ControllerEffect::Redraw
}

fn workspace_max_scroll(model: &TuiModel) -> u16 {
    if model.active_view == View::Audit {
        return 0;
    }
    views::workspace_content_height(model, model.workspace_body_width)
        .saturating_sub(model.workspace_body_height)
}

fn clamp_workspace_scroll(model: &mut TuiModel) {
    model.workspace_scroll = model.workspace_scroll.min(workspace_max_scroll(model));
}

fn adjacent_view(view: View, forward: bool) -> View {
    match (view, forward) {
        (View::Overview, true) | (View::Audit, false) => View::Setup,
        (View::Setup, true) | (View::Help, false) => View::Audit,
        (View::Audit, true) | (View::Overview, false) => View::Help,
        (View::Help, true) | (View::Setup, false) => View::Overview,
    }
}

fn merge_committed_audit(model: &mut TuiModel, committed: Vec<AuditEntry>) {
    let mut entries = std::mem::take(&mut model.audit_entries);
    for entry in committed {
        if let Some(existing) = entries
            .iter_mut()
            .find(|existing| existing.sequence == entry.sequence)
        {
            *existing = entry;
        } else {
            entries.push(entry);
        }
    }
    entries.sort_by_key(|entry| entry.sequence);
    model.replace_audit(entries);
}

fn edit(model: &mut TuiModel, operation: impl FnOnce(&mut TuiModel)) -> ControllerEffect {
    operation(model);
    ControllerEffect::Redraw
}

fn requests_shutdown(disposition: ShutdownDisposition) -> bool {
    match disposition {
        ShutdownDisposition::Continue => false,
        ShutdownDisposition::Requested => true,
    }
}

fn is_ctrl_c(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL
}

fn is_plain_char(key: KeyEvent, expected: char) -> bool {
    key.code == KeyCode::Char(expected) && no_modifiers(key.modifiers)
}

fn no_modifiers(modifiers: KeyModifiers) -> bool {
    modifiers == KeyModifiers::NONE
}

fn backtab_modifiers(modifiers: KeyModifiers) -> bool {
    matches!(modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
}

fn text_modifiers(modifiers: KeyModifiers) -> bool {
    matches!(modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use uuid::Uuid;

    use super::{ControllerEffect, apply_outcome, handle_event};
    use crate::{
        app::{
            ApplicationCommand, ApplicationEvent, AuditLimit, AuditTailView, CommandOutcome,
            CommandView, EventEnvelope, HelpView, InputRejectedView, InputRejection,
            InputRejectionCategory, MAX_INPUT_BYTES, PresentationSnapshot, SetupStatusView,
            ShutdownDisposition, ShutdownReason, ShutdownView, StatusView,
        },
        audit::AuditEntry,
        domain::{
            Actor, CommandId, ConfigurationVersionId, CorrelationId, EventId, InstallationId,
            SessionId, sha256,
        },
        setup::SetupStatus,
        ui::tui::{
            TuiEvent,
            model::{Focus, LayoutMode, Severity, TuiModel, View},
        },
    };

    fn model() -> TuiModel {
        TuiModel::new(
            PresentationSnapshot {
                installation_id: installation_id(1),
                session_id: session_id(2),
                database_readiness: crate::app::DatabaseReadiness::Ready,
                process_guard_ownership: crate::app::ProcessGuardOwnership::Held,
                setup_status: SetupStatus::NotStarted,
                recent_audit: Vec::new(),
            },
            false,
        )
    }

    fn command_model(text: &str) -> TuiModel {
        let mut model = model();
        model.focus = Focus::Command;
        for character in text.chars() {
            model.command.insert(character);
        }
        model
    }

    fn key(character: char) -> TuiEvent {
        key_code(KeyCode::Char(character), KeyModifiers::NONE)
    }

    fn key_code(code: KeyCode, modifiers: KeyModifiers) -> TuiEvent {
        TuiEvent::Key(KeyEvent::new(code, modifiers))
    }

    fn enter() -> TuiEvent {
        key_code(KeyCode::Enter, KeyModifiers::NONE)
    }

    fn assert_redraw_and_view(model: &mut TuiModel, event: TuiEvent, view: View) {
        assert_eq!(handle_event(model, event), ControllerEffect::Redraw);
        assert_eq!(model.active_view, view);
    }

    fn installation_id(value: u128) -> InstallationId {
        InstallationId::from_uuid(Uuid::from_u128(value))
    }

    fn session_id(value: u128) -> SessionId {
        SessionId::from_uuid(Uuid::from_u128(value))
    }

    fn audit_entry(sequence: u64) -> AuditEntry {
        AuditEntry {
            sequence,
            occurred_at_ms: sequence as i64,
            actor: Actor::System,
            kind: "help_viewed".to_owned(),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(sequence as u128 + 100)),
            summary: "help viewed".to_owned(),
        }
    }

    fn envelope(sequence: u64, event: ApplicationEvent) -> EventEnvelope {
        EventEnvelope {
            sequence,
            event_id: EventId::from_uuid(Uuid::from_u128(sequence as u128 + 200)),
            event_schema_version: crate::app::EVENT_SCHEMA_VERSION,
            actor: Actor::Human,
            occurred_at_ms: sequence as i64,
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(sequence as u128 + 300)),
            causation_id: None,
            object: None,
            event,
            previous_event_digest: (sequence > 1).then(|| sha256(b"previous")),
            event_digest: sha256(format!("event-{sequence}").as_bytes()),
        }
    }

    fn outcome(
        view: CommandView,
        committed_events: Vec<EventEnvelope>,
        shutdown: ShutdownDisposition,
    ) -> CommandOutcome {
        CommandOutcome {
            command_id: CommandId::from_uuid(Uuid::from_u128(400)),
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(401)),
            committed_events,
            view,
            shutdown,
        }
    }

    #[test]
    fn global_keys_switch_views_focus_inspector_command_and_shutdown() {
        let mut model = model();
        assert_redraw_and_view(&mut model, key('2'), View::Setup);
        assert_redraw_and_view(&mut model, key('3'), View::Audit);
        assert_redraw_and_view(&mut model, key('4'), View::Help);
        assert_eq!(handle_event(&mut model, key('i')), ControllerEffect::Redraw);
        assert!(model.inspector_open);
        assert_eq!(model.focus, Focus::Inspector);
        assert_eq!(handle_event(&mut model, key('/')), ControllerEffect::Redraw);
        assert_eq!(model.focus, Focus::Command);
        assert_eq!(model.command.text(), "/");
        assert_eq!(handle_event(&mut model, key('q')), ControllerEffect::Redraw);
        assert_eq!(model.command.text(), "/q");
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Esc, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert!(matches!(
            handle_event(&mut model, key('q')),
            ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
        ));
    }

    #[test]
    fn ctrl_c_and_interrupt_request_the_canonical_interrupted_shutdown_globally() {
        for mut model in [command_model("q"), model()] {
            model.layout_mode = LayoutMode::TooSmall;
            assert_eq!(
                handle_event(
                    &mut model,
                    key_code(KeyCode::Char('c'), KeyModifiers::CONTROL),
                ),
                ControllerEffect::RequestShutdown(ShutdownReason::Interrupted)
            );
        }
        assert_eq!(
            handle_event(&mut model(), TuiEvent::Interrupt),
            ControllerEffect::RequestShutdown(ShutdownReason::Interrupted)
        );
    }

    #[test]
    fn unsupported_modified_chords_are_ignored_without_changing_model_state() {
        let mut navigation = model();
        navigation.focus = Focus::Navigation;
        navigation.active_view = View::Setup;

        let cases = [
            (
                model(),
                key_code(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                ),
            ),
            (
                model(),
                key_code(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL | KeyModifiers::ALT,
                ),
            ),
            (model(), key_code(KeyCode::Tab, KeyModifiers::CONTROL)),
            (
                command_model("/status"),
                key_code(KeyCode::Enter, KeyModifiers::ALT),
            ),
            (
                command_model("abc"),
                key_code(KeyCode::Left, KeyModifiers::ALT),
            ),
            (
                command_model("abc"),
                key_code(KeyCode::Delete, KeyModifiers::SUPER),
            ),
            (
                command_model("abc"),
                key_code(KeyCode::Char('x'), KeyModifiers::META),
            ),
            (navigation, key_code(KeyCode::Down, KeyModifiers::ALT)),
            (model(), key_code(KeyCode::Char('q'), KeyModifiers::SHIFT)),
        ];

        for (mut model, event) in cases {
            let before = model.clone();
            assert_eq!(handle_event(&mut model, event), ControllerEffect::None);
            assert_eq!(model, before);
        }
    }

    #[test]
    fn hyper_modified_enter_is_ignored_without_submitting_or_editing() {
        let mut model = command_model("/status");
        let before = model.clone();

        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Enter, KeyModifiers::HYPER),),
            ControllerEffect::None
        );
        assert_eq!(model, before);
    }

    #[test]
    fn shifted_question_mark_opens_help_outside_command_focus() {
        let mut model = model();
        model.active_view = View::Setup;

        assert_eq!(
            handle_event(
                &mut model,
                key_code(KeyCode::Char('?'), KeyModifiers::SHIFT),
            ),
            ControllerEffect::Redraw
        );
        assert_eq!(model.active_view, View::Help);
        assert_eq!(model.focus, Focus::Workspace);
    }

    #[test]
    fn exact_shift_bindings_enter_text_and_move_focus_backward() {
        let mut command = command_model("a");
        assert_eq!(
            handle_event(
                &mut command,
                key_code(KeyCode::Char('B'), KeyModifiers::SHIFT),
            ),
            ControllerEffect::Redraw
        );
        assert_eq!(command.command.text(), "aB");

        let mut workspace = model();
        assert_eq!(
            handle_event(
                &mut workspace,
                key_code(KeyCode::BackTab, KeyModifiers::SHIFT),
            ),
            ControllerEffect::Redraw
        );
        assert_eq!(workspace.focus, Focus::Navigation);
    }

    #[test]
    fn enter_uses_the_authoritative_parser_and_marks_one_command_in_flight() {
        let mut model = command_model("/status");
        let effect = handle_event(&mut model, enter());
        assert_eq!(
            effect,
            ControllerEffect::Submit(ApplicationCommand::ShowStatus)
        );
        assert!(model.command_in_flight);
        assert_eq!(model.command.text(), "");
        assert_eq!(model.command.history_back(), Some("/status"));
    }

    #[test]
    fn enter_during_an_in_flight_command_does_not_submit_again() {
        let mut model = command_model("/audit 5");
        model.command_in_flight = true;
        assert_eq!(handle_event(&mut model, enter()), ControllerEffect::None);
        assert_eq!(model.message.as_ref().unwrap().severity, Severity::Warning);
        assert_eq!(model.command.text(), "/audit 5");
    }

    #[test]
    fn ignored_and_rejected_input_do_not_leave_raw_text_in_the_model() {
        let mut blank = command_model("   ");
        assert_eq!(handle_event(&mut blank, enter()), ControllerEffect::None);
        assert_eq!(blank.focus, Focus::Command);
        assert_eq!(blank.command.text(), "");

        let secret = "/unknown credential=top-secret";
        let mut rejected = command_model(secret);
        assert!(matches!(
            handle_event(&mut rejected, enter()),
            ControllerEffect::Submit(ApplicationCommand::RejectInput(_))
        ));
        assert_eq!(rejected.command.text(), "");
        assert_eq!(rejected.command.history_len(), 0);
        assert!(!format!("{rejected:?}").contains("top-secret"));
    }

    #[test]
    fn command_focus_edits_text_and_traverses_memory_only_history() {
        let mut model = command_model("ac");
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Left, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(handle_event(&mut model, key('b')), ControllerEffect::Redraw);
        assert_eq!(model.command.text(), "abc");
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Backspace, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.command.text(), "ac");
        model.command.remember("/help".to_owned());
        model.command.remember("/status".to_owned());
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Up, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.command.text(), "/status");
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Down, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.command.text(), "");
    }

    #[test]
    fn too_small_ignores_keys_except_quit_and_ctrl_c() {
        let mut tiny = model();
        tiny.layout_mode = LayoutMode::TooSmall;
        let before = tiny.clone();
        for event in [
            key('1'),
            key('i'),
            key('/'),
            key('?'),
            key_code(KeyCode::Tab, KeyModifiers::NONE),
            key_code(KeyCode::Enter, KeyModifiers::NONE),
        ] {
            assert_eq!(handle_event(&mut tiny, event), ControllerEffect::None);
            assert_eq!(tiny, before);
        }
        assert_eq!(
            handle_event(&mut tiny, key('q')),
            ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
        );

        let mut command_tiny = command_model("");
        command_tiny.layout_mode = LayoutMode::TooSmall;
        assert_eq!(
            handle_event(&mut command_tiny, key('q')),
            ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
        );
        assert_eq!(command_tiny.command.text(), "");
    }

    #[test]
    fn resize_focus_and_navigation_keys_update_only_model_state() {
        let mut model = model();
        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(59, 18)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.layout_mode, LayoutMode::TooSmall);
        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(120, 30)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.layout_mode, LayoutMode::Wide);

        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Tab, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.focus, Focus::Inspector);
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::BackTab, KeyModifiers::SHIFT)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.focus, Focus::Workspace);

        model.active_view = View::Audit;
        model.replace_audit(vec![audit_entry(1), audit_entry(2), audit_entry(3)]);
        model.audit_selection = Some(1);
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Up, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.audit_selection, Some(0));
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::End, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.audit_selection, Some(2));
        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::Home, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.audit_selection, Some(0));
    }

    #[test]
    fn outcomes_map_every_native_view_clear_the_gate_and_keep_only_safe_state() {
        let mut model = model();
        model.command_in_flight = true;
        assert_eq!(
            apply_outcome(
                &mut model,
                outcome(
                    CommandView::Help(HelpView),
                    vec![envelope(1, ApplicationEvent::HelpViewed)],
                    ShutdownDisposition::Continue,
                ),
            ),
            ControllerEffect::Redraw
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.active_view, View::Help);
        assert_eq!(
            model.audit_entries,
            vec![audit_entry_from_event(1, ApplicationEvent::HelpViewed)]
        );

        let next_installation = installation_id(11);
        let next_session = session_id(12);
        model.command_in_flight = true;
        assert_eq!(
            apply_outcome(
                &mut model,
                outcome(
                    CommandView::Status(StatusView {
                        installation_id: next_installation,
                        session_id: next_session,
                    }),
                    Vec::new(),
                    ShutdownDisposition::Continue,
                ),
            ),
            ControllerEffect::Redraw
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.active_view, View::Overview);
        assert_eq!(model.installation_id, next_installation);
        assert_eq!(model.session_id, next_session);

        let setup_status = SetupStatus::Applied {
            configuration_id: ConfigurationVersionId::from_uuid(Uuid::from_u128(13)),
        };
        model.command_in_flight = true;
        apply_outcome(
            &mut model,
            outcome(
                CommandView::SetupStatus(SetupStatusView {
                    status: setup_status.clone(),
                }),
                Vec::new(),
                ShutdownDisposition::Continue,
            ),
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.active_view, View::Setup);
        assert_eq!(model.setup_status, setup_status);

        model.command_in_flight = true;
        apply_outcome(
            &mut model,
            outcome(
                CommandView::AuditTail(AuditTailView {
                    limit: AuditLimit::new(100).unwrap(),
                    entries: (1..=105).map(audit_entry).collect(),
                }),
                Vec::new(),
                ShutdownDisposition::Continue,
            ),
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.active_view, View::Audit);
        assert_eq!(model.audit_entries.len(), 100);
        assert_eq!(model.audit_entries.first().unwrap().sequence, 6);

        let secret = b"/unknown credential=top-secret";
        model.command_in_flight = true;
        apply_outcome(
            &mut model,
            outcome(
                CommandView::InputRejected(InputRejectedView {
                    rejection: InputRejection::from_input(
                        InputRejectionCategory::Unknown,
                        None,
                        secret,
                    ),
                }),
                Vec::new(),
                ShutdownDisposition::Continue,
            ),
        );
        assert!(!model.command_in_flight);
        assert_eq!(model.message.as_ref().unwrap().severity, Severity::Error);
        assert!(!format!("{model:?}").contains("top-secret"));

        model.command_in_flight = true;
        assert_eq!(
            apply_outcome(
                &mut model,
                outcome(
                    CommandView::Shutdown(ShutdownView {
                        disposition: ShutdownDisposition::Requested,
                    }),
                    Vec::new(),
                    ShutdownDisposition::Requested,
                ),
            ),
            ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
        );
        assert!(!model.command_in_flight);
    }

    #[test]
    fn committed_events_append_as_the_newest_one_hundred_audit_entries() {
        let mut model = model();
        model.replace_audit((1..=75).map(audit_entry).collect());
        let committed = (76..=125)
            .map(|sequence| envelope(sequence, ApplicationEvent::StatusViewed))
            .collect();
        apply_outcome(
            &mut model,
            outcome(
                CommandView::Status(StatusView {
                    installation_id: installation_id(1),
                    session_id: session_id(2),
                }),
                committed,
                ShutdownDisposition::Continue,
            ),
        );
        assert_eq!(model.audit_entries.len(), 100);
        assert_eq!(model.audit_entries.first().unwrap().sequence, 26);
        assert_eq!(model.audit_entries.last().unwrap().sequence, 125);
        assert!(
            model.audit_entries[75..]
                .iter()
                .all(|entry| entry.kind == "status_viewed")
        );
    }

    #[test]
    fn too_small_q_quits_even_after_command_focus_was_active_before_resize() {
        let mut model = model();
        assert_eq!(handle_event(&mut model, key('/')), ControllerEffect::Redraw);
        assert_eq!(model.focus, Focus::Command);
        assert_eq!(
            handle_event(&mut model, TuiEvent::Resize(59, 17)),
            ControllerEffect::Redraw
        );

        assert_eq!(
            handle_event(&mut model, key('q')),
            ControllerEffect::RequestShutdown(ShutdownReason::UserQuit)
        );
    }

    #[test]
    fn paste_is_ignored_outside_command_focus_and_bounded_and_sanitized_inside() {
        let mut model = model();
        assert_eq!(
            handle_event(&mut model, TuiEvent::Paste("/status".to_owned())),
            ControllerEffect::None
        );
        assert!(model.command.text().is_empty());

        model.set_focus(Focus::Command);
        let pasted = format!("{}\n\t界", "a".repeat(MAX_INPUT_BYTES - 1));
        assert_eq!(
            handle_event(&mut model, TuiEvent::Paste(pasted)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.command.text().len(), MAX_INPUT_BYTES - 1);
        assert!(!model.command.text().chars().any(char::is_control));
        assert!(
            model
                .command
                .text()
                .is_char_boundary(model.command.text().len())
        );
    }

    #[test]
    fn audit_end_and_page_navigation_remain_selection_based() {
        let mut model = model();
        model.select_view(View::Audit);
        model.replace_audit((1..=30).map(audit_entry).collect());
        handle_event(&mut model, TuiEvent::Resize(60, 18));

        assert_eq!(
            handle_event(&mut model, key_code(KeyCode::End, KeyModifiers::NONE)),
            ControllerEffect::Redraw
        );
        assert_eq!(model.audit_selection, Some(29));
        assert_eq!(model.workspace_scroll, 0);

        handle_event(&mut model, key_code(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(model.audit_selection, Some(29));
        assert_eq!(model.workspace_scroll, 0);
    }

    fn audit_entry_from_event(sequence: u64, event: ApplicationEvent) -> AuditEntry {
        AuditEntry::from_event(&envelope(sequence, event))
    }
}
