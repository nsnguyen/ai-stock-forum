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
- Typed binding references can only be selected from an application catalog.
  Injected catalog tests distinguish `Unbound`, `Binding unavailable`, and
  `Ready`; the production Milestone 1 catalog is empty.
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
agent process. Binding controls never accept free-form IDs, keys, or labels.

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

1. Press bare `a` and confirm Agents opens. Use bare `1`, `2`, `3`, and `4` in
   turn, then return with bare `a`. Each shortcut must work from every non-text
   browsing pane and confirmation, and Agents must return to the same pane and
   selection. While command, profile, or skill text entry owns input, these
   characters remain text; switching preserves tab state, drafts, and pending
   confirmations.
2. Press `c` to copy the selected pinned template. Keep bindings empty, rename
   the draft `Research North`, walk every guided step, and inspect the Review
   screen. No durable profile exists before confirmation.
3. Enter `:create`, then type the exact phrase `create` at `Confirm Create` and
   press `Enter`. The new active profile must show version 1 and `Not Ready`.
4. Select `Research North`, open detail with `Enter`, and press `e`.
5. Change primary specialty, specialty tags, personality, and instructions.
   Leave the unavailable production bindings unbound. Enter `:review` and
   inspect every ordered Before/After field diff.
6. Enter `:activate`, then press `Esc` at `Confirm Activate`. Confirm the editor
   returns to the unchanged review and history still has only version 1.
7. Enter `:activate` again, then type the exact displayed `activate
   <review-digest>` phrase and press `Enter`. Confirm detail shows version 2 and
   remains honestly `Not Ready`.
8. Press `h`. Confirm history is newest-first, then select version 1 and inspect
   its complete accepted content, immutable metadata, and predecessor diff.
   Inspection must not move the active pointer.
9. Resize to at least `70x24`, `100x30`, and `140x40`. Confirm the Agents view
   respectively uses one-pane narrow, two-pane medium, and three-pane wide
   presentation without losing selection, editor/detail state, or history.
10. Resize below `60x18`; press `Esc` and confirm the app remains open, then
    press `q` and confirm unconditional normal quit from the Too Small view.
11. Confirm shell usability after exit.

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
/agent list
/agent show <name-or-id-from-agent-list>
/agent history <name-or-id-from-agent-list>
/agent history <name-or-id-from-agent-list> 1
/quit
```

`/agent list` must show `Research North`, version 2, and `not ready`. `/agent
show` must display accepted fields and immutable metadata through typed labels.
The history commands are the supported way to inspect version history without
exposing raw SQLite `payload_json`; the list must show versions 2 and 1
newest-first, while the exact-version form must show version 1 content and its
predecessor diff.
Do not use a database browser or dump raw profile rows for routine inspection.

For a complete fallback-only create/edit rehearsal, use `/agent create bull`
and the same guided colon controls. Creation uses `:create` followed by exact
`create`; editing uses `:activate` followed by exact `activate
<review-digest>`. Cause one recoverable duplicate-name or transient submission
error and verify the exact draft, review, and confirmation remain available for
retry. EOF and `:cancel` discard local draft/review state.

Oversized and invalid UTF-8 command lines must produce the authoritative typed
`RejectInput` result, with bounded metadata, and must not be pre-dispatch local
substitutes. List and history outcomes, events, and receipt payloads are capped
at 100 rows before durable metadata is constructed.

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

Migration `0002_agent_profiles.sql` is amendable while Phase 2 is prerelease.
The automated contract proves a real schema-v1 database upgrades without
altering legacy rows and proves rollback at each migration boundary. A database
created by an intermediate Phase 2 development build is not an upgrade fixture
and must be recreated.

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
