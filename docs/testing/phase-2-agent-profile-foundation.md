# Phase 2 Agent Profile Foundation specialized acceptance guide

This is the specialized historical persistence and recovery acceptance
procedure for Agent Profiles Milestone 1. It covers the release binary,
fallback command mode, immutable history, recovery, terminal restoration, and
the single-instance guard. Run it only with deliberately isolated application
state. It does not test or claim skills, hybrid memory, provider execution,
rooms, debates, or market data.

For a normal check of the current two-pane TUI, quit any other running instance
and run `make dev` in the worktree. That normal path uses existing local app
data: it needs no new macOS account or home-directory reassignment. Follow the
[two-pane shell and Agents guide](two-pane-shell-agents.md) for the current
navigation and usability checklist. Do not use normal state for the specialized
corruption, migration, recovery, or destructive acceptance work retained below.

The older isolated-state instructions in this document remain as a historical
acceptance reference for the persistence invariants. The two-pane Agents
controls are present and their automated gates pass, but the interactive
`make dev` acceptance run has not been performed; do not treat this guide as a
claim that the new interactive behavior has passed.

## What is accepted

- A pinned Bull, Bear, Chief, Engineering, or Custom template can be copied into
  a local draft and explicitly activated as immutable version 1.
- An unbound profile is valid. The two-pane TUI labels it `Needs connection`;
  historical and fallback output may use `Not Ready`.
- Typed binding references can only be selected from an application catalog.
  Injected catalog tests distinguish `Unbound`, `Binding unavailable`, and
  `Ready`; the two-pane TUI renders those states as `Needs connection`,
  `Connection unavailable`, and `Bindings configured`. The production
  Milestone 1 catalog is empty.
- Edit preview is local-only and passive. It writes no event, receipt, profile
  row, active pointer, persistent draft, or generic audit entry.
- Activation is separate from preview and requires explicit confirmation.
- Every activation creates the next immutable version; earlier bytes remain
  unchanged and history inspection never activates a historical version.
- Restart reproduces the active profile and complete history from verified
  events and the immutable version mirror.
- Generic audit and error output omits personality, instructions, provider
  material, and rejected hostile text.

`Bindings configured` does not mean a provider was contacted or a model can
run. Milestone 1
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

## Historical isolated-state procedure

This procedure is not the normal usability path. It is retained for a dedicated
disposable acceptance environment and must never target existing local app
data. Build before changing `HOME` or `XDG_DATA_HOME` so Cargo and rustup keep
their normal configuration. On macOS or Linux:

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

## Specialized isolated TUI persistence flow

Record terminal type and starting geometry, then launch:

```sh
printf 'TERM=%s\n' "$TERM"
stty size
terminal_before="$(stty -g)"
target/release/ai-stock-forum
```

Use the current two-pane navigation while exercising the isolated persistence
state. Its global destinations contain
exactly these ordered labels:

```text
1 Home
2 Chat
3 Agents
4 Skills
5 Connections
6 Activity
7 Setup
8 Audit
9 Help
```

There is no dedicated Memory destination; Memory is nested under Agents.
Modified shortcuts are inert. In command, profile, skill, or Memory text
editors, typed shortcut characters remain editor text. Bare `q` is inert and
`/quit` requests normal shutdown. `?` is a Help alias but is not a destination
label.

Perform this flow only as a deliberate specialized manual acceptance run; it
has not been performed for the current two-pane slice:

1. Press bare `3` and confirm Agents opens. Choose an agent with `W`/`S`, press
   `Tab` to focus its workspace, use `A`/`D` among Profile, Memory, Skills, and
   History, and press `Enter` to open the highlighted choice. No UUID lookup is
   part of this path. Use bare `1` through `9` in turn, use bare `4` to open
   Skills, then return with bare `3`. Each shortcut must work from every
   non-text browsing pane and confirmation, and Agents must return to the same
   pane and selection.
2. Press `N` for the visible New action and copy the selected pinned template.
   Keep bindings empty and rename the draft `Research North`. Profile fields
   begin in NAV; select a field and press `Enter` deliberately to enter TYPE.
   In Display name, type `wasd123456789/n` and a leading `:back`, then verify
   these remain literal text. `Esc` must retain text and return to NAV; `Tab`
   must retain text and move to the next field in NAV. Invalid raw input and its
   error must remain available for correction. Profile normalization remains
   single-line, and the fallback colon grammar is not a TUI prose parser.
   Restore Display name to `Research North` before continuing. Inspect Review
   and confirm that no durable profile exists yet.
3. Move from Review to the separate Confirm Create state, then deliberately
   press `Enter` on the labeled confirmation. `D` or a held/repeated `Enter`
   must not cross the boundary and write. The new active profile must show
   version 1 and `Needs connection` in the TUI.
4. Select `Research North`, open Profile, and press `E` for the visible Edit
   action.
5. Change primary specialty, specialty tags, personality, and instructions.
   Leave unavailable production bindings unbound. Suspend and resume the draft
   once by leaving Agents; only the labeled Discard action may abandon it. Move
   through the fields, then inspect the authoritative ordered Before/After
   preview on Review.
6. Press `Enter` on the authoritative Review to open Confirm Activate, then
   press `Esc`. Confirm the editor returns to the unchanged review and history
   still has only version 1.
7. Open Confirm Activate again and deliberately press `Enter` on its labeled
   action. Confirm detail shows version 2 and remains honestly `Needs
   connection` in the TUI.
8. Press `H`. Confirm history is newest-first with readable version and date
   labels, then select version 1 and inspect its accepted content and readable
   field changes from its predecessor. Inspection must not move the active
   pointer.
9. Resize to at least `60x18`, `80x24`, `100x24`, `120x30`, and `160x40`.
   Confirm widths from 60 through 99 show one logical pane, while widths from
   100 show both list and workspace. Confirm the Agents view changes layout
   without losing selection, editor/detail state, or history.
10. Resize below `60x18`; press `Esc` and confirm the app remains open, then
    press bare `q` and confirm it is inert. Open command entry with `/`, type
    `quit`, and press `Enter` to request normal shutdown with `/quit`.
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

Profile mutation acceptance is exercised through the keyboard-first cockpit
above and the automated host-parity contracts. In fallback mode, follow the
rendered guided prompts and exact displayed confirmation action rather than
assuming a TUI shortcut. Cause one recoverable duplicate-name or transient
submission error and verify the exact draft, review, and confirmation remain
available for retry. EOF discards local draft/review state.

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
