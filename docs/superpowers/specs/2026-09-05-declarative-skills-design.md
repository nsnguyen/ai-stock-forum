# Phase 2 Milestone 2: Declarative Skills Design

Date: 2026-09-05
Status: Approved for implementation planning

## Summary

Phase 2 Milestone 2 adds a local, versioned skill library to the adaptive
cockpit. A skill is inert guidance that can be assigned to an agent profile.
It contains a purpose, usage guidance, tags, instructions, and optional
reference notes. It cannot execute code, call tools, access files, fetch URLs,
or grant capabilities.

Users manage skills in a dedicated, keyboard-first Skills workspace. Every
skill edit creates an immutable version. Every agent assignment pins one exact
version, so an agent never changes behavior because a skill was edited later.

This milestone establishes the data, application, persistence, audit,
recovery, and TUI foundations that Phase 3 chat will use to assemble an
agent's context.

## Goals

- Provide a dedicated Skills workspace that works without memorized commands.
- Bundle four useful finance-focused starter skills.
- Let users create custom skills and create new versions of existing skills.
- Let users inspect current and historical versions.
- Assign, upgrade, and unassign exact skill versions on agent profiles.
- Preserve immutable history and deterministic audit behavior.
- Persist skills and assignments across restart and recovery.
- Expose bounded retrieval of assigned skill content for future chat use.
- Preserve compatibility with existing Phase 0, Phase 1, and Phase 2 profile
  data.

## Non-goals

- Executing a skill.
- Giving a skill shell, filesystem, Git, MCP, provider, or network access.
- Fetching or opening URLs found in skill content.
- Importing files or remote skill packages.
- Permanent deletion or archival.
- Inference, chat, orchestration, or multi-agent behavior.
- Automatically upgrading agents when a skill changes.

## Product decisions

The approved product decisions are:

- Skills use a guided model with purpose, usage guidance, tags, instructions,
  and optional reference notes.
- The Skills workspace is a first-class peer of the Agents workspace.
- Edits create immutable versions rather than overwriting existing content.
- Agents remain pinned to their assigned versions until explicitly upgraded.
- The initial library contains Evidence Review, Filing Analysis, Catalyst
  Mapping, and Risk Checklist.
- Normal workflows use arrow keys, Enter, and Esc. Slash commands are optional
  shortcuts rather than required controls.
- Skills remain declarative and inert.

## User experience

### Navigation

Pressing `s` from the cockpit opens the Skills workspace. Existing numbered
views remain available, and `/quit` remains the only normal quit command. A
bare `q` remains inert.

The Skills workspace adapts to terminal width:

- Narrow terminals show one focused pane at a time.
- Medium terminals show the skill list beside the active detail or editor.
- Wide terminals show the list, detail, and contextual action or history pane.

The footer always shows the actions valid for the current state. A user must
not need to discover a hidden command to continue.

### Skill list

The skill list shows display name, active version, provenance, and a compact
purpose. Built-in and custom skills are visually distinguishable without
changing their assignment behavior.

Controls:

- Up and Down move selection.
- Enter opens the selected skill.
- `c` opens the guided creator.
- Esc returns to the previous workspace.

The empty state says that `c` creates the first skill and explains that skills
are saved guidance, not executable extensions.

### Skill detail

The detail view shows:

- Display name and active version.
- Purpose and usage guidance.
- Tags.
- Instructions.
- Reference note names and bodies.
- Provenance and content digest.
- Agents currently assigned to this exact version or another version.

A visible action selector offers Assign, Create Version, and History. Left and
Right change the selected action, Enter opens it, and Esc returns to the list.

### Create and edit flow

The guided editor has five stages:

1. Identity: display name and purpose.
2. Usage: when to use the skill and tags.
3. Instructions: the accepted guidance body.
4. References: zero or more named, inert text notes.
5. Review: the complete candidate and the version that will be created.

Typing edits the focused value. Enter advances or accepts the visible action.
Esc moves back without destroying prior entries. The review screen uses Enter
to commit and Esc to return to editing. Neither `:next` nor `:create` is
required.

Editing an existing skill starts with the active version's content and creates
the next version. Identical content is rejected with a clear explanation.

### History

History lists every immutable version newest first. Up and Down move selection
and Enter opens an exact version. Historical versions are read-only, but the
user may intentionally assign one.

### Assignment

Assign opens an agent picker. Up and Down select an agent and Enter opens a
review screen. The review identifies the agent, exact skill version, and
whether the operation is an assignment or an upgrade.

An agent may have at most one version of a given skill. Assigning a different
version replaces the prior version in a new immutable agent profile version.
Assigning the already-pinned version is rejected as an understandable no-op.

