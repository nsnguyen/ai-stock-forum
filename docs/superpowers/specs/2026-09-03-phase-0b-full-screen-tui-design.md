# Phase 0B Full-Screen TUI Design

**Status:** Approved for implementation planning  
**Date:** 2026-09-03  
**Base:** `main` at merge commit `fea0085`  
**Feature branch:** `codex/phase-0b-full-screen-tui`

## 1. Summary

Phase 0B replaces the interactive default presentation with a full-screen,
responsive terminal user interface while preserving the Phase 0 application,
runtime, persistence, recovery, audit, process-guard, panic, and privacy
boundaries.

The selected visual structure is the **Adaptive Cockpit**. Wide terminals show
navigation, a primary workspace, and a contextual inspector together. Medium
terminals show navigation and the workspace with an inspector overlay. Narrow
terminals use focused tabs. The existing line-oriented command host remains a
first-class adapter for explicit command mode and non-interactive execution.

Phase 0B is presentation work only. It does not add financial calculations,
agent orchestration, market-data access, brokerage integration, configuration
editing, or order execution.

## 2. Relationship to the Product Roadmap

The product roadmap already names the deterministic risk core as Product Phase
1. The Rust foundation design separately reserved `src/ui/tui/` as the next UI
adapter boundary. To avoid reusing the Phase 1 label for two different efforts,
this work is named **Phase 0B: Full-Screen TUI Adapter**.

Product Phase 1 remains the deterministic risk core and may follow Phase 0B or
proceed independently after this adapter is stable.

## 3. Goals

Phase 0B must:

1. Make the full-screen TUI the default for an interactive terminal.
2. Preserve the command host for `--command-mode` and non-interactive streams.
3. Present existing status, setup-status, audit, help, and command capabilities
   as native views.
4. Reuse typed application commands and results instead of parsing rendered
   command-host text.
5. Preserve exactly-once application command auditing without auditing passive
   navigation or redraws.
6. Adapt safely from wide terminals down to a documented minimum size.
7. Restore the terminal after every normal and abnormal exit path.
8. Remain cross-platform across macOS, Linux, and Windows.
9. Keep rendering deterministic and testable without a real terminal.
10. Preserve all Phase 0 security, privacy, recovery, and process-ownership
    guarantees.

## 4. Non-Goals

Phase 0B does not include:

- setup or policy editing;
- credential, token, provider, or model-profile entry;
- live data, networking, or arbitrary filesystem access;
- deterministic financial risk calculations;
- agents, debates, recommendations, approvals, or trading;
- a browser or React frontend;
- mouse capture or mouse-driven navigation;
- persisted command history or UI preferences;
- a shell, subprocess launcher, or arbitrary command execution;
- rich charts, candlesticks, or market visualizations;
- background database polling;
- shell suspension and resume support beyond normal platform behavior.

## 5. Approved Product Decisions

The following decisions were approved during design:

- Interactive launches use the TUI by default.
- `--command-mode` explicitly selects the existing line-oriented host.
- Non-interactive stdin or stdout automatically selects command mode.
- The layout is the Adaptive Cockpit.
- Keyboard interaction is a hybrid of direct shortcuts, focus traversal, and a
  slash-command bar.
- Scope is UI parity with existing Phase 0 behavior.
- Slash-command results update native views rather than a transcript.
- Layout collapses responsively instead of imposing one large fixed size.
- Ratatui provides layout and widgets; Crossterm provides terminal I/O and
  lifecycle control.
- Mouse capture remains disabled.

No unresolved product decision remains before implementation planning.

## 6. Architecture

The TUI is a presentation adapter over the existing typed application service.
It must not own persistence or duplicate domain behavior.

```text
                       +----------------------+
terminal events ------>| TUI controller       |
                       +----------+-----------+
                                  |
                    navigation intent or ApplicationCommand
                                  |
                +-----------------+-----------------+
                |                                   |
                v                                   v
        +---------------+                 +--------------------+
        | TUI model     |<----------------| application/runtime|
        +-------+-------+  typed outcome  +--------------------+
                |
                v
        +---------------+
        | pure renderer |
        +---------------+
```

The command host and TUI remain sibling adapters:

```text
                     +---------------------+
                     | typed application   |
                     | and runtime boundary|
                     +----------+----------+
                                |
                 +--------------+--------------+
                 |                             |
          +------v-------+              +------v------+
          | command host |              | full-screen |
          | text adapter |              | TUI adapter |
          +--------------+              +-------------+
```

