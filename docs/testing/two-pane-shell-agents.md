# Two-pane shell and Agents: local testing

This guide covers the approved first slice of the two-pane redesign: the shared
shell and the complete Agents experience. Memory and Skills use the shared
navigation while retaining their existing workflows until their own usability
passes. Chat and Connections remain honest Phase 3 placeholders.

> **Verification status:** The controls below are present in the current TUI,
> and the automated gates and production-render preview matrix pass. The
> interactive `make dev` acceptance run has not been performed, so this guide
> does not claim that interactive acceptance has already passed.

## Normal testing path

Quit any other running AI Stock Forum instance. From the repository root,
run:

```sh
make dev
```

This is the normal local launch and uses the existing local application data.
It does not require a new macOS account or a changed `HOME`. Browsing and
suspending a draft do not activate it, but a deliberately confirmed create or
edit changes that local data.

Do not perform corruption, recovery, migration, or destructive persistence
experiments on normal state. Those specialized historical checks belong in the
disposable-state procedure in the
[Agent Profile Foundation guide](phase-2-agent-profile-foundation.md).

## Find your way around

The top menu is always ordered as follows:

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

| Key | NAV behavior |
| --- | --- |
| `1`–`9` | Open the matching top-menu destination. |
| `Tab` / `Shift+Tab` | Move to the next / previous enabled logical section. |
| `W` / `S` | Move up / down in the focused section. |
| `A` / `D` | Move left / right in the focused section. |
| `Enter` | Open the highlighted item or activate the selected action; on the labeled final confirmation, a deliberate press performs the write. |
| `Esc` | Go back or cancel according to the current contextual hint. |
| `/` | Open the optional command input when no pending workflow owns it. |
| `/quit`, then `Enter` | Request normal shutdown from command input. |

WASD is the primary NAV vocabulary. Arrow keys are quiet aliases outside text
entry. The footer shows the controls for the current context. Command input and
the transitional Memory and Skills editors use their own actual contextual
hints rather than the generic Agents profile hints.

At widths from 60 through 99 columns, one logical pane is visible and focus
reveals the relevant pane. At 100 columns and above, the list and workspace are
both visible. `Tab` and `Shift+Tab` traverse the same enabled-section order in
either layout. The supported minimum is `60x18`; below it the app preserves
state and shows resize guidance with safe exit available.

## Agents path

Home now offers three destination cards: Agents, Skills, and Setup. Use
`A`/`D` (or `W`/`S`) to choose a card, then `Enter` to open it. `Tab`
moves between the cards and the top menu. First-run guidance uses real local
state; it does not display raw audit summaries or invent agents.

Agent lists use stable colored initial badges, a shaded selected card, and
readable connection status. The workspace uses Profile, Memory, Skills, and
History icon cards when space permits, with compact text choices at the
minimum terminal size. The artwork uses ordinary terminal characters and
does not require a special icon font. NO_COLOR keeps explicit selection
markers. Long names remain available in the scrollable profile.

Use this path without looking up a UUID:

1. Press `3` to open Agents.
2. Choose an agent with `W`/`S`. The workspace identity should update
   immediately; if details are pending, it should show a loading message for
   the newly selected agent rather than stale details from another one.
3. Press `Tab` to focus the workspace.
4. Select Memory with `A`/`D`; in a narrow vertical presentation, `W`/`S`
   performs that selection instead. The same four workspace choices are
   Profile, Memory, Skills, and History.
5. Press `Enter` to open Memory. Repeat the selection-and-open check for the
   other workspace choices as needed.

The Agents shortcuts are `N` New, `E` Edit, and `H` History whenever
the corresponding visible action is available. Profile History uses readable
version and date labels. Inspecting an older version is read-only and must not
silently make it the active edit target.

The normal profile card uses these human status labels:

| Existing readiness state | Agents label |
| --- | --- |
| Unbound | Needs connection |
| Required binding unavailable | Connection unavailable |
| Required bindings available | Bindings configured |

These labels report binding configuration only. They do not claim provider
contact, model execution, chat capability, or permission to run an agent.

## Profile editing checks

Press `N` to open the separate template picker. `W`/`S` browses the template
list, `Tab` moves to its preview, and `Enter` uses the selected template.
Profile Home then shows Identity, Focus, Personality, and Instructions cards.
`E` for an existing agent opens this Home directly, without a template selector.
Template provenance is reference-only inside an established draft.

