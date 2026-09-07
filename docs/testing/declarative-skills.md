# Declarative Skills Testing and Workflow Guide

Declarative Skills Milestone 2 provides a local library of reusable guidance
for agent profiles. A skill is inert, bounded context: accepted text describing
what to consider and how to work. It cannot execute code or commands, and it
cannot grant or use shell, filesystem, Git, MCP, provider, browser, or network
capability. Text that looks like a command, path, URL, or tool name remains
text. Inference and chat begin in Phase 3; this milestone does not send skill
content to a model.

## Library and version model

Every installation starts with four deterministic built-ins at version 1:

- Evidence Review
- Filing Analysis
- Catalyst Mapping
- Risk Checklist

Built-in and custom skills follow the same assignment rules. Creating a skill
accepts immutable version 1. Editing accepted content creates the next
immutable version and leaves every earlier version available in History. An
agent assignment stores an exact version identity and digest, not merely a
skill name or its current pointer.

That exact pin is deliberate. Creating version 2 does not auto-upgrade an agent
pinned to version 1. Use an explicit upgrade to move that agent to version 2,
or open a historical version and deliberately assign it when older guidance is
the right choice. Unassign removes the pin by creating a new immutable agent
profile version; it does not remove any skill history.

## Keyboard-first workflow

Launch the Adaptive Cockpit and Press `Option+6` on macOS or `Alt+6` on Windows and Linux to open Skills. Controls are pane-specific: `Up` and `Down` select
vertical rows or items, while `Left` and `Right` select horizontal actions.
`Enter` opens or accepts the visible action, and `Esc` returns or cancels. The
footer shows the controls available in the current state. You never need `:next` or `:create`
for this workflow.

`Option/Alt+1` through `Option/Alt+6` remain available in every Skills pane,
including editors and confirmations. Returning to Skills restores the exact
pane, selection, draft input, and pending confirmation; switching alone never
submits or cancels an action.

### Pane controls

| Pane | Selection | `Enter` | `Esc` |
| --- | --- | --- | --- |
| Library | `Up/Down` selects skill rows. | Opens the selected exact active version. | Returns to the originating workspace. |
| Create source | `Up/Down` selects a starting point: blank or a built-in. | Opens the editor with that starting point. | Returns to Library. |
| Editor | Typing edits the field; `Left/Right` moves the text cursor. | Accepts the field or requests Review validation at the final step. | Returns to the prior field or cancels from the first step. |
| Detail actions | `Left/Right` selects the Assign, Create Version, or History action. | Opens the selected action. | Returns to Library. |
| History | `Up/Down` selects immutable version rows. | Opens the exact read-only version. | Returns to Detail. |
| Agent picker | `Up/Down` selects agent rows. | Opens Review for that agent and exact skill version. | Returns to Detail. |
| Agent assigned skills | `Up/Down` selects pinned skill rows; `Left/Right` selects available View, Upgrade, or Unassign actions; `r` reloads active skill data. | Opens the selected action or its Review. | Closes the assigned-skills panel. |
| Review | No selection changes. | `Enter` validates the displayed operation and opens Confirmation. | Returns to the originating picker, detail, or editor. |
| Confirmation | No selection changes. | `Enter` commits only the displayed reviewed operation. | Cancels confirmation and returns without mutation. |
| Result | No selection changes. | Returns to Detail. | Returns to Detail. |

### Create a custom skill

1. Press `c` in the Skills library.
2. Type the identity, usage guidance, tags, instructions, and optional reference
   notes. Use `Enter` to accept each visible step and `Esc` to go back without
   losing earlier fields.
3. Inspect the complete candidate on Review. `Enter` advances to confirmation;
   `Esc` returns to editing.
4. Confirm with `Enter`. Only confirmation accepts immutable version 1.

### Create and inspect versions

1. Select a skill row with `Up` and `Down`, then open it with `Enter`.
2. Select Create Version with `Left` and `Right`, then press `Enter`.
3. Edit the copied active content, review it, and confirm with `Enter`. Unchanged
   content is rejected rather than creating a duplicate immutable version.
4. Select History, choose a historical version with `Up` and `Down`, and press
   `Enter` to inspect its exact read-only content.

### Assign, deliberately use history, upgrade, and unassign

1. On skill detail or a historical version, select Assign with `Left` and
   `Right`, then press `Enter`.
