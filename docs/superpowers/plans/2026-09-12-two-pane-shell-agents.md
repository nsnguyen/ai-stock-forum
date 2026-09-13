# Two-pane shell and Agents Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first usable slice of the approved redesign: numbered top navigation, two-pane geometry, WASD guidance, and a readable, editable Agents workspace.

**Architecture:** Retain the existing TuiModel, controller effects, host operations, and typed domain identities. Add explicit list/action focus and profile input ownership to the presentation model; preserve current target-bound outcomes and reviews. Adapt shared geometry once, then complete Agents without rewriting the Memory and Skills workflows.

**Tech Stack:** Rust 1.98.0, ratatui 0.29.0, crossterm 0.28.1, existing SQLite/application boundary. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-12-two-pane-tui-design.md`

## Global Constraints

- Tab means next panel or section; Shift+Tab reverses that order.
- WASD is the advertised directional navigation. Arrow keys remain quiet equivalents in navigation mode.
- Number keys select the main destinations; list rows do not reuse them.
- UUIDs, hashes, and technical bookkeeping are hidden in everyday screens.
- At 100 columns and above, show both panes. The list uses about 28% of width, clamped to 24–36 cells; the workspace gets the remainder.
- From 60–99 columns, retain both logical panes but display one at a time.
- The supported minimum remains 60×18.
- Chat and connection setup are Phase 3 features.
- Existing slash-command grammar remains available.
- The redesign changes presentation and keyboard routing; it does not turn displayed labels into persistence keys.
- All work stays in `/Users/nguyen-mini/.codex/worktrees/ai-stock-forum-two-pane-design`; leave the main checkout unchanged.
- This plan implements the first slice only. Memory and Skills retain their current feature workflows, data-read boundaries, validation, and exact review guarantees while receiving the shared navigation/geometry. Their complete field/presentation redesign and the remaining status/history presentation improvements belong to subsequent slices. Help must accurately describe any transitional controls.

## File map and boundaries

`model.rs` owns destinations, focus, retained state, and input ownership. `controller.rs` routes keys and emits existing typed effects; `host.rs` continues authoritative operations. `layout.rs`, `render.rs`, and `theme.rs` own the shell. `views/agents.rs` owns readable Agents content. Add small presentation modules for navigation labels or friendly formatting if this avoids duplicating logic across controller and render; do not broadly split or rewrite host/runtime files. `profile_editor.rs` gains a TUI-only literal-field interface while its fallback line protocol stays intact.

### Task 1: Shared numbered navigation and two-pane shell

**Files:**
- Modify: `src/ui/tui/model.rs`, `controller.rs`, `layout.rs`, `render.rs`, `theme.rs`, `views/mod.rs`, `views/help.rs`.
- Adapt geometry consumers: `src/ui/tui/views/agents.rs`, `skills.rs`, `memory.rs`; status placeholder rendering may live in `views/mod.rs` or a small `views/future.rs`.
- Test: `tests/tui_navigation_contract.rs`, `tests/tui_hardening_contract.rs`, `tests/tui_two_pane_contract.rs` (new), and existing layout/render unit tests and affected UI contract tests.

**Interfaces:**
- Consumes `TuiModel::switch_tab`, `active_navigation_tab`, retained TabState, `ControllerEffect`, existing `handle_event` and `render` public boundaries.
- Produces `View::{Chat,Connections,Activity}` in addition to current variants; keep `View::Overview` internally, label it Home. Skills continues its existing active flag for compatibility.
- Produces nine-entry `NavigationTab`/TabState routing and one authoritative mapping for labels and keys: 1 Home, 2 Chat, 3 Agents, 4 Skills, 5 Connections, 6 Activity, 7 Setup, 8 Audit, 9 Help.
- Produces `Focus::List` and `Focus::Actions` alongside Navigation/Workspace/Command. Inspector may remain as a legacy internal variant but is never a permanent third pane or invisible Tab stop.
- Produces `InputMode::{Nav,Type}` and `TuiModel.input_mode` for the new profile interaction; legacy Memory/Skills input ownership remains authoritative until their slice. Shell footer derives actual mode, not a cosmetic label.

- [ ] **Step 1: Add behavioral shell tests and run them red.** Reuse the real snapshot fixture structure in `tests/tui_navigation_contract.rs`. The new contract tests exercise real controller and TestBackend output, for example:

```rust
#[test]
fn numbered_routes_do_not_consume_literal_command_text() {
    let mut model = model();
    handle_event(&mut model, key(KeyCode::Char('3')));
    assert_eq!(model.active_view, View::Agents);
    handle_event(&mut model, key(KeyCode::Char('4')));
    assert!(model.skills.active);
    handle_event(&mut model, key(KeyCode::Char('2')));
    assert!(!model.skills.active);
    assert_eq!(model.active_view, View::Chat);
    handle_event(&mut model, key(KeyCode::Char('/')));
    handle_event(&mut model, TuiEvent::Paste("wasd123456789".into()));
    assert_eq!(model.command.text(), "/wasd123456789");
    assert_eq!(model.active_view, View::Chat);
}
```

Add literal expected route cases for all nine numbers, Shift uppercase WASD, ignored Ctrl/Alt/Meta shortcuts, no navigation on paste outside input, no phantom inspector focus, and retained selection/scroll after switching destinations. Test rendered full numbered labels at 60×18, 80×24, 100×24, 120×30, 160×40; no third pane at wide size and honest Phase 3 placeholder text.

Run: `cargo test --locked --test tui_two_pane_contract`. Confirm the old 3→Audit mapping or old geometry fails the new expectations before production edits.

- [ ] **Step 2: Implement destination and focus routing.** Use the nine destinations as the single ordered source instead of independent six-element matches. Keep the existing pending-outcome navigation generation and per-tab draft swapping. For NAV directions, normalize unmodified lowercase or shifted uppercase WASD to directional intent before dispatching feature navigation; never normalize text/picker ownership incorrectly.

```rust
// Conceptual route entries: use this order in the existing NavigationTab mapping.
const LABELS: [&str; 9] = [
    "1 Home", "2 Chat", "3 Agents", "4 Skills", "5 Connections",
    "6 Activity", "7 Setup", "8 Audit", "9 Help",
];
// Regions are logical, not conditional on pane visibility at compact widths.
const AGENT_REGIONS: &[Focus] = &[
    Focus::Navigation, Focus::List, Focus::Workspace, Focus::Actions,
];
```

Only actual enabled regions enter the cycle; generic read-only pages need Navigation/Workspace, and command focus is explicitly requested with `/`, not a permanent stop. Agent region-specific actions and field traversal land in Task 2. Preserve legacy Memory/Skills interactions while replacing their global a/s shortcuts with 3/4. Do not let global normalization turn D into final confirmation Enter. Repeated key events cannot write. Keep Ctrl+C and `/quit` safe below the minimum size.

- [ ] **Step 3: Implement shared geometry, truthful destination content, and footer.** Remove the vertical main menu and permanent inspector. Allocate top title/nav rows with whole-label wrapping, workspace, message, and contextual footer; the command field occupies space only while an actual input owns it. Use a single two-pane split for Agents/Skills and at most two panes for Memory, folding its optional context into the workspace. Compact mode reveals the logically focused region; do not reset selection on resize.

```rust
let list_width = ((u32::from(area.width) * 28) / 100) as u16;
let list_width = list_width.clamp(24, 36);
let columns = Layout::horizontal([
    Constraint::Length(list_width), Constraint::Min(0),
]).split(area);
```

Use dark charcoal, near-white, muted secondary text, cyan active focus, NO_COLOR modifiers/markers. Selected-but-inactive rows must not masquerade as the focused region. Render Chat/Connections as Coming in Phase 3 without inputs or mock data. Activity can reuse bounded audit summaries read-only. Update shared Help and footer to teach numbers, Tab, WASD, Enter, Esc, and preserve truthful legacy field help until its respective slice.

- [ ] **Step 4: Verify and commit.** Run focused new shell tests plus existing navigation, hardening, Memory, Skills, and Agents UI contracts. Migrate assertions that hard-code old destinations, geometry, or global hints while retaining behavioral assertions on state/identity/review handling. Run `cargo test --locked`, `cargo fmt --all -- --check`, and `git diff --check` before the task commit. Commit only the changed implementation and its tests.

### Task 2: Complete the friendly Agents workspace and profile editor

**Files:**
- Modify: `src/ui/tui/model.rs`, `controller.rs`, `host.rs`, `views/agents.rs`, `render.rs`, `theme.rs`, `src/ui/profile_editor.rs`.
- Optional small module: `src/ui/tui/views/presentation.rs` for stable initials, readable dates/status/diff labels if reused; register in `views/mod.rs`.
- Test: `tests/agent_profile_tui_controller_contract.rs`, `agent_profile_tui_render_contract.rs`, `agent_profile_tui_host_contract.rs`, `agent_profile_editor_contract.rs`, `tui_two_pane_contract.rs`.

**Interfaces:**
- Consumes Task 1's `Focus::{Navigation,List,Workspace,Actions}`, `InputMode::{Nav,Type}`, nine destinations, two-pane geometry.
- Expands `AgentDetailAction` to `Profile`, `Memory`, `AssignedSkills`, `History`, default Profile. Preserve existing typed `ControllerEffect::LoadAgentMemory`, `LoadAgentProfileHistory`, `StartProfileEdit`, and existing assignment origin/return behavior.
- Adds `ProfileTuiField` in profile_editor.rs for Template, DisplayName, Role, Description, PrimarySpecialty, Tags, Personality, Instructions, Bindings, Review, Discard. TUI-only literal methods: `set_tui_field(&mut self, field: ProfileTuiField, text: &str) -> bool`, `move_tui_field(&mut self, forward: bool) -> ProfileTuiField`, `tui_field(&self) -> ProfileTuiField`, `tui_field_text(&self, field: ProfileTuiField) -> &str`. Store field-keyed raw text/errors separately from the validated draft and a dedicated active-field text buffer/cursor in Agents presentation state, not the global command field. Invalid edits still invalidate old previews. Keep `submit_line` fallback behavior unchanged; do not call `submit_keyboard_line` for TUI field prose.
- Selection-driven profile loads bind `AgentProfileId` plus a selection generation and expected active version, not a mutable row index. Queue/coalesce passive profile loads in the runner using its existing pending-request pattern. A result may install only for the matching current target/generation, and must not change the user's selection or focus. Explicit command-mode outcomes keep their current command routing contract.

- [ ] **Step 1: Add failing real-controller and editor tests.** Extend actual profile fixtures, not mock renderers. Tests must prove Tab from agent list exposes the workspace without a hidden Enter prerequisite; W/S changes selection but Tab does not; Profile/Memory/Skills/History choices all activate their existing exact target; wrong/late profile result cannot become the new selected agent's body; historical version is clearly read-only and Edit uses the active target.

```rust
#[test]
fn tab_enters_selected_agents_workspace_without_reselecting_it() {
    let mut model = model_with_profiles();
    model.select_view(View::Agents);
    model.set_focus(Focus::List);
    let selected = model.agents.selected_profile;
    handle_event(&mut model, key(KeyCode::Tab));
    assert_eq!(model.focus, Focus::Workspace);
    assert_eq!(model.agents.selected_profile, selected);
    // Render must expose Profile, Memory, Skills and History for this identity.
    let screen = render_text(&model, 120, 30);
    for label in ["Profile", "Memory", "Skills", "History"] {
        assert!(screen.contains(label), "missing {label}");
    }
}
```

Additional tests use literal `wasd123456789/n` and `:back` in profile fields, invalid draft retention on Esc/Tab, NAV template selection using WASD, global number navigation after leaving TYPE, distinct review/confirm Enter and repeat rejection. Profile text stays single-line consistent with the existing whitespace-normalizing domain. Render fixtures include nonzero IDs/digests and assert none leaks to default Agents/detail/history/review/confirmation/error output.

Run: `cargo test --locked --test agent_profile_tui_controller_contract --test agent_profile_tui_render_contract --test agent_profile_editor_contract`; first run the new targeted test names to see intended red failures before implementation.

- [ ] **Step 2: Implement readable selection and actions.** Render a stable colored monogram/name, short specialty, status, name/purpose workspace heading, and four obvious choices. Right identity comes immediately from selected list row. Show Loading <name> for absent/mismatched detail and discard stale detail. A/D moves horizontally with clamped ends; Esc backs out; W/S scrolls the workspace or its narrow vertical choice list. N starts existing template creation, E edits, H opens history, with visible actions. Tab doesn't open a note body or enter another object.

```rust
let selected = model.agents.profiles.profiles.get(model.agents.selected_profile);
let detail = model.agents.detail.as_ref().filter(|detail| {
    selected.is_some_and(|row| row.profile_id == detail.profile.profile_id())
});
// Render selected identity regardless of whether full detail is loaded.
```

Use readable readiness mapping (Needs connection / Connection unavailable / Bindings configured, plus required engineering binding qualification). Keep IDs and digests internal; show Version N · Current or Historical with human dates and readable changed fields. Expose no raw immutable metadata block. Review and confirmation name operations and objects without requiring ID/token entry. Preserve exact internal review/authentication logic and history version selection. Skills and Memory targets use IDs internally and retain existing protected-workflow constraints. Full actions stay disabled with a readable loading hint until matching detail exists. Profile/history/version result installation cannot reselect the returned object. Starting Edit captures the selected identity and expected active version before dispatch.

- [ ] **Step 3: Implement deliberate TYPE fields without changing fallback protocol.** Profile fields render in the workspace, retain exact editable buffers including invalid input, and validate beside the affected field. Enter enters TYPE; Esc retains draft and returns NAV; Tab leaves TYPE, moves to the next field/section without saving, and preserves old field input. Templates/reference/binding choices remain NAV. N/E opens the workflow; fields can start TYPE when appropriate, never pickers. Profile Enter accepts its single-line field: all durable profile text currently normalizes whitespace, so do not introduce a domain-level line-break change. Memory Note multiline editing remains part of the later Memory slice. Existing fallback colon commands remain exclusively in `submit_line`.

```rust
match (model.input_mode, key.code) {
    (InputMode::Type, KeyCode::Esc) => { /* retain field; return Nav */ }
    (InputMode::Type, KeyCode::Tab) => { /* retain field; next field in Nav */ }
    (InputMode::Type, KeyCode::Enter) => {
        /* validate literal field, retain errors, return Nav; no durable write */
    }
    (InputMode::Type, KeyCode::Char(character)) => {
        /* ordinary text, including numbers, slash and leading colon */
    }
    _ => { /* explicit navigation/action routing */ }
}
```

Keep draft suspension separate from explicit Discard. Changes invalidate preview generations. Never interpret a TUI field's leading `:` through the existing keyboard helper that currently forwards to control parsing. Keep readonly binding status honest: no Phase 3 credential entry. All currently supported profile draft fields remain reachable, including tags and role through deliberate controls.

- [ ] **Step 4: Verify and commit.** Run the profile editor/TUI contracts and fallback contracts (fallback colon protocol must still work), navigation contracts, and full `cargo test --locked`. Run formatting and whitespace checks. Read the diff, preserving identity/race/review safeguards; commit the tested changes.

### Task 3: Visual verification, integration coverage, and test guide

**Files:**
- Modify: `README.md`, `docs/testing/phase-2-agent-profile-foundation.md` with current first-slice instructions, without rewriting historical specs.
- Create: `docs/testing/two-pane-shell-agents.md`.
- Create: `examples/two_pane_preview.rs` only if a reproducible TestBackend preview is needed for visual inspection; use synthetic fixture data and actual production render, not a separate mock UI.
- Test: `tests/tui_two_pane_contract.rs` and related behavior/render contracts for any uncovered integration gaps.

**Interfaces:**
- Consumes public `render`, `handle_event`, `TuiModel`, actual profile fixture constructors; no application database is required for rendering examples.
- Produces an honest first-slice guide: 3 Agents, W/S choose, Tab workspace, A/D Memory, Enter open, Esc return; N New and E Edit; NAV/TYPE; 1–9 destinations; later Memory/Skills visual work explicitly outstanding.

- [ ] **Step 1: Inspect the actual rendered terminal output.** Use TestBackend fixtures for selected/unbound agent, empty Agents, history, profile editor, review, and Phase 3 placeholders at 60×18, 80×24, 100×24, 120×30, 160×40 and NO_COLOR. Make any newly discovered behavioral regression a failing test before fixing it. Inspect focus, wrapping, footer, cursor, and retained state when resizing. Do not represent illustration PNGs as implemented screenshots.

```rust
let mut terminal = Terminal::new(TestBackend::new(width, height))?;
terminal.draw(|frame| render(frame, &model, &Theme::from_no_color(no_color)))?;
for row in terminal.backend().buffer().content().chunks(usize::from(width)) {
    println!("{}", row.iter().map(|cell| cell.symbol()).collect::<String>());
}
```

- [ ] **Step 2: Document normal local testing.** Confirm CLI options from existing code/README, then document `make dev` in this worktree. If isolated application data is supported, show the existing explicit app-data override with a temporary directory, not a new macOS account. Do not launch a runtime that modifies the user's real app data during automated validation. Record automated coverage separately from unperformed interactive manual checks.

- [ ] **Step 3: Run integration gates and commit.** Run `cargo test --locked`, `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `git diff --check`. Update the plan checkboxes with actual progress. Commit the guide and any verified integration fixes. Do not push or merge without user direction. Keep the worktree for testing and review.
