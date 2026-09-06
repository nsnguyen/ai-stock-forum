# Declarative Skills Implementation Plan

> Execution workflow: use `superpowers:subagent-driven-development` with a
> fresh implementer and reviewer for each task. Follow
> `superpowers:test-driven-development` for every production behavior.

**Goal:** Add a safe, local, versioned skill library with keyboard-first skill
management and exact agent assignments.

**Architecture:** Introduce an immutable `skills` domain parallel to agent
profiles. Store exact `SkillVersionRef` values inside immutable agent profile
versions. Persist authoritative skill versions separately from rebuildable
active projections, route every mutation through typed commands and review
boundaries, and expose all workflows through an adaptive Skills workspace.

**Technology:** Rust, SQLite, serde-compatible canonical payloads, Ratatui,
typed application commands/events/outcomes, and the existing digest, receipt,
audit, recovery, and optimistic-concurrency infrastructure.

**Spec:** `docs/superpowers/specs/2026-09-05-declarative-skills-design.md`

## Global constraints

- Work only in `/private/tmp/ai-stock-forum-phase-2-declarative-skills` on
  `codex/phase-2-declarative-skills`.
- Never edit or clean the dirty main checkout.
- Never modify `migrations/0001_phase0.sql` or
  `migrations/0002_agent_profiles.sql`.
- Write a failing behavioral test and observe the intended failure before
  adding each production behavior.
- Skills are inert accepted text. Do not add execution, shell, filesystem,
  Git, MCP, provider, browser, or network capability.
- Every edit creates an immutable skill version.
- Every assignment pins an exact version and creates an immutable agent
  profile version.
- One agent may hold one version per skill and at most 16 skills total.
- Existing agents do not auto-upgrade.
- Empty skill references must preserve legacy profile digest behavior.
- Full skill prose must not be copied into routine audit event payloads.
- Built-in manifests must have deterministic IDs, versions, content, and
  digests.
- Bare `q` remains inert and `/quit` remains the normal shutdown command.
- Keep current Agent Readiness semantics; skills do not make an unbound agent
  ready.
- Do not implement deletion, archival, file import, inference, or chat.
- Use stable typed errors at every domain and persistence boundary.
- Run focused tests during each TDD loop and the complete suite only at the
  final verification gate.

## Canonical limits

- Display name: 64 UTF-8 bytes.
- Description: 256 UTF-8 bytes.
- Usage guidance: 512 UTF-8 bytes.
- Tags: 8 maximum, 32 UTF-8 bytes each.
- Instructions: 4096 UTF-8 bytes.
- Reference notes: 8 maximum.
- Reference name: 64 UTF-8 bytes.
- Reference body: 4096 UTF-8 bytes.
- Combined reference bodies: 16384 UTF-8 bytes.
- Complete canonical skill payload: 32768 UTF-8 bytes.
- Assigned skills per agent: 16 maximum.

Normalize CRLF and CR to LF. Reject NUL, escape, C1 control characters, and
bidi override controls. Apply existing Unicode normalization and case-folding
conventions to comparable names. Canonically sort tags, resources, and agent
skill references before hashing.

## Task 1: Establish the immutable skill domain

**Files:**

- Create `src/skills/normalization.rs`.
- Create `src/skills/skill.rs`.
- Create `src/skills/review.rs`.
- Create `src/skills/projection.rs`.
- Modify `src/skills/mod.rs`.
- Modify `src/domain/id.rs`.
- Modify `src/domain/error.rs`.
- Modify `src/domain/mod.rs` only if exports require it.
- Modify `src/lib.rs` only if the new module is not already exported.
- Create `tests/skill_domain_contract.rs`.
- Create `tests/skill_review_contract.rs`.

### Step 1: Write failing domain contracts

Cover these behaviors with real values rather than mocks:

- `SkillId` and `SkillVersionId` are distinct typed IDs.
- A valid draft accepts into immutable version 1.
- A next version retains `SkillId`, gets a new `SkillVersionId`, and increments
  `ObjectVersion` by exactly one.
- An unchanged normalized candidate is rejected.
- Every canonical limit and forbidden-control rule is enforced at its exact
  boundary.
- CRLF and CR become LF before digesting.
- Tags and resources canonicalize deterministically.
- Duplicate normalized tags and resource names are rejected.
- The same accepted content produces the same content digest.
- `SkillVersionRef` includes logical ID, exact version ID, object version, and
  content digest.
- Review tokens reject changed candidates and stale expected versions.
- Projection insertion and active-version movement are deterministic.

Run:

```bash
cargo test --test skill_domain_contract --test skill_review_contract
```

