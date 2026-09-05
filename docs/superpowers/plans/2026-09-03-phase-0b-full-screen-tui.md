# Phase 0B Full-Screen TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Adaptive Cockpit full-screen terminal interface the default interactive experience while preserving the Phase 0 command host, application boundaries, persistence guarantees, shutdown behavior, and platform safety.

**Architecture:** Add a Ratatui/Crossterm UI adapter above the existing `ApplicationService` and `ApplicationRuntime`. A pure model/controller/rendering core owns presentation state; a thin terminal/event shell owns Crossterm resources; all user commands still flow through the existing parser and runtime. Interactive terminals launch the cockpit, `--command-mode` launches the existing line host, and redirected stdin or stdout automatically selects command mode.

**Tech Stack:** Rust 1.98.0, Ratatui 0.29.0, Crossterm 0.28.1, crossbeam-channel, rusqlite, the existing Phase 0 domain/runtime layers, and Proptest 1.7.0 as a development-only dependency.

**Spec:** `docs/superpowers/specs/2026-09-03-phase-0b-full-screen-tui-design.md`

## Global Constraints

- Work only in `/private/tmp/ai-stock-forum-phase-0b-full-screen-tui` on branch `codex/phase-0b-full-screen-tui`.
- Preserve `main` and the user's dirty checkout at `/Users/nguyen-mini/Documents/dev/ai-stock-forum`.
- Keep every existing Phase 0 command, event, audit record, database invariant, panic boundary, process guard, and clean-shutdown behavior intact.
- Do not read or write SQLite directly from `src/ui/tui`; the TUI may use only typed application snapshots, commands, and outcomes.
- Keep one command in flight at a time. Never block the render/input loop waiting for a command response.
- Never render command transcripts. Map outcomes into native Overview, Setup, Audit, Help, and message-region state.
- Do not enable mouse capture.
- Support terminals down to `60x18`; below either minimum dimension, render only the size warning and retain quit/interrupt handling.
- Honor `NO_COLOR` with a monochrome theme that retains focus, status, and severity distinctions through text and modifiers.
- Restore cursor visibility, alternate-screen state, raw mode, and output flushing on success, typed error, interrupt, and panic.
- Keep tests deterministic: no real terminal, sleeps, network, wall-clock dependence, or user home-directory writes in automated tests.
- Follow red-green-refactor for every production change and commit only after the task's focused test passes.
- Do not broaden Phase 0B into setup editing, networking, stock data, agents, rooms, memory, or MCP behavior.

## Execution Preflight

- [ ] Confirm the worktree path and branch before changing files:

```bash
pwd
git branch --show-current
git status --short
```

Expected: the path is `/private/tmp/ai-stock-forum-phase-0b-full-screen-tui`, the branch is `codex/phase-0b-full-screen-tui`, and only intentional plan/spec commits are present.

- [ ] Establish the Phase 0 baseline before the first production edit:

```bash
cargo test --workspace --all-targets --locked
```

Expected: all existing tests pass. Stop and investigate any baseline failure rather than attributing it to Phase 0B.

## Planned File Map

| File | Responsibility |
|---|---|
| `Cargo.toml`, `Cargo.lock` | Pin Ratatui, Crossterm, and test-only Proptest dependencies. |
| `src/cli.rs` | Pure launch-mode selection and process terminal detection. |
| `src/lib.rs` | Export the CLI module. |
| `src/main.rs` | Bootstrap once, then route into command mode or full-screen mode. |
| `src/app/service.rs`, `src/app/mod.rs` | Read-only typed `PresentationSnapshot` boundary. |
| `src/runtime/mod.rs` | Non-blocking polling of a submitted command's outcome. |
| `src/ui/mod.rs` | Register shared interrupt and TUI modules. |
| `src/ui/interrupt.rs` | One process-wide Ctrl+C installation shared by both UI adapters. |
| `src/ui/command/runner.rs` | Consume the shared interrupt source without changing command-host behavior. |
| `src/ui/command/renderer.rs` | Safely render launch/TUI failures after terminal restoration. |
| `src/ui/tui/mod.rs` | Public TUI entry point and re-exports. |
| `src/ui/tui/error.rs` | Typed, redacted TUI errors. |
| `src/ui/tui/model.rs` | Presentation state, command editor, history, focus, and selection. |
| `src/ui/tui/layout.rs` | Pure responsive breakpoint and rectangle calculation. |
| `src/ui/tui/theme.rs` | Color and `NO_COLOR` palettes. |
| `src/ui/tui/render.rs` | Top-level frame composition and too-small warning. |
| `src/ui/tui/views/mod.rs` | Native view dispatch. |
| `src/ui/tui/views/overview.rs` | Runtime/session overview. |
| `src/ui/tui/views/setup.rs` | Read-only setup state. |
| `src/ui/tui/views/audit.rs` | Bounded audit table and selected-entry inspector. |
| `src/ui/tui/views/help.rs` | Keys and slash-command reference. |
| `src/ui/tui/event.rs` | Crossterm-to-typed-event translation and interrupt polling. |
| `src/ui/tui/terminal.rs` | Terminal lifecycle guard and drawing surface. |
| `src/ui/tui/controller.rs` | Pure key/event-to-effect transition logic and outcome mapping. |
| `src/ui/tui/host.rs` | Responsive event loop, runtime submission, and orderly shutdown. |
| `tests/tui_cli_contract.rs` | Launch-mode contract. |
| `tests/tui_application_contract.rs` | Passive snapshot and runtime polling contract. |
| `tests/tui_host_contract.rs` | End-to-end UI-adapter behavior with fake screen/events. |
| `tests/tui_hardening_contract.rs` | Layout/editor properties, redaction, and restoration safety. |
| `README.md` | Default launch, command-mode fallback, controls, and limitations. |
| `docs/phase-0b-testing.md` | Manual cross-platform acceptance script. |

