# Phase 2 Milestone 1: Agent Profile Foundation Design

**Status:** Approved in design discussion; awaiting written-spec review  
**Date:** 2026-09-05  
**Base:** `main` at merge commit `d0b35ee`  
**Target branch:** `codex/phase-2-agent-profile-foundation`

## 1. Context

The canonical roadmap in `phases.md` defines Phase 2 as Agent Profiles,
Skills, and Hybrid Memory. This document covers only the first vertical slice:
the durable agent-profile foundation.

Phase 1 already established the Rust application boundary, append-only event
core, SQLite persistence, deterministic recovery, fallback command adapter, and
full-screen Adaptive Cockpit. This milestone extends those boundaries rather
than creating a second profile-specific architecture.

The retired Python deterministic-risk plan remains superseded and is not an
input to this work. Deterministic finance behavior belongs to a later phase in
the canonical Rust roadmap.

This is an offline configuration capability. It does not call a model, connect
to a provider, launch an engineering runtime, load a skill, read memory, or
grant an MCP capability.

## 2. User-visible result

The user can:

- create an agent from a Bull, Bear, Chief, Engineering, or Custom starter;
- customize identity, specialty, personality, and operating instructions;
- activate a valid new profile even when it has no provider binding;
- list and inspect active profiles;
- edit a profile through a guided workflow;
- review an exact field-level diff before activating an edit;
- inspect every immutable prior version; and
- perform the same operations through the TUI and fallback command mode.

New profiles become active when the user confirms the final creation review.
Edits never replace the active version until the user reviews the diff and
confirms the exact candidate.

## 3. Goals

1. Add immutable, deterministic `AgentProfileVersion` records.
2. Preserve one stable profile identity across all versions.
3. Keep active display names unambiguous for future `@Name` routing.
4. Pin exact built-in template provenance without automatic upgrades.
5. Allow active but unbound profiles and report their readiness honestly.
6. Apply profile writes through the existing policy, command, event,
   transaction, projection, and recovery boundaries.
7. Provide native Adaptive Cockpit list, detail, editor, review, and history
   experiences.
8. Preserve fallback-mode behavioral parity without storing draft text in
   command history.
9. Keep roles, personalities, and specialties authority-neutral.
10. Rebuild profile state deterministically from the verified event stream.

## 4. Non-goals

This milestone does not add:

- real provider connections, API keys, OAuth, or model calls;
- real Codex CLI, Claude Code, or other engineering-runtime bindings;
- skill creation, assignment, loading, or execution;
- KV memory, episodic summaries, or memory proposals;
- MCP entries, grants, schemas, or processes;
- rooms, messages, Chief orchestration, or agent execution;
- setup-draft integration;
- profile deletion, archival, duplication, or import/export;
- `$EDITOR` integration;
- policy-profile editing;
- secret fields of any kind; or
- network access in production code or default tests.

## 5. Approved product decisions

The following decisions were approved during design discussion:

1. Creation starts from a versioned template and remains fully editable.
2. An unbound profile may be active, but it is visibly `Not Ready`.
3. Roles are Bull, Bear, Chief, Engineering, and Custom.
4. New profiles activate after final validation and creation confirmation.
5. Edits require an authoritative diff and explicit activation confirmation.
6. The TUI editor is a guided sequence, not one dense form.
7. Active display names are unique under Unicode-aware case-insensitive
   normalization.
8. Existing profiles never change when a built-in template changes.
9. A profile has one primary specialty and at most five secondary tags.
10. The implementation is a dedicated profile aggregate and complete vertical
    slice, not a generic document framework or a combined profiles/skills/memory
    schema.

## 6. Architectural boundaries

The data path remains:

```text
TUI or fallback editor
        |
        | typed draft/query/command
        v
ApplicationService and policy
        |
        | typed event in one transaction
        v
Event repository + immutable profile versions + active projection
        |
        | typed bounded outcome/snapshot
        v
TUI or fallback renderer
```

The `agents` domain owns profile types, validation, canonicalization, template
resolution, diff calculation, and version invariants. It does not know about
terminal widgets, command-line tokens, SQLite, or future provider adapters.

The application layer owns authorization, IDs, timestamps, edit-preview tokens,
command idempotency, transaction orchestration, and typed outcomes.

