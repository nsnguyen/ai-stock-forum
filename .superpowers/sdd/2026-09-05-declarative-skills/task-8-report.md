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

## GREEN evidence

Fresh focused command:

```text
cargo test --test skill_editor_contract --test skill_tui_controller_contract --test skill_tui_host_contract
```

Result: exit 0, 11 passed, 0 failed.

- `skill_editor_contract`: 5 passed.
- `skill_tui_controller_contract`: 5 passed.
- `skill_tui_host_contract`: 1 passed.

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

## Concerns

- Rendering and visual discoverability are intentionally deferred to Task 9.
- Only the three authorized focused test targets were executed; broader integration remains outside this task's verification scope.