Use WASD to select a card, `Enter` to open it, then WASD to select a field.
In Identity, moving across Role must not change it: press `Enter` to choose a
role, use WASD to select, and `Enter` or `Esc` to leave the choice control.
`Esc` from a section returns to Profile Home, retaining the exact draft.

Profile fields start in NAV. Select a text field and deliberately press `Enter`
to enter TYPE. While TYPE owns input, type a sample such as
`wasd123456789/n` and a value beginning with `:back`. WASD, digits, slash, and
the leading colon must remain literal profile text rather than navigation or
commands.

- `Esc` retains the exact field text and returns to NAV on that field.
- `Tab` retains the exact field text, leaves TYPE, and moves to the next pane;
  `Shift+Tab` moves to the previous pane. The pane cycle is main navigation,
  list, workspace. Returning to
  the workspace retains the selected field and its raw input.
- Invalid raw input and its inline error remain available for correction after
  leaving the field or switching destinations.
- Leaving Agents suspends the draft. Discard first opens a separate warning;
  `Esc` keeps the draft, and only a second deliberate `Enter` abandons it.

The footer names only actions available to its current owner. A retained draft
shows Resume instead of New/Edit. History offers Edit for the current profile;
assigned Skills and main navigation show their own controls. Shifted N/E/H work
like their lowercase NAV forms, while TYPE preserves them as text.

Profile input remains single-line and follows the existing profile
normalization rules. The unchanged fallback command-mode editor still has its
own colon-control grammar. That compatibility grammar is not applied as a TUI
profile prose parser.

Review the draft, then move to the separate confirmation step. A durable create
or activation requires a deliberate `Enter` on the labeled confirmation
action. `D` must not perform the final write, and one held or repeated `Enter`
must not cross review and confirmation to cause a durable mutation. Cancel the
confirmation if the purpose of the run is only visual or navigation testing.

## Scope boundaries

The redesign does not add credentials, provider/model execution, encryption,
or a multiline profile editor. Full Memory and Skills redesigns are the next
usability slices. Chat and Connections remain Phase 3 placeholders. Their pages
must not imply live chat, provider availability, or credential setup.

## Safe synthetic preview

The preview uses synthetic fixtures, ratatui's `TestBackend`, and the production
renderer. It does not open or change application data. Run a representative
Agents scene with separated option values:

```sh
cargo run --locked --example two_pane_preview -- --scene agents --width 120 --height 30
```

Available scenes are `home`, `home-populated`, `agents`, `empty`, `history`,
`profile-home` (also `editor`), `template-picker`, `identity`, `focus`,
`personality`, `instructions`, `type`, `invalid-field`, `review`, `confirmation`, `chat`, and
`connections`. `home` models a zero-agent first run. Its synthetic snapshot
includes the audit summary `agent profiles listed: total_count=0,
returned_count=0, truncated=false` as input so visual review can verify that
Home does not expose that internal text. `home-populated` uses the same three
synthetic profiles as the Agents preview. Add `--no-color` for the color-free
presentation or `--svg` to emit SVG. Options such as `--width 120` and
`--height 30` require separate values.

For visual review, inspect representative scenes at `60x18`, `80x24`,
`100x24`, `120x30`, and `160x40`. Check focus, pane visibility, selected
identity, wrapping, field text, errors, confirmation controls, footer hints,
and the absence of routine UUIDs or digests.

Durable production-render evidence is available as native PNG:

Profile Home and its separate template picker:

- [Profile Home at 160x40](assets/two-pane/editor-profile-home-160x40.png)
- [Profile Home at 120x30](assets/two-pane/editor-profile-home-120x30.png)
- [Compact Profile Home at 60x18](assets/two-pane/editor-profile-home-60x18.png)
- [NO_COLOR Profile Home at 80x24](assets/two-pane/editor-profile-home-80x24-no-color.png)
- [Template picker at 160x40](assets/two-pane/editor-template-picker-160x40.png)
- [Compact template picker at 60x18](assets/two-pane/editor-template-picker-60x18.png)
- [Identity section at 120x30](assets/two-pane/editor-identity-120x30.png)
- [Literal TYPE input at 120x30](assets/two-pane/editor-type-120x30.png)
- [Compact NO_COLOR validation at 60x18](assets/two-pane/editor-invalid-field-60x18-no-color.png)