Agent detail shows assigned skills and their exact versions. Selecting an
assigned skill offers View, Upgrade when a newer version exists, and Unassign.
Unassignment also uses a review screen and Enter confirmation.

If the agent profile changes while a review is open, confirmation fails safely
and asks the user to review the current state. It never overwrites the newer
profile.

### Optional slash commands

The command bar provides accessibility and automation shortcuts:

- `/skill list`
- `/skills`
- `/skill add`
- `/skill show <name-or-id> [version]`
- `/skill assign <skill> <agent> [version]`
- `/skill unassign <skill> <agent>`

Mutation shortcuts open the same review flow as the visual workspace. If a
version is omitted, the review pins the exact active version displayed at the
time the flow begins.

## Domain model

### Identity and selectors

Skills use distinct typed identifiers:

- `SkillId` identifies the logical skill across versions.
- `SkillVersionId` identifies one immutable version record.
- `NormalizedSkillName` provides deterministic lookup and duplicate checks.
- `SkillSelector` accepts an ID or normalized display name at application
  boundaries.

Identifiers must not be interchangeable with agent, profile, command, or event
identifiers.

### Skill draft

A `SkillDraft` contains:

- `display_name`
- `description`
- `use_when`
- `tags`
- `instructions`
- `resources`

Each `SkillResource` contains a display name and an inert plain-text or
Markdown body. A resource is a reference note, not a path, URL fetch request,
script, command, or provider binding.

### Immutable skill version

A `SkillVersion` contains:

- `skill_id`
- `skill_version_id`
- A positive, monotonically increasing object version.
- Canonical accepted content.
- A stable content digest.
- Creation time.
- Provenance identifying built-in or user-created origin.
- Optional predecessor version identity.

Once accepted, every field in a skill version is immutable.

### Exact assignment reference

Agent profile versions store `SkillVersionRef` values containing:

- `skill_id`
- `skill_version_id`
- Object version.
- Content digest.

The reference is sufficient to detect substitution or corruption. Agent skill
references are canonicalized by `SkillId`, and one profile may contain no more
than 16 distinct skills.

Assignment, upgrade, and unassignment create a new immutable agent profile
version. Existing profile versions and skill versions never change.

### Validation and canonicalization

Validation uses bounded UTF-8 content:

- Display name: 64 bytes.
- Description: 256 bytes.
- Usage guidance: 512 bytes.
- Tags: at most 8, each at most 32 bytes.
- Instructions: 4096 bytes.
- Reference notes: at most 8 per version.
- Reference name: 64 bytes.
- Reference body: 4096 bytes.
- Combined reference bodies: 16384 bytes.
- Complete canonical payload: 32768 bytes.

CRLF and CR are normalized to LF. NUL, escape, C1 control characters, and bidi
override controls are rejected. Names, tags, and reference names use the
project's Unicode normalization and case-folding rules for comparison.
Duplicate normalized tags and duplicate normalized reference names are
rejected.

Tags and resources are sorted canonically before hashing. Exact accepted
content, version metadata, and provenance contribute to the version digest.

### Starter skills

The application bundles four deterministic version 1 manifests:

- Evidence Review
- Filing Analysis
- Catalyst Mapping
- Risk Checklist

Each manifest has a stable identity, accepted content, provenance identifier,
manifest version, and expected digest. Startup reconciliation inserts missing
built-in records and rejects an existing record whose immutable content does
not match its expected digest.

Built-in version 1 manifests are fixed for this milestone. A user may create a
new version while the original remains intact and attributable.

## Application architecture

### Commands

The application layer adds typed commands for:

- Creating a skill and accepting version 1.
- Previewing and accepting a new skill version.
- Listing skills.
- Showing active or exact historical versions.
- Listing version history.
- Assigning an exact skill version to an agent.
- Upgrading an agent from one exact version to another.
- Unassigning the expected exact version from an agent.

Every mutation carries command identity and expected current versions needed
for optimistic concurrency. Receipt replay returns the original typed outcome
instead of applying a mutation twice.

### Review boundaries

Create, version, assign, upgrade, and unassign operations cross the existing
review boundary before persistence. Review tokens bind the normalized
candidate, expected current versions, and resulting digest. A stale or altered
candidate cannot be committed with an old token.

The TUI's Enter confirmation and optional slash commands both use these same
application commands. Safety does not depend on which UI path initiated the
operation.

### Events and audit

The typed event model adds events equivalent to:

- Skill created.
- Skill version activated.
- Skill listed.
- Skill viewed.
- Skill history viewed.
- Historical skill version viewed.
- Agent skill assigned.
- Agent skill upgraded.
- Agent skill unassigned.

Mutation events contain the resulting immutable identities and digests needed
for deterministic reduction. Read-event audit payloads remain metadata-only;
full instructions and reference bodies appear only in typed command outcomes.
This prevents routine audit views from duplicating potentially sensitive
content.