---

## Task 1: Pin UI Dependencies and Select Launch Mode

**Files:** `Cargo.toml`, `Cargo.lock`, `src/cli.rs`, `src/lib.rs`, `tests/tui_cli_contract.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Tui,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    #[error("invalid command-line arguments")]
    InvalidArguments,
}

pub fn select_launch_mode(
    args: &[std::ffi::OsString],
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
) -> Result<LaunchMode, CliError>;

pub fn detect_launch_mode() -> Result<LaunchMode, CliError>;
```

- [ ] Add failing launch-mode tests:

```rust
use std::ffi::OsString;
use ai_stock_forum::cli::{CliError, LaunchMode, select_launch_mode};

#[test]
fn interactive_input_and_output_select_the_tui() {
    assert_eq!(select_launch_mode(&[], true, true), Ok(LaunchMode::Tui));
}

#[test]
fn command_mode_flag_always_selects_the_line_host() {
    let args = [OsString::from("--command-mode")];
    assert_eq!(select_launch_mode(&args, true, true), Ok(LaunchMode::Command));
}

#[test]
fn either_redirected_stream_selects_the_line_host() {
    assert_eq!(select_launch_mode(&[], false, true), Ok(LaunchMode::Command));
    assert_eq!(select_launch_mode(&[], true, false), Ok(LaunchMode::Command));
    assert_eq!(select_launch_mode(&[], false, false), Ok(LaunchMode::Command));
}

#[test]
fn unknown_or_repeated_arguments_are_rejected() {
    assert_eq!(
        select_launch_mode(&[OsString::from("--unknown")], true, true),
        Err(CliError::InvalidArguments)
    );
    assert_eq!(
        select_launch_mode(
            &[OsString::from("--command-mode"), OsString::from("--command-mode")],
            true,
            true,
        ),
        Err(CliError::InvalidArguments)
    );
}
```

- [ ] Run `cargo test --test tui_cli_contract --locked`; expect failure because `cli` does not exist.
- [ ] Add these exact production dependencies:

```toml
ratatui = { version = "0.29.0", default-features = false, features = ["crossterm"] }
crossterm = "0.28.1"
```

- [ ] Implement `select_launch_mode` as a pure function. Accept exactly zero arguments or one `--command-mode`; reject every other shape. Without the flag, require both streams to be terminals before choosing `Tui`.
- [ ] Implement `detect_launch_mode` with `std::env::args_os().skip(1)` and `std::io::IsTerminal` for stdin and stdout.
- [ ] Export `pub mod cli;` from `src/lib.rs`.
- [ ] Run `cargo test --test tui_cli_contract --locked`; expect all tests to pass.
- [ ] Commit:

```bash
git add Cargo.toml Cargo.lock src/cli.rs src/lib.rs tests/tui_cli_contract.rs
git commit -m "feat: select interactive tui launch mode"
```

---

## Task 2: Share One Interrupt Source Across UI Adapters

**Files:** `src/ui/interrupt.rs`, `src/ui/mod.rs`, `src/ui/command/runner.rs`

**Interface:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum InterruptError {
    #[error("interrupt handler unavailable")]
    Install,
}

pub(crate) fn receiver() -> Result<crossbeam_channel::Receiver<()>, InterruptError>;
```

- [ ] Add this failing unit test to the new module:

```rust
#[cfg(test)]
mod tests {
    use crossbeam_channel::{TryRecvError, bounded};
    use super::drain_pending;

    #[test]
    fn stale_interrupts_are_drained_before_a_host_starts() {
        let (sender, receiver) = bounded(2);
        sender.try_send(()).unwrap();
        sender.try_send(()).unwrap();
        drain_pending(&receiver);
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    }
}
```

- [ ] Run `cargo test ui::interrupt::tests::stale_interrupts_are_drained_before_a_host_starts --lib --locked`; expect a missing-function failure.
- [ ] Move the existing `OnceLock`, bounded channel, Ctrl+C handler installation, and stale-signal draining from `ui::command::runner` into `ui::interrupt`.
- [ ] Keep channel capacity one in production so repeated signals coalesce safely.
- [ ] Make command mode call `ui::interrupt::receiver()` and map `InterruptError::Install` to its existing `UiError::InterruptHandler` variant.
- [ ] Remove only the duplicated private interrupt implementation from `runner.rs`; do not alter fallback cancellation or shutdown logic.
- [ ] Run:

```bash
cargo test ui::interrupt::tests::stale_interrupts_are_drained_before_a_host_starts --lib --locked
cargo test ui::command --lib --locked
```

Expected: the new unit test and all existing command UI tests pass.

- [ ] Commit:

```bash
git add src/ui/interrupt.rs src/ui/mod.rs src/ui/command/runner.rs
git commit -m "refactor: share ui interrupt source"
```

---

## Task 3: Add a Passive Presentation Snapshot

**Files:** `src/app/service.rs`, `src/app/mod.rs`, `tests/tui_application_contract.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationSnapshot {
    pub installation_id: InstallationId,
    pub session_id: SessionId,
    pub setup_status: SetupStatus,
    pub recent_audit: Vec<AuditEntry>,
}

