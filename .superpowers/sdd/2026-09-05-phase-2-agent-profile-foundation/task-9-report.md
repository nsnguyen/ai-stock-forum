# Task 9 Report: Fallback Terminal Profile Workflows

## Status

Implemented and verified the fallback terminal workflows for agent profile list, show, history, create, and edit. The implementation uses the existing bounded fallback input host, routes active workflow input to `ProfileEditor` before command parsing, and keeps draft field input out of application commands, command receipts, audit events, and generic logs.

The brief names `src/ui/fallback/*`, but the reviewed baseline at `e99dd9a7944fdc55065c10b2be99ed46032e6b98` contains the fallback implementation at `src/ui/command/*`. The implementation preserves that established public path and existing Phase 0/1 and TUI parser behavior.

## TDD Evidence

### Initial RED

Command:

```text
cargo test --test agent_profile_fallback_contract
```

Result: expected failure, `0 passed; 5 failed`.

Observed failures:

- `parser_accepts_only_the_six_exact_agent_forms_and_typed_ids`: `agent list` was rejected.
- `create_selects_a_pinned_template_edits_fields_and_waits_for_yes`: no template selection screen existed.
- `edit_loads_active_version_previews_ordered_diffs_and_no_returns_to_review`: no editor workflow loaded the active profile.
- `eof_cancel_and_editor_prose_create_no_durable_draft_or_generic_log_entry`: no local editor mode consumed draft prose.
- `list_show_and_history_are_deterministic_terminal_safe_typed_views`: no profile table or typed detail/history rendering existed.

### Initial GREEN

Command:

```text
cargo test --test agent_profile_fallback_contract
```

Result: `5 passed; 0 failed`.

### Full-suite regression RED and correction

The first full suite exposed one existing runtime contract regression after the request transport was extended:

```text
cargo test
```

Result before correction: `273 passed; 1 failed` before Cargo stopped at `fallback_fix_round_contract`.

Failing test:

```text
command_panic_best_effort_finishes_real_service_before_worker_exit
left: Err(Closed)
right: Err(WorkerPanicked)
```

Root cause: the generalized request dispatcher moved the response sender inside the unwind boundary, so a panic dropped it before the worker could send the typed panic result. The dispatcher was corrected to retain each command, preview, or cancel response sender outside its unwind boundary and preserve the original best-effort finish semantics.

Focused regression evidence:

```text
cargo test --test fallback_fix_round_contract command_panic_best_effort_finishes_real_service_before_worker_exit -- --exact
```

RED: `0 passed; 1 failed` with `Err(Closed)` instead of `Err(WorkerPanicked)`.

GREEN: `1 passed; 0 failed`.

### Every copied field RED/GREEN

Self-review identified that the copied template role was not editable through the shared editor. The create and edit transcripts were extended first with `:role custom`.

RED:

```text
cargo test --test agent_profile_fallback_contract
```

Result: `3 passed; 2 failed`; create could not advance after the unknown control and edit had no role diff.

GREEN after adding the typed role control:

```text
cargo test --test agent_profile_fallback_contract
```

Result: `5 passed; 0 failed`.

### Final full suite

Command:

```text
cargo test --quiet
```

Result: `386 passed; 0 failed; 0 ignored` across all unit, integration, platform contract, and documentation test binaries.

The final Task 9 contract result included in that suite was `5 passed; 0 failed`.

## Transcript Coverage