2. Choose an agent with `Up` and `Down`; `Enter` opens a review naming the
   agent, operation, and exact version. A second `Enter` opens confirmation and
   confirmation `Enter` commits the new immutable agent profile version.
3. To upgrade, open Agents with `Option+5` on macOS or `Alt+5` on Windows and
   Linux. In assigned skills, use `Up` and `Down` to
   select the pinned skill row and `Left` and `Right` to choose Upgrade, then
   review and confirm the exact replacement version. A newer active skill alone
   never changes the pin.
4. To unassign, use `Left` and `Right` to choose Unassign from the same
   assigned-skill row, then review and confirm. `Esc` backs out at every review
   or confirmation step without applying the mutation.

Assigning a historical version is intentionally the same reviewed operation as
assigning the active version. The review must identify that exact version so
the choice cannot be mistaken for an upgrade to the latest content.

## Confirmation, cancellation, and recovery

Create, version, assign, explicit upgrade, and unassign all cross a separate
review and confirmation boundary. No mutation occurs merely by opening an
editor, picker, or review. `Esc` cancels the pending step and retains safe draft
state where the interface offers a return to editing.

If validation is rejected, correct the retained draft and request a fresh
review. If a review becomes stale because the active skill or agent profile
changed, confirmation fails without overwriting the newer state. Dismiss the
message with `Esc`, reload the current detail, and stage a fresh review. An
uncommitted review is process-local and should be started again after restart.

Accepted skill versions, active pointers, exact agent assignments, resulting
agent profile versions, and audit entries persist across restart. Restart with
the same isolated state directory to verify them; drafts and review tokens are
not durable state.

## Optional slash fallbacks

The command bar and line-command host offer these optional forms:

```text
/skill list
/skills
/skill add
/skill show <name-or-id> [version]
/skill assign <skill> <agent> [version]
/skill unassign <skill> <agent>
```

`/skill list`, `/skills`, and `/skill show` are reads. `/skill add` opens the
guided creator. `/skill assign` stages a review for assign or upgrade, and
`/skill unassign` stages a review; each mutation command does not mutate directly.
When assignment omits `[version]`, its review displays and pins the
exact active version resolved when the flow starts. The fallback host may ask
for the exact confirmation phrase shown on screen, but the primary cockpit
workflow remains keyboard-first.

Bare `q` is inert and neither confirms nor quits. `/quit` exits through the
normal audited shutdown path.

## Compact terminal expectations

The cockpit supports terminals from `60x18`. A compact terminal shows one
focused pane at a time; medium and wide terminals progressively add context
without adding required controls. The operation, distinguishing exact-version
identity, validation or recovery guidance, and relevant `Enter` and `Esc`
actions stay visible while long instructions or reference notes may be
truncated with a disclosure. Below `60x18`, the Too Small screen retains `/quit`
and keeps bare `q` inert.

## Exact local commands

From the repository worktree, run the documentation and topology checkpoint:

```sh
cargo test --test documentation_contract --test topology_contract
```

Build before isolating application state so Cargo and rustup continue to use
the normal development environment:

```sh
cargo build --release --locked
skills_smoke_root="$(mktemp -d)"
mkdir -p "$skills_smoke_root/home" "$skills_smoke_root/xdg-data"
HOME="$skills_smoke_root/home" XDG_DATA_HOME="$skills_smoke_root/xdg-data" target/release/ai-stock-forum
```

For the restart check, exit with `/quit` and run the final launch command again
with the same `skills_smoke_root`. Do not point acceptance work at normal user
state. The release orchestrator owns the fresh full-suite run and the final
manual TUI evidence for this milestone.

## Manual acceptance checklist

- Press `Option+6` on macOS or `Alt+6` on Windows and Linux; inspect all four
  built-ins and their version 1 provenance.
- Create a custom skill using typing, arrows, `Enter`, and `Esc` only.
- Assign custom version 1 to an agent and confirm the displayed exact pin.
- Create version 2 and confirm the existing assignment remains on version 1.
- Explicitly upgrade the agent to version 2.
- Open History and deliberately reassign version 1.
- Unassign, cancel one review, and confirm cancellation writes nothing.
- Exercise one rejected edit and one stale review; recover through a fresh
  review without losing or overwriting accepted state.
- Restart with the same isolated state and confirm versions, active pointers,
  assignments, agent profile history, and audit entries persist.
- At a compact terminal size, confirm controls remain visible; verify bare `q`
  is inert and `/quit` exits normally.