impl ApplicationService {
    pub fn presentation_snapshot(
        &self,
        limit: AuditLimit,
    ) -> Result<PresentationSnapshot, AppError>;
}
```

- [ ] Add a `SnapshotHarness` in `tests/tui_application_contract.rs` that owns a `TempDir`, isolated `AppPaths`, deterministic clock/ID providers, and bootstrapped `ApplicationService`. Give it `new()` and `persisted_events()` methods using the existing test-support constructors and `EventRepository::load_all`.
- [ ] Add the failing contract test:

```rust
#[test]
fn presentation_snapshot_is_typed_bounded_and_does_not_append_events() {
    let harness = SnapshotHarness::new();
    let before = harness.persisted_events();
    let limit = AuditLimit::new(3).unwrap();

    let snapshot = harness.service.presentation_snapshot(limit).unwrap();
    let after = harness.persisted_events();

    assert_eq!(snapshot.installation_id, harness.service.installation_id());
    assert_eq!(snapshot.session_id, harness.service.session_id());
    assert_eq!(snapshot.setup_status, SetupStatus::NotStarted);
    assert!(snapshot.recent_audit.len() <= usize::from(limit.get()));
    assert_eq!(after, before);
}
```

- [ ] Run `cargo test --test tui_application_contract presentation_snapshot_is_typed_bounded_and_does_not_append_events --locked`; expect the snapshot API to be missing.
- [ ] Implement the method from typed state and a bounded repository read:

```rust
pub fn presentation_snapshot(
    &self,
    limit: AuditLimit,
) -> Result<PresentationSnapshot, AppError> {
    let projection = self.state.projection();
    let events = EventRepository::tail_through(
        self.executor.database.connection(),
        limit,
        projection.last_sequence,
    )?;

    Ok(PresentationSnapshot {
        installation_id: self.state.installation_id(),
        session_id: self.state.session_id(),
        setup_status: projection.setup_status.clone(),
        recent_audit: events.iter().map(AuditEntry::from_event).collect(),
    })
}
```

- [ ] Keep the snapshot free of paths, database handles, raw events, process-guard internals, and secrets.
- [ ] Re-export `PresentationSnapshot` from `src/app/mod.rs`.
- [ ] Run:

```bash
cargo test --test tui_application_contract presentation_snapshot_is_typed_bounded_and_does_not_append_events --locked
cargo test --test application_contract --locked
```

Expected: both contracts pass and snapshot reads add no event.

- [ ] Commit:

```bash
git add src/app/service.rs src/app/mod.rs tests/tui_application_contract.rs
git commit -m "feat: expose passive presentation snapshot"
```

---

## Task 4: Poll Pending Runtime Outcomes Without Blocking

**Files:** `src/runtime/mod.rs`, `tests/tui_application_contract.rs`

**Interface:**

```rust
impl PendingOutcome {
    pub fn try_recv(&self) -> Result<Option<CommandOutcome>, RuntimeError>;
}
```

- [ ] Add deterministic tests backed by local bounded channels:

```rust
#[test]
fn pending_outcome_poll_returns_none_before_the_worker_replies() {
    let (pending, worker_gate) = gated_pending_outcome();
    assert_eq!(pending.try_recv().unwrap(), None);
    worker_gate.release();
}

#[test]
fn pending_outcome_poll_returns_the_typed_outcome_once() {
    let (pending, expected) = completed_pending_outcome();
    assert_eq!(pending.try_recv().unwrap(), Some(expected));
}

#[test]
fn polling_and_blocking_receive_share_the_disconnection_error() {
    let polling = disconnected_pending_outcome().try_recv().unwrap_err();
    let blocking = disconnected_pending_outcome().recv().unwrap_err();
    assert_eq!(polling.code(), blocking.code());
}
```

- [ ] Run `cargo test --test tui_application_contract pending_outcome_poll --locked`; expect `try_recv` to be missing.
- [ ] Implement `try_recv` over the pending response channel. Map `Empty` to `Ok(None)`, a delivered application result to `Some` or its typed error, and `Disconnected` through the exact helper already used by blocking `recv`.
- [ ] Refactor `recv(self)` and `try_recv(&self)` through one private disconnection-error constructor so their safe error codes cannot diverge.
- [ ] Do not expose `crossbeam_channel` errors in the public API or rendered text.
- [ ] Run:

```bash
cargo test --test tui_application_contract pending_outcome_poll --locked
cargo test --test runtime_contract --locked
```

Expected: focused polling and existing runtime contracts pass.

- [ ] Commit:

```bash
git add src/runtime/mod.rs tests/tui_application_contract.rs
git commit -m "feat: poll runtime command outcomes"
```

---

## Task 5: Build the Pure TUI Model and Unicode-Safe Command Editor

**Files:** `src/ui/tui/mod.rs`, `src/ui/tui/model.rs`, `src/ui/mod.rs`

**Core types:**

```rust
pub const COMMAND_HISTORY_CAPACITY: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View { Overview, Setup, Audit, Help }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus { Navigation, Workspace, Inspector, Command }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode { Wide, Medium, Narrow, TooSmall }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity { Info, Warning, Error }

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
    pub previous_session_interrupted: bool,
}
```

- [ ] Add failing model tests:

```rust
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
```

- [ ] Run `cargo test ui::tui::model::tests --lib --locked`; expect missing module/type failures.
- [ ] Implement editor movement with `String::is_char_boundary`, `char_indices`, and byte indices. Never index UTF-8 by display column or unchecked arithmetic.
- [ ] Ignore blank history entries, collapse consecutive duplicates, evict oldest entries over 100, and keep history recall read-only.
- [ ] Clear history navigation whenever recalled text is edited.
- [ ] Implement `TuiModel::new`, bounded audit replacement, view switching, saturating scroll, focus changes, and typed message setters as pure methods.
- [ ] Run `cargo test ui::tui::model::tests --lib --locked`; expect all model tests to pass.
- [ ] Commit:

```bash
git add src/ui/mod.rs src/ui/tui/mod.rs src/ui/tui/model.rs
git commit -m "feat: add tui presentation model"
```

---

## Task 6: Calculate the Adaptive Cockpit Layout

**Files:** `src/ui/tui/layout.rs`, `src/ui/tui/mod.rs`

**Breakpoints and output:**

```rust
pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 18;
pub const MEDIUM_WIDTH: u16 = 80;
pub const WIDE_WIDTH: u16 = 120;
pub const MEDIUM_HEIGHT: u16 = 24;
pub const WIDE_HEIGHT: u16 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CockpitLayout {
    pub mode: LayoutMode,
    pub viewport: Rect,
    pub header: Rect,
    pub navigation: Option<Rect>,
    pub workspace: Rect,
    pub inspector: Option<Rect>,
    pub message: Rect,
    pub command: Rect,
}