- Parses only `agent list`, `agent show <profile-id>`, `agent history <profile-id>`, `agent create`, `agent create <template-id>`, and `agent edit <profile-id>` in the fallback parser.
- Rejects missing arguments, extra arguments, unknown agent verbs, unknown template IDs, and malformed UUID profile IDs.
- Keeps the original shared `parse_line` behavior intact for TUI and Phase 0/1 consumers; fallback-only forms use `parse_fallback_line`.
- `agent create` renders deterministic built-in template selection and supports local cancellation.
- `agent create <template-id>` copies the pinned template, permits role, identity, specialty, tags, personality, instructions, and optional binding changes, and submits no create command until `y` or `yes` confirms activation.
- `agent edit <profile-id>` loads the active typed profile view into `ProfileEditor`.
- Edit `:review` sends the editor request through the runtime to the service's passive preview API, then applies the response using the same editor request generation.
- Review output renders service-provided diffs in fixed semantic order with escaped before/after values.
- `:activate` enters local confirmation; `n` or `no` restores the review without activation, and `y` or `yes` alone submits the typed activation command.
- Editor field lines and confirmation lines are consumed before normal parsing and never become application commands, receipts, command history, audit events, or generic log content.
- EOF and `:cancel` clear local editor state and send service review cancellation. Host interrupt also cancels before shutdown; application finish remains the final service-side cancellation boundary for all host failures.
- Oversized input remains bounded by the existing `BoundedLineReader` and receives the existing typed safe rejection without entering the editor.
- List, show, and history use typed views with deterministic columns and labels; all free text is escaped and bounded, and no raw JSON is rendered.

## Files

- `src/ui/command/parser.rs`: fallback-specific exact grammar and typed workflow commands.
- `src/ui/command/renderer.rs`: deterministic safe list, detail, history, template, editor, diff, confirmation, and completion output.
- `src/ui/command/runner.rs`: local selection/editor/confirmation state machine and lifecycle cancellation.
- `src/ui/command/mod.rs`: fallback parser exports.
- `src/runtime/mod.rs`: bounded passive preview and review-cancel requests to the real application service, with panic semantics preserved.
- `src/ui/profile_editor.rs`: typed role editing so every copied template field can change.
- `tests/agent_profile_fallback_contract.rs`: parser, transcript, cancellation, non-leakage, rendering, and persistence contracts.
- `.superpowers/sdd/2026-09-05-phase-2-agent-profile-foundation/task-9-report.md`: this report.

## Self-review

- Confirmed active editor input is checked before `parse_fallback_line` and never falls through to generic command submission.
- Confirmed create and activation commands are held locally until exact affirmative confirmation.
- Confirmed declined confirmation retains the editor and authoritative review metadata.
- Confirmed preview generation is passed unchanged from `PreviewEditRequest` to `ProfileEditor::apply_preview`.
- Confirmed runtime preview is passive and runtime cancellation calls the service's review registry cancellation.
- Confirmed EOF and interrupt paths invoke local/service cancellation before returning.
- Confirmed terminal free text uses bounded escaping and structured views rather than serialization.
- Confirmed the original `parse_line` enum and TUI match remain unchanged.
- Confirmed runtime panic callers still receive `WorkerPanicked` and best-effort application finish still runs.
- `git diff --check` completed successfully with no whitespace errors.

## Concerns

No unresolved functional concerns. Two intentional scope adaptations are recorded:

- The baseline fallback modules are under `src/ui/command`, not the brief's nonexistent `src/ui/fallback` path.
- Meeting passive preview/cancel and every-copied-field requirements required small supporting changes in `src/runtime/mod.rs` and `src/ui/profile_editor.rs` in addition to the four fallback modules named by the brief.

## Commit

The implementation and contract tests were committed with the brief's exact message:

```text
feat: add fallback agent profile workflow
```

## Fix Round 1

### Status

Addressed all four Important review findings: create review diffs, typed reference values, bounded list/history rows, and recoverable confirmation backpressure.

### Focused RED

Command:

```text
cargo test --test agent_profile_fallback_contract
```

Result before production changes: `3 passed; 5 failed`.

The failures mapped directly to the reviewed gaps:

- Create review did not render the template-baseline field diffs.
- Show output did not render explicit `none` reference markers or reference values.
- List rendered 102 rows instead of the 100-row cap and omitted-count marker.
- Create confirmation was lost after `RuntimeError::Backpressure`, so retry timed out.
- Edit confirmation was lost after `RuntimeError::Backpressure`, so local cancel could not clean the service review.

