use ai_stock_forum::{
    app::{ApplicationCommand, ApplicationEvent, PresentationSnapshot, ShutdownReason},
    domain::{InstallationId, SessionId},
    setup::SetupStatus,
    ui::tui::{
        ControllerEffect, TuiEvent, apply_outcome, handle_event,
        layout::calculate,
        model::{CommandEditor, TuiModel},
        render,
        theme::Theme,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proptest::prelude::*;
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use uuid::Uuid;

mod support;

fn contains(outer: Rect, inner: Rect) -> bool {
    if inner.x < outer.x || inner.y < outer.y {
        return false;
    }

    let x_offset = inner.x - outer.x;
    let y_offset = inner.y - outer.y;
    x_offset <= outer.width
        && y_offset <= outer.height
        && inner.width <= outer.width - x_offset
        && inner.height <= outer.height - y_offset
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_048))]

    #[test]
    fn calculated_rectangles_are_always_contained(
        x in any::<u16>(),
        y in any::<u16>(),
        width in any::<u16>(),
        height in any::<u16>(),
        inspector_open in any::<bool>(),
    ) {
        let area = Rect::new(x, y, width, height);
        let layout = calculate(area, inspector_open);

        prop_assert_eq!(layout.viewport, area);
        prop_assert!(contains(area, layout.header));
        prop_assert!(contains(area, layout.workspace));
        prop_assert!(contains(area, layout.message));
        prop_assert!(contains(area, layout.command));
        if let Some(rect) = layout.navigation {
            prop_assert!(contains(area, rect));
        }
        if let Some(rect) = layout.inspector {
            prop_assert!(contains(area, rect));
        }
    }
}

#[derive(Debug, Clone)]
enum EditorOperation {
    Insert(char),
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    Backspace,
    Delete,
    Clear,
    TakeText,
    Remember(String),
    HistoryPrevious,
    HistoryNext,
}

fn editor_operation() -> impl Strategy<Value = EditorOperation> {
    prop_oneof![
        8 => any::<char>().prop_map(EditorOperation::Insert),
        1 => Just(EditorOperation::MoveLeft),
        1 => Just(EditorOperation::MoveRight),
        1 => Just(EditorOperation::MoveHome),
        1 => Just(EditorOperation::MoveEnd),
        1 => Just(EditorOperation::Backspace),
        1 => Just(EditorOperation::Delete),
        1 => Just(EditorOperation::Clear),
        1 => Just(EditorOperation::TakeText),
        1 => any::<String>().prop_map(EditorOperation::Remember),
        1 => Just(EditorOperation::HistoryPrevious),
        1 => Just(EditorOperation::HistoryNext),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_unicode_editor_sequences_preserve_cursor_boundaries(
        operations in prop::collection::vec(editor_operation(), 0..=128),
    ) {
        let mut editor = CommandEditor::default();

        for operation in operations {
            match operation {
                EditorOperation::Insert(character) => editor.insert(character),
                EditorOperation::MoveLeft => editor.move_left(),
                EditorOperation::MoveRight => editor.move_right(),
                EditorOperation::MoveHome => editor.move_home(),
                EditorOperation::MoveEnd => editor.move_end(),
                EditorOperation::Backspace => editor.backspace(),
                EditorOperation::Delete => editor.delete(),
                EditorOperation::Clear => editor.clear(),
                EditorOperation::TakeText => {
                    let _ = editor.take_text();
                }
                EditorOperation::Remember(entry) => editor.remember(entry),
                EditorOperation::HistoryPrevious => editor.history_previous(),
                EditorOperation::HistoryNext => editor.history_next(),
            }

            prop_assert!(editor.cursor_byte() <= editor.text().len());
            prop_assert!(editor.text().is_char_boundary(editor.cursor_byte()));
        }
    }
}

#[test]
fn rejected_secret_input_is_absent_before_the_next_rendered_frame() {
    const INPUT: &str = "/unknown password=hunter2 token=abc";
    const FORBIDDEN: [&str; 3] = [INPUT, "hunter2", "token=abc"];
    const GENERIC_REJECTION: &str = "Command rejected. Check the command and try again.";

    let runtime = support::runtime();
    let mut model = TuiModel::new(snapshot(), false);
    for character in INPUT.chars() {
        let effect = handle_event(&mut model, key(KeyCode::Char(character)));
        assert_eq!(effect, ControllerEffect::Redraw);
    }

    let submitted = handle_event(&mut model, key(KeyCode::Enter));
    let command = match submitted {
        ControllerEffect::Submit(command @ ApplicationCommand::RejectInput(_)) => command,
        other => panic!("expected rejected submission, got {other:?}"),
    };
    assert_absent(&format!("{command:?}"), &FORBIDDEN);
    assert_eq!(model.command.text(), "");
    assert_eq!(model.command.history_len(), 0);

    let outcome = runtime
        .client()
        .submit(command)
        .expect("real runtime rejects unknown input");
    assert_eq!(outcome.committed_events.len(), 1);
    assert!(matches!(
        outcome.committed_events[0].event,
        ApplicationEvent::CommandRejected { .. }
    ));
    assert_absent(&format!("{outcome:?}"), &FORBIDDEN);

    let effect = apply_outcome(&mut model, outcome);
    assert_eq!(effect, ControllerEffect::Redraw);

    assert_absent(&format!("{model:?}"), &FORBIDDEN);
    let message = model.message.as_ref().expect("generic rejection message");
    assert_eq!(message.text, GENERIC_REJECTION);
    assert_absent(&message.text, &FORBIDDEN);
    assert_eq!(model.audit_entries.len(), 1);
    assert_eq!(model.audit_entries[0].kind, "command_rejected");
    assert_absent(&format!("{:?}", model.audit_entries), &FORBIDDEN);
    assert_eq!(model.command.text(), "");
    assert_eq!(model.command.history_len(), 0);
    let frame = render_text(&model);
    assert!(frame.contains(GENERIC_REJECTION));
    assert_absent(&frame, &FORBIDDEN);

    runtime.finish_and_join(ShutdownReason::ApplicationError);
}

fn snapshot() -> PresentationSnapshot {
    PresentationSnapshot {
        installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
        session_id: SessionId::from_uuid(Uuid::from_u128(2)),
        database_readiness: ai_stock_forum::app::DatabaseReadiness::Ready,
        process_guard_ownership: ai_stock_forum::app::ProcessGuardOwnership::Held,
        setup_status: SetupStatus::NotStarted,
        recent_audit: Vec::new(),
    }
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn render_text(model: &TuiModel) -> String {
    let backend = TestBackend::new(140, 40);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render::render(frame, model, &Theme::from_no_color(true)))
        .expect("render rejected input outcome");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn assert_absent(value: &str, forbidden: &[&str]) {
    for secret in forbidden {
        assert!(!value.contains(secret), "secret remained in output");
    }
}