Expected RED result: compilation or assertions fail only because the new skill
types and behavior do not yet exist.

### Step 2: Implement the minimum domain

Define these core shapes, using existing project wrappers for IDs, versions,
timestamps, and digests:

```rust
pub struct SkillResource {
    pub name: String,
    pub body: String,
}

pub struct SkillDraft {
    pub display_name: String,
    pub description: String,
    pub use_when: String,
    pub tags: Vec<String>,
    pub instructions: String,
    pub resources: Vec<SkillResource>,
}

pub enum SkillProvenance {
    BuiltIn {
        manifest_id: String,
        manifest_version: u32,
        manifest_digest: ContentDigest,
    },
    User,
}

pub struct SkillVersionRef {
    pub skill_id: SkillId,
    pub skill_version_id: SkillVersionId,
    pub version: ObjectVersion,
    pub content_digest: ContentDigest,
}

pub struct SkillVersion {
    pub skill_id: SkillId,
    pub skill_version_id: SkillVersionId,
    pub version: ObjectVersion,
    pub content: SkillDraft,
    pub content_digest: ContentDigest,
    pub created_at_ms: i64,
    pub provenance: SkillProvenance,
    pub predecessor: Option<SkillVersionId>,
}
```

Add a normalized-name selector and a `SkillsProjection` keyed by `SkillId`.
Mirror the existing agent review-token pattern rather than inventing a second
security model. Keep constructors private where needed so invalid accepted
versions cannot be assembled directly.

### Step 3: Verify GREEN and refactor

Re-run the focused command. Refactor only after both test binaries pass.

## Task 2: Add deterministic starter manifests and bounded retrieval

**Files:**

- Create `src/skills/builtin.rs`.
- Create `src/skills/retrieval.rs`.
- Modify `src/skills/mod.rs`.
- Create `tests/skill_builtin_manifest_contract.rs`.
- Create `tests/skill_retrieval_contract.rs`.

### Step 1: Write failing manifest and retrieval contracts

Assert:

- Exactly four built-ins exist: Evidence Review, Filing Analysis, Catalyst
  Mapping, and Risk Checklist.
- Each built-in has a stable ID, manifest ID, manifest version 1, accepted
  content, and expected digest.
- Manifest order does not affect reconciliation output.
- Retrieval loads only exact assigned versions.
- Retrieval follows canonical assignment order.
- Count, byte, and resource budgets include only complete sections and report
  omitted counts.
- Retrieval never interprets commands, paths, tool names, or URLs.

Run:

```bash
cargo test --test skill_builtin_manifest_contract --test skill_retrieval_contract
```

Expected RED result: the built-in catalog and retrieval API are absent.

### Step 2: Implement built-ins and retrieval

Use source-controlled declarative Rust data or embedded static manifests. Do
not read mutable files at runtime. Validate each manifest through the same
acceptance path as user drafts and assert its expected digest.

Define a retrieval budget with skill count, total accepted bytes, and resource
count. Return included exact versions plus omitted counters. Do not perform
external I/O.

### Step 3: Verify GREEN and refactor

Re-run both focused tests and keep manifest digests explicit so accidental
content edits fail loudly.

## Task 3: Integrate exact skill references with agent profiles

**Files:**

- Modify `src/agents/profile.rs`.
- Modify `src/agents/diff.rs`.
- Modify `src/agents/review.rs`.
- Modify `src/agents/projection.rs` only if projection validation requires it.
- Modify `src/agents/mod.rs`.
- Create `tests/agent_profile_skill_refs_contract.rs`.
- Modify `tests/agent_profile_domain_contract.rs` only for compatibility cases.
- Modify `tests/agent_profile_version_contract.rs` only for compatibility
  cases.
- Modify `tests/agent_profile_template_golden_contract.rs` only if an existing
  serialized golden shape must explicitly represent empty skill references.

### Step 1: Write failing profile contracts

Assert:

- An empty skill list retains the prior canonical profile digest.
- Non-empty exact refs are accepted only when structurally valid.
- References sort canonically by `SkillId`.
- Two versions of the same `SkillId` are rejected.
- A seventeenth skill is rejected.
- Assignment adds an exact ref in a new candidate profile version.
- Upgrade replaces one exact ref rather than appending another.
- Unassignment removes only the expected exact ref.
- Assigning the same version and unassigning a missing version are typed
  no-ops/errors.
- Profile diffs report added, upgraded, and removed skill references.
- Skill changes do not alter binding-derived readiness.

Run:

```bash
cargo test --test agent_profile_skill_refs_contract \
  --test agent_profile_domain_contract \
  --test agent_profile_version_contract
```