pub fn layout_mode(area: Rect) -> LayoutMode;
pub fn calculate(area: Rect, inspector_open: bool) -> CockpitLayout;
```

- [ ] Add the failing breakpoint table test:

```rust
#[test]
fn exact_breakpoints_choose_the_documented_modes() {
    let cases = [
        (Rect::new(0, 0, 120, 30), LayoutMode::Wide),
        (Rect::new(0, 0, 119, 30), LayoutMode::Medium),
        (Rect::new(0, 0, 80, 24), LayoutMode::Medium),
        (Rect::new(0, 0, 79, 24), LayoutMode::Narrow),
        (Rect::new(0, 0, 60, 18), LayoutMode::Narrow),
        (Rect::new(0, 0, 59, 18), LayoutMode::TooSmall),
        (Rect::new(0, 0, 120, 17), LayoutMode::TooSmall),
    ];
    for (area, expected) in cases {
        assert_eq!(layout_mode(area), expected, "area={area:?}");
    }
}
```

- [ ] Add failing structure tests:

```rust
#[test]
fn wide_has_three_columns_and_medium_uses_an_overlay_inspector() {
    let wide = calculate(Rect::new(0, 0, 140, 40), true);
    assert!(wide.navigation.is_some());
    assert!(wide.inspector.is_some());
    assert!(wide.inspector.unwrap().x > wide.workspace.x);

    let medium = calculate(Rect::new(0, 0, 100, 30), true);
    assert!(medium.navigation.is_some());
    assert!(medium.inspector.is_some());
    assert!(medium.inspector.unwrap().width < medium.viewport.width);
}

#[test]
fn narrow_uses_tabs_and_too_small_uses_the_whole_viewport() {
    let narrow = calculate(Rect::new(0, 0, 70, 20), true);
    assert_eq!(narrow.navigation, None);
    assert!(narrow.inspector.is_some());

    let tiny = calculate(Rect::new(4, 7, 40, 10), true);
    assert_eq!(tiny.mode, LayoutMode::TooSmall);
    assert_eq!(tiny.workspace, tiny.viewport);
    assert_eq!(tiny.navigation, None);
    assert_eq!(tiny.inspector, None);
}
```

- [ ] Run `cargo test ui::tui::layout::tests --lib --locked`; expect missing layout failures.
- [ ] Implement precedence: TooSmall first; Wide only when width is at least 120 and height at least 30; Medium only when width is at least 80 and height at least 24; otherwise Narrow.
- [ ] Reserve three rows for header, one for messages, and three for command input before splitting content.
- [ ] Wide uses a 22-column navigation rail, then approximately 2:1 workspace/inspector. Medium uses a 20-column rail and centered 70%-by-70% inspector overlay. Narrow gives workspace the full content width and uses the same centered overlay.
- [ ] Build rectangles with Ratatui constraints and checked/saturating geometry; never subtract unchecked `u16` values.
- [ ] Run `cargo test ui::tui::layout::tests --lib --locked`; expect all layout tests to pass.
- [ ] Commit:

```bash
git add src/ui/tui/layout.rs src/ui/tui/mod.rs
git commit -m "feat: calculate adaptive cockpit layout"
```

---

## Task 7: Render Native Views and the Monochrome Theme

**Files:** `src/ui/tui/theme.rs`, `src/ui/tui/render.rs`, `src/ui/tui/views/mod.rs`, `src/ui/tui/views/overview.rs`, `src/ui/tui/views/setup.rs`, `src/ui/tui/views/audit.rs`, `src/ui/tui/views/help.rs`, `src/ui/tui/mod.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub accent: Style,
    pub focus: Style,
    pub muted: Style,
    pub success: Style,
    pub warning: Style,
    pub error: Style,
}

impl Theme {
    pub fn from_no_color(no_color: bool) -> Self;
    pub fn styles(self) -> [Style; 6];
}

