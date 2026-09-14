# Declarative Skills: local testing

Declarative Skills are reusable inert, bounded context for agent profiles. A
skill can describe what to consider and how to work, but it cannot execute
commands or grant shell, filesystem, Git, MCP, provider, browser, or network
capability. Command-like text, paths, URLs, and reference notes remain saved
text. Inference and chat begin in Phase 3.

> **Verification status:** The keyboard contracts and synthetic
> production-render previews cover the redesigned Skills workflow. An
> interactive `make dev` run is still a separate manual check and is not
> claimed by this guide.

## Library and version model

Every installation starts with Evidence Review, Filing Analysis, Catalyst Mapping,
and Risk Checklist. A new skill accepts immutable version 1. Editing
accepted content proposes the next immutable version while History keeps every
earlier historical version.

An assignment stores an exact version identity and digest. Creating a newer
version does not auto-upgrade an existing pin. Move a pin only through an
explicit upgrade or deliberate historical reassignment. Unassign creates a new
immutable agent profile version without deleting skill history.

## Normal testing path

Quit any other running AI Stock Forum instance, then launch from the repository
root:

```sh
make dev
```

This is the normal local launch. It does not require a temporary macOS account
or a changed `HOME`. It uses the application's existing local data, so inspect
and cancel freely but do not confirm create, edit, assignment, upgrade,
reassignment, or unassignment merely for a visual check. Do not run corruption,
recovery, migration, or destructive persistence experiments against normal user
data.

## Keyboard-first workflow

Launch the Adaptive Cockpit and press bare `4` to open Skills. The same
navigation works in one-pane compact layouts and wider two-pane layouts.
Bare `1`-`9` navigate from non-text browsing panes. During TYPE, characters remain text.
Returning to Skills restores the retained pane, draft, and pending confirmation.
You never need `:next` or `:create` for this workflow.

| Key | NAV behavior |
| --- | --- |
| `Tab` / `Shift+Tab` | Move to the next / previous enabled pane or section. |
| `W` / `S` | Move up / down in the focused pane. |
| `A` / `D` | Move left / right through visible actions or cards. |
| `Enter` | Open the selected item or action. On a text field, explicitly enter TYPE. |
| `Esc` | Retain safe draft text and return, or cancel the visible review or confirmation. |
| `N` | Start a new skill, or resume the retained draft if one already exists. |
| `E` | Edit the active skill by starting its next immutable version, or resume its retained draft. |

Arrow keys are quiet NAV aliases. While TYPE owns a field, letters, digits,
slashes, colons, and spaces are text rather than navigation or commands. `Esc`
retains the exact field input and returns to NAV on that field. `Tab` also
retains the field before moving panes. The footer names the actions available
to the current owner.

The library begins with four deterministic built-ins: Evidence Review, Filing
Analysis, Catalyst Mapping, and Risk Checklist. Built-in and custom skills obey
the same immutable version and assignment rules.

Library highlights update immediately while the selected skill's preview loads
in the background. When you browse quickly, only the latest waiting selection
is loaded; an older reply cannot move the selection or replace another page.
The same behavior applies to the blank/copy starter picker. Returning to Skills
retries a missing preview that finished while you were away. Opening or saving
still follows the existing pending-command and explicit confirmation guards.

To check responsiveness, hold or tap W/S in the library and starter picker, then
switch to Home and back with `1` and `4`. Selection should keep moving even if
the preview takes longer. These are navigation-only checks; no save is needed.

### Pane controls

| Pane | Selection | `Enter` | `Esc` |
| --- | --- | --- | --- |
| Library | `W/S` (`Up/Down`) selects skill rows. | Opens the selected exact active version. | Returns to the originating workspace. |
| Create source | `W/S` (`Up/Down`) selects a starting point. | Opens the editor with that starting point. | Returns to Library. |
| Editor Home | WASD selects section cards and actions. | Opens the selected section or action. | Returns to Skill Home with the draft retained. |
| Editor section | WASD selects fields. | Explicitly enters TYPE on the selected field. | Returns to Editor Home with field text retained. |
| Detail actions | `Left/Right` selects an action; `W/S` scrolls content. | Opens the selected action. | Returns to Library. |
| History | `W/S` (`Up/Down`) selects version rows. | Opens the exact read-only version. | Returns to Detail. |
| Agent picker | `W/S` (`Up/Down`) selects agent rows. | Opens Review for that agent and exact skill version. | Returns to Detail. |
| Agent assigned skills | `W/S` (`Up/Down`) selects pinned skill rows; `Left/Right` selects actions. | Opens View or the selected mutation Review. | Closes the assigned-skills panel. |
| Review | No selection changes. | `Enter` validates the displayed operation and opens Confirmation. | Returns to the originating picker, detail, or editor. |
| Confirmation | No selection changes. | `Enter` commits only the displayed reviewed operation. | Returns without mutation. |

## Skill Home and starting points

Select a library row with `W`/`S`, then press `Enter` to open Skill Home. Home
shows the saved purpose, when-to-use guidance, instructions, reference notes,
provenance, and human-readable version. Use `A`/`D` to select Assign to agent,
Edit skill, or Version history, then press `Enter` to open that action. Routine
Home does not need raw UUIDs or digests; technical details are a separate
disclosure. `W`/`S` scrolls long guidance while keeping the selected action
available. Press `I` to show or hide technical identifiers on Home and on the
Review and Confirmation screens.