Expected RED result: current profiles reject non-empty placeholder skill refs
or lack exact assignment behavior.

### Step 2: Implement profile integration

Replace the unchecked placeholder `SkillRef` with `SkillVersionRef`. Add pure
candidate-building operations for assign, upgrade, and unassign. Preserve the
existing immutable acceptance and review flow. Add skill-reference differences
to profile diffs and canonical payloads while special-casing the empty list to
retain legacy digest bytes.

### Step 3: Verify GREEN and refactor

Re-run the focused tests. Do not update old golden values unless the test first
proves the old value is impossible to preserve; the spec requires preservation
for empty refs.

## Task 4: Add schema version 3 and the skill repository

**Files:**

- Create `migrations/0003_declarative_skills.sql`.
- Create `src/persistence/skill_repository.rs`.
- Modify `src/persistence/mod.rs`.
- Modify `src/persistence/migrations.rs`.
- Modify `src/persistence/database.rs`.
- Modify `src/persistence/agent_profile_repository.rs`.
- Modify `src/persistence/command_receipt_repository.rs` only if its capability
  constraint requires version 3 support.
- Create `tests/skill_migration_contract.rs`.
- Create `tests/skill_persistence_contract.rs`.
- Create `tests/skill_atomicity_contract.rs`.

### Step 1: Write failing persistence contracts

Assert:

- Schema version 2 migrates to exactly version 3.
- Migration 3 creates immutable `skill_versions` and rebuildable
  `active_skills` structures with foreign-key and uniqueness enforcement.
- Migration failure rolls back every version 3 object and leaves schema version
  2 intact.
- Existing agent rows survive byte-for-byte and remain readable.
- Immutable skill versions insert once and cannot be mutated or substituted.
- Exact lookup, normalized-name lookup, active lookup, and ordered history work.
- Moving an active pointer never mutates an old version.
- Agent profile persistence validates every exact skill ref and digest.
- Assignment transaction failure leaves profile, active pointer, event, and
  receipt unchanged.

Run:

```bash
cargo test --test skill_migration_contract \
  --test skill_persistence_contract \
  --test skill_atomicity_contract
```

Expected RED result: schema version 3 and the skill repository do not exist.

### Step 2: Implement migration and repository

Set `LATEST_SCHEMA_VERSION` to 3. Add migration fault injection for version 3
without weakening existing version 2 coverage. Store canonical accepted
payloads and digests in the immutable table. Keep active pointers in a separate
projection table. Rebuild SQLite receipt constraints safely if new capability
values require it.

Validate exact skill references at the SQLite transaction boundary, not only
in the UI or application layer.

### Step 3: Verify GREEN and refactor

Re-run focused persistence tests. Include foreign-key enforcement and rollback
assertions in every connection mode already supported by the project.

## Task 5: Add typed skill commands, outcomes, events, policy, and service flow

**Files:**

- Modify `src/app/command.rs`.
- Modify `src/app/outcome.rs`.
- Modify `src/app/event.rs`.
- Modify `src/app/service.rs`.
- Modify `src/app/mod.rs`.
- Modify `src/policy/capability.rs`.
- Modify `src/audit/mod.rs`.
- Create `tests/skill_application_contract.rs`.
- Create `tests/skill_event_contract.rs`.
- Create `tests/skill_concurrency_contract.rs`.
- Create `tests/skill_receipt_contract.rs`.
- Create `tests/skill_audit_contract.rs`.

### Step 1: Write failing application contracts

Cover typed operations for:

- Preview and accept skill creation.
- Preview and accept a new version against an expected active version.
- List skills and view active or historical versions.
- List version history.
- Preview and commit assignment, upgrade, and unassignment against the expected
  active agent profile.
- Replay each mutation receipt without duplicate writes or events.
- Reject stale profile and active-skill versions.
- Produce one winner for concurrent conflicting mutations.
- Keep full instructions and reference bodies out of read-event audit payloads.
- Deny any unrecognized or execution-like skill capability.

Run:

```bash
cargo test --test skill_application_contract \
  --test skill_event_contract \
  --test skill_concurrency_contract \
  --test skill_receipt_contract \
  --test skill_audit_contract
```

Expected RED result: typed skill commands and outcomes are absent.

### Step 2: Implement the service boundary

Add capabilities equivalent to skill read, create, version, assign, and
unassign. Do not add `SkillExecute`.

Add events equivalent to skill created, version activated, listed, viewed,
history viewed, exact version viewed, agent skill assigned, upgraded, and
unassigned. Mutation events carry immutable identities and resulting profile
version metadata. Read events carry metadata only.

