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

## Fix Round 1

### Review Findings Addressed

- End-to-end redaction now uses the existing isolated `support::runtime()`
  fixture, which bootstraps a real `ApplicationService` and runs it through
  `ApplicationRuntime`. The exact secret-bearing line still enters through
  controller key events and `handle_event`; the resulting authoritative
  `ApplicationCommand::RejectInput` is submitted to the real runtime.
- The real service outcome is required to contain exactly one committed event,
  and that event must be `ApplicationEvent::CommandRejected`. `apply_outcome`
  then converts the committed event into exactly one `command_rejected` audit
  entry before the next frame is rendered.
- The stronger contract proves the full raw line, `hunter2`, and `token=abc`
  are absent from the submitted command metadata, real service outcome,
  editor, history, generic message, converted audit entry, complete model, and
  rendered buffer. It also proves the fixed generic rejection message remains
  in both the model and rendered frame.
- The lockfile was compared directly with pre-Task 12 commit `9a662f2`. The
  original `windows-sys 0.61.2` edges for `dirs-sys 0.5.0`, `errno 0.3.14`,
  `rustix 1.1.4`, and `tempfile 3.27.0` are restored. The original
  `rustix 0.38.44` edge remains on `windows-sys 0.59.0`.
- The final base-to-current lock diff contains only the root Proptest
  development edge, genuinely new Proptest 1.7.0 transitive packages, and the
  necessary explicit `r-efi 6.0.0` qualification of the unchanged existing
  edge after adding `r-efi 5.3.0`.
- Terminal restoration now uses a private generic `retain_first_error` helper.
  Each OS operation is still attempted before its result is passed to the
  helper, and each OS error is mapped immediately to fixed
  `TuiError::TerminalOutput`; public errors remain payload-free and visibility
  was not widened.

### RED/GREEN Evidence

- RED: the new distinct-sentinel accumulator unit failed to compile with
  unresolved import `super::retain_first_error`, demonstrating that the
  required generic helper did not exist.
- GREEN: after adding the minimal private helper and routing restoration
  results through it, the exact sentinel unit passed. It proves an initial
  success stores nothing, the first distinct error is retained, and a later
  distinct error cannot replace it.
- The end-to-end redaction replacement passed on its first valid run against
  the existing production redaction path; no production redaction fix was
  needed.
- During lock repair, all-target locked graph validation caught an over-broad
  intermediate edit to the `rustix 0.38.44` edge. That edge was restored to its
  exact `9a662f2` value before final verification.

### Fix Round 1 Verification

- `cargo tree --locked --target all -i windows-sys@0.61.2`: passed and showed
  the preserved root, `dirs-sys`, `errno`, `rustix 1.1.4`, and `tempfile`
  consumers.
- `cargo tree --locked --target all -i windows-sys@0.59.0`: passed and showed
  only the preserved `rustix 0.38.44` consumer path.
- `cargo test --test tui_hardening_contract --locked`: 3 passed, 0 failed.
- `cargo test --lib ui::tui::terminal::tests --locked`: 5 passed, 0 failed.
- `cargo test --lib ui::tui::host::tests --locked`: 11 passed, 0 failed.
- `cargo test --test windows_source_static_contract --locked`: 5 passed,
  0 failed.
- `cargo test --locked --quiet`: 303 passed, 0 failed, including 0 doctest
  failures.

### Fix Round 1 Files and Self-Review

- `tests/tui_hardening_contract.rs`: replaced the synthetic empty-event outcome
  with the real isolated service/runtime flow and asserted committed-event audit
  conversion.
- `Cargo.lock`: restored all pre-Task 12 production selections and edges while
  retaining only the required Proptest graph additions.
- `src/ui/tui/terminal.rs`: added the private generic first-error helper and its
  distinct-sentinel test.
- `.superpowers/sdd/2026-09-03-phase-0b-full-screen-tui/task-12-report.md`:
  appended complete fix-round evidence.
- Reviewed the complete fix-round diff and the full `9a662f2`-to-current
  lockfile diff. No unrelated source, dependency version, dependency edge,
  visibility, feature, or ledger change remains.

### Fix Round 1 Concerns

- None.