Persistence stores and rebuilds immutable versions and the active pointer. It
does not decide whether a profile is valid or whether a user may activate it.

Presentation owns only temporary editor state, focus, navigation, and rendering.
It never constructs persistence records or reads a repository directly.

## 7. Domain model

### 7.1 Stable identity and immutable versions

Each logical profile has a stable `AgentProfileId`. Every accepted creation or
edit has a distinct `AgentProfileVersionId` and a monotonically increasing,
nonzero `ProfileVersionNumber`.

An `AgentProfileVersion` contains:

```text
AgentProfileVersion
|- profile_id
|- version_id
|- version_number
|- version_digest
|- display_name
|- description
|- role
|- primary_specialty
|- specialty_tags[]
|- personality
|- operating_instructions
|- inference_binding_ref?
|- engineering_binding_ref?
|- skill_version_refs[]
|- mcp_grant_refs[]
|- memory_namespace
|- policy_profile_ref
|- template_provenance
|- created_at
`- supersedes_version_id?
```

`profile_id` and `memory_namespace` never change across versions. Creation is
version 1 and has no predecessor. Every later version must be exactly one more
than the active version and must name that active version as its predecessor.

Historical versions remain addressable and immutable. An edit that produces no
semantic change is rejected as `no_profile_changes` rather than creating an
empty version.

### 7.2 Roles

`AgentRole` is a closed enum:

- `Bull`
- `Bear`
- `Chief`
- `Engineering`
- `Custom`

The role is descriptive configuration. It does not grant policy capabilities,
provider access, tools, memory, MCP access, filesystem access, or authority.
Changing a role requires a new profile version.

### 7.3 Specialties

Every profile has exactly one nonempty primary specialty and zero to five
secondary specialty tags.

Tags are canonicalized, sorted by normalized key, and unique under the same
case-insensitive comparison used for display names. A tag that normalizes to the
primary specialty is rejected as a duplicate.

Role and specialty are independent. A built-in template supplies a default
specialty, but the user may replace it before creation or in a reviewed edit.
`Custom` requires a user-accepted primary specialty before creation can finish.

### 7.4 Personality and operating instructions

Personality controls voice, perspective, and reasoning emphasis. Operating
instructions describe desired working behavior. Neither field changes policy or
capabilities.

Both are plain bounded Unicode paragraphs, not executable templates or markup.
Embedded terminal controls, escape characters, NUL, bidirectional override
controls, and line-control characters are rejected. Presentation wraps these
fields visually without interpreting their contents.

### 7.5 Bindings and readiness

Profile versions reserve typed optional references for:

- an inference connection and model; and
- an engineering runtime.

The binding step receives only references supplied by a typed application
snapshot. It never accepts a free-form object ID. In this milestone the
production catalog is empty, so the normal selection is `Unbound`. Deterministic
tests may inject placeholder references through a fake resolver.

Readiness is computed and never inferred from profile prose:

- `Unbound`: no required inference binding is selected;
- `BindingUnavailable`: a selected reference is not currently available; and
- `Ready`: every binding required by the profile role is available.

This milestone cannot report a real provider as `Ready`. An active unbound
profile is valid configuration and is rendered as `Active / Not Ready`.

### 7.6 Reserved future fields

`skill_version_refs` and `mcp_grant_refs` are present as typed empty lists.
They are not editable in this milestone. Later assignment or grant changes must
create reviewed profile versions rather than mutate existing versions.

Every profile receives a unique, non-secret memory namespace ID at creation.
No memory records or retrieval behavior are added here.

Every profile pins the built-in default policy-profile reference. The profile
editor cannot change it in this milestone.

## 8. Field normalization and limits

Limits are measured in UTF-8 bytes after normalization:

| Field | Rule |
|---|---|
| Display name | 1 to 64 bytes |
| Description | 0 to 256 bytes |
| Primary specialty | 1 to 64 bytes |
| Secondary specialty tag | 1 to 48 bytes each, at most 5 |
| Personality | 1 to 1,024 bytes |
| Operating instructions | 1 to 4,096 bytes |

Display strings are normalized to a canonical display form. The uniqueness key
uses Unicode compatibility normalization followed by Unicode default case
folding and deterministic whitespace folding. The implementation plan must pin
one Rust normalization dependency and golden vectors so dependency upgrades
cannot silently change stored keys.

Leading and trailing whitespace is removed. Runs of display whitespace become
one ordinary space. Empty normalized values fail validation. Names and tags may
contain international text and emoji when the result remains visible and safe.

The application stores both the display form and the comparison key. Only the
display form is shown to the user. The comparison key is an internal index and
never becomes a routing label.

## 9. Built-in template catalog

The binary contains a closed, versioned catalog for:

- Bull
- Bear
- Chief
- Engineering
- Custom

Each template has a stable `ProfileTemplateId`, positive template version,
canonical content, and SHA-256 digest. Built-in templates contain:

- a default role;
- a default description;
- a default primary specialty and tags;
- bounded personality and operating instructions;
- no provider or runtime binding;
- no skills;
- no MCP grants; and
- the default policy-profile reference.

The Custom template intentionally requires the user to supply its primary
specialty and meaningful personality/instructions before creation.

Creation copies template content into the new profile version and records exact
template provenance. A later template release never modifies, invalidates, or
notifies an existing profile in this milestone. Adopting a newer template is a
normal reviewed edit in a later milestone.

Template IDs, versions, canonical bytes, and digests are covered by golden
tests. Removing or reusing a released template version is forbidden.

## 10. Deterministic digests and diffs

The existing canonical JSON and SHA-256 primitives are reused.

`version_digest` covers every semantic and provenance field of the immutable
version, including stable profile identity, version number, predecessor,
template provenance, bindings, empty future-reference lists, memory namespace,
policy reference, and creation timestamp.

An edit preview computes:

- the expected active version ID and digest;
- the canonical candidate body;
- an ordered list of changed field identifiers;
- old and new values for the explicit review view; and
- a `review_digest` binding the profile ID, base version, candidate body, and
  ordered diff.

Diff order is fixed by the domain schema, not map iteration or edit order.
Reordering equivalent specialty tags does not create a change.

Full old and new values are available only in the explicit review view. Audit
summaries retain changed field names and digests, not profile prose.

## 11. Application operations

### 11.1 Durable commands

The application command surface adds typed operations equivalent to:

- `CreateAgentProfile`
- `ListAgentProfiles`
- `ShowAgentProfile`
- `ActivateAgentProfileRevision`
- `ShowAgentProfileHistory`

Selectors accept a stable profile ID or a normalized active display name.
Ambiguous selectors are impossible because active names are unique.

`CreateAgentProfile` carries the resolved candidate and exact template
provenance. The application resolves the template again, validates every field,
allocates IDs and timestamp, computes the version, and commits it atomically.

`ActivateAgentProfileRevision` carries the complete candidate, expected base
version, review token, and review digest. The application recalculates the
candidate and diff and rejects any mismatch before authorization or mutation.

### 11.2 Passive edit-preview operation

Edit preview is a typed application-boundary operation, not a durable command.
It creates no event, command receipt, profile row, or persistent draft.

The operation validates the candidate, calculates the authoritative diff, and
returns a one-use `ProfileReviewToken` plus `review_digest`. The application
retains only bounded metadata for the pending review:

- token;
- profile ID;
- expected base version ID and digest;
- candidate digest; and
- review digest.

It does not retain the candidate's personality or instructions. Only one review
may be pending for the active presentation session. A newer preview, explicit
cancel, successful activation, shutdown, or process exit invalidates the old
token. Reviews do not survive restart; the user repeats preview after restart.

### 11.3 Typed outcomes

Outcomes include bounded forms of:

- profile created;
- deterministic profile list;
- profile detail;
- edit preview and field diff;
- profile revision activated; and
- immutable version history.

Lists sort by normalized display name and stable profile ID. History is returned
newest-first with a bounded page size. Detail/history pagination cannot allocate
from unbounded caller input.

## 12. Events and audit projection

Mutating domain events are:

- `AgentProfileCreated`
- `AgentProfileVersionActivated`

Their versioned payloads carry the complete canonical profile version required
for deterministic replay. Event decoding rejects unknown fields, invalid
versions, invalid digests, illegal predecessors, and malformed profile content.

Read commands follow the existing typed audit convention with safe events for
list, detail, and history access. Exact event names and schema versions are
locked by contract tests.

Generic audit rows may display:

- profile ID;
- profile version;
- role;
- template ID and version;
- version/review digest prefix;
- changed field identifiers; and
- safe result or rejection code.

Generic audit rows never display:

- personality text;
- operating instructions;
- descriptions or specialty text from rejected input;
- normalized comparison keys;
- full binding references;
- raw parser input;
- filesystem paths; or
- panic payloads.

## 13. Policy model

The deny-wins capability vocabulary adds separate profile capabilities for:

- listing and reading profiles;
- creating a profile;
- previewing an edit; and
- activating a profile revision.

User-originated profile management receives only the exact required capability.
No profile role, specialty, personality phrase, template, or binding reference
can add a capability. Agent-originated profile mutations remain denied because
agent execution is not present and self-modification is outside this milestone.

Policy denial occurs before IDs, clock values, review tokens, or writes are
consumed. Retry and concurrency behavior follows the existing command-receipt
contract.

## 14. Persistence design

The next ordered SQLite migration adds normalized profile projections.

### 14.1 Immutable version table

`agent_profile_versions` stores:

- profile and version IDs;
- positive version number;
- canonical version digest;
- predecessor ID;
- template provenance and digest;
- indexed role and display-name fields;
- canonical profile JSON;
- memory namespace and policy reference; and
- creation timestamp.

Primary, unique, foreign-key, shape, and digest constraints reject duplicate or
incoherent versions. Triggers reject update and delete operations.

### 14.2 Active profile projection

`active_agent_profiles` stores one row per stable profile:

- profile ID;
- active version ID and number;
- active version digest; and
- normalized active display-name key.

The normalized active display-name key is unique. Historical versions may
retain names now used by another profile because future routing resolves only
the active projection.

An active pointer may move only to the next valid immutable version for the
same profile. The application transaction checks this rule, and the repository
boundary independently rejects stale or cross-profile pointers.

### 14.3 Atomic transaction

Profile creation or activation commits all of the following together:

```text
authorization decision
event envelope
immutable profile version
active profile projection
command receipt and exact outcome
```

Any error rolls back every effect and consumes no externally visible partial
state. Replaying the same command ID returns the exact stored outcome. Reusing a
command ID with different content returns the existing deterministic conflict.

## 15. Recovery and corruption behavior

The verified event stream remains authoritative. Startup extends the existing
reducer and recovery coordinator with an `AgentProfilesProjection` containing
immutable versions and active pointers.

Recovery must:

1. verify every profile event before mutating projection state;
2. rebuild missing or stale profile projections transactionally;
3. reproduce canonical version rows and active pointers byte-for-byte;
4. refuse startup when the authoritative event stream is malformed;
5. reject a profile event that skips a version, names the wrong predecessor,
   changes stable identity fields, or collides with an active normalized name;
6. retain the existing process guard throughout verification and rebuild; and
7. expose only a safe startup error category.

Corrupt projection rows are never treated as empty state. A failed rebuild
leaves the previous durable state intact.

## 16. Create and edit workflows

### 16.1 Creation

1. The user selects a built-in template.
2. Presentation creates a memory-only draft from that exact template version.
3. The user edits identity, specialty, personality, instructions, and available
   binding selections.
4. Local typed validation provides immediate field feedback.
5. The final Review screen displays the complete candidate and template
   provenance.
6. `Create and activate` submits one typed creation command.
7. The application independently resolves, validates, authorizes, and commits.
8. The UI opens the new active profile detail or renders typed field errors.

EOF or cancellation before step 6 discards the draft and writes nothing.

### 16.2 Editing

1. The user opens the current active version.
2. Presentation copies it into a memory-only candidate.
3. The user edits fields through the same guided steps.
4. Presentation requests an authoritative preview from the application.
5. The application returns an ordered diff, one-use token, and review digest.
6. The UI displays every changed field and the expected base version.
7. Exact activation confirmation submits the complete candidate, token, digest,
   and base version.
8. The application revalidates and atomically advances the active pointer.

Any intervening activation makes the preview stale. A stale edit never merges
automatically; the user reloads the new active version and repeats the edit.

## 17. Adaptive Cockpit experience

Existing `1` through `4` navigation remains unchanged. Pressing `a` outside the
command editor opens Agents. Inside command entry, `a` remains ordinary text.
`/agent list` also opens the Agents workspace after its typed outcome.

### 17.1 Responsive layouts

- Wide: profile list, selected detail/editor, and contextual inspector are
  visible together.
- Medium: list or detail/editor uses the workspace; history and inspector use
  the existing overlay model.
- Narrow: list, detail, history, editor, and review are separate full-width
  screens with an explicit breadcrumb.
- Too Small: the existing resize guidance and unconditional quit behavior remain.

No profile field, action, readiness state, or validation result depends on color
alone. `NO_COLOR` remains fully supported.

### 17.2 Guided editor

The ordered creation flow is:

```text
Template -> Identity -> Specialty -> Personality -> Instructions
         -> Optional bindings -> Review
