# Task 12 Report: Property, Redaction, and Restoration Hardening

## Status

PASS. Task 12 was completed from formatting checkpoint `9a662f2` on
`codex/phase-0b-full-screen-tui`.

The valid adversarial properties and injected-boundary tests were already green
against the existing production implementation. In accordance with the task
brief, no unnecessary production behavior change was made.

## Property Domains and Case Counts

- Layout containment: 2,048 Proptest cases. Inputs independently cover arbitrary
  `u16` `x`, `y`, `width`, and `height` values plus arbitrary inspector-open
  state. Every calculated header, workspace, message, command, navigation, and
  inspector rectangle is checked against an independently calculated containment
  predicate, and the viewport must equal the input area.
- Unicode editor operations: 512 Proptest cases. Each case contains 0 through
  128 operations selected from arbitrary Unicode scalar insertion, left/right,
  home/end, backspace/delete, clear, take-text, arbitrary Unicode history entry,
  and previous/next history navigation. After every operation, the cursor must
  be at or before the buffer length and on a UTF-8 character boundary.
- Secret rejection: one deterministic public contract submits
  `/unknown password=hunter2 token=abc`, consumes the typed rejection, applies
  its outcome, and renders the next frame. The full line, `hunter2`, and
  `token=abc` are absent from rejection metadata, model debug state, command
  history, message, audit entries, and the rendered frame.
- Terminal initialization: three injected failures, one at each acquisition
  operation (`enable_raw`, `enter_alt`, and `hide_cursor`). Each case checks the
  exact best-effort release sequence for resources acquired before the failure,
  a generic `TerminalInitialization` result, and absence of the injected OS
  error text.
- Terminal restoration: four individual injected failures (`show_cursor`,
  `leave_alt`, `disable_raw`, and `flush`) plus one case where all four fail.
  Every case checks that all remaining restoration operations are attempted,
  the result is only `TerminalOutput`, and the injected OS error text is absent.
- Panic host: the existing panicking fake-screen case now also checks that
  runtime finish is attempted with `ApplicationError`, screen restoration occurs
  exactly once, the result is `TuiError::Panicked`, and the panic payload is not
  formatted into that result.

## RED/GREEN Evidence

- The initial dependency-resolution compile found a test-harness-only mistake:
  the public `render` module was called as a function and one import was unused.
  This was corrected before any behavioral run and is not counted as RED.
- The first private focused compile found a test-harness-only `Debug` bound from
  formatting a `Result` whose success type was `TerminalGuard`. The test was
  changed to extract and inspect only the error. This is also not counted as RED.
- First valid locked public hardening run: 3 passed, 0 failed. Both properties
  and the secret rejection/render contract were already green before any
  production behavior change.
- First valid focused terminal run: 4 passed, 0 failed, including every new
  individual initialization and restoration injection.
- First valid focused panic-host run: 1 passed, 0 failed with the new finish,
  restore-once, typed-error, and payload-redaction assertions.
- Honest conclusion: no genuine property or injected boundary failed. Existing
  saturating/clamped geometry, checked UTF-8 cursor handling, generic rejection
  messaging, best-effort restoration, and panic redaction already satisfy the
  expanded contracts. No production fix was warranted.

## Verification

- `cargo test --test tui_hardening_contract --locked`: 3 passed, 0 failed.
- `cargo test --lib ui::tui:: --locked`: 64 passed, 0 failed.
- `cargo test --test windows_source_static_contract --locked`: 5 passed,
  0 failed.
- `cargo test --locked`: 302 passed, 0 failed across unit, integration, static,
  acceptance, and documentation tests; 0 doctest failures.

## Files

- `Cargo.toml`: added development-only `proptest = "1.7.0"`.
- `Cargo.lock`: locked Proptest 1.7.0 and its transitive development
  dependencies.
- `tests/tui_hardening_contract.rs`: added public layout, Unicode editor, and
  rendered rejection contracts.
- `src/ui/tui/terminal.rs`: extended the existing private fake-terminal tests
  across every acquisition and restoration operation.
- `src/ui/tui/host.rs`: strengthened the existing private panic-host test.
- `.superpowers/sdd/2026-09-03-phase-0b-full-screen-tui/task-12-report.md`:
  recorded Task 12 evidence and review.

No changes were required in `src/ui/tui/model.rs`, `src/ui/tui/layout.rs`, or
`src/ui/tui/controller.rs`; their existing behavior passed the new public
contracts. No production visibility was widened for testing.

## Fixes

- Added the required development-only property-testing dependency and locked it
  to Proptest 1.7.0.
- Added adversarial public and private coverage without changing runtime
  behavior.
- Corrected only test-harness compile issues encountered before valid RED/GREEN
  execution.

## Self-Review

- Reviewed the complete scoped source/test diff and the generated lockfile diff.
- Confirmed the integration test exercises public APIs and the private terminal
  and host seams remain private.
- Confirmed the existing fake terminal, screen, event, and runtime infrastructure
  was extended rather than duplicated.
- Confirmed assertions report generic failures and do not echo secret input,
  panic payloads, or injected OS error text.
- Confirmed no unrelated file was reformatted or edited.
- `git diff --check` reported no whitespace errors.

## Concerns

- The requested genuine behavioral RED did not occur. This is recorded
  explicitly rather than manufacturing a failure or making an unsupported
  production change, as directed by the brief.
- No conflict with an approved breakpoint or Phase 0 invariant was exposed.
