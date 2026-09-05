# Phase 2 Agent Profile Foundation Testing Guide

This guide is the release and local acceptance procedure for Agent Profiles
Milestone 1. Run it only with isolated application state. It covers the release
binary, Adaptive Cockpit, fallback command mode, immutable history, recovery,
terminal restoration, and the single-instance guard. It does not test or claim
skills, hybrid memory, provider execution, rooms, debates, or market data.

## What is accepted

- A pinned Bull, Bear, Chief, Engineering, or Custom template can be copied into
  a local draft and explicitly activated as immutable version 1.
- An unbound profile is valid and displays `Not Ready`.
- A profile with both local provider and model labels displays `Ready`.
- Edit preview is local-only and passive. It writes no event, receipt, profile
  row, active pointer, persistent draft, or generic audit entry.
- Activation is separate from preview and requires explicit confirmation.
- Every activation creates the next immutable version; earlier bytes remain
  unchanged and history inspection never activates a historical version.
- Restart reproduces the active profile and complete history from verified
  events and the immutable version mirror.
- Generic audit and error output omits personality, instructions, provider
  material, and rejected hostile text.

`Ready` does not mean a provider was contacted or a model can run. Milestone 1
has no provider adapter, secret input, model execution, automatic fallback, or
agent process. Binding values are local labels only and must not contain keys or
credentials.

## Release gates

From the repository worktree, run the exact gates:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
```

Do not proceed to live smoke if a gate fails.

## Create isolated state

Build before changing `HOME` or `XDG_DATA_HOME` so Cargo and rustup keep their
normal configuration. On macOS or Linux:

```sh
profile_smoke_root="$(mktemp -d)"
mkdir -p "$profile_smoke_root/home" "$profile_smoke_root/xdg-data"
export HOME="$profile_smoke_root/home"
export XDG_DATA_HOME="$profile_smoke_root/xdg-data"
```

Keep the shell open for all TUI, fallback, restart, and second-instance checks.
The generated state is under the temporary home or XDG data directory, never
the normal user state directory. Record the exact temporary root in local test
evidence, but do not commit its database or lock file.

## Adaptive Cockpit flow

Record terminal type and starting geometry, then launch:

```sh
printf 'TERM=%s\n' "$TERM"
stty size
terminal_before="$(stty -g)"
target/release/ai-stock-forum
```

Perform this exact flow in the cockpit:

1. Press `a`. Confirm Agents opens. Press `1`, `2`, `3`, and `4` in turn, then
   press `a` again. The numeric views must remain Overview, Setup, Audit, and
   Help; `a` must open Agents only when command entry does not own input.
2. Press `c` to copy the selected pinned template. Keep bindings empty, rename
   the draft `Research North`, walk every guided step, and inspect the Review
   screen. No durable profile exists before confirmation.
3. Enter `:activate`, then press `Enter` on `Confirm Create`. The new active
   profile must show version 1 and `Not Ready`.
4. Select `Research North`, open detail with `Enter`, and press `e`.
5. Change primary specialty, specialty tags, personality, instructions, provider
   label, and model label. Enter `:review` and inspect every ordered Before/After
   field diff.
6. Enter `:activate`, then press `Esc` at `Confirm Activate`. Confirm the editor
   returns to the unchanged review and history still has only version 1.
7. Enter `:activate` again and press `Enter`. Confirm detail shows version 2 and
   `Ready`.
8. Press `h`. Confirm history is newest-first, version 2 supersedes version 1,
   and inspection does not move the active pointer.
9. Resize to at least `70x24`, `100x30`, and `140x40`. Confirm the Agents view
   respectively uses one-pane narrow, two-pane medium, and three-pane wide
   presentation without losing selection, editor/detail state, or history.
10. Return outside editor/confirmation and press `q`. Confirm normal success
    exit and shell usability.

If the terminal harness cannot resize, record exactly that limitation. Do not
claim those resize actions; run the automated ratatui narrow/medium/wide
acceptance path and manually exercise every other supported action.

## Terminal restoration

Immediately after normal quit, compare modes and restore only if the comparison
fails:

```sh
terminal_after="$(stty -g)"
test "$terminal_before" = "$terminal_after"
printf 'shell input restored\n'
```

The terminal must leave the alternate screen, show the cursor, disable bracketed
paste if the application enabled it, and restore canonical/echo terminal modes.
Bounded PTY evidence should record the matching enter/leave control pairs and
the exact before/after terminal-mode comparison. Never include profile prose or
environment secrets in raw terminal evidence.

## Fallback read and history flow

Use the same isolated environment after the TUI exits:

```sh
target/release/ai-stock-forum --command-mode
```

At the prompt run:

```text
agent list
agent show <profile-id-from-agent-list>
agent history <profile-id-from-agent-list>
/quit
```

`agent list` must show `Research North`, version 2, and `ready`. `agent show`
must display accepted fields and immutable metadata through typed labels.
`agent history` is the supported way to inspect version history without
exposing raw SQLite `payload_json`; it must show versions 2 and 1 newest-first.
Do not use a database browser or dump raw profile rows for routine inspection.

For a complete fallback-only create/edit rehearsal, use `agent create
builtin.bull`, the same guided colon controls, `n` or `no` to decline once, and
`y` or `yes` to confirm. EOF and `:cancel` must discard local draft/review state.

## Restart and recovery

Launch the release binary again with the same isolated environment. Confirm
`Research North` remains active at version 2 with both immutable history rows.
Preview state must not survive restart; begin a fresh edit preview before any
later activation.

Recovery fails closed for suspicious immutable history:

- a missing expected immutable row may be backfilled from its verified event;
- an existing byte-equivalent immutable row is retained;
- an altered immutable row causes safe startup refusal;
- an unexpected immutable row with no verified event causes safe startup
  refusal; and
- recovery never updates or deletes suspicious immutable history to make
  startup succeed.

Only the active pointer may be transactionally rebuilt after immutable-history
reconciliation succeeds. Corruption experiments belong in automated temporary
database tests, not a user's normal state directory.

## Second-instance guard

With the first release binary still open on the isolated state, use a second
terminal with the same `HOME` and `XDG_DATA_HOME` and run:

```sh
target/release/ai-stock-forum --command-mode
```

The second process must exit nonzero with the safe already-running startup
category. It must not expose a path or profile field and must not disturb the
first process. Quit the first process normally afterward.

## Evidence record

Store only bounded local evidence in the task's SDD workspace. Record:

- host operating system, terminal/harness, release binary path, and isolated
  state arrangement;
- release-gate commands, exit codes, and exact pass/fail summaries;
- create, decline, confirm, detail, history, restart, fallback, and
  second-instance outcomes;
- every automated resize and any exact harness limitation;
- alternate-screen, cursor, bracketed-paste, and terminal-mode restoration
  evidence; and
- concerns without converting an unperformed action into a claim.

After inspection, remove only the recorded temporary root using an interactive
or otherwise explicitly targeted operation. Never target a normal application
directory.