Neither adapter may read SQLite, mutate projections, or interpret event-store
rows directly.

## 7. Launch and Mode Selection

Mode selection happens before application bootstrap and before any terminal
mutation.

1. Parse process arguments.
2. If `--command-mode` is present, select command mode.
3. Otherwise, inspect stdin and stdout.
4. If both are terminals, select TUI mode.
5. If either is not a terminal, select command mode.
6. Reject unknown arguments with a safe usage error before bootstrap.

Examples:

```bash
# Interactive stdin and stdout: full-screen TUI
cargo run --locked

# Explicit line-oriented adapter
cargo run --locked -- --command-mode

# Automatic line-oriented adapter because stdin is a pipe
printf '/status\n/quit\n' | cargo run --locked
```

After TUI mode is selected, a terminal initialization failure is fatal. The
program restores any completed initialization steps and emits one safe error
line. It does not silently switch adapters after partial terminal mutation.

The existing command-host output and exit-status contracts remain unchanged.

## 8. Dependencies and Compatibility

Production dependencies added by Phase 0B are limited to:

- `ratatui` for deterministic layout, widgets, styles, and buffer rendering;
- `crossterm` for terminal capability detection, events, raw mode, alternate
  screen control, cursor control, and resize handling.

Versions must support the repository's pinned Rust 1.98 toolchain and must be
locked in `Cargo.lock`. Optional features must be minimized. No async runtime,
network client, telemetry package, or terminal mouse feature is introduced.

User-visible strings and layout symbols use ASCII-compatible labels. User data
may contain Unicode, but width calculation, truncation, and wrapping must use
terminal cell width rather than byte length.

`NO_COLOR` selects a monochrome style that preserves focus and severity through
labels and attributes. The normal palette uses standard terminal colors so it
remains readable without true-color support.

## 9. Proposed Module Boundaries

