# Task 8 Report: Keyboard-first Skills state machines

Date: 2026-09-06
Branch: `codex/phase-2-declarative-skills`
Base commit: `c5d4fe6fad58a5e3aa43e34c6849d08132de954f`
Checkpoint: this report is committed with the implementation; the exact resulting SHA is returned in the task handoff because a commit cannot contain its own hash.

## Files

- `src/ui/skill_editor.rs`
- `src/ui/mod.rs`
- `src/ui/tui/model.rs`
- `src/ui/tui/event.rs`
- `src/ui/tui/controller.rs`
- `src/ui/tui/host.rs`
- `src/ui/tui/mod.rs`
- `src/runtime/mod.rs`
- `tests/skill_editor_contract.rs`
- `tests/skill_tui_controller_contract.rs`
- `tests/skill_tui_host_contract.rs`
- `.superpowers/sdd/2026-09-05-declarative-skills/task-8-report.md`

## RED evidence

Command:

```text
cargo test --test skill_editor_contract --test skill_tui_controller_contract --test skill_tui_host_contract
```

Initial corrected RED exited 101. The compiler reported the intended absent `ui::skill_editor`, Skills model panes/actions/state, controller effects, host entry point, and `CommandExecutor::preview_skill_version` route.

A focused recoverable-validation regression also exited 101 after compiling and failed with:

```text
left: Some("")
right: Some("Source note")
```

The exact agent-detail unassignment contract exited 101 for the intended absent agent skill action state.

The independent-review fix round added focused RED evidence for each valid defect:

- Skills ownership: with Skills active over the Agents view, Down was consumed by Agents and left the Skills selection at `0` instead of advancing to `1`.
- Historical reference isolation: after viewing skill A history and loading skill B detail, exact-ref selection still returned A.
- Editor input ownership: seeded fields submitted empty values; invalid values disappeared; and Esc did not restore the prior field value.
- Origin tracking: the explicit workspace-origin contract initially failed to compile because no typed origin state existed. Agent-origin unassign cancellation also exposed unconditional Skills activation in the host.
- Confirmation cancellation: Esc retained the cancelled editor preview, allowing the next Enter to reuse it instead of requesting a fresh preview.
- Exact-once cancellation: after a cancellation error, `review_registered` remained true. The strengthened route-level regression records the cancellation inside the worker before returning its configured error.
- Host lifecycle coverage: a pending review initially could not complete `/quit` because confirmation dispatch preceded explicit command-mode dispatch; the host-exit regression reached EOF rather than a shutdown result.

## GREEN evidence

Fresh focused command:

```text
cargo test --test skill_editor_contract --test skill_tui_controller_contract --test skill_tui_host_contract
```

Result: exit 0, 25 passed, 0 failed.

- `skill_editor_contract`: 5 passed.
- `skill_tui_controller_contract`: 12 passed.
- `skill_tui_host_contract`: 8 passed.

No full suite was run, per Task 8 instructions.

## Behavior decisions

- `s` activates a dedicated non-rendered Skills workspace state and loads the library without mutating cached data. Adaptive rendering remains Task 9.
- Up/Down select skill, history, create-source, agent, and assigned-skill rows. Arrow keys also select contextual actions. Enter advances or confirms; Esc unwinds one safe level.
- The editor owns five stages: Identity, Usage, Instructions, References, and Review. Custom and library-seeded creation paths feed the same editor.
- Validation remains in `SkillDraft`; presentation errors identify the active field and retain recoverable raw input, including invalid reference bodies.
- Create/version confirmation commands use only `SkillEditPreview` tokens and digests returned by existing application preview paths.
- Assignment classification is explicit Add, Upgrade, Already Assigned, or Unassign. Upgrade and unassign retain exact `SkillVersionRef` values; no auto-upgrade exists.
- Agent detail exposes exact assigned refs through a keyboard-controlled selection panel without changing Task 9 rendering files.
- The host owns review registration. Esc/cancel/host exit clears each registered Skill review at most once; successful commands mark consumed reviews without cancelling again.
- Stale, policy, backpressure, and invalid-review errors clear pending confirmation/review state and retain an actionable editor or detail stage.
- Bare `q` never requests shutdown. `/quit` remains the normal user shutdown path.
- Runtime version preview uses a distinct typed request/reply variant and delegates directly to `ApplicationService::preview_skill_version`; it adds no token generation, commit path, or business logic.

## Independent-review findings

1. **Skills routing ownership: fixed.** Active Skills state is dispatched before Agents state. Workspace origin is explicit for cockpit and agent-skill-panel entry, and `/quit` command mode, once explicitly entered with `/`, owns subsequent input even while a protected confirmation is present.
2. **Historical exact-ref isolation: fixed.** Replacing detail clears historical version detail and cross-skill history, so A history cannot supply the selected ref after B detail loads. The A-history to B-detail assignment regression passes.
3. **Editor input preservation and seeding: fixed.** The command buffer is synchronized from the editor draft, validation reads without destructively taking input, invalid text remains editable, unchanged seeded fields advance on Enter, and Esc restores the prior field value. Reference-name/body state remains recoverable.
4. **Origin recovery: fixed for the reviewed paths.** Protected operations carry a typed Skills-pane or agent-panel origin. Cancellation and tested terminal stale-profile, policy-denied, and invalid-review outcomes restore the agent skill panel without also activating Skills or leaving an orphaned review.
5. **Editor confirmation Esc: fixed.** Esc clears the editor's installed preview before restoring the editor, so the next Enter requests a new application preview.
6. **Exact-once cancellation: fixed.** The host takes `review_registered` before dispatch. The worker-received-then-failed regression records one cancellation, returns an error, and proves a subsequent cleanup call does not dispatch again.
7. **Focused regression coverage: added.** Contracts cover Skills-over-Agents ownership, explicit workspace return, historical A-to-B switching, seeded and unchanged edits, invalid input, Esc restoration, agent-origin cancellation and terminal failures, fresh preview after confirmation Esc, and cleanup on `/quit`, interruption, terminal input/EOF, terminal output failure, and cancellation failure. Host-exit tests assert no mutation command is dispatched.

## Concerns

- Rendering and visual discoverability are intentionally deferred to Task 9.
- Only the three authorized focused test targets were executed; broader integration remains outside this task's verification scope.
- No new dedicated backpressure-injection regression was added in this fix round; the report therefore makes no new claim beyond preserving the existing retryable-failure path.
