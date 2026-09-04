use std::io::{self, Stdout, Write, stdout};

use crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};

use super::{error::TuiError, model::TuiModel, render, theme::Theme};

pub trait Screen {
    fn size(&self) -> Result<Rect, TuiError>;
    fn draw(&mut self, model: &TuiModel, theme: &Theme) -> Result<(), TuiError>;
    fn restore(&mut self) -> Result<(), TuiError>;
}

trait TerminalControl {
    fn enable_raw(&mut self) -> io::Result<()>;
    fn enter_alt(&mut self) -> io::Result<()>;
    fn hide_cursor(&mut self) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
    fn leave_alt(&mut self) -> io::Result<()>;
    fn disable_raw(&mut self) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

struct TerminalGuard<C: TerminalControl> {
    control: C,
    raw_enabled: bool,
    alternate_screen_entered: bool,
    cursor_hidden: bool,
    flush_required: bool,
}

impl<C: TerminalControl> TerminalGuard<C> {
    fn enter(control: C) -> Result<Self, TuiError> {
        let mut guard = Self {
            control,
            raw_enabled: false,
            alternate_screen_entered: false,
            cursor_hidden: false,
            flush_required: false,
        };

        guard
            .control
            .enable_raw()
            .map_err(|_| TuiError::TerminalInitialization)?;
        guard.raw_enabled = true;
        guard.flush_required = true;

        if guard.control.enter_alt().is_err() {
            let _ = guard.restore();
            return Err(TuiError::TerminalInitialization);
        }
        guard.alternate_screen_entered = true;

        if guard.control.hide_cursor().is_err() {
            let _ = guard.restore();
            return Err(TuiError::TerminalInitialization);
        }
        guard.cursor_hidden = true;

        Ok(guard)
    }

    fn restore(&mut self) -> Result<(), TuiError> {
        let mut first_error = None;

        if self.cursor_hidden {
            self.cursor_hidden = false;
            retain_first_error(
                &mut first_error,
                self.control
                    .show_cursor()
                    .map_err(|_| TuiError::TerminalOutput),
            );
        }
        if self.alternate_screen_entered {
            self.alternate_screen_entered = false;
            retain_first_error(
                &mut first_error,
                self.control.leave_alt().map_err(|_| TuiError::TerminalOutput),
            );
        }
        if self.raw_enabled {
            self.raw_enabled = false;
            retain_first_error(
                &mut first_error,
                self.control
                    .disable_raw()
                    .map_err(|_| TuiError::TerminalOutput),
            );
        }
        if self.flush_required {
            self.flush_required = false;
            retain_first_error(
                &mut first_error,
                self.control.flush().map_err(|_| TuiError::TerminalOutput),
            );
        }

        first_error.map_or(Ok(()), Err)
    }
}

impl<C: TerminalControl> Drop for TerminalGuard<C> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn retain_first_error<E>(first_error: &mut Option<E>, result: Result<(), E>) {
    if let Err(error) = result
        && first_error.is_none()
    {
        *first_error = Some(error);
    }
}

struct CrosstermTerminalControl {
    output: Stdout,
}

impl CrosstermTerminalControl {
    fn new() -> Self {
        Self { output: stdout() }
    }
}

impl TerminalControl for CrosstermTerminalControl {
    fn enable_raw(&mut self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn enter_alt(&mut self) -> io::Result<()> {
        execute!(self.output, EnterAlternateScreen)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        execute!(self.output, Hide)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(self.output, Show)
    }

    fn leave_alt(&mut self) -> io::Result<()> {
        execute!(self.output, LeaveAlternateScreen)
    }

    fn disable_raw(&mut self) -> io::Result<()> {
        disable_raw_mode()
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

pub struct CrosstermScreen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    _guard: TerminalGuard<CrosstermTerminalControl>,
}

impl CrosstermScreen {
    pub fn new() -> Result<Self, TuiError> {
        let guard = TerminalGuard::enter(CrosstermTerminalControl::new())?;
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend).map_err(|_| TuiError::TerminalInitialization)?;
        terminal
            .clear()
            .map_err(|_| TuiError::TerminalInitialization)?;
        Ok(Self {
            terminal,
            _guard: guard,
        })
    }
}

impl Screen for CrosstermScreen {
    fn size(&self) -> Result<Rect, TuiError> {
        self.terminal
            .size()
            .map(|size| Rect::new(0, 0, size.width, size.height))
            .map_err(|_| TuiError::TerminalOutput)
    }

    fn draw(&mut self, model: &TuiModel, theme: &Theme) -> Result<(), TuiError> {
        self.terminal
            .draw(|frame| render::render(frame, model, theme))
            .map(|_| ())
            .map_err(|_| TuiError::TerminalOutput)
    }

    fn restore(&mut self) -> Result<(), TuiError> {
        self._guard.restore()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{Arc, Mutex},
    };

    use super::{TerminalControl, TerminalGuard, retain_first_error};
    use crate::ui::tui::error::TuiError;

    #[derive(Clone)]
    struct FakeTerminalControl {
        log: Arc<Mutex<Vec<&'static str>>>,
        failures: Arc<Vec<&'static str>>,
    }