pub fn render(frame: &mut ratatui::Frame<'_>, model: &TuiModel, theme: &Theme);
```

- [ ] Add a `render_text` test helper using `TestBackend`, `Terminal::draw`, and flattened buffer symbols.
- [ ] Add failing renderer tests:

```rust
#[test]
fn wide_overview_renders_identity_health_navigation_and_command_bar() {
    let text = render_text(model(View::Overview), 140, 40, false);
    assert!(text.contains("AI STOCK FORUM"));
    assert!(text.contains("Overview"));
    assert!(text.contains("Installation"));
    assert!(text.contains("Session"));
    assert!(text.contains("Runtime"));
    assert!(text.contains("Type /help"));
}

#[test]
fn each_command_view_is_native_and_no_transcript_heading_exists() {
    for view in [View::Overview, View::Setup, View::Audit, View::Help] {
        let text = render_text(model(view), 100, 30, false);
        assert!(!text.contains("Transcript"));
        assert!(!text.contains("Command output"));
    }
}

#[test]
fn too_small_screen_contains_only_size_guidance() {
    let text = render_text(model(View::Audit), 59, 17, false);
    assert!(text.contains("Terminal too small"));
    assert!(text.contains("60 x 18"));
    assert!(!text.contains("Installation"));
}

#[test]
fn no_color_theme_uses_modifiers_without_terminal_colors() {
    let theme = Theme::from_no_color(true);
    for style in theme.styles() {
        assert_eq!(style.fg, None);
        assert_eq!(style.bg, None);
    }
    assert!(theme.focus.add_modifier.contains(Modifier::REVERSED));
}
```

- [ ] Run `cargo test ui::tui::render::tests --lib --locked`; expect missing rendering modules.
- [ ] Render one persistent header with product name, active view, layout mode, and interrupted-session warning when present.
- [ ] Render navigation as a left rail in Wide/Medium and numbered tabs in the Narrow header.
- [ ] Render Overview with installation ID, session ID, runtime readiness, setup state, and recent-activity summary.
- [ ] Render Setup as read-only state with explicit guidance that editing is deferred.
- [ ] Render Audit as a bounded sequence/kind/actor/summary table. Highlight the selected row and put full typed entry details in the inspector.
- [ ] Render Help with every approved key and every slash command recognized by the existing parser.
- [ ] Render typed severity in the message row and the command editor with a visible insertion cursor when command focus is active.
- [ ] Compute cursor display position with `Line::from(editor.prefix()).width()` so wide Unicode characters do not corrupt placement.
- [ ] In TooSmall mode, clear the frame and render only centered minimum-size guidance plus `q`/Ctrl+C exit guidance.
- [ ] Run `cargo test ui::tui::render::tests --lib --locked`; expect all renderer tests to pass.
- [ ] Commit:

```bash
git add src/ui/tui/theme.rs src/ui/tui/render.rs src/ui/tui/views src/ui/tui/mod.rs
git commit -m "feat: render adaptive cockpit views"
```

---

## Task 8: Guard Terminal State and Translate Crossterm Events

**Files:** `src/ui/tui/error.rs`, `src/ui/tui/terminal.rs`, `src/ui/tui/event.rs`, `src/ui/tui/mod.rs`

**Interfaces:**

```rust
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    #[error("terminal initialization failed")]
    TerminalInitialization,
    #[error("terminal input failed")]
    TerminalInput,
    #[error("terminal output failed")]
    TerminalOutput,
    #[error("interrupt handler unavailable")]
    InterruptHandler,
    #[error("application runtime failed")]
    Runtime(#[from] RuntimeError),
    #[error("terminal interface stopped unexpectedly")]
    Panicked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiEvent {
    Key(crossterm::event::KeyEvent),
    Resize(u16, u16),
    Interrupt,
}

pub trait EventSource {
    fn next_event(&mut self, timeout: Duration) -> Result<Option<TuiEvent>, TuiError>;
}

pub trait Screen {
    fn size(&self) -> Result<Rect, TuiError>;
    fn draw(&mut self, model: &TuiModel, theme: &Theme) -> Result<(), TuiError>;
}
```

- [ ] Add failing lifecycle tests around an injected `TerminalControl`:

```rust
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
            "enable_raw", "enter_alt", "hide_cursor", "show_cursor",
            "leave_alt", "disable_raw", "flush",
        ]
    );
}

#[test]
fn partial_initialization_failure_restores_acquired_state() {
    let control = FakeTerminalControl::failing_at("hide_cursor");
    let result = TerminalGuard::enter(control.clone());
    assert!(matches!(result, Err(TuiError::TerminalInitialization)));
    assert_eq!(
        control.log(),
        ["enable_raw", "enter_alt", "hide_cursor", "leave_alt", "disable_raw", "flush"]
    );
}
```

- [ ] Add failing translation tests:

```rust
#[test]
fn key_press_resize_and_interrupt_become_typed_events() {
    assert_eq!(translate(Event::Resize(90, 25)), Some(TuiEvent::Resize(90, 25)));
    assert!(matches!(
        translate(Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))),
        Some(TuiEvent::Key(_))
    ));
    assert_eq!(translate(Event::FocusGained), None);
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
```

- [ ] Run `cargo test ui::tui::terminal::tests ui::tui::event::tests --lib --locked`; expect missing-module failures.
- [ ] Implement `TerminalGuard` with acquisition flags. Restoration must attempt every applicable operation after a failure, retain only the first safe typed error, and be idempotent.
- [ ] Set up production in this order: enable raw mode, enter alternate screen, hide cursor, construct `Terminal<CrosstermBackend<Stdout>>`, clear once.
- [ ] Restore in this order: show cursor, leave alternate screen, disable raw mode, flush stdout.
- [ ] Implement `CrosstermEventSource` with the shared interrupt receiver. Check interrupts before polling, cap polling at the supplied duration, and translate only key press/repeat and resize events.
- [ ] Do not emit `EnableMouseCapture`, `DisableMouseCapture`, or any mouse event branch.
- [ ] Implement `CrosstermScreen` behind `Screen`; drawing delegates only to the pure renderer.
- [ ] Run `cargo test ui::tui::terminal::tests ui::tui::event::tests --lib --locked`; expect focused tests to pass.
- [ ] Commit:

```bash
git add src/ui/tui/error.rs src/ui/tui/terminal.rs src/ui/tui/event.rs src/ui/tui/mod.rs
git commit -m "feat: guard tui terminal lifecycle"
```

---

## Task 9: Map Keys, Commands, and Outcomes Through a Pure Controller

**Files:** `src/ui/tui/controller.rs`, `src/ui/tui/mod.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControllerEffect {
    None,
    Redraw,
    Submit(ApplicationCommand),
    RequestShutdown(ShutdownReason),
}