```

Single-paragraph fields wrap visually but reject embedded line controls. Enter
accepts the current step, Esc returns to the prior step, and Ctrl+C retains its
global shutdown behavior. Field limits are visible before the user reaches a
boundary. Pasted content follows the same normalization and size rules as typed
content.

Validation errors are attached to stable field identifiers and do not erase the
draft. The review view is scrollable and shows `Create and activate` for a new
profile.

Editing follows the same steps but the last view is an authoritative field diff
with expected base version, digest prefix, and `Activate new version`. A generic
`yes` is not an activation command.

### 17.3 Native profile views

The list view shows normalized-safe display name, role, primary specialty,
active version, and readiness. Sorting is deterministic.

The detail view shows all accepted profile fields, exact template provenance,
safe binding labels, memory namespace ID, policy-profile label, active version,
and digest prefix.

History is newest-first. Selecting a historical version shows immutable content
and its diff from the preceding version. Historical inspection never changes
the active pointer.

## 18. Fallback command-mode experience

Fallback mode supports:

- `/agent create <bull|bear|chief|engineering|custom>`
- `/agent list`
- `/agent show <name-or-id>`
- `/agent edit <name-or-id>`
- `/agent history <name-or-id>`

Create and edit enter a presentation-local guided editor. While that editor is
active, bounded field responses are not parsed as global slash commands and are
not added to command history. Stable colon-prefixed editor controls provide
back, cancel, review, and activation actions. Literal slash-prefixed prose is
therefore safe profile content rather than accidental authority.

The fallback editor uses the same typed draft validator and application
operations as the TUI. It can be driven by an injected line source for headless
tests and scripted command-mode use. EOF or cancellation discards the local
draft without an event.

Creation ends with an exact `create` action. Edit preview prints the ordered
field diff and requires the exact returned review digest for activation. The
adapter retains the complete pending candidate only until activation, cancel,
replacement, EOF, or shutdown.

List, show, and history remain single commands with bounded deterministic text
rendering. Renderers escape all untrusted content and never print an internal
normalized-name key.

## 19. Error handling

Profile failures are typed and stable. Required categories include:

- invalid profile field;
- invalid or unavailable template version;
- duplicate active display name;
- unknown profile;
- no semantic changes;
- stale active version;
- missing, expired, or mismatched review token;
- review digest mismatch;
- binding reference unavailable;
- policy denied;
- projection conflict or corruption; and
- safe persistence failure.

Errors identify a bounded field code when useful but never echo rejected values,
personality, instructions, raw commands, paths, SQL, panic payloads, or complete
binding references.

If a primary application error and terminal-cleanup error occur together, the
existing primary-error precedence remains unchanged.

## 20. Concurrency and idempotency

Creation races for equivalent normalized names have exactly one winner. The
loser returns `duplicate_profile_name` without allocating a committed profile
identity or version.

Concurrent edits of one active base version have exactly one winner. The loser
returns `stale_profile_version`; candidates are never field-merged implicitly.

Command receipts make retried successful creates and activations return the
same IDs, timestamps, digests, events, and outcomes without duplicate versions.
Conflicting reuse of a command ID remains deterministic.

Preview tokens are one-use and process-local. They are not command authority,
secrets, or durable approvals. They only bind an activation request to the exact
candidate and base version that the presentation displayed.

## 21. Testing strategy

The 314 tests passing on merged Phase 1 are the baseline.

### 21.1 Domain and property tests

- every field at below, exact, and above-limit byte boundaries;
- arbitrary Unicode normalization and cursor-safe presentation;
- normalization idempotence and golden comparison keys;
- role/template combinations and Custom requirements;
- specialty sorting, deduplication, and maximum cardinality;
- canonical serialization and digest stability;
- tag input-order invariance;
- deterministic diff order and diff symmetry;
- immutable stable fields across versions; and
- no-op edit rejection.

### 21.2 Persistence and recovery tests

- fresh migration and upgrade from the exact Phase 1 schema;
- migration rollback after every schema-change boundary;
- immutable-row update/delete rejection;
- unique normalized active-name enforcement;
- stale and cross-profile active-pointer rejection;
- event/version/projection/receipt atomic rollback;
- missing, stale, partial, and corrupt projection handling;
- deterministic replay from profile events;
- malformed event and digest refusal; and
- process-guard retention during rebuild.

### 21.3 Application and policy tests

- exact command-to-capability-to-event-to-outcome mapping;
- policy denial before dependency consumption;
- creation and activation idempotency;
- conflicting command-ID behavior;
- passive preview creates no durable effect;
- token replacement, cancellation, mismatch, and one-use behavior;
- candidate/base/diff digest mismatch refusal;
- concurrent duplicate creation and edit races;
- deterministic list and history bounds; and
- role/personality text cannot change authorization.

### 21.4 Presentation tests

- fallback parser grammar and guided editor state transitions;
- quoted selectors, malformed input, EOF, cancellation, and redaction;
- TUI list, detail, editor, review, diff, history, and overlays;
- Wide, Medium, Narrow, and Too Small rendering;
- focus, scroll, resize, Unicode, wrapping, paste, and field limits;
- `a` behavior inside and outside command focus;
- TUI/fallback parity for identical drafts and edits; and
- terminal restoration on success, error, interruption, and panic.

### 21.5 End-to-end and security tests

- create -> restart -> inspect -> edit -> activate -> rebuild;
- two profiles sharing a fake binding while retaining distinct identities,
  personalities, memory namespaces, and policy references;
- rejected and interrupted operations leave no partial profile state;
- generic audit/error output contains no profile prose or raw rejected input;
- control and terminal escape injection cannot spoof UI chrome; and
- production dependency and source scans find no network, provider, model,
  runtime, MCP, skill execution, or secret-storage path.

## 22. Implementation decomposition

The implementation plan should stage this milestone as independently reviewed
vertical boundaries:

1. profile IDs, value objects, normalization, templates, and domain validation;
2. canonical versions, diffs, digests, and property tests;
3. typed events and deterministic profile reducer;
4. SQLite migration, immutable repository, projection, and rebuild behavior;
5. application commands, preview operation, policy, outcomes, and concurrency;
6. fallback commands and guided editor;
7. Adaptive Cockpit profile workspaces and editor;
8. end-to-end hardening, documentation, independent review, and release gates.

Each boundary starts with failing tests and ends with a focused review. Broad
release gates run only after the complete slice is integrated.

## 23. Exit criteria

This milestone is complete only when:

- a user can create all five profile roles through the TUI;
- template-based creation records exact immutable provenance;
- unbound profiles activate and display `Not Ready` without inventing a provider;
- duplicate normalized active names fail deterministically;
- an edit displays an authoritative diff and stale or altered activation fails;
- every accepted edit creates exactly one immutable next version;
- list, detail, editor, diff, and history work at every supported layout;
- fallback and TUI create identical versions from identical normalized drafts;
- restart and event replay reproduce every version and active pointer;
- corruption and transaction failures fail closed without partial state;
- profile prose never appears in generic audit or error output;
- no role, specialty, personality, or template grants a capability;
- no provider, model, runtime, skill, memory, MCP, secret, or network behavior is
  introduced;
- formatting, strict Clippy, all baseline and new tests, independent review, and
  live isolated TUI acceptance pass; and
- the canonical roadmap and user documentation accurately report milestone
  status without marking all of Phase 2 complete.

## 24. Deferred work

The next Phase 2 milestones add declarative versioned skills, then private KV
memory and approval-based memory proposals. Assigning a skill or memory policy
will create a reviewed profile version through the same activation boundary.

Provider connections and real model readiness arrive in Phase 3. First-run
setup reuses these profile validators and templates in Phase 5. Rooms pin exact
profile versions in Phase 6. None of those later phases may bypass the immutable
version, review, policy, or event boundaries defined here.