    impl FakeTerminalControl {
        fn new(log: Arc<Mutex<Vec<&'static str>>>) -> Self {
            Self {
                log,
                failures: Arc::new(Vec::new()),
            }
        }

        fn failing_at(operation: &'static str) -> Self {
            Self::failing_at_all(&[operation])
        }

        fn failing_at_all(operations: &[&'static str]) -> Self {
            Self {
                log: Arc::new(Mutex::new(Vec::new())),
                failures: Arc::new(operations.to_vec()),
            }
        }

        fn log(&self) -> Vec<&'static str> {
            self.log.lock().unwrap().clone()
        }

        fn perform(&mut self, operation: &'static str) -> io::Result<()> {
            self.log.lock().unwrap().push(operation);
            if self.failures.contains(&operation) {
                Err(io::Error::other("injected terminal failure"))
            } else {
                Ok(())
            }
        }
    }

    impl TerminalControl for FakeTerminalControl {
        fn enable_raw(&mut self) -> io::Result<()> {
            self.perform("enable_raw")
        }

        fn enter_alt(&mut self) -> io::Result<()> {
            self.perform("enter_alt")
        }

        fn hide_cursor(&mut self) -> io::Result<()> {
            self.perform("hide_cursor")
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.perform("show_cursor")
        }

        fn leave_alt(&mut self) -> io::Result<()> {
            self.perform("leave_alt")
        }

        fn disable_raw(&mut self) -> io::Result<()> {
            self.perform("disable_raw")
        }

        fn flush(&mut self) -> io::Result<()> {
            self.perform("flush")
        }
    }

    #[test]
    fn generic_accumulator_retains_the_first_distinct_error() {
        #[derive(Debug, PartialEq, Eq)]
        enum SentinelError {
            First,
            Later,
        }

        let mut first_error = None;
        retain_first_error(&mut first_error, Ok(()));
        retain_first_error(&mut first_error, Err(SentinelError::First));
        retain_first_error(&mut first_error, Err(SentinelError::Later));

        assert_eq!(first_error, Some(SentinelError::First));
    }

    #[test]
    fn terminal_guard_restores_every_acquired_state_in_reverse_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        {
            let control = FakeTerminalControl::new(log.clone());
            let _guard = TerminalGuard::enter(control).unwrap();
        }
        assert_eq!(
            *log.lock().unwrap(),
            [
                "enable_raw",
                "enter_alt",
                "hide_cursor",
                "show_cursor",
                "leave_alt",
                "disable_raw",
                "flush",
            ]
        );
    }

    #[test]
    fn every_initialization_failure_restores_only_acquired_state() {
        let cases: [(&str, &[&str]); 3] = [
            ("enable_raw", &["enable_raw"]),
            (
                "enter_alt",
                &["enable_raw", "enter_alt", "disable_raw", "flush"],
            ),
            (
                "hide_cursor",
                &[
                    "enable_raw",
                    "enter_alt",
                    "hide_cursor",
                    "leave_alt",
                    "disable_raw",
                    "flush",
                ],
            ),
        ];

        for (operation, expected_log) in cases {
            let control = FakeTerminalControl::failing_at(operation);
            let result = TerminalGuard::enter(control.clone());
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("operation {operation} unexpectedly succeeded"),
            };

            assert!(
                matches!(error, TuiError::TerminalInitialization),
                "operation={operation}"
            );
            assert_eq!(control.log(), expected_log, "operation={operation}");
            assert!(!format!("{error:?}").contains("injected terminal failure"));
        }
    }

    #[test]
    fn every_restoration_failure_attempts_all_steps_and_returns_only_a_safe_error() {
        let cases = [
            vec!["show_cursor"],
            vec!["leave_alt"],
            vec!["disable_raw"],
            vec!["flush"],
            vec!["show_cursor", "leave_alt", "disable_raw", "flush"],
        ];

        for failures in cases {
            let control = FakeTerminalControl::failing_at_all(&failures);
            let observer = control.clone();
            let mut guard = TerminalGuard::enter(control).unwrap();
            let result = guard.restore();

            assert!(matches!(result, Err(TuiError::TerminalOutput)));
            assert_eq!(
                observer.log(),
                [
                    "enable_raw",
                    "enter_alt",
                    "hide_cursor",
                    "show_cursor",
                    "leave_alt",
                    "disable_raw",
                    "flush",
                ],
                "failures={failures:?}"
            );
            assert!(!format!("{result:?}").contains("injected terminal failure"));
        }
    }

    #[test]
    fn restoration_is_idempotent_after_success_and_failure() {
        for failures in [Vec::new(), vec!["show_cursor"]] {
            let control = FakeTerminalControl::failing_at_all(&failures);
            let observer = control.clone();
            let mut guard = TerminalGuard::enter(control).unwrap();

            let _ = guard.restore();
            let _ = guard.restore();
            drop(guard);

            assert_eq!(
                observer.log(),
                [
                    "enable_raw",
                    "enter_alt",
                    "hide_cursor",
                    "show_cursor",
                    "leave_alt",
                    "disable_raw",
                    "flush",
                ]
            );
        }
    }
}