Use the existing command receipt, transaction, review token, and optimistic
concurrency patterns. Assignment and unassignment must atomically validate the
exact skill record, create a profile version, move the active profile pointer,
record the event, and store the receipt.

### Step 3: Verify GREEN and refactor

Re-run all five focused tests. Preserve canonical event wire behavior and
stable error codes.

## Task 6: Extend recovery and startup reconciliation

**Files:**

- Modify `src/recovery/reducer.rs`.
- Modify `src/recovery/coordinator.rs`.
- Modify `src/recovery/mod.rs`.
- Modify `src/persistence/projection_repository.rs`.
- Modify `src/app/service.rs` only for startup sequencing.
- Create `tests/skill_recovery_contract.rs`.
- Create `tests/skill_integrity_contract.rs`.

### Step 1: Write failing recovery contracts

Assert:

- Skill versions reconcile before agent skill references are validated.
- Missing authoritative skill rows can be restored from canonical records.
- Altered or unexpected immutable rows fail closed.
- Active skill projections rebuild deterministically.
- Agent assignments rebuild through active immutable profile versions.
- Built-in records reconcile idempotently.
- Empty skill state leaves existing projection digests unchanged.
- Repeating recovery produces the same database and projection digest.

Run:

```bash
cargo test --test skill_recovery_contract --test skill_integrity_contract
```

Expected RED result: recovery knows nothing about skills.

### Step 2: Implement reduction and reconciliation

Extend the reducer with a `SkillsProjection`. Rebuild active pointers from
authoritative events and immutable records. Order startup as: migrate, verify
required pragmas, reconcile built-ins and skill versions, rebuild skills, then
validate/rebuild agent projections.

### Step 3: Verify GREEN and refactor

Re-run both focused tests and existing recovery contracts affected by startup
ordering.

## Task 7: Add optional slash-command skill workflows

**Files:**

- Modify `src/ui/command/parser.rs`.
- Modify `src/ui/command/runner.rs`.
- Modify `src/ui/command/renderer.rs`.
- Modify `src/ui/command/mod.rs`.
- Create `tests/skill_fallback_contract.rs`.
- Modify `tests/command_contract.rs` only for shared parser behavior.
- Modify `tests/fallback_contract.rs` only for shared help behavior.

### Step 1: Write failing fallback contracts

Parse and route:

- `/skill list`
- `/skills`
- `/skill add`
- `/skill show <name-or-id> [version]`
- `/skill assign <skill> <agent> [version]`
- `/skill unassign <skill> <agent>`

Assert quoted selectors, positive version validation, unknown subcommand help,
and typed errors. Mutation commands must open the same review state as the TUI
instead of bypassing confirmation. Assert again that bare `q` does not exit and
`/quit` does.

Run:

```bash
cargo test --test skill_fallback_contract --test command_contract \
  --test fallback_contract
```

Expected RED result: `/skill` routes are unknown.

### Step 2: Implement the smallest parser and runner changes

Keep parsed selectors unresolved until the application boundary. If assignment
omits a version, resolve and display the exact active version when review
begins. Do not mutate directly from parser or renderer code.

### Step 3: Verify GREEN and refactor

Re-run focused fallback tests and keep help text synchronized with actual
syntax.

## Task 8: Build the keyboard-first skill state machines

**Files:**

- Create `src/ui/skill_editor.rs`.
- Modify `src/ui/tui/model.rs`.
- Modify `src/ui/tui/event.rs`.
- Modify `src/ui/tui/controller.rs`.
- Modify `src/ui/tui/host.rs`.
- Modify `src/ui/tui/mod.rs`.
- Create `tests/skill_editor_contract.rs`.
- Create `tests/skill_tui_controller_contract.rs`.
- Create `tests/skill_tui_host_contract.rs`.

### Step 1: Write failing state-machine contracts

Assert:

- `s` opens Skills without mutating data.
- Up and Down select list, history, action, and agent-picker rows.
- Enter opens details and advances every valid editor step.
- Esc moves back while preserving safe draft state.
- Create has Identity, Usage, Instructions, References, and Review stages.
- Detail exposes Assign, Create Version, and History actions.
- Assignment distinguishes Add, Upgrade, and Already Assigned.
- Confirmation uses Enter and rejects stale review state.
- Agent detail exposes exact skill versions and unassignment state.
- No stage requires `:next` or `:create`.
- Bare `q` remains inert in every new state.

Run:

```bash
cargo test --test skill_editor_contract \
  --test skill_tui_controller_contract \
  --test skill_tui_host_contract
```

Expected RED result: skill editor and TUI states are absent.

### Step 2: Implement pure state transitions first