Press `N` to open the separate Starting Point picker. Choose Blank skill or a
copy of a visible library skill with `W`/`S`. The preview must match the exact
selected starting point. `Enter` continues into the editor and `Esc` returns to
the library without creating anything.

## Create or edit a skill

The editor opens on Skill Editor Home. Its independent destinations are:

- Basics: display name and purpose.
- When to use: use guidance and tags.
- Instructions: inert working guidance.
- Reference notes: optional inert name/body pairs.
- Review changes.
- Discard draft.

Use WASD to select a section or action and `Enter` to open it. In a section,
select a field in NAV and press `Enter` deliberately to enter TYPE. `Esc` leaves
TYPE while retaining the exact raw text; another `Esc` returns to Editor Home
with the draft intact. Invalid raw input and its inline correction guidance
remain available after leaving and returning to a field.

Reference notes are saved text, never executable resources. Add or select a
note from the Reference notes section, edit its name and body through explicit
TYPE, then choose the visible save action. Removing a note has its own warning;
`Esc` keeps the note.

Leaving Skills suspends the draft. Returning and pressing `N` or `E` resumes it
instead of silently replacing it. Discard opens a separate warning, and only a
deliberate confirmation abandons the retained draft.

## Confirmation, cancellation, and recovery

Review shows the complete candidate and the exact operation. For a new skill,
the target is immutable version 1. Editing an active skill proposes the next
immutable version; it never overwrites accepted content. `Enter` on Review
validates the candidate and advances to a separate Confirmation. Only `Enter`
on that labeled confirmation performs the write. `Esc` returns without a
mutation and retains the draft where editing can resume.

Unchanged content is rejected instead of producing a duplicate version. A
stale review also fails without overwriting newer state. Reload the current
skill, inspect the new state, and stage a fresh review. A rejected candidate
retains its draft for correction, and `Esc` can cancel without mutation.
Accepted versions and assignments persist
across restart; unfinished drafts and process-local reviews should not be
treated as durable recovery records.

## History and exact assignments

History lists immutable versions as ACTIVE or HISTORICAL. Use `W`/`S` to select
a row and `Enter` to open that exact read-only version. Creating a newer active
version never changes an existing agent assignment.

Assignment always pins an exact skill version and digest into a new immutable
agent profile version:

1. Choose Assign to agent from active or historical Skill Home.
2. Select an agent with `W`/`S`, then press `Enter` to open Review.
3. Verify the agent, operation, and human-readable exact version.
4. Press `Enter` to validate, then press `Enter` again only on the separate
   labeled Confirmation to commit.

In an agent's assigned Skills panel, select a pin with `W`/`S` and choose View,
Upgrade, or Unassign with `A`/`D`. Upgrade is explicit; it replaces the displayed
exact pin only after review and confirmation. Assigning an older historical
version is an explicit reassignment, not an upgrade. Unassign removes only the
displayed exact pin by creating another immutable agent profile version.

## Optional slash fallbacks

The optional command input and line-command host support only these Skills
forms:

```text
/skill list
/skills
/skill add
/skill show <name-or-id> [version]
/skill assign <skill> <agent> [version]
/skill unassign <skill> <agent>
```

Each mutation command stages a review and does not mutate directly. Bare `q` is inert;
`/quit` exits through the normal shutdown path.

## Compact terminal expectations

A compact terminal from `60x18` shows one focused pane at a time. `Tab` reveals
the next pane; W/S keeps selected rows or cards visible. Operation identity,
validation guidance, and the relevant `Enter` and `Esc` actions remain visible.
Below the supported size, resize guidance keeps bare `q` inert and `/quit`
available.

## Safe synthetic production-render preview

The preview uses synthetic fixtures, ratatui's test backend, and the real
production renderer. It does not open or mutate application data:

```sh
cargo run --locked --example two_pane_preview -- --scene skill-home --width 120 --height 30
cargo run --locked --example two_pane_preview -- --scene skill-editor --width 160 --height 40 --svg
```

Skills scenes are `skill-home`, `skill-editor`, `skill-starter`,
`skill-section`, `skill-type`, `skill-history`, and `skill-assignment`. Add
`--no-color` to inspect the color-independent presentation. Review representative
scenes at `60x18`, `80x24`, `100x24`, `120x30`, and `160x40`.

Run the focused automated checks with:

```sh
cargo test --locked --example two_pane_preview
cargo test --locked --test skill_home_render_contract --test skill_tui_render_contract
cargo test --locked --test skill_home_navigation_contract --test skill_tui_controller_contract
```

## Exact local commands

Run the documentation checkpoint and build in the normal development
environment before an interactive pass:

```sh
cargo test --test documentation_contract --test topology_contract
cargo build --release --locked
make dev
```

Do not create a temporary macOS account, override `HOME`, or redirect
`XDG_DATA_HOME` for the normal path. Because `make dev` opens existing local
application data, browse and cancel rather than confirming test mutations.

## Manual acceptance checklist

- Open all four built-ins and verify inert guidance and provenance.
- Start and cancel Create a custom skill; confirm retained TYPE text.
- Review the path that would accept immutable version 1 without committing it.
- Inspect History and the path that would Create version 2.
- Inspect the exact-pin review for assign, Explicitly upgrade, historical
  reassignment, and Unassign.
- Cancel Review and Confirmation and verify no visible state mutation.
- Inspect rejected input and stale review recovery.
- Restart only when validating deliberately accepted non-user test state.

For an interactive visual pass on normal local data, use `make dev`, browse the
same paths, and cancel before the final confirmation. Use the synthetic preview
for reproducible screenshots and mutation-free evidence.