At compact sizes, W/S scrolls the selected card into view; all four sections
and both review/discard actions remain reachable. In the two-column grid,
WASD follows the visible row/column and stops at edges rather than jumping
sideways. The screenshot scenes use synthetic fixtures, not saved user data.

Earlier Home and Agents visual-fidelity evidence:

- [Home at 120x30](assets/two-pane/visual-fidelity-home-120x30.png)
- [Populated Home at 120x30](assets/two-pane/visual-fidelity-home-populated-120x30.png)
- [Home at 160x40](assets/two-pane/visual-fidelity-home-160x40.png)
- [Home at 60x18](assets/two-pane/visual-fidelity-home-60x18.png)
- [NO_COLOR Home at 80x24](assets/two-pane/visual-fidelity-home-no-color-80x24.png)
- [Agents at 120x30](assets/two-pane/visual-fidelity-agents-120x30.png)
- [Empty Agents at 120x30](assets/two-pane/visual-fidelity-agents-empty-120x30.png)
- [Agents at 160x40](assets/two-pane/visual-fidelity-agents-160x40.png)
- [Agents at 60x18](assets/two-pane/visual-fidelity-agents-60x18.png)
- [NO_COLOR Agents at 80x24](assets/two-pane/visual-fidelity-agents-no-color-80x24.png)

Earlier first-slice evidence remains available for comparison:

- [Agents at 120x30](assets/two-pane/agents-120x30.png)
- [Profile TYPE at 120x30](assets/two-pane/type-120x30.png)
- [NO_COLOR Agents at 80x24](assets/two-pane/agents-no-color-80x24.png)

These files come from the synthetic preview's real production-render cells and
the native SVG-to-PNG exporter. The `--svg` command above reproducibly emits
their source format. They do not represent an interactive `make dev` run.

## Verification record

### Profile Home update

- Baseline: `cargo test --locked` — 1,293 passed, 0 failed, 1 ignored.
- Final: `cargo test --locked --no-fail-fast` — 1,308 passed, 0 failed,
  1 ignored across 102 result summaries.
- New keyboard contracts: 9 passed, including spatial grid edges, shifted
  WASD, literal input, template selection, pane focus, and separate confirmation.
- New renderer contracts: 6 passed, including compact layouts, draft summaries,
  visible validation, and no-color behavior.
- Synthetic preview example: 11 passed.
- Nine native PNGs above were inspected at their stated terminal sizes.
- Formatting, strict all-target/all-feature Clippy, and diff whitespace checks
  passed. Independent review found a directional grid-edge issue; its regression
  test reproduced it, the fix passed, and re-review approved it.
- The macOS PTY route was updated for the new template/Home flow but was not
  executed. Interactive `make dev` testing of saved user data is not claimed.

### Earlier Home / Agents visual-fidelity update

Manual interactive testing of the user's application data is not claimed by
this documentation preparation.

| Evidence | Result |
| --- | --- |
| Baseline `cargo test --locked` before the visual-fidelity correction | 1,282 passed; 0 failed; 1 ignored across 99 result summaries |
| Focused Home and shared visual contracts | 6 passed after integration |
| Preview example unit tests | 10 passed, including both Home fixtures and the hidden raw-audit-input guard |
| Visual-fidelity render set | Home and Agents rendered at `120x30`, `160x40`, and `60x18` in color plus `80x24` in NO_COLOR; populated Home and empty Agents also rendered at `120x30`. All 10 PNGs were visually inspected |
| Final `cargo test --locked --no-fail-fast` after correction | 1,293 passed; 0 failed; 1 ignored across 100 result summaries |
| Final formatting and lint | `cargo fmt --all -- --check` and `cargo clippy --locked --all-targets --all-features -- -D warnings` passed |
| Interactive `make dev` usability run | Not performed or claimed here |

Record exact commands, dimensions, results, and any blocked actions during the
final review. Do not turn an unperformed action into a pass claim.

An earlier suite attempt saw `WorkerExited` in the existing fallback shutdown
acceptance test. Its isolated recheck and the final complete suite both passed;
no runtime or persistence change was made for that intermittent failure.
