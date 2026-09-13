use std::time::Duration;

use crossbeam_channel::Receiver;
use crossterm::event::{self, Event, KeyEventKind};

use super::{error::TuiError, model::NavigationTab};
use crate::ui::interrupt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Open,
    Create,
}

impl SkillKey {
    pub fn from_key(key: crossterm::event::KeyEvent) -> Option<Self> {
        use crossterm::event::{KeyCode, KeyModifiers};
        if key.modifiers != KeyModifiers::NONE {
            return None;
        }
        match key.code {
            KeyCode::Up => Some(Self::Up),
            KeyCode::Down => Some(Self::Down),
            KeyCode::Left => Some(Self::Left),
            KeyCode::Right => Some(Self::Right),
            KeyCode::Enter => Some(Self::Enter),
            KeyCode::Esc => Some(Self::Escape),
            KeyCode::Char(character)
                if NavigationTab::for_number(character) == Some(NavigationTab::Skills) =>
            {
                Some(Self::Open)
            }
            KeyCode::Char('c') => Some(Self::Create),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiEvent {
    Key(crossterm::event::KeyEvent),
    Paste(String),
    Resize(u16, u16),
    Interrupt,
}

pub trait EventSource {
    fn next_event(&mut self, timeout: Duration) -> Result<Option<TuiEvent>, TuiError>;
}

pub struct CrosstermEventSource {
    interrupt: Receiver<()>,
}

impl CrosstermEventSource {
    pub fn new() -> Result<Self, TuiError> {
        interrupt::receiver()
            .map(Self::from_receiver)
            .map_err(|_| TuiError::InterruptHandler)
    }

    fn from_receiver(interrupt: Receiver<()>) -> Self {
        Self { interrupt }
    }

    fn take_interrupt(&self) -> bool {
        self.interrupt.try_recv().is_ok()
    }
}

impl EventSource for CrosstermEventSource {
    fn next_event(&mut self, timeout: Duration) -> Result<Option<TuiEvent>, TuiError> {
        if self.take_interrupt() {
            return Ok(Some(TuiEvent::Interrupt));
        }

        if event::poll(timeout).map_err(|_| TuiError::TerminalInput)? {
            return event::read()
                .map(translate)
                .map_err(|_| TuiError::TerminalInput);
        }

        Ok(self.take_interrupt().then_some(TuiEvent::Interrupt))
    }
}

fn translate(event: Event) -> Option<TuiEvent> {
    match event {
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            Some(TuiEvent::Key(key))
        }
        Event::Resize(width, height) => Some(TuiEvent::Resize(width, height)),
        Event::Paste(text) => Some(TuiEvent::Paste(text)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossbeam_channel::bounded;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::{CrosstermEventSource, EventSource, SkillKey, TuiEvent, translate};

    #[test]
    fn skill_open_uses_the_authoritative_bare_four_destination() {
        assert_eq!(
            SkillKey::from_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE)),
            Some(SkillKey::Open)
        );
        assert_eq!(
            SkillKey::from_key(KeyEvent::new(KeyCode::Char('6'), KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            SkillKey::from_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn key_press_resize_and_interrupt_become_typed_events() {
        assert_eq!(
            translate(Event::Resize(90, 25)),
            Some(TuiEvent::Resize(90, 25))
        );
        assert!(matches!(
            translate(Event::Key(KeyEvent::new(
                KeyCode::Char('q'),
                KeyModifiers::NONE
            ))),
            Some(TuiEvent::Key(_))
        ));
        assert_eq!(translate(Event::FocusGained), None);

        let (sender, receiver) = bounded(1);
        sender.try_send(()).unwrap();
        let mut source = CrosstermEventSource::from_receiver(receiver);
        assert_eq!(
            source.next_event(Duration::from_secs(60)).unwrap(),
            Some(TuiEvent::Interrupt)
        );
    }

    #[test]
    fn key_repeat_events_are_forwarded() {
        let event =
            KeyEvent::new_with_kind(KeyCode::Char('x'), KeyModifiers::NONE, KeyEventKind::Repeat);
        assert_eq!(translate(Event::Key(event)), Some(TuiEvent::Key(event)));
    }

    #[test]
    fn key_release_events_are_ignored() {
        let event = KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(translate(Event::Key(event)), None);
    }

    #[test]
    fn unsupported_terminal_events_are_ignored() {
        assert_eq!(translate(Event::FocusLost), None);
    }

    #[test]
    fn paste_becomes_a_typed_tui_event() {
        assert_eq!(
            translate(Event::Paste("/status\n".to_owned())),
            Some(TuiEvent::Paste("/status\n".to_owned()))
        );
    }
}