### Focused GREEN

Command:

```text
cargo test --test agent_profile_fallback_contract
```

Result: `8 passed; 0 failed`.

Transcript coverage added in this round:

- Create retains the pinned template copy as an immutable presentation-local baseline.
- Create review renders typed changed fields in display-name, description, role, primary-specialty, specialty-tags, personality, instructions, and bindings order.
- Create review explicitly states `Unchanged fields omitted.`.
- Show renders bounded escaped skill and MCP reference values, with explicit `none` for empty accepted lists.
- List and history each render at most 100 typed rows, preserve input order, and show deterministic omitted counts.
- Create confirmation survives queue saturation and a second `y`/`yes` succeeds after capacity returns.
- Edit confirmation survives queue saturation and `:cancel` explicitly clears the service review without activation.

### Targeted regression verification

Command:

```text
cargo test --test fallback_contract --test fallback_fix_round_contract --test fallback_fix_round_2_contract --test runtime_contract --test agent_profile_editor_contract --test agent_profile_review_race_contract
```

Result: `54 passed; 0 failed`.

Coverage included:

- Existing fallback queue backpressure and input ordering.
- Host EOF, quit, interrupt, writer failure, reader failure, and panic lifecycle behavior.
- Runtime capacity, worker panic, shutdown race, reservation drain, and termination behavior.
- Profile editor navigation, mutation, review generation, stale preview, and safe cancellation behavior.
- Service review reservation and cancellation races.
- Unix bounded-input cancellation and EOF behavior.

### Final full suite

Command:

```text
cargo test --quiet
```

Result: `389 passed; 0 failed; 0 ignored`.

### Files changed in Fix Round 1

- `src/agents/profile.rs`: read-only string accessors for typed skill and MCP references.
- `src/ui/profile_editor.rs`: immutable create baseline retained only in local editor state.
- `src/ui/command/renderer.rs`: create diffs, reference values, and 100-row list/history caps with omitted counts.
- `src/ui/command/runner.rs`: confirmation-state restoration on backpressure and explicit abandoned-edit review cancellation.
- `tests/agent_profile_fallback_contract.rs`: create diff, reference marker, cap, retry, and cleanup regressions.
- `.superpowers/sdd/2026-09-05-phase-2-agent-profile-foundation/task-9-report.md`: Fix Round 1 evidence.

### Self-review

- The create baseline is cloned once from the pinned template and exposed immutably; it is never included in control summaries, commands, receipts, audit entries, or generic logs.
- Create diffs use typed `ProfileFieldDiff` and `ProfileFieldValue` values in the same fixed semantic order as edit diffs.
- Free-text before/after values and reference values use the existing bounded terminal escaping.
- Empty reference lists remain the only accepted current-milestone profile state and render as `none`; non-empty typed values are rendered if present in a view.
- Row caps apply in the renderer independently of service-side limits and report the exact omitted count.
- Backpressure restores the owned editor and command before rendering the transient error, preserving exact retry/cancel state.
- Runtime errors after confirmation acceptance explicitly attempt edit-review cancellation before returning.
- Existing runtime panic semantics and fallback lifecycle tests remain green.
- `git diff --check` passed with no whitespace errors.

### Concerns

No unresolved concerns. Current profile validation intentionally continues to reject non-empty skill and MCP references; this round changes presentation only, so accepted empty refs remain unchanged while renderer behavior is ready for typed non-empty views in a later milestone.

### Commit

Fix Round 1 uses the requested commit message:

```text
fix: preserve fallback profile workflows
```

Commit:

```text
09681134a3b6b3e2243dba60cc6292a3e70d6abc
```

The `.superpowers` directory is repository-ignored, so this report remains at the required local path and is not included in the commit.