```text
src/
|-- cli.rs                         launch-mode parsing and TTY selection
`-- ui/
    |-- command/                   existing command-host adapter
    `-- tui/
        |-- mod.rs                 public adapter entry point
        |-- model.rs               presentation state only
        |-- intent.rs              navigation and UI intent types
        |-- controller.rs          event-to-intent and command dispatch
        |-- terminal.rs            RAII terminal ownership
        |-- event_source.rs        production and scripted event sources
        |-- responsive.rs          total layout calculation
        |-- render.rs              pure top-level renderer
        `-- views/
            |-- mod.rs
            |-- overview.rs
            |-- setup.rs
            |-- audit.rs
            `-- help.rs

tests/
|-- tui_application_contract.rs
|-- tui_cli_contract.rs
`-- tui_terminal_contract.rs
```

The implementation plan may combine very small files when that improves
clarity, but it must retain these responsibility boundaries. In particular,
terminal ownership, event interpretation, model transitions, and rendering may
not collapse into one event-loop module.

## 10. Presentation State

`TuiModel` contains presentation state only:

- active view;
- focused region;
- responsive layout mode;
- command-buffer contents and cursor position;
- memory-only command history and history cursor;
- per-view selection and scroll offsets;
- inspector open or closed state where applicable;
- the most recent typed presentation snapshot;
- at most one in-flight application command;
- a safe, dismissible notice or error message;
- busy-indicator frame state.

The model must not contain a database connection, repository, raw event-store
record, terminal handle, secret, filesystem path, panic payload, or command-host
rendered text.

Command history is capped at 100 non-empty entries. Consecutive duplicate
entries collapse to one. History is never persisted. A message remains until
the next relevant action or explicit dismissal; behavior does not depend on a
short timeout that could harm readability or test determinism.

Only one application command may be in flight. While it is running, navigation
and resize remain responsive, duplicate submission is rejected locally, and
shutdown remains available.

## 11. Typed Presentation Snapshot

The TUI needs passive state for initial render, view switching, and the recent
events inspector. Passive rendering must not append audit events. If the Phase
0 application boundary does not yet expose sufficient typed data, Phase 0B adds
a read-only typed presentation query at that boundary.

The snapshot may contain only the sanitized data needed by the four views:

- runtime and application health state;
- database readiness without its path;
- process-guard held state without lock metadata;
- setup readiness and named non-secret missing requirements;
- bounded recent audit summaries;
- stable identifiers and display timestamps where already permitted.

Snapshot reads:

- go through the application boundary;
- do not access SQLite from TUI code;
- do not append events;
- are bounded and deterministic;
- do not return raw serialized event payloads;
- apply the same redaction policy as command results.

User-invoked slash commands keep existing semantics. For example, `/status`
still commits its existing typed audit event exactly once and refreshes the
Overview view from its typed result. Opening the Overview view with `1` is local
navigation and does not pretend that the user issued `/status`.

This separation prevents passive redraws or inspector refreshes from creating
recursive audit traffic.

## 12. Event Loop and Data Flow

The production event loop owns the model and terminal session. External work is
submitted through the existing runtime boundary.

1. Read a Crossterm event from an injected event source.
2. Translate it into a typed UI intent.
3. Apply local navigation intents directly to `TuiModel`.
4. Parse submitted slash commands with the existing command parser.
5. Submit valid `ApplicationCommand` values through the existing runtime.
6. Map typed outcomes into presentation-model updates.
7. Render only after a state change, resize, busy-frame change, or command
   completion.

The loop may use a bounded event wait so it can observe shutdown and command
completion. It must not busy-loop or poll SQLite. Time and events are injected
for deterministic tests.

Unexpected terminal event variants are ignored safely. Paste events are
treated as text only while the command bar is focused and pass through the same
length and control-character rules as typed input.

## 13. Adaptive Cockpit Layout

### 13.1 Persistent regions

The full-screen frame contains:

- a one-row header with product name, local mode, and safe status;
- navigation appropriate to the current breakpoint;
- one primary view;
- a contextual inspector when space permits or when explicitly opened;
- a message row for safe notices and errors;
- a command bar and concise key hints.

The header never displays usernames, filesystem paths, process IDs, database
locations, tokens, or account data.

### 13.2 Breakpoints

Breakpoint calculations are total for every `Rect`, including zero dimensions.

| Terminal size | Layout |
|---|---|
| Width at least 120 and height at least 30 | Navigation, workspace, inspector |
| Width 80-119 and height at least 24 | Navigation and workspace; inspector overlay |
| Width 60-79 and height at least 18 | Top tabs and one focused view; inspector overlay |
| Width below 60 or height below 18 | Minimum-size screen |

The minimum-size screen states the required `60x18` size and current size. It
must still process resize, `q`, and `Ctrl+C`.

Wide layout starts with a 20-column navigation region and a 32-column
inspector, then gives the remaining width to the workspace. Calculations clamp
all regions before rendering. Medium layout preserves a readable navigation
region and gives all remaining width to the workspace. Narrow layout replaces
the navigation region with one-row tabs.

The inspector is persistent when wide. On medium and narrow layouts, `i`
toggles it as a bounded overlay. `Esc` closes it before performing any broader
cancel behavior.

## 14. Views

### 14.1 Overview

Overview presents:

- application and runtime health;
- database readiness;
- process-guard ownership state;
- setup readiness;
- bounded recent event summaries in the inspector.

States use both a label and style, such as `OK`, `PENDING`, and `ERROR`.

### 14.2 Setup

Setup presents existing setup readiness and non-secret missing requirements.
It is explicitly read-only and explains that editing is deferred. No empty
input controls imply that credentials can be entered.

### 14.3 Audit

Audit presents a bounded, scrollable list of sanitized event summaries. The
inspector presents the selected event's safe summary. It never presents raw
JSON, debug formatting, opaque panic data, terminal-control bytes, or a direct
database record dump.

The current command contract continues to bound `/audit tail` limits to its
existing range. TUI list navigation does not silently request an unbounded
history.

### 14.4 Help

Help lists:

- view shortcuts;
- focus, list, inspector, and command-bar controls;
- supported slash commands;
- the explicit `--command-mode` launch option;
- automatic command-mode behavior for redirected streams;
- safe shutdown keys.

Help content is static, local, and versioned with the executable.

## 15. Keyboard Interaction

Shortcuts apply only when the command bar is not capturing text unless stated
otherwise.

| Key | Behavior |
|---|---|
| `1`, `2`, `3`, `4` | Open Overview, Setup, Audit, or Help |
| `Tab` | Move focus forward through visible regions |
| `Shift+Tab` | Move focus backward through visible regions |
| Arrow keys | Move within the focused list or field |
| `PageUp`, `PageDown` | Move by a visible page in a scrollable view |
| `Home`, `End` | Move to the bounded start or end of a list |
| `/` | Focus the command bar and begin a slash command |
| `Enter` | Submit a non-empty command or activate a focused item |
| `Up`, `Down` in command bar | Traverse memory-only command history |
| `Esc` | Close inspector, cancel command input, or clear a message |
| `i` | Toggle or focus the contextual inspector |
| `?` | Open Help |
| `q` | Request typed shutdown outside command entry |
| `Ctrl+C` | Request typed shutdown from any focus state |

`q`, `Ctrl+C`, and `/quit` converge on the existing audited shutdown command.
The controller must prevent `q` inside command input from becoming shutdown.

Mouse reporting is never enabled. This preserves terminal text selection,
reduces platform-specific behavior, and keeps Phase 0B's input contract small.

## 16. Native Command Results

The TUI does not emulate a shell transcript.

- `/status` opens or refreshes Overview.
- `/setup status` opens or refreshes Setup.
- `/audit tail N` opens or refreshes Audit with the requested bounded limit.
- `/help` opens Help.
- `/quit` requests shutdown.
- Rejected input remains in the current view and produces a safe message.

The existing parser remains authoritative for whitespace, limits, unknown
commands, and rejection semantics. The TUI must not maintain a second parser.

Application outcomes are mapped by variant or typed fields. They are never
reparsed from bytes emitted by the command-host renderer.

## 17. Terminal Ownership and Restoration

Application bootstrap completes before terminal mutation:

1. discover and validate application paths;
2. open the database and apply migrations;
3. rebuild or verify projections;
4. acquire the process guard;
5. construct the application/runtime boundary;
6. enter raw mode;
7. enter the alternate screen;
8. hide the cursor;
9. run the TUI event loop.

Mouse capture is omitted. Initialization records each completed step. A failure
rolls back completed steps in reverse order.

`TerminalSession` owns restoration through idempotent RAII cleanup:

1. show the cursor if hidden;
2. leave the alternate screen if entered;
3. disable raw mode if enabled;
4. flush the output stream as appropriate.

Cleanup executes for:

- normal `/quit` or `q` shutdown;
- `Ctrl+C` and existing interruption handling;
- input closure where the platform exposes it;
- render or event-source errors;
- application/runtime failures;
- unwinding panics;
- partial terminal initialization failures.

The primary failure determines the safe diagnostic and exit status. Cleanup
failures must not replace it with debug details. If cleanup is the only failure,
the program returns failure and emits one safe line after the best available
restoration attempt.

The process guard remains held until after the event loop and terminal session
finish. A competing process is rejected before raw mode or alternate-screen
entry.

## 18. Error and Notice Semantics

Errors have two classes:

### Recoverable presentation or command errors

- Display a bounded safe message.
- Preserve the current view and usable navigation.
- Do not expose causes, paths, SQL, payloads, or panic text.
- Dismiss on `Esc` or the next relevant action.
- Preserve existing command auditing and typed rejection behavior.

### Fatal startup, runtime, or terminal errors

- Stop accepting commands.
- Restore the terminal first.
- Render exactly one safe line through the existing error boundary.
- Return the existing failure exit status.
- Never attempt to redraw after terminal teardown begins.

Messages sanitize control characters before entering the model. Rendering code
must not receive terminal escape sequences as trusted styling input.

## 19. Visual and Accessibility Rules

The TUI uses a restrained operational palette:

- terminal default or charcoal background;
- terminal default or warm light foreground;
- green for selected and healthy states;
- yellow for pending and caution states;
- red only for actionable errors;
- dim styling for secondary metadata.

Color never carries meaning alone. Every status has a text label. Focused
regions have a visible border and title marker. Monochrome mode remains fully
navigable.

All truncation is cell-width aware and panic-free. Selected content remains
visible as lists move. Empty states explain what is absent without presenting a
fake success. The only animation is a low-frequency busy indicator while a
command is in flight.

## 20. Security and Privacy Invariants

Phase 0B preserves and extends the Phase 0 trust boundary:

1. TUI modules cannot open the database or state directory.
2. TUI modules cannot spawn processes or access the network.
3. TUI modules receive only typed, sanitized application data.
4. Passive snapshot reads never return secrets or raw persisted payloads.
5. Errors never contain paths, SQL, panic payloads, environment values, or
   sensitive records.
6. Event text is control-character sanitized before rendering.
7. Input length is bounded before parsing or storing command history.
8. The process guard is acquired before terminal mutation.
9. Existing secure path traversal and `NOFOLLOW` protections remain unchanged.
10. Navigation and redraws do not create hidden audit events.
11. User commands retain exactly-once event and projection semantics.
12. Command mode remains available for recovery and deterministic automation.

## 21. Testing Strategy

### 21.1 Pure unit tests

Test model transitions for:

- focus order in each responsive layout;
- view selection and per-view scroll state;
- command editing and cursor movement;
- bounded history and consecutive duplicate collapse;
- message dismissal precedence;
- inspector behavior;
- one-command-in-flight enforcement;
- shutdown availability in every state.

Test controller mappings for every key in the approved table, including `q`
inside command input and `Ctrl+C` from every focus target.

### 21.2 Responsive and renderer tests

Use Ratatui's in-memory backend and direct buffer assertions. Required boundary
dimensions include:

- zero width and height;
- `59x18` and `60x17` minimum-size failures;
- `60x18` narrow entry;
- widths 79 and 80;
- widths 119 and 120;
- heights 23, 24, 29, and 30;
- very large dimensions;
- long Unicode and combining-character content;
- empty, maximum-length, and control-character-sanitized values.

Property tests generate arbitrary rectangles, selections, offsets, and bounded
display strings. Rendering must never panic, produce an out-of-bounds buffer
write, or leave a selected index invalid.

### 21.3 Application integration tests

Prove that:

- initial presentation snapshots are typed and non-auditing;
- navigation and resize append no events;
- each slash command reaches the application boundary exactly once;
- each existing command retains its event and projection semantics;
- command outcomes update the intended native view;
- rejected commands remain safe and recoverable;
- audit display is bounded and sanitized;
- TUI code has no direct persistence dependency.

### 21.4 Terminal lifecycle tests

Inject terminal operations and scripted event sources to prove exact setup and
reverse cleanup order for:

- complete initialization and normal exit;
- failure after each individual initialization step;
- event read failure;
- render failure;
- application failure;
- interruption;
- panic unwind;
- repeated cleanup.

Tests assert that cleanup is idempotent and that the primary error is retained.
Platform-specific adapters cover Windows console behavior without weakening the
existing cancellation tests.

### 21.5 CLI and process contracts

Subprocess tests prove:

- TTY stdin plus TTY stdout selects TUI mode;
- piped stdin selects command mode;
- redirected stdout selects command mode;
- `--command-mode` selects command mode even with a TTY;
- unknown arguments fail before bootstrap;
- a competing process is rejected before terminal entry;
- existing command-host byte and exit contracts remain unchanged.

Headless CI does not depend on a human terminal. A small platform-appropriate
pseudo-terminal contract may prove mode selection and teardown, while detailed
screen behavior remains in deterministic buffer tests.

### 21.6 Quality gates

Before completion, run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

All 217 Phase 0 baseline tests must remain green alongside new Phase 0B tests.
The implementation plan must add focused red-green commands for each task.

## 22. Manual Acceptance

Phase 0B passes manual acceptance only when all of the following are observed:

1. `cargo run --locked` opens the Adaptive Cockpit in an interactive terminal.
2. Overview, Setup, Audit, and Help show real Phase 0 data.
3. The wide, medium, narrow, and minimum-size states match this specification.
4. Slash commands update native views without a transcript.
5. Keyboard focus, scrolling, history, inspector, and dismissal behave as
   documented.
6. `q`, `/quit`, and `Ctrl+C` restore the original screen, cursor, echo, and
   canonical input behavior.
7. A forced application error and a controlled panic restore the terminal
   before one safe line is emitted.
8. A second process is rejected without entering alternate-screen mode.
9. Piped execution and `--command-mode` preserve the line-oriented adapter.
10. Restarting the app shows persisted Phase 0 audit state without hidden event
    growth caused by rendering.
11. Screenshots are captured at representative wide, medium, and narrow sizes.
12. Formatting, Clippy, the full workspace suite, and independent code review
    pass with no unresolved critical or important findings.

## 23. Documentation Deliverables

Implementation updates must cover:

- README launch instructions for interactive and command modes;
- complete keyboard reference;
- breakpoint and minimum-size behavior;
- terminal recovery guidance;
- architecture documentation for the two sibling UI adapters;
- Phase 0B completion status only after every exit criterion passes.

## 24. Explicitly Deferred Work

The following remain later-phase work rather than omissions:

- editable setup and policy workflows;
- model-provider and Hermes configuration;
- the deterministic risk core;
- agent forum data and live debate streaming;
- recommendations, approval integrity, and execution;
- charts and market-specific visualizations;
- mouse interaction;
- persisted UI preferences and command history;
- browser dashboard integration.

The Adaptive Cockpit intentionally leaves its primary workspace and inspector
extensible for those later capabilities without implementing placeholders that
pretend they already exist.