Keep editor validation in the domain and presentation state in the UI. Route
commits through application review commands. Preserve draft data after
recoverable errors and discard it only after explicit cancellation or a
successful accepted outcome.

### Step 3: Verify GREEN and refactor

Re-run the focused tests before adding rendering.

## Task 9: Render the adaptive Skills workspace and agent assignments

**Files:**

- Create `src/ui/tui/views/skills.rs`.
- Modify `src/ui/tui/views/mod.rs`.
- Modify `src/ui/tui/views/agents.rs`.
- Modify `src/ui/tui/views/help.rs`.
- Modify `src/ui/tui/layout.rs`.
- Modify `src/ui/tui/render.rs`.
- Modify `src/ui/tui/theme.rs` only if a semantic style is missing.
- Create `tests/skill_tui_render_contract.rs`.
- Create `tests/declarative_skills_acceptance.rs`.

### Step 1: Write failing render and acceptance contracts

Assert:

- Narrow mode renders one focused pane without clipped required controls.
- Medium mode renders list plus active context.
- Wide mode renders list, detail, and contextual pane.
- The four starters appear with provenance and active version.
- Empty, validation, stale, already-assigned, not-assigned, and persistence
  errors contain corrective text.
- Footer hints match current valid keys.
- Agent detail shows exact assigned versions and upgrade availability.
- Long instructions and reference bodies wrap or scroll without corrupting
  borders.
- The complete create, version, pin, upgrade, unassign, and restart workflow is
  possible through controller events.

Run:

```bash
cargo test --test skill_tui_render_contract \
  --test declarative_skills_acceptance
```

Expected RED result: the Skills workspace is not rendered.

### Step 2: Implement rendering

Use the established cockpit visual language and semantic theme colors. Keep
action labels visible rather than relying on remembered single-key shortcuts.
Render version and provenance prominently on every review screen.

### Step 3: Verify GREEN and refactor

Re-run both focused tests and any existing TUI contracts touched by shared
layout behavior.

## Task 10: Complete documentation and verification

**Files:**

- Create `docs/testing/declarative-skills.md`.
- Modify `README.md`.
- Modify `phases.md`.
- Modify `tests/documentation_contract.rs`.
- Modify `tests/topology_contract.rs` only if the new module topology is
  contractually enumerated.
- Add or modify a macOS PTY smoke fixture only if the existing harness supports
  the Skills workflow without environment-specific assumptions.

### Step 1: Write failing documentation contracts

Require the user documentation to explain:

- How to open Skills.
- How to create, version, assign, upgrade, and unassign with Enter and Esc.
- How exact version pinning works.
- Why skills cannot execute or access external systems.
- The optional slash commands.
- The manual restart and persistence workflow.
- That inference and chat begin in Phase 3.

Run:

```bash
cargo test --test documentation_contract --test topology_contract
```

Expected RED result: documentation does not describe the new milestone.

### Step 2: Update durable documentation

Mark only Phase 2 Milestone 2 complete. Leave Hybrid Memory and Phase 3
pending. Include concise local testing instructions for nontechnical users.

### Step 3: Run focused regression groups

Run every new skill contract together, then existing agent, application,
persistence, recovery, command, and TUI contract groups affected by the
changes. Resolve failures with a new failing regression test before changing
production code.

### Step 4: Run the complete automated suite

```bash
cargo test --all-targets
```

Expected result: exit code 0 with zero failed tests.

### Step 5: Perform the manual TUI workflow

Launch the application using the documented local command and use an isolated
temporary data directory. Verify:

1. Open Skills with `s` and inspect all four starters.
2. Create a custom skill using only typing, Enter, and Esc.
3. Assign its version 1 to an agent.
4. Create version 2 and confirm the agent remains pinned to version 1.
5. Upgrade the agent explicitly to version 2.
6. View version 1 from History and intentionally reassign it.
7. Unassign the skill.
8. Restart and confirm versions, active pointers, assignments, and audit entries
   persist.
9. Confirm bare `q` is inert and `/quit` exits cleanly.

### Step 6: Request final review

Use `superpowers:requesting-code-review` against the complete branch diff. Fix
Critical and Important findings with regression tests, request a scoped
re-review, then repeat the complete verification command before any completion
claim.

## Final deliverables

- Approved design specification.
- Detailed implementation plan.
- Versioned skill domain and four built-in manifests.
- Schema version 3 and transactional persistence.
- Typed commands, outcomes, events, capabilities, audit, and recovery.
- Keyboard-first adaptive Skills workspace.
- Exact agent assignment, upgrade, and unassignment.
- Optional slash-command surface.
- Automated and manual testing documentation.
- Fresh full-suite verification evidence.