pub fn handle_event(model: &mut TuiModel, event: TuiEvent) -> ControllerEffect;
pub fn apply_outcome(model: &mut TuiModel, outcome: CommandOutcome) -> ControllerEffect;
```

- [ ] Add the failing global-key test:

```rust
#[test]
fn global_keys_switch_views_focus_inspector_command_and_shutdown() {
    let mut model = model();
    assert_redraw_and_view(&mut model, key('2'), View::Setup);
    assert_redraw_and_view(&mut model, key('3'), View::Audit);
    assert_redraw_and_view(&mut model, key('4'), View::Help);
    assert_eq!(handle_event(&mut model, key('i')), ControllerEffect::Redraw);
    assert!(model.inspector_open);
    assert_eq!(handle_event(&mut model, key('/')), ControllerEffect::Redraw);
    assert_eq!(model.focus, Focus::Command);
    assert!(matches!(
        handle_event(&mut model, key('q')),
        ControllerEffect::RequestShutdown(ShutdownReason::UserRequested)
    ));
}
```

- [ ] Add failing command tests:

```rust
#[test]
fn enter_uses_the_authoritative_parser_and_marks_one_command_in_flight() {
    let mut model = command_model("/status");
    let effect = handle_event(&mut model, enter());
    assert_eq!(effect, ControllerEffect::Submit(ApplicationCommand::ShowStatus));
    assert!(model.command_in_flight);
    assert_eq!(model.command.text(), "");
}

#[test]
fn enter_during_an_in_flight_command_does_not_submit_again() {
    let mut model = command_model("/audit 5");
    model.command_in_flight = true;
    assert_eq!(handle_event(&mut model, enter()), ControllerEffect::None);
    assert_eq!(model.message.as_ref().unwrap().severity, Severity::Warning);
}
```

- [ ] Add an outcome table test that applies one outcome for each `CommandView`: Help selects Help, Status selects Overview and refreshes IDs, SetupStatus selects Setup, AuditTail selects Audit and replaces bounded entries, InputRejected sets an Error message, and Shutdown returns `RequestShutdown(UserRequested)`.
- [ ] Run `cargo test ui::tui::controller::tests --lib --locked`; expect missing controller failures.
- [ ] Implement the approved hybrid map: `1`-`4`, Tab/BackTab, arrows, PageUp/PageDown, Home/End, `/`, Enter, Up/Down history in command focus, Esc, `i`, `?`, `q`, and Ctrl+C.
- [ ] In TooSmall mode, ignore every action except `q` and Ctrl+C shutdown.
- [ ] Route entered bytes through `ui::command::parse_line`; do not copy parser rules into the controller.
- [ ] Treat `ParsedLine::Ignored` as a local no-op with command focus retained.
- [ ] Map committed events into bounded `AuditEntry` additions so Audit reflects commands without querying persistence.
- [ ] Clear `command_in_flight` before applying the returned view. Do not retain raw `CommandOutcome` or `EventEnvelope` values in `TuiModel`.
- [ ] Render rejected input and runtime-safe failures as generic messages; never include raw payload bytes.
- [ ] Run `cargo test ui::tui::controller::tests --lib --locked`; expect all controller tests to pass.
- [ ] Commit:

```bash
git add src/ui/tui/controller.rs src/ui/tui/mod.rs
git commit -m "feat: control tui navigation and commands"
```

---

## Task 10: Run the Responsive TUI Without Blocking

**Files:** `src/ui/tui/host.rs`, `src/ui/tui/mod.rs`, `tests/tui_host_contract.rs`

**Entry point:**

```rust
pub fn run_tui(
    runtime: ApplicationRuntime,
    snapshot: PresentationSnapshot,
    previous_session_interrupted: bool,
) -> Result<(), TuiError>;
```

**Loop state:**

```rust
struct TuiRunner {
    runtime: Option<ApplicationRuntime>,
    client: RuntimeClient,
    model: TuiModel,
    pending: Option<PendingOutcome>,
    queued_shutdown: Option<ShutdownReason>,
}
```

- [ ] Add fake-screen/fake-event contract tests:

```rust
#[test]
fn host_draws_processes_resize_and_redraws_without_waiting_for_input() {
    let events = FakeEvents::from([
        TuiEvent::Resize(70, 20),
        key_event('2'),
        key_event('q'),
    ]);
    let screen = RecordingScreen::new(Rect::new(0, 0, 140, 40));
    let result = run_with_adapters(runtime(), snapshot(), false, screen.clone(), events);
    assert!(result.is_ok());
    assert!(screen.frames().iter().any(|frame| frame.mode == LayoutMode::Narrow));
    assert!(screen.frames().iter().any(|frame| frame.active_view == View::Setup));
}