### Capabilities

Policy adds narrowly scoped capabilities for skill reading, creation,
versioning, assignment, and unassignment. There is deliberately no
`SkillExecute` capability.

A skill cannot add or imply another capability. Text that resembles a command,
path, URL, provider, or tool name remains inert content.

### Retrieval contract

The domain exposes deterministic retrieval of exact assigned versions. The
caller supplies limits for skill count, total accepted bytes, and reference
count. Retrieval walks canonical assignment order, includes only complete
bounded sections, and reports omitted counts when a budget is reached.

Retrieval performs no I/O beyond reading local accepted records. Phase 3 may
use this result when constructing an inference request, but Phase 2 does not
send it to a model.

## Persistence

### Schema migration

Migration `0003_declarative_skills.sql` advances the schema version from 2 to
3. Earlier migration files remain unchanged.

The migration adds:

- An immutable skill-version table with unique logical version and digest
  constraints.
- A rebuildable active-skill projection mapping each skill to one exact active
  version.
- Any receipt capability constraint changes required for the new typed
  commands.

Agent profile payloads already reserve skill references. The implementation
replaces the unchecked placeholder reference with the exact typed reference
and permits non-empty references only after validating every target version.

Migration runs transactionally. Injected or real migration failure must leave
the database at schema version 2 with no partial version 3 objects.

### Repositories and transactions

The skill repository supports immutable insertion, exact lookup, normalized
selector lookup, history listing, active projection updates, and integrity
reconciliation.

Assignment transactions:

- Load the expected active agent profile.
- Validate the exact immutable skill reference and digest.
- Enforce one version per skill and the 16-skill limit.
- Insert the resulting immutable agent profile version.
- Move the active profile projection.
- Record the typed audit event and command receipt.

The transaction either commits all records or none.

### Startup reconciliation and recovery

Startup reconciles immutable skill versions before validating agent profile
skill references. Missing authoritative rows may be restored from canonical
records. Altered or unexpected immutable rows fail closed with a typed
integrity error.

Active-skill projections are rebuildable from authoritative skill-version
records and events. Agent assignments rebuild as part of the existing active
profile projection. An empty skill projection must not change legacy
projection digests.

## Error behavior

The feature uses stable typed errors for at least:

- Skill not found.
- Skill version not found.
- Duplicate normalized skill name.
- Invalid or oversized skill content.
- Unchanged version candidate.
- Skill already assigned at the requested version.
- Skill not assigned.
- Agent skill limit exceeded.
- Stale active skill or profile version.
- Invalid review token.
- Immutable skill integrity failure.
- Unsupported capability request.

The TUI translates these into concise corrective messages and preserves the
user's draft whenever retry is safe.

## Compatibility

- Schema version 2 databases migrate to version 3 without changing existing
  profile content or identities.
- Profiles with no skills retain their prior canonical digest behavior.
- Existing commands, receipts, events, setup, audit, shutdown, and
  single-instance protection remain valid.
- Existing agent readiness remains binding-derived; assigning a skill does not
  make an otherwise unready agent ready.
- MCP references remain unsupported in this milestone.

## Verification strategy

Implementation follows test-driven development and covers:

- Domain validation, normalization, canonical ordering, and stable digests.
- Immutable version creation and unchanged-content rejection.
- Exact assignment, upgrade, unassignment, limits, and stale-write handling.
- Built-in manifest identity and digest reconciliation.
- Migration from schema version 2 and migration rollback on failure.
- Transaction rollback and command-receipt replay.
- Startup integrity rejection and deterministic projection rebuild.
- Event wire compatibility and metadata-only audit payloads.
- Slash-command parsing and typed outcomes.
- Keyboard-driven list, detail, editor, history, assignment, and unassignment
  flows.
- Adaptive rendering at narrow, medium, and wide terminal sizes.
- Bare `q` remaining inert and `/quit` remaining the normal shutdown path.
- Regression coverage for all existing test suites.

The final manual path creates a custom skill, creates version 2, assigns
version 1, proves the assignment does not auto-upgrade, explicitly upgrades,
unassigns, restarts the application, and verifies persistence and audit views.

## Acceptance criteria

The milestone is accepted when:

- All four starter skills appear in the Skills workspace.
- A custom skill can be created without colon commands.
- Editing creates immutable history.
- Assignments pin exact versions and survive restart.
- Updating a skill does not change existing assignments.
- Historical assignment, explicit upgrade, and unassignment work.
- Existing databases migrate without profile loss or digest drift.
- Invalid, duplicate, stale, and corrupted states fail safely.
- Audit and recovery remain deterministic.
- The complete automated suite passes.
- The focused manual TUI workflow passes.