#[test]
fn second_submission_is_prevented_until_the_pending_outcome_arrives() {
    let harness = blocked_runtime_harness();
    let events = FakeEvents::from(command_sequence(["/status", "/help"]));
    run_until_idle(&harness, events);
    assert_eq!(harness.submitted_commands(), [ApplicationCommand::ShowStatus]);
}

#[test]
fn shutdown_requested_while_busy_runs_after_the_current_outcome() {
    let harness = blocked_runtime_harness();
    let events = FakeEvents::from(command_then_quit("/status"));
    run_until_complete(&harness, events);
    assert_eq!(
        harness.submitted_commands(),
        [ApplicationCommand::ShowStatus, ApplicationCommand::RequestShutdown]
    );
}
```

- [ ] Run `cargo test --test tui_host_contract --locked`; expect the host API to be missing.
- [ ] Implement an initial draw followed by a loop with a 50 ms maximum event-poll interval.
- [ ] On each iteration: poll the pending response once, apply a ready outcome, submit a queued shutdown when the slot becomes free, update size, process at most one input event, and redraw only when dirty.
- [ ] Keep exactly one `PendingOutcome`. If quit is requested while a normal command is pending, queue exactly one `RequestShutdown` command and stop accepting normal submissions.
- [ ] On Ctrl+C, use the runtime external-signal shutdown path immediately. On `q`, submit the auditable `RequestShutdown` command before finishing.
- [ ] On `ShutdownDisposition::Requested`, finish and join with `ShutdownReason::UserRequested`.
- [ ] On event, draw, or runtime failure, finish and join with `ShutdownReason::ApplicationError` before returning the typed safe error.
- [ ] Wrap only the event-loop body in `catch_unwind(AssertUnwindSafe(...))`. Convert panic payloads to `TuiError::Panicked` without formatting or logging them.
- [ ] Scope `TerminalGuard` outside the panic boundary and ensure it drops before callers render safe errors to normal stderr.
- [ ] Run `cargo test --test tui_host_contract --locked`; expect all host contracts to pass.
- [ ] Commit:

```bash
git add src/ui/tui/host.rs src/ui/tui/mod.rs tests/tui_host_contract.rs
git commit -m "feat: run nonblocking tui host"
```

---

## Task 11: Wire Main, Preserve Command Mode, and Render Safe Errors

**Files:** `src/main.rs`, `src/ui/command/renderer.rs`, `tests/tui_cli_contract.rs`, `tests/phase0_acceptance.rs`

**Routing shape:**

```rust
fn run() -> Result<(), MainError> {
    let mode = detect_launch_mode().map_err(MainError::Cli)?;
    let paths = AppPaths::discover().map_err(MainError::Startup)?;
    let mut service = ApplicationService::bootstrap(
        paths,
        Arc::new(SystemClock),
        Arc::new(UuidGenerator),
    )?;
    let previous_session_interrupted = service.previous_session_interrupted();

    match mode {
        LaunchMode::Command => run_command_mode(service, previous_session_interrupted),
        LaunchMode::Tui => run_full_screen(service, previous_session_interrupted),
    }
}
```

- [ ] Add failing subprocess assertions:

```rust
#[test]
fn explicit_command_mode_accepts_the_existing_protocol() {
    let output = binary()
        .arg("--command-mode")
        .write_stdin("/status\n/quit\n")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("installation"));
}

#[test]
fn redirected_stdio_automatically_preserves_command_mode() {
    let output = binary()
        .write_stdin("/help\n/quit\n")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("/status"));
}

#[test]
fn invalid_arguments_emit_one_safe_line_and_fail() {
    let output = binary().arg("--unknown").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.lines().count(), 1);
    assert!(!stderr.contains("--unknown"));
}
```

- [ ] Run `cargo test --test tui_cli_contract --locked`; expect launch-routing failures.
- [ ] Add `MainError::Cli(CliError)` and `MainError::Tui(TuiError)` without exposing nested debug text.
- [ ] Keep `StdioResources::initialize` and `run_stdio` exclusively inside `run_command_mode`.
- [ ] In `run_full_screen`, take `PresentationSnapshot` with the default audit limit before moving the service into `ApplicationRuntime`, then call `run_tui`.
- [ ] If snapshot creation fails before runtime spawn, call `service.finish(ShutdownReason::ApplicationError)` and return the typed safe failure.
- [ ] Extend `TextRenderer` with fixed safe lines for CLI and TUI failure categories. Never pass argument values, OS errors, database paths, panic payloads, or channel debug output.
- [ ] Preserve the top-level exit mapping: success for clean shutdown and failure for startup, runtime, CLI, or UI errors.
- [ ] Run:

```bash
cargo test --test tui_cli_contract --locked
cargo test --test phase0_acceptance --locked
```

Expected: explicit and automatic command mode pass, and existing Phase 0 acceptance remains unchanged.

- [ ] Commit:

```bash
git add src/main.rs src/ui/command/renderer.rs tests/tui_cli_contract.rs tests/phase0_acceptance.rs
git commit -m "feat: launch adaptive cockpit on terminals"
```

---

## Task 12: Add Property, Redaction, and Restoration Hardening

**Files:** `Cargo.toml`, `Cargo.lock`, `tests/tui_hardening_contract.rs`, `src/ui/tui/model.rs`, `src/ui/tui/layout.rs`, `src/ui/tui/terminal.rs`, `src/ui/tui/controller.rs`

- [ ] Add the exact development dependency:

```toml
proptest = "1.7.0"
```

- [ ] Add a failing layout property over arbitrary `u16` dimensions and origins:

```rust
proptest! {
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
```

- [ ] Add editor properties over arbitrary Unicode insertion/deletion/movement sequences. After every operation assert `cursor_byte <= buffer.len()` and `buffer.is_char_boundary(cursor_byte)`.
- [ ] Add a redaction test that submits `/unknown password=hunter2 token=abc`, applies its rejection outcome, renders the next frame, and asserts the frame contains none of `hunter2`, `token=abc`, or the full rejected input.
- [ ] Add failure injection for every terminal initialization and restoration operation. Assert every acquired resource is released, restoration continues after individual failures, and callers receive only `TerminalInitialization` or `TerminalOutput`.
- [ ] Add a panic-path host test whose fake screen panics during draw. Assert runtime finish is attempted, restoration occurs exactly once, and the result is `TuiError::Panicked` without panic text.
- [ ] Run `cargo test --test tui_hardening_contract --locked`; expect at least one property or injected-failure test to fail before hardening.
- [ ] Fix only exposed invariant violations using saturating/clamped geometry, checked UTF-8 boundaries, generic rejection messages, and best-effort restoration.
- [ ] Run:

```bash
cargo test --test tui_hardening_contract --locked
cargo test --test windows_source_static_contract --locked
```

Expected: hardening and platform-static contracts pass.

- [ ] Commit:

```bash
git add Cargo.toml Cargo.lock src/ui/tui/model.rs src/ui/tui/layout.rs src/ui/tui/terminal.rs src/ui/tui/controller.rs tests/tui_hardening_contract.rs
git commit -m "test: harden tui boundaries"
```

---

## Task 13: Document Manual Acceptance and Run Release Gates

**Files:** `README.md`, `docs/phase-0b-testing.md`

- [ ] Update `README.md` with these launch examples:

```text
cargo run --locked
cargo run --locked -- --command-mode
printf '/status\n/quit\n' | cargo run --quiet --locked
```

Document that the first command launches the Adaptive Cockpit only when both stdin and stdout are terminals. Document the four views, every approved key, `NO_COLOR=1`, the `60x18` minimum, automatic redirected-stream fallback, single-instance behavior, and the read-only Phase 0B scope.

- [ ] Create `docs/phase-0b-testing.md` with this acceptance matrix:

| Scenario | Expected result |
|---|---|
| Interactive launch | Alternate-screen cockpit appears; shell contents are not overwritten. |
| `--command-mode` | Existing line protocol and text rendering remain unchanged. |
| Redirected stdin | Command mode is selected automatically. |
| Redirected stdout | Command mode is selected automatically. |
| Widths 59, 60, 79, 80, 119, 120 | TooSmall, Narrow, Narrow, Medium, Medium, Wide when the matching height threshold is met. |
| Heights 17, 18, 23, 24, 29, 30 | TooSmall below 18; width-dependent Narrow/Medium/Wide above it. |
| Keys `1`-`4`, Tab, arrows, paging, `/`, Esc, `i`, `?` | Focus and native views update without transcript output. |
| `/help`, `/status`, `/setup status`, `/audit`, invalid input | Existing application command semantics and audit behavior are preserved. |
| `q` | Auditable clean shutdown, terminal restored, success exit. |
| Ctrl+C | Clean external-signal shutdown and terminal restoration. |
| Forced UI error/panic seam | One safe line after restoration; no payload or path leakage. |
| `NO_COLOR=1` | No foreground/background colors; focus remains visibly distinct. |
| Second process | Existing single-instance protection rejects it safely. |
| macOS, Linux, Windows | Keyboard input, resizing, shutdown, and restoration meet the same contract. |

- [ ] Run formatting and address only formatter output:

```bash
cargo fmt --all --check
```

- [ ] Run lint with warnings denied:

```bash
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

- [ ] Run the complete locked test suite:

```bash
cargo test --workspace --all-targets --locked
```

- [ ] Invoke `superpowers:requesting-code-review`. Resolve every confirmed correctness, safety, regression, and missing-test finding with a new failing test before its fix.
- [ ] Invoke `superpowers:verification-before-completion` and rerun all three release gates after review changes.
- [ ] Perform the manual acceptance matrix on the available host. Record unavailable operating systems as unverified instead of claiming cross-platform execution.
- [ ] Commit documentation and final verified corrections without amending earlier commits:

```bash
git add README.md docs/phase-0b-testing.md
git commit -m "docs: add phase 0b tui testing guide"
```

- [ ] Confirm final branch state:

```bash
git status --short
git log --oneline --decorate -14
```

Expected: no unintended changes, all release gates passed in the final state, and Phase 0B commits are individually reviewable.

## Completion Criteria

- [ ] Interactive TTY sessions default to the full-screen Adaptive Cockpit.
- [ ] `--command-mode` and redirected streams preserve the existing Phase 0 line host.
- [ ] All commands cross the existing parser, runtime, application service, policy, event, audit, and persistence boundaries.
- [ ] Wide, Medium, Narrow, and TooSmall layouts match the approved breakpoints.
- [ ] Overview, Setup, Audit, and Help are native views with no transcript.
- [ ] Input remains responsive while one command is pending, and a second command cannot race it.
- [ ] Quit, Ctrl+C, errors, and panics restore the terminal and finish the runtime cleanly.
- [ ] No secret-bearing input, path, panic payload, or raw internal error is emitted after failure.
- [ ] `NO_COLOR` remains usable and mouse capture remains disabled.
- [ ] Existing Phase 0 tests plus all new Phase 0B tests, formatting, and lint gates pass.
- [ ] Documentation provides a reproducible local and cross-platform acceptance procedure.
