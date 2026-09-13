# Phase 2 Milestone 3: Hybrid Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver local plaintext Hybrid Memory with immutable key/value history, Human-reviewed edits, Agent-authored proposals requiring exact Human resolution, source-linked episodic summaries, deterministic bounded retrieval, recovery, and matching fallback/TUI workflows.

**Architecture:** Extend the existing event-sourced single-crate architecture with a `memory` aggregate. Every durable operation remains one typed command, one authoritative event, one immediate SQLite transaction, one receipt, and one safe audit record. Immutable event payloads are authoritative; constrained SQLite mirrors are reconciled from verified events; current pointers and proposal status are rebuildable. Memory belongs to the stable profile namespace, while every producer and retrieval request pins an exact profile version.

**Tech Stack:** Rust 2024, serde and canonical JSON/SHA-256, SQLite through rusqlite, Ratatui/crossterm, proptest, tempfile, and the existing command/event/receipt/audit/recovery infrastructure.

**Spec:** `docs/superpowers/specs/2026-09-07-phase-2-hybrid-memory-design.md`

## Global Constraints

- Work only in `/private/tmp/ai-stock-forum-phase-2-hybrid-memory` on `codex/phase-2-hybrid-memory`.
- Never edit, clean, reset, or otherwise change the user's main checkout at `/Users/nguyen-mini/Documents/dev/ai-stock-forum`.
- Start each production behavior with a focused failing test, observe the intended RED failure, implement the minimum behavior, and rerun the same test GREEN before continuing.
- Commit after every task with the exact commit message shown. Do not combine tasks into one commit.
- Keep `EVENT_SCHEMA_VERSION` and the event digest format at `1`; add tagged event variants without reshaping legacy Human/System event bytes.
- Preserve one command → one authoritative event. Use bounded, deterministically ordered composite memory events for sibling proposal expirations.
- Store memory as plaintext locally. Do not add encryption, key management, credential storage, secret import, secret references, or claims that deletion erases history.
- Reject credential-oriented keys and the exact `CredentialPatternSetV1` patterns before any durable ID, clock read, event, receipt, audit row, or memory row is created.
- Never expose a production proposal-create, episodic-write, or snapshot-build route in fallback or TUI presentation code.
- Only Human may read, preview, directly mutate, approve, or reject memory. Only a matching Agent actor may use the internal proposal command. System receives no memory operation.
- Direct Human edits use process-local one-use review tokens and never create `ApprovalRecord` rows. Agent proposals create exactly one pending `MemoryMutation` approval.
- Preserve exact-replay receipt semantics. A matching command replay returns the original fully materialized outcome without reauthorization, reselection, new IDs, clock reads, or writes.
- Use `BEGIN IMMEDIATE` for every durable memory command, including snapshot selection/materialization.
- Preserve the exact service order: lifecycle check; canonicalize without IDs/clocks; begin immediate transaction; receipt replay/conflict; durable lifecycle/projection verification; actor guard; policy; review reservation; authoritative re-resolution; exact-state/effect/capacity checks; minimal ID/time allocation; one event append; cloned reduction; immutable/approval reconciliation; rebuildable pointer replacement; outcome/audit; one receipt/ref; commit; then review consumption.
- Keep audit, routine status, overview, and errors free of raw keys, values, tags, rationale, labels, summary bodies, rejected input, and SQL details.
- Keep immutable aggregate/reference fields private, expose read-only accessors, and use validating custom deserialization so malformed states cannot bypass constructors.
- Keep all list/history selection bounded in SQL before materializing domain records. Retrieval must stream eligible rows and retain only accepted bounded items plus checked scalar counters.
- Preserve the merged global navigation shortcuts exactly: `1` Overview, `2` Setup, `3` Audit, `4` Help, `a` Agents, `s` Skills. Memory is nested under Agents and has no global shortcut.
- Bare `q` remains inert; `/quit` remains normal shutdown. Editors own unmodified text keys, and modified navigation keys never navigate.
- No default test may require a terminal, network, provider, credential, paid subscription, model, room, or real market data.
- Do not mark `phases.md` complete until the complete offline quality gate and manual acceptance procedure pass.

## Pre-Implementation Baseline Checkpoint

Planning-time verification on 2026-09-07 found no Rust, Cargo, or toolchain
delta between this branch and `origin/main`, and
`cargo test --all-targets --all-features --quiet` passed. The inherited baseline
does not yet pass the other two required gates: pinned Rustfmt reports changes
in 39 existing Rust files, and Clippy with `-D warnings` reports five existing
findings (`too_many_arguments` in two skill-review methods and the Skills view
renderer, plus `large_enum_variant` in the fallback skill workflow and skill
editor effect).

Before Task 1, fetch and compare the latest navigation baseline. If upstream
has not repaired these failures, keep their repair in one explicitly approved,
dedicated prerequisite commit in this isolated worktree; do not hide a mass
format or unrelated lint suppression inside a Hybrid Memory task commit. Rerun
Rustfmt, Clippy, and the full tests after that prerequisite. The final Phase 2
gate may not waive or grandfather these baseline failures.

---

## Canonical Limits and Ordering

- Memory display key: 1–96 UTF-8 bytes after canonicalization.
- Value: 1–4,096 UTF-8 bytes; rationale: at most 512 bytes.
- Purpose tags: at most 8, each 1–32 bytes; normalized `general` is reserved.
- Active logical keys per namespace: 1,024; pending proposals per namespace: 256.
- Episodic label: 1–128 bytes; body: 1–8,192 bytes; sources: 1–128.
- List/history page: 100 rows with exact checked `u64` counts.
- Retrieval hard/default maxima: 32 KV items, 8 episodic summaries, 32,768 canonical JSON bytes, and 128 source references.
- Current entries: normalized key, then stable entry ID. Entry history: object version descending.
- Pending proposals: creation time ascending, then proposal ID. All proposals: creation time descending, then proposal ID.
- Episodic summaries: creation time descending, then summary ID.
- Sibling expiration records: proposal ID ascending.

## Implementation File Map

Create these focused modules and contracts:

```text
migrations/0004_hybrid_memory.sql              — schema-v4 approval/receipt rebuilds and constrained memory tables
src/memory/normalization.rs                    — canonical keys/tags/text and CredentialPatternSetV1
src/memory/entry.rs                            — immutable KV versions, refs, tombstones, and diffs
src/memory/proposal.rs                         — immutable proposals, refs, and terminal resolutions
src/memory/episodic.rs                         — source-linked episodic summary records
src/memory/projection.rs                       — rebuildable current-entry and proposal-status state
src/memory/retrieval.rs                        — bounded deterministic selection and snapshot digests
src/memory/review.rs                           — process-local one-use Human review registry
src/persistence/memory_repository.rs           — authenticated row codecs and bounded SQL reads/writes
src/ui/memory_editor.rs                        — reusable generation-guarded editor state machine
src/ui/tui/views/memory.rs                     — adaptive nested Memory rendering
tests/memory_actor_contract.rs                 — Agent actor and exact profile-reference compatibility
tests/memory_normalization_contract.rs         — canonicalization and credential rejection boundaries
tests/memory_entry_contract.rs                 — immutable version/tombstone invariants
tests/memory_review_contract.rs                — review binding and token lifecycle
tests/memory_proposal_contract.rs              — proposal and resolution invariants
tests/memory_episodic_contract.rs              — summary provenance and source binding
tests/memory_retrieval_contract.rs             — deterministic budgets, ordering, and counters
tests/memory_migration_contract.rs             — fresh/upgrade/rollback schema-v4 behavior
tests/memory_persistence_contract.rs           — exact repository round trips and bounded queries
tests/memory_event_contract.rs                 — legacy-safe tagged memory event vocabulary
tests/memory_projection_contract.rs            — reduction, validation, and byte compatibility
tests/memory_application_contract.rs           — authorization and shared command semantics
tests/memory_application_read_contract.rs      — bounded reads, snapshots, and replay
tests/memory_application_mutation_contract.rs  — passive preview and direct Human mutation
tests/memory_proposal_application_contract.rs  — internal Agent proposal and Human resolution
tests/memory_receipt_contract.rs               — exact replay without new work
tests/memory_audit_contract.rs                 — safe metadata-only audit records
tests/memory_atomicity_contract.rs             — injected rollback at every write boundary
tests/memory_concurrency_contract.rs           — deterministic stale/racing command outcomes
tests/memory_recovery_contract.rs              — no-snapshot and stale-snapshot reconciliation
tests/memory_integrity_contract.rs             — tamper and immutable-row mismatch rejection
tests/memory_isolation_contract.rs             — namespace isolation and copy behavior
tests/memory_editor_contract.rs                — pure editor transitions and generation guards
tests/memory_fallback_contract.rs              — exact grammar, review, retry, and cleanup
tests/memory_tui_controller_contract.rs        — nested navigation and typed effects
tests/memory_tui_host_contract.rs              — runtime routing and delayed-outcome guards
tests/memory_tui_render_contract.rs            — width adaptation, escaping, and warnings
tests/hybrid_memory_acceptance.rs              — cross-host end-to-end Milestone 3 scenarios
docs/testing/phase-2-hybrid-memory.md           — offline operator and manual acceptance procedure
```

Modify the existing integration seams rather than adding parallel infrastructure:

```text
src/domain/{id,error,mod}.rs                         — typed IDs, safe errors, and module exports
src/agents/{profile,projection,mod}.rs               — exact profile-version refs and namespace resolution
src/policy/{capability,approval,mod}.rs               — five capabilities and memory approval transitions
src/app/{command,event,outcome,service,mod}.rs        — typed command/event/outcome orchestration
src/audit/mod.rs                                      — bounded metadata-only memory audit entries
src/persistence/{mod,migrations,database}.rs          — repository export, migration inventory, transaction hooks
src/persistence/{event_repository,projection_repository}.rs — Agent event wire shape and projection storage
src/recovery/{reducer,coordinator}.rs                 — verified-event reduction and mirror reconciliation
src/runtime/mod.rs                                    — passive review and typed memory runtime surface
src/ui/mod.rs                                         — shared editor and view exports
src/ui/command/{mod,parser,renderer,runner}.rs         — fallback grammar, rendering, and reviewed workflows
src/ui/tui/{model,controller,host,render,layout}.rs    — nested state, effects, runtime bridge, layout, and view dispatch
src/ui/tui/views/{mod,agents,help}.rs                  — Agents-owned Memory view and help text
tests/support/mod.rs                                  — deterministic fixtures, fault hooks, and database snapshots
tests/{migration_contract,fallback_fix_round_contract}.rs — legacy migration/navigation regression coverage
README.md                                             — local plaintext/storage disclosure and usage overview
architecture.md                                       — authoritative-event/mirror/recovery architecture
phases.md                                             — Milestone 3 checklist and Phase 2 exit gate
```

---

### Task 1: Add Agent Actors and Exact Profile-Version References Without Legacy Drift

**Files:**

- Modify: `src/domain/id.rs`
- Modify: `src/domain/mod.rs`
- Modify: `src/agents/profile.rs`
- Modify: `src/agents/mod.rs`
- Modify: `src/agents/projection.rs`
- Modify: `src/app/event.rs`
- Modify: `src/persistence/event_repository.rs`
- Modify: `src/ui/command/renderer.rs`
- Modify: `src/ui/tui/views/mod.rs`
- Modify: `tests/fallback_fix_round_contract.rs`
- Create: `tests/memory_actor_contract.rs`

**Interfaces:**

- Consumes: `uuid_id!($name:ident)`; `Actor::{Human, System}`; `AgentProfileVersion::{profile_id, profile_version_id, version, content_digest}(&self)`; `EventRepository::append(&ImmediateTransaction<'_>, PendingEvent) -> Result<EventEnvelope, PersistenceError>`; and `EventRepository::load_all(&Connection) -> Result<Vec<EventEnvelope>, RecoveryError>`.
- Produces: `MemoryEntryId`, `MemoryEntryVersionId`, `MemoryProposalId`, `MemoryReviewToken`, and `EpisodicSummaryId`; `Actor::Agent(AgentProfileId)`; `AgentProfileVersionRef::new(AgentProfileId, AgentProfileVersionId, ObjectVersion, Digest) -> Result<Self, DomainError>`; `AgentProfileVersion::reference(&self) -> AgentProfileVersionRef`; `AgentProfilesProjection::resolve_reference(&self, &AgentProfileVersionRef) -> Result<&AgentProfileVersion, DomainError>`; and `actor_wire(&Actor) -> (&'static str, Option<String>)`.

- [ ] **Step 1: Write the failing actor and provenance contracts**

Cover all three actor database shapes, Agent serde, exact profile-version reference validation, event round-trip, event digest actor-ID binding, invalid kind/ID combinations, and unchanged legacy Human/System canonical bytes and golden digests.

```rust
#[test]
fn agent_actor_id_is_part_of_the_event_digest() {
    let first = pending_with_actor(Actor::Agent(profile_id(1)));
    let second = pending_with_actor(Actor::Agent(profile_id(2)));
    assert_ne!(seal(first).event_digest, seal(second).event_digest);
}

#[test]
fn legacy_human_and_system_event_goldens_do_not_change() {
    assert_eq!(serde_json::to_string(&Actor::Human).unwrap(), "\"Human\"");
    assert_eq!(serde_json::to_string(&Actor::System).unwrap(), "\"System\"");
    assert_eq!(legacy_human_help_digest(), LEGACY_HUMAN_HELP_DIGEST);
    assert_eq!(legacy_system_start_digest(), LEGACY_SYSTEM_START_DIGEST);
}
```

- [ ] **Step 2: Run the actor contracts to verify RED**

Run:

```bash
cargo test --test memory_actor_contract --test event_repository_hardening_contract --test fallback_fix_round_contract
```

Expected RED: compilation fails because `Actor::Agent` and `AgentProfileVersionRef` do not exist.

- [ ] **Step 3: Add typed IDs, Agent actor, and exact profile reference**

Extend the UUID macro with all memory IDs now so downstream tasks do not churn the public vocabulary. The following is the exact Rust interface contract; copy the data definitions and implement every declaration ending in `;` in this step:

```text
uuid_id!(MemoryEntryId);
uuid_id!(MemoryEntryVersionId);
uuid_id!(MemoryProposalId);
uuid_id!(MemoryReviewToken);
uuid_id!(EpisodicSummaryId);

pub enum Actor {
    Human,
    System,
    Agent(AgentProfileId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentProfileVersionRef {
    profile_id: AgentProfileId,
    profile_version_id: AgentProfileVersionId,
    version: ObjectVersion,
    content_digest: Digest,
}

impl AgentProfileVersionRef {
    pub fn new(
        profile_id: AgentProfileId,
        profile_version_id: AgentProfileVersionId,
        version: ObjectVersion,
        content_digest: Digest,
    ) -> Result<Self, DomainError>;
    pub fn profile_id(&self) -> AgentProfileId;
    pub fn profile_version_id(&self) -> AgentProfileVersionId;
    pub fn version(&self) -> ObjectVersion;
    pub fn content_digest(&self) -> &Digest;
}

impl AgentProfilesProjection {
    pub fn resolve_reference(
        &self,
        reference: &AgentProfileVersionRef,
    ) -> Result<&AgentProfileVersion, DomainError>;
}
```

Implement custom `Deserialize` through `AgentProfileVersionRef::new`, `AgentProfileVersion::reference()`, and projection/repository resolution that requires every field to match the authoritative immutable profile version. The namespace is always read from that resolved version.

- [ ] **Step 4: Bind event serialization and SQLite actor columns exactly**

Change event digest material to carry `actor_id: Option<&str>` and derive one canonical pair:

```rust
fn actor_wire(actor: &Actor) -> (&'static str, Option<String>) {
    match actor {
        Actor::Human => ("human", None),
        Actor::System => ("system", None),
        Actor::Agent(id) => ("agent", Some(id.to_string())),
    }
}
```

Insert `actor_id` instead of hard-coded `NULL`. Decode only `(human,NULL)`, `(system,NULL)`, and `(agent,valid profile UUID)`. Reject human/system IDs, missing Agent IDs, unknown kinds, and malformed UUIDs. Update every exhaustive production/test renderer without including Agent profile prose.

- [ ] **Step 5: Rerun GREEN and the legacy event regression set**

```bash
cargo test --test memory_actor_contract --test event_repository_contract --test event_repository_hardening_contract --test agent_profile_event_contract --test skill_event_contract --test recovery_contract --test fallback_fix_round_contract
```

- [ ] **Step 6: Commit**

```bash
git add src/domain src/agents src/app/event.rs src/persistence/event_repository.rs src/ui/command/renderer.rs src/ui/tui/views/mod.rs tests/memory_actor_contract.rs tests/fallback_fix_round_contract.rs
git commit -m "feat: add exact agent actor provenance"
```

### Task 2: Implement Memory Canonicalization and Credential Rejection

**Files:**

- Create: `src/memory/normalization.rs`
- Create: `src/memory/entry.rs`
- Modify: `src/memory/mod.rs`
- Modify: `src/domain/error.rs`
- Create: `tests/memory_normalization_contract.rs`

**Interfaces:**

- Consumes: `UnicodeNormalization::nfkc`, `UnicodeCaseFold::case_fold`, the established profile/skill rules for whitespace folding, line-ending normalization, and unsafe terminal characters, `canonical_json_bytes<T: Serialize>(&T) -> Result<Vec<u8>, DomainError>`, and `DomainError::code(&self) -> &'static str`. It does not consume or pretend to overload either existing profile/skill `canonicalize_visible_text` signature.
- Produces: `pub(crate) fn canonicalize_memory_single_line(field: &'static str, value: &str, min_bytes: usize, max_bytes: usize) -> Result<String, DomainError>`; `pub(crate) fn canonicalize_memory_multiline(field: &'static str, value: &str, min_bytes: usize, max_bytes: usize) -> Result<String, DomainError>`; `NormalizedMemoryKey::new(&str) -> Result<Self, DomainError>`; `normalize_memory_key(&str) -> Result<NormalizedMemoryKey, DomainError>`; `MemoryEntryDraft::new(String, String, Vec<String>) -> Result<Self, DomainError>` plus its accessors; `CredentialPatternSetV1::validate(PlaintextField, &str) -> Result<(), DomainError>`; `validate_plaintext(u16, PlaintextField, &str) -> Result<(), DomainError>`; `PLAINTEXT_VALIDATION_VERSION_V1: u16`; and content-free `DomainError::{InvalidMemoryField { field }, UnsafeMemoryText { field }}` variants with stable codes.

- [ ] **Step 1: Write failing boundary, property, and credential-pattern tests**

Test every exact byte boundary; CRLF/CR normalization; internal key whitespace folding; NFKC/case-fold identity; tag sorting and duplicate rejection; tab, C0/C1, escape, NUL, bidi, and unsafe-line-separator rejection; reserved `general`; every reserved credential-key separator/plural spelling; and exact positive/near-miss/delimiter cases for all three pattern families.

```rust
fn valid_drafts() -> impl Strategy<Value = (String, String, Vec<String>)> {
    ("[A-Za-z][A-Za-z0-9 ]{0,20}", "[A-Za-z0-9 .,!?]{1,64}")
        .prop_map(|(key, value)| (key, value, vec!["analysis".to_owned()]))
}

proptest! {
    #[test]
    fn canonical_memory_drafts_round_trip_without_digest_drift(
        (key, value, tags) in valid_drafts()
    ) {
        let canonical = MemoryEntryDraft::new(key, value, tags).unwrap();
        let bytes = canonical_json_bytes(&canonical).unwrap();
        let decoded: MemoryEntryDraft = serde_json::from_slice(&bytes).unwrap();
        prop_assert_eq!(decoded, canonical);
    }
}

#[test]
fn credential_pattern_v1_is_delimiter_aware_and_versioned() {
    assert!(CredentialPatternSetV1::validate(
        PlaintextField::MemoryValue,
        "sk-ant-abcdefghijklmnopqrst",
    ).is_err());
    assert!(CredentialPatternSetV1::validate(
        PlaintextField::MemoryValue,
        "task-ant-abcdefghijklmnopqrst-note",
    ).is_ok());
    assert!(validate_plaintext(
        99,
        PlaintextField::MemoryValue,
        "ordinary prose",
    ).is_err());
}
```

- [ ] **Step 2: Run the normalization contract to verify RED**

Run:

```bash
cargo test --test memory_normalization_contract
```

Expected RED: compilation fails because the memory normalization and draft APIs are absent.

- [ ] **Step 3: Implement canonical value types and stable safe errors**

The following is the exact Rust interface contract; copy the data definitions and implement every declaration ending in `;` in this step:

```text
pub const PLAINTEXT_VALIDATION_VERSION_V1: u16 = 1;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct NormalizedMemoryKey(String);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryDraft {
    display_key: String,
    value: String,
    purpose_tags: Vec<String>,
}

impl NormalizedMemoryKey {
    pub fn new(display_key: &str) -> Result<Self, DomainError>;
    pub fn as_str(&self) -> &str;
}

impl MemoryEntryDraft {
    pub fn new(
        display_key: String,
        value: String,
        purpose_tags: Vec<String>,
    ) -> Result<Self, DomainError>;
    pub fn display_key(&self) -> &str;
    pub fn normalized_key(&self) -> NormalizedMemoryKey;
    pub fn value(&self) -> &str;
    pub fn purpose_tags(&self) -> &[String];
}

pub fn normalize_memory_key(display_key: &str) -> Result<NormalizedMemoryKey, DomainError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaintextField {
    MemoryValue,
    ProposalRationale,
    EpisodicLabel,
    EpisodicBody,
}
```

Expose validating constructors; keep invalid states unconstructible; use static `DomainError::code()` values without rejected content. Reuse the existing profile NFKC/full-case-fold behavior for comparable keys and the existing tag convention for purpose tags.

Implement the two memory-local canonicalizers instead of changing the existing
profile or skill helper signatures. Both normalize CRLF/CR to LF before byte
checks, enforce the inclusive `min_bytes..=max_bytes` range, and return only
the static field label in `InvalidMemoryField` or `UnsafeMemoryText`.
`canonicalize_memory_single_line` rejects LF plus NUL, tab, the remaining
C0/C1 controls, escape, bidi controls, and unsafe Unicode line separators, then
folds safe whitespace. `canonicalize_memory_multiline` preserves normalized LF
but rejects the same remaining unsafe characters and does not fold content
across lines. Use single-line mode for keys/tags/labels and multiline mode for
values, proposal rationale, and episodic bodies. This reproduces established
behavior without coupling memory errors to `ProfileField` or `SkillField`.

Implement custom `Deserialize` for `NormalizedMemoryKey` through
`normalize_memory_key` and for `MemoryEntryDraft` through
`MemoryEntryDraft::new`; add
`display_key()`, `value()`, and `purpose_tags()` accessors. Negative serde tests
must reject unnormalized keys/tags, duplicate tags, reserved keys, controls, and
credential material rather than merely rejecting them through direct
constructors.

For credential-oriented key rejection, derive comparison tokens from the normalized key by folding each run of ASCII space, `-`, `_`, `.`, `:`, `/`, or `\\` to one space. Reject exactly `password(s)`, `passphrase(s)`, `api key(s)`, `access token(s)`, `refresh token(s)`, `session token(s)`, `private key(s)`, `secret(s)`, and `credential(s)`; do not use substring matching that would reject benign keys such as `secretary`.

- [ ] **Step 4: Implement the exact `CredentialPatternSetV1` scanner**

The scanner declaration is an exact Rust interface contract; implement its body before adding the executable dispatcher below:

```text
pub struct CredentialPatternSetV1;

impl CredentialPatternSetV1 {
    pub fn validate(field: PlaintextField, value: &str) -> Result<(), DomainError>;
}
```

```rust
pub fn validate_plaintext(
    validation_version: u16,
    field: PlaintextField,
    value: &str,
) -> Result<(), DomainError> {
    match validation_version {
        1 => CredentialPatternSetV1::validate(field, value),
        _ => Err(DomainError::UnknownPlaintextValidationVersion),
    }
}
```

Scan canonical UTF-8 bytes without regex or external data and reject exactly:

1. ASCII-case-insensitive `-----BEGIN `, optional `RSA `, `DSA `, `EC `, or `OPENSSH `, then `PRIVATE KEY-----`.
2. ASCII-case-insensitive `authorization`, optional ASCII space, `:`, optional ASCII space, ASCII-case-insensitive `bearer`, at least one ASCII space, then 16 through the enclosing field maximum characters from `[A-Za-z0-9._~+/=-]`.
3. A field/token-delimited ASCII-case-insensitive `sk-`, `sk-ant-`, or `xai-` prefix followed by 20 through the enclosing field maximum characters from `[A-Za-z0-9_-]`. A token character immediately before or after makes it a near miss rather than a match.

Run the scanner after control-safe canonicalization and before durable work. Validate recovered records with their stored version; never rescan v1 records under a future rule set.

- [ ] **Step 5: Rerun the memory normalization contract to verify GREEN**

```bash
cargo test --test memory_normalization_contract
```

- [ ] **Step 6: Refactor only shared canonicalization helpers and run regressions**

```bash
cargo test --test agent_profile_domain_contract --test skill_domain_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/memory src/domain/error.rs tests/memory_normalization_contract.rs
git commit -m "feat: validate plaintext memory content"
```

### Task 3: Build Immutable KV Versions, Tombstones, Diffs, and Human Reviews

**Files:**

- Modify: `src/memory/entry.rs`
- Create: `src/memory/proposal.rs` with the exact proposal reference type used by accepted-entry provenance
- Create: `src/memory/review.rs`
- Modify: `src/memory/mod.rs`
- Modify: `src/domain/error.rs`
- Create: `tests/memory_entry_contract.rs`
- Create: `tests/memory_review_contract.rs`

**Interfaces:**

- Consumes: `AgentProfileVersionRef`, `Actor`, `MemoryEntryDraft`, `NormalizedMemoryKey`, `ObjectVersion::new(u64) -> Result<Self, DomainError>`, `canonical_json_bytes<T: Serialize>(&T) -> Result<Vec<u8>, DomainError>`, `Digest`, `EventId`, `CommandId`, `Mutex<T>`, and `MutexGuard<'a, T>`.
- Produces: `MemoryProposalRef::new(MemoryProposalId, ObjectVersion, Digest) -> Result<Self, DomainError>`; `MemoryEntryVersion::{create_present, next_present, next_deleted, from_parts, reference}` with the exact signatures in Step 3; the exact `MemoryEditReviewBinding::new` constructor in Step 4; `MemoryReviewRegistry::{replace_direct, reserve_direct, cancel, finish}`; and `ReservedMemoryReview::{consume, release, invalidate} -> Result<(), DomainError>` with the exact ownership transitions in Step 4.

- [ ] **Step 1: Write failing immutable-version and review-registry tests**

Cover create, overwrite, delete, tombstone recreation, exact predecessor adjacency, same logical entry ID, canonical display-key changes, digest stability, typed no-ops, stale expected states, malformed serde, one-use tokens, flow ownership, Actor::Human-only reservation, candidate/action swapping, cancellation, recoverable release, terminal invalidation, successful consumption, preview-versus-confirm serialization, and cancel-versus-confirm serialization.

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ExpectedMemoryEntryState {
    Absent,
    Present(MemoryEntryRef),
    Deleted(MemoryEntryRef),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryNoChange {
    IdenticalContent,
    AlreadyAbsent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryMutationKind {
    Set,
    Delete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryField {
    DisplayKey,
    State,
    Value,
    PurposeTags,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryFieldDiff {
    pub field: MemoryField,
    pub before: MemoryFieldValue,
    pub after: MemoryFieldValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryFieldValue {
    Missing,
    Text(String),
    Tags(Vec<String>),
    State(MemoryEntryState),
}

#[test]
fn delete_then_recreate_preserves_logical_id_and_advances_exactly_one_version() {
    let present = present_entry_fixture();
    let deleted = present.next_deleted(next_version_id(), human(), 20, None, event_id(2)).unwrap();
    let recreated = deleted.next_present(
        next_version_id(),
        replacement_draft(),
        human(),
        30,
        None,
        event_id(3),
    ).unwrap();
    assert_eq!(recreated.reference().entry_id(), present.reference().entry_id());
    assert_eq!(recreated.reference().version().get(), present.reference().version().get() + 2);
    assert_eq!(recreated.predecessor_version_id(), Some(deleted.reference().entry_version_id()));
}
```

- [ ] **Step 2: Run the entry and review contracts to verify RED**

Run:

```bash
cargo test --test memory_entry_contract --test memory_review_contract
```

Expected RED: compilation fails because immutable entry and memory review APIs do not exist.

- [ ] **Step 3: Implement exact entry references and immutable records**

The following data definitions are executable Rust:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryEntryState {
    Present,
    Deleted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryEntryRef {
    namespace_id: MemoryNamespaceId,
    entry_id: MemoryEntryId,
    entry_version_id: MemoryEntryVersionId,
    version: ObjectVersion,
    normalized_key: NormalizedMemoryKey,
    state: MemoryEntryState,
    content_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryEntryVersion {
    namespace_id: MemoryNamespaceId,
    entry_id: MemoryEntryId,
    entry_version_id: MemoryEntryVersionId,
    version: ObjectVersion,
    predecessor_version_id: Option<MemoryEntryVersionId>,
    display_key: String,
    normalized_key: NormalizedMemoryKey,
    state: MemoryEntryState,
    value: Option<String>,
    purpose_tags: Vec<String>,
    created_by: Actor,
    created_at_ms: i64,
    accepted_proposal: Option<MemoryProposalRef>,
    plaintext_validation_version: u16,
    creation_event_id: EventId,
    content_digest: Digest,
    record_digest: Digest,
}
```

Define the small reference before the full proposal aggregate so entry records can bind exact accepted provenance without a temporary ID-only representation. The following is the exact Rust interface contract; copy the data definition and implement every declaration ending in `;`:

```text
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryProposalRef {
    proposal_id: MemoryProposalId,
    version: ObjectVersion,
    content_digest: Digest,
}

impl MemoryProposalRef {
    pub fn new(
        proposal_id: MemoryProposalId,
        version: ObjectVersion,
        content_digest: Digest,
    ) -> Result<Self, DomainError>;
    pub fn proposal_id(&self) -> MemoryProposalId;
    pub fn version(&self) -> ObjectVersion;
    pub fn content_digest(&self) -> &Digest;
}

impl MemoryEntryRef {
    pub fn namespace_id(&self) -> MemoryNamespaceId;
    pub fn entry_id(&self) -> MemoryEntryId;
    pub fn entry_version_id(&self) -> MemoryEntryVersionId;
    pub fn version(&self) -> ObjectVersion;
    pub fn normalized_key(&self) -> &NormalizedMemoryKey;
    pub fn state(&self) -> MemoryEntryState;
    pub fn content_digest(&self) -> &Digest;
}

impl MemoryEntryVersion {
    pub fn create_present(
        namespace_id: MemoryNamespaceId,
        entry_id: MemoryEntryId,
        entry_version_id: MemoryEntryVersionId,
        draft: MemoryEntryDraft,
        created_by: Actor,
        created_at_ms: i64,
        accepted_proposal: Option<MemoryProposalRef>,
        creation_event_id: EventId,
    ) -> Result<Self, DomainError>;
    pub fn next_present(
        &self,
        entry_version_id: MemoryEntryVersionId,
        draft: MemoryEntryDraft,
        created_by: Actor,
        created_at_ms: i64,
        accepted_proposal: Option<MemoryProposalRef>,
        creation_event_id: EventId,
    ) -> Result<Self, DomainError>;
    pub fn next_deleted(
        &self,
        entry_version_id: MemoryEntryVersionId,
        created_by: Actor,
        created_at_ms: i64,
        accepted_proposal: Option<MemoryProposalRef>,
        creation_event_id: EventId,
    ) -> Result<Self, DomainError>;
    pub fn reference(&self) -> MemoryEntryRef;
    pub fn display_key(&self) -> &str;
    pub fn value(&self) -> Option<&str>;
    pub fn purpose_tags(&self) -> &[String];
    pub fn predecessor_version_id(&self) -> Option<MemoryEntryVersionId>;
    pub fn created_by(&self) -> &Actor;
    pub fn created_at_ms(&self) -> i64;
    pub fn accepted_proposal(&self) -> Option<&MemoryProposalRef>;
    pub fn plaintext_validation_version(&self) -> u16;
    pub fn creation_event_id(&self) -> EventId;
    pub fn content_digest(&self) -> &Digest;
    pub fn record_digest(&self) -> &Digest;

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        namespace_id: MemoryNamespaceId,
        entry_id: MemoryEntryId,
        entry_version_id: MemoryEntryVersionId,
        version: ObjectVersion,
        predecessor_version_id: Option<MemoryEntryVersionId>,
        display_key: String,
        normalized_key: NormalizedMemoryKey,
        state: MemoryEntryState,
        value: Option<String>,
        purpose_tags: Vec<String>,
        created_by: Actor,
        created_at_ms: i64,
        accepted_proposal: Option<MemoryProposalRef>,
        plaintext_validation_version: u16,
        creation_event_id: EventId,
        content_digest: Digest,
        record_digest: Digest,
    ) -> Result<Self, DomainError>;
}
```

Provide separate `create_present`, `next_present`, and `next_deleted` constructors. Custom `Deserialize` for `MemoryProposalRef`, `MemoryEntryRef`, and `MemoryEntryVersion` calls context-free structural constructors. `MemoryEntryVersion::from_parts` validates version/predecessor nullability, key canonicality, state/value shape, Human creation actor, validation version, and recomputed content/record digests. It cannot prove predecessor adjacency from one record, so Task 7 repository reads and Task 12 recovery must load and validate the exact predecessor before trusting a later version. Approved proposal provenance belongs in `accepted_proposal`.

- [ ] **Step 4: Implement pure preview diff and the process-local registry**

The following is the exact Rust interface and transition contract. Copy the data definitions and concrete method bodies, and implement every declaration ending in `;` in this step:

```text
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryEditReview {
    pub profile: AgentProfileVersionRef,
    pub namespace_id: MemoryNamespaceId,
    pub expected: ExpectedMemoryEntryState,
    pub operation: MemoryMutationKind,
    pub candidate: Option<MemoryEntryDraft>,
    pub diff: Vec<MemoryFieldDiff>,
    pub plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
    pub review_token: MemoryReviewToken,
    pub review_digest: Digest,
}

pub const MEMORY_PLAINTEXT_WARNING: &str =
    "Plaintext local memory — do not store credentials; history is retained after overwrite or delete";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryPlaintextAcknowledgement {
    LocalPlaintextHistoryV1,
}

impl MemoryPlaintextAcknowledgement {
    pub fn warning(&self) -> &'static str { MEMORY_PLAINTEXT_WARNING }
}

pub struct MemoryReviewRegistry {
    operation: Mutex<()>,
    state: Mutex<Option<MemoryReviewRegistration>>,
}

struct MemoryReviewRegistration {
    token: MemoryReviewToken,
    binding: MemoryEditReviewBinding,
    state: MemoryReviewState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemoryEditReviewBinding {
    actor: Actor,
    profile: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    expected: ExpectedMemoryEntryState,
    operation: MemoryMutationKind,
    candidate_digest: Option<Digest>,
    plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
    review_digest: Digest,
}

impl MemoryEditReviewBinding {
    pub(crate) fn new(
        actor: Actor,
        profile: AgentProfileVersionRef,
        namespace_id: MemoryNamespaceId,
        expected: ExpectedMemoryEntryState,
        operation: MemoryMutationKind,
        candidate_digest: Option<Digest>,
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
        review_digest: Digest,
    ) -> Result<Self, DomainError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MemoryReviewState {
    Available,
    Reserved(CommandId),
}

pub struct ReservedMemoryReview<'a> {
    registry: &'a MemoryReviewRegistry,
    token: MemoryReviewToken,
    command_id: CommandId,
    _ownership: MutexGuard<'a, ()>,
    finished: bool,
}

impl MemoryReviewRegistry {
    pub(crate) fn replace_direct(
        &self,
        token: MemoryReviewToken,
        binding: MemoryEditReviewBinding,
    );
    pub(crate) fn reserve_direct(
        &self,
        command_id: CommandId,
        token: MemoryReviewToken,
        supplied: &MemoryEditReviewBinding,
    ) -> Result<ReservedMemoryReview<'_>, DomainError>;
    pub(crate) fn cancel(&self) -> Result<(), DomainError>;
    pub(crate) fn finish(&self);
}

impl ReservedMemoryReview<'_> {
    pub(crate) fn command_id(&self) -> CommandId { self.command_id }
    pub(crate) fn token(&self) -> MemoryReviewToken { self.token }

    pub(crate) fn consume(mut self) -> Result<(), DomainError> {
        self.registry.transition_reserved(self.command_id, self.token, None)?;
        self.finished = true;
        Ok(())
    }

    pub(crate) fn release(mut self) -> Result<(), DomainError> {
        self.registry.transition_reserved(
            self.command_id,
            self.token,
            Some(MemoryReviewState::Available),
        )?;
        self.finished = true;
        Ok(())
    }

    pub(crate) fn invalidate(self) -> Result<(), DomainError> { self.consume() }
}

impl Drop for ReservedMemoryReview<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.registry.release_if_owned(self.command_id, self.token);
        }
    }
}
```

Implement both private helpers in `MemoryReviewRegistry` in this task:

```rust
impl MemoryReviewRegistry {
    fn transition_reserved(
        &self,
        command_id: CommandId,
        token: MemoryReviewToken,
        next: Option<MemoryReviewState>,
    ) -> Result<(), DomainError> {
        let mut slot = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let registration = slot
            .as_mut()
            .ok_or(DomainError::MemoryReviewUnavailable)?;
        if registration.token != token
            || registration.state != MemoryReviewState::Reserved(command_id)
        {
            return Err(DomainError::MemoryReviewUnavailable);
        }
        match next {
            Some(state) => registration.state = state,
            None => *slot = None,
        }
        Ok(())
    }

    fn release_if_owned(&self, command_id: CommandId, token: MemoryReviewToken) {
        let mut slot = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(registration) = slot.as_mut()
            && registration.token == token
            && registration.state == MemoryReviewState::Reserved(command_id)
        {
            registration.state = MemoryReviewState::Available;
        }
    }
}
```

Add the content-free `DomainError::MemoryReviewUnavailable` variant and re-export both binding types from `memory::mod` with `pub(crate) use` so `app::service` can construct them without exposing them publicly.

No-op detection returns before token/ID/time generation. Bind Actor, owning flow/command intent, exact profile reference, derived namespace, expected state, action, and candidate digest. Replacement preview invalidates the previous token.

`replace_direct`, `replace_resolution`, `cancel`, and `finish` acquire `operation` before touching `state`. Each reserve method acquires that same ownership mutex before validating state and moves the guard into `ReservedMemoryReview`; `consume`, `release`, or `invalidate` transitions state while still holding it and then drops the guard. Therefore a preview replacement or cancellation cannot interleave with the confirmation transaction.

- [ ] **Step 5: Run the immutable-entry contract to verify GREEN**

```bash
cargo test --test memory_entry_contract
```

- [ ] **Step 6: Run the review-registry contract to verify GREEN**

```bash
cargo test --test memory_review_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/memory src/domain/error.rs tests/memory_entry_contract.rs tests/memory_review_contract.rs
git commit -m "feat: add immutable reviewed memory entries"
```

### Task 4: Model Agent Proposals, Exact Resolution, and Memory Projection State

**Files:**

- Modify: `src/memory/proposal.rs`
- Create: `src/memory/projection.rs`
- Modify: `src/memory/review.rs`
- Modify: `src/memory/mod.rs`
- Modify: `src/policy/approval.rs`
- Modify: `src/policy/mod.rs`
- Modify: `src/domain/error.rs`
- Create: `tests/memory_proposal_contract.rs`
- Create: `tests/memory_projection_contract.rs`

**Interfaces:**

- Consumes: `AgentProfileVersionRef`, `AgentProfileVersion`, `Actor`, `MemoryEntryDraft`, `NormalizedMemoryKey`, `ExpectedMemoryEntryState`, `MemoryProposalRef`, `MemoryReviewRegistry`, `ApprovalId`, `ApprovalStatus`, and `ApprovalRecord`.
- Produces: `MemoryProposal::{new, from_parts, reference, object_ref}` with the exact signatures in Step 3; `MemoryProposalResolution::new(MemoryProposalRef, MemoryProposalStatus, ApprovalId, Actor, i64, EventId) -> Result<Self, DomainError>`; the exact `MemoryResolutionReviewBinding::new` constructor in Step 4; `MemoryReviewRegistry::{replace_resolution, reserve_resolution}`; `ApprovalAction::MemoryMutation`; `ApprovalRecord::resolve(&self, ApprovalStatus, Actor, i64) -> Result<Self, ApprovalError>`; and `MemoryProjection::{from_parts, apply_entry, create_proposal, resolve_proposal}` with the exact signatures in Step 5.

- [ ] **Step 1: Write failing proposal, resolution, and projection tests**

Cover exact proposer reference/actor match, namespace derivation, fixed version 1, set/delete shapes, no-effect rejection, rationale limit/scanning, pending capacity, exact approval binding, accepted/rejected/expired-only memory terminal status, Human-only memory resolution, cancellation rejection, single resolution, action-specific approve/reject reviews, sorted unique sibling expiration records, current tombstone retention, bounded proposal metadata, and deterministic projection validation.

```rust
#[test]
fn approve_and_reject_reviews_are_not_interchangeable() {
    let pending = pending_proposal_fixture();
    let approve = resolution_review(&pending, MemoryResolutionAction::Approve).unwrap();
    let reject = resolution_review(&pending, MemoryResolutionAction::Reject).unwrap();
    assert_ne!(approve.review_digest, reject.review_digest);
    assert!(MemoryProjection::default().resolve_proposal(&terminal_without_creation()).is_err());
}
```

- [ ] **Step 2: Run the proposal contracts to verify RED**

Run:

```bash
cargo test --test memory_proposal_contract --test memory_projection_contract
```

Expected RED: compilation fails because proposal, resolution, and projection types do not exist.

- [ ] **Step 3: Implement immutable proposal and reference types**

The following is the exact Rust interface contract; copy the data definitions and implement every declaration ending in `;` in this step:

```text
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum MemoryProposalOperation {
    Set { candidate: MemoryEntryDraft },
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryProposal {
    proposal_id: MemoryProposalId,
    version: ObjectVersion,
    proposer: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    operation: MemoryProposalOperation,
    display_key: String,
    normalized_key: NormalizedMemoryKey,
    expected: ExpectedMemoryEntryState,
    rationale: String,
    plaintext_validation_version: u16,
    created_at_ms: i64,
    creation_event_id: EventId,
    approval_id: ApprovalId,
    content_digest: Digest,
    record_digest: Digest,
}

impl MemoryProposal {
    pub fn new(
        proposal_id: MemoryProposalId,
        proposer: &AgentProfileVersion,
        actor: &Actor,
        operation: MemoryProposalOperation,
        display_key: String,
        expected: ExpectedMemoryEntryState,
        rationale: String,
        created_at_ms: i64,
        creation_event_id: EventId,
        approval_id: ApprovalId,
    ) -> Result<Self, DomainError>;
    pub fn reference(&self) -> MemoryProposalRef;
    pub fn object_ref(&self) -> Result<ObjectRef, DomainError>;
    pub fn proposer(&self) -> &AgentProfileVersionRef;
    pub fn namespace_id(&self) -> MemoryNamespaceId;
    pub fn operation(&self) -> &MemoryProposalOperation;
    pub fn display_key(&self) -> &str;
    pub fn normalized_key(&self) -> &NormalizedMemoryKey;
    pub fn expected(&self) -> &ExpectedMemoryEntryState;
    pub fn rationale(&self) -> &str;
    pub fn plaintext_validation_version(&self) -> u16;
    pub fn created_at_ms(&self) -> i64;
    pub fn creation_event_id(&self) -> EventId;
    pub fn approval_id(&self) -> ApprovalId;
    pub fn content_digest(&self) -> &Digest;
    pub fn record_digest(&self) -> &Digest;

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        proposal_id: MemoryProposalId,
        version: ObjectVersion,
        proposer: AgentProfileVersionRef,
        namespace_id: MemoryNamespaceId,
        operation: MemoryProposalOperation,
        display_key: String,
        normalized_key: NormalizedMemoryKey,
        expected: ExpectedMemoryEntryState,
        rationale: String,
        plaintext_validation_version: u16,
        created_at_ms: i64,
        creation_event_id: EventId,
        approval_id: ApprovalId,
        content_digest: Digest,
        record_digest: Digest,
    ) -> Result<Self, DomainError>;
}
```

Custom `Deserialize` calls `from_parts`, which enforces fixed version 1, operation/expected-state shape, canonical text, validation version, and both digests without pretending to resolve external state. Fresh creation uses `new`, resolves `AgentProfileVersion`, and rejects Human/System proposers or Agent IDs that differ from `proposer.profile_id()`. Repository/service/recovery subsequently resolve the embedded profile ref and prove its namespace. Delete must expect a current present ref. Set may expect absent, present, or tombstone.

- [ ] **Step 4: Implement terminal resolutions and action-specific review binding**

The following is the exact Rust interface contract; copy the data definitions and implement every declaration ending in `;` in this step:

```text
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryProposalStatus { Pending, Accepted, Rejected, Expired }
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryResolutionAction { Approve, Reject }
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryProposalFilter { Pending, All }
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryProposalOperationKind { Set, Delete }

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryProposalResolution {
    proposal: MemoryProposalRef,
    status: MemoryProposalStatus,
    approval_id: ApprovalId,
    resolved_by: Actor,
    resolved_at_ms: i64,
    resolution_event_id: EventId,
}

impl MemoryProposalResolution {
    pub fn new(
        proposal: MemoryProposalRef,
        status: MemoryProposalStatus,
        approval_id: ApprovalId,
        resolved_by: Actor,
        resolved_at_ms: i64,
        resolution_event_id: EventId,
    ) -> Result<Self, DomainError>;
    pub fn proposal(&self) -> &MemoryProposalRef;
    pub fn status(&self) -> MemoryProposalStatus;
    pub fn approval_id(&self) -> ApprovalId;
    pub fn resolved_by(&self) -> &Actor;
    pub fn resolved_at_ms(&self) -> i64;
    pub fn resolution_event_id(&self) -> EventId;
}

enum MemoryReviewBinding {
    Direct(MemoryEditReviewBinding),
    Resolution(MemoryResolutionReviewBinding),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemoryResolutionReviewBinding {
    actor: Actor,
    action: MemoryResolutionAction,
    proposal: MemoryProposalRef,
    approval_id: ApprovalId,
    expected_approval_status: ApprovalStatus,
    expected_entry: ExpectedMemoryEntryState,
    plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
    review_digest: Digest,
}

impl MemoryResolutionReviewBinding {
    pub(crate) fn new(
        actor: Actor,
        action: MemoryResolutionAction,
        proposal: MemoryProposalRef,
        approval_id: ApprovalId,
        expected_approval_status: ApprovalStatus,
        expected_entry: ExpectedMemoryEntryState,
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
        review_digest: Digest,
    ) -> Result<Self, DomainError>;
}
```

Change `MemoryReviewRegistration.binding` from `MemoryEditReviewBinding` to `MemoryReviewBinding`, wrapping existing direct registrations in `Direct`. Add `ApprovalAction::MemoryMutation`. Keep generic `ApprovalStatus::Cancelled`, but add no memory cancellation command/event. Only Human can construct terminal memory resolutions. Reuse `MemoryReviewToken` with disjoint bindings so approve cannot authorize reject. Both direct and resolution review digests bind `MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1`; confirmation of that digest is the explicit acknowledgement, and no command accepts an unbound Boolean supplied by presentation code.

Extend the Task 3 registry with the resolution-specific calls used by Tasks 11, 16, and 18:

```text
impl MemoryReviewRegistry {
    pub(crate) fn replace_resolution(
        &self,
        token: MemoryReviewToken,
        binding: MemoryResolutionReviewBinding,
    );
    pub(crate) fn reserve_resolution(
        &self,
        command_id: CommandId,
        token: MemoryReviewToken,
        supplied: &MemoryResolutionReviewBinding,
    ) -> Result<ReservedMemoryReview<'_>, DomainError>;
}
```

Add an immutable transition helper rather than allowing a terminal record to be assembled through the pending builder:

```text
impl ApprovalRecord {
    pub fn resolve(
        &self,
        status: ApprovalStatus,
        actor: Actor,
        resolved_at_millis: i64,
    ) -> Result<Self, ApprovalError>;
}
```

It accepts one transition from pending to a terminal status, rejects a second transition, and preserves approval ID, requester, action, object binding, creation time, and expiry exactly. When `action == ApprovalAction::MemoryMutation`, it additionally accepts only `Accepted`, `Rejected`, or `Expired` and only `Actor::Human`; `Cancelled` and every non-Human resolver return stable approval errors.

- [ ] **Step 5: Implement `MemoryProjection`**

The following is the exact Rust interface contract; copy the data definitions and implement every declaration ending in `;` in this step:

```text
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct MemoryProjection {
    current_entries: BTreeMap<(MemoryNamespaceId, NormalizedMemoryKey), MemoryEntryRef>,
    proposals: BTreeMap<MemoryProposalId, ProjectedMemoryProposal>,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectedMemoryProposal {
    proposal: MemoryProposalRef,
    namespace_id: MemoryNamespaceId,
    normalized_key: NormalizedMemoryKey,
    expected: ExpectedMemoryEntryState,
    approval_id: ApprovalId,
    status: MemoryProposalStatus,
    resolution_event_id: Option<EventId>,
}

impl ProjectedMemoryProposal {
    pub(crate) fn proposal(&self) -> &MemoryProposalRef;
    pub(crate) fn namespace_id(&self) -> MemoryNamespaceId;
    pub(crate) fn normalized_key(&self) -> &NormalizedMemoryKey;
    pub(crate) fn expected(&self) -> &ExpectedMemoryEntryState;
    pub(crate) fn approval_id(&self) -> ApprovalId;
    pub(crate) fn status(&self) -> MemoryProposalStatus;
    pub(crate) fn resolution_event_id(&self) -> Option<EventId>;
}

impl MemoryProjection {
    fn from_parts(
        current_entries: BTreeMap<(MemoryNamespaceId, NormalizedMemoryKey), MemoryEntryRef>,
        proposals: BTreeMap<MemoryProposalId, ProjectedMemoryProposal>,
    ) -> Result<Self, DomainError>;
    pub fn is_empty(&self) -> bool;
    pub fn current_entry(
        &self,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
    ) -> Option<&MemoryEntryRef>;
    pub(crate) fn current_entries(
        &self,
    ) -> impl Iterator<Item = (&(MemoryNamespaceId, NormalizedMemoryKey), &MemoryEntryRef)>;
    pub(crate) fn proposals(
        &self,
    ) -> impl Iterator<Item = (&MemoryProposalId, &ProjectedMemoryProposal)>;
    pub(crate) fn apply_entry(&mut self, entry: &MemoryEntryVersion) -> Result<(), DomainError>;
    pub(crate) fn create_proposal(&mut self, proposal: &MemoryProposal) -> Result<(), DomainError>;
    pub(crate) fn resolve_proposal(
        &mut self,
        resolution: &MemoryProposalResolution,
    ) -> Result<(), DomainError>;
}
```

Implement custom `Deserialize` with a context-free `MemoryProjection::from_parts(current_entries, proposals)` that verifies map-key identity, canonical ordering, status/resolution nullability, and duplicate-free refs. Store current entry references, including tombstones, plus immutable proposal reference/status metadata only. Do not embed values, rationale, labels, summaries, or source lists. Task 7 adds `MemoryRepository::validate_projection` to resolve every ref against immutable rows; Task 8 reducer and Task 12 verified-stream recovery prove monotonic versions, namespace/key identity, one terminal resolution, approval identity, and sorted unique expirations before trusting or storing the projection.

- [ ] **Step 6: Rerun GREEN**

```bash
cargo test --test memory_proposal_contract --test memory_projection_contract --test policy_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/memory src/policy src/domain/error.rs tests/memory_proposal_contract.rs tests/memory_projection_contract.rs
git commit -m "feat: model agent memory proposals"
```

### Task 5: Add Source-Linked Episodic Summaries and Deterministic Retrieval

**Files:**

- Create: `src/memory/episodic.rs`
- Create: `src/memory/retrieval.rs`
- Modify: `src/memory/mod.rs`
- Modify: `src/domain/error.rs`
- Create: `tests/memory_episodic_contract.rs`
- Create: `tests/memory_retrieval_contract.rs`

**Interfaces:**

- Consumes: `AgentProfileVersion::{reference, memory_namespace_id}`, `validate_plaintext(u16, PlaintextField, &str) -> Result<(), DomainError>`, `MemoryEntryVersion::reference(&self) -> MemoryEntryRef`, `EventId`, `Digest`, and `canonical_json_bytes<T: Serialize>(&T) -> Result<Vec<u8>, DomainError>`.
- Produces: `EpisodicSourceRef::new(u64, EventId, String, Digest) -> Result<Self, DomainError>`; `EpisodicSummary::{new, from_parts, reference}` with the exact signatures in Steps 1 and 4; `MemoryPurposeScope::tagged(Vec<String>) -> Result<Self, DomainError>`; the retrieval constructors/accessors in Step 5; `MemorySnapshotBuilder::{new, consider_entry, consider_summary, finish}`; and `select_snapshot<E, S>(&MemoryRetrievalRequest, E, S) -> Result<MemorySnapshot, DomainError>` with the iterator bounds in Step 5.

- [ ] **Step 1: Write failing episodic provenance tests**

Test label/body/tag boundaries, credential scanning, fixed object version 1, exact pinned profile namespace, 1–128 sources, strict sequence order, uniqueness, event ID/type/digest binding, source-set digest, future/self-source rejection, canonical record digest, malformed serde rejection, and the visible semantic label `Summary — verify sources`.

The following is the exact interface-and-test contract: copy the data definitions and test, and implement every declaration ending in `;` during Step 4.

```text
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EpisodicSourceRef {
    sequence: u64,
    event_id: EventId,
    event_type: String,
    event_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EpisodicSummaryRef {
    summary_id: EpisodicSummaryId,
    version: ObjectVersion,
    namespace_id: MemoryNamespaceId,
    profile: AgentProfileVersionRef,
    creation_event_sequence: u64,
    creation_event_id: EventId,
    source_set_digest: Digest,
    content_digest: Digest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EpisodicQualification {
    #[serde(rename = "Summary — verify sources")]
    SummaryVerifySources,
}

impl EpisodicQualification {
    pub fn label(self) -> &'static str { "Summary — verify sources" }
}

#[test]
fn episodic_sources_must_be_unique_and_strictly_in_sequence_order() {
    let mut sources = source_refs_fixture();
    sources.swap(0, 1);
    let error = EpisodicSummary::new(
        summary_id(1),
        profile_version_fixture(),
        "Weekly thesis".to_owned(),
        "Evidence-backed summary".to_owned(),
        vec!["analysis".to_owned()],
        sources,
        30,
        10,
        event_id(10),
    ).unwrap_err();
    assert_eq!(error.code(), "episodic_sources_not_ordered");
}

impl EpisodicSourceRef {
    pub fn new(
        sequence: u64,
        event_id: EventId,
        event_type: String,
        event_digest: Digest,
    ) -> Result<Self, DomainError>;
    pub fn sequence(&self) -> u64;
    pub fn event_id(&self) -> EventId;
    pub fn event_type(&self) -> &str;
    pub fn event_digest(&self) -> &Digest;
}

impl EpisodicSummaryRef {
    pub fn summary_id(&self) -> EpisodicSummaryId;
    pub fn version(&self) -> ObjectVersion;
    pub fn namespace_id(&self) -> MemoryNamespaceId;
    pub fn profile(&self) -> &AgentProfileVersionRef;
    pub fn creation_event_sequence(&self) -> u64;
    pub fn creation_event_id(&self) -> EventId;
    pub fn source_set_digest(&self) -> &Digest;
    pub fn content_digest(&self) -> &Digest;
}
```

- [ ] **Step 2: Write failing retrieval ordering, budget, and overflow tests**

Cover General and Tagged selection; tag intersection once; untagged fallback; KV and summary group ordering; tombstone/mismatched-scope ineligibility; individually oversized skip-and-continue; whole-item admission; exact canonical JSON byte cost; separate source budget; exact checked `u64` eligible/accepted/omitted/omitted-byte/omitted-source counters; hard-max rejection; arithmetic overflow; stable digest; serialized `Summary — verify sources` qualification on every episodic context item; and inert command-looking text.

```rust
#[test]
fn oversized_item_is_skipped_without_blocking_later_items() {
    let snapshot = select_snapshot(
        &request_with_byte_budget(256),
        vec![Ok(kv_item_with_value_bytes(512)), Ok(kv_item_with_value_bytes(8))],
        Vec::<Result<EpisodicContextItem, DomainError>>::new(),
    ).unwrap();
    assert_eq!(snapshot.entries().len(), 1);
    assert_eq!(snapshot.entries()[0].value(), "12345678");
    assert_eq!(snapshot.accounting().omitted_entry_count(), 1);
}
```

- [ ] **Step 3: Run episodic and retrieval contracts to verify RED**

Run:

```bash
cargo test --test memory_episodic_contract --test memory_retrieval_contract
```

Expected RED: compilation fails because episodic and retrieval APIs are absent.

- [ ] **Step 4: Implement episodic records with a validated domain constructor**

The following is the exact Rust interface contract; copy the data definition and implement every declaration ending in `;` in this step:

```text
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EpisodicSummary {
    summary_id: EpisodicSummaryId,
    version: ObjectVersion,
    namespace_id: MemoryNamespaceId,
    profile: AgentProfileVersionRef,
    label: String,
    body: String,
    purpose_tags: Vec<String>,
    sources: Vec<EpisodicSourceRef>,
    plaintext_validation_version: u16,
    created_at_ms: i64,
    creation_event_sequence: u64,
    creation_event_id: EventId,
    source_set_digest: Digest,
    content_digest: Digest,
    record_digest: Digest,
}

impl EpisodicSummary {
    pub fn new(
        summary_id: EpisodicSummaryId,
        profile: &AgentProfileVersion,
        label: String,
        body: String,
        purpose_tags: Vec<String>,
        sources: Vec<EpisodicSourceRef>,
        created_at_ms: i64,
        creation_event_sequence: u64,
        creation_event_id: EventId,
    ) -> Result<Self, DomainError>;
    pub fn reference(&self) -> EpisodicSummaryRef;
    pub fn label(&self) -> &str;
    pub fn body(&self) -> &str;
    pub fn purpose_tags(&self) -> &[String];
    pub fn sources(&self) -> &[EpisodicSourceRef];
    pub fn plaintext_validation_version(&self) -> u16;
    pub fn created_at_ms(&self) -> i64;
    pub fn creation_event_sequence(&self) -> u64;
    pub fn creation_event_id(&self) -> EventId;
    pub fn source_set_digest(&self) -> &Digest;
    pub fn content_digest(&self) -> &Digest;
    pub fn record_digest(&self) -> &Digest;

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        summary_id: EpisodicSummaryId,
        version: ObjectVersion,
        namespace_id: MemoryNamespaceId,
        profile: AgentProfileVersionRef,
        label: String,
        body: String,
        purpose_tags: Vec<String>,
        sources: Vec<EpisodicSourceRef>,
        plaintext_validation_version: u16,
        created_at_ms: i64,
        creation_event_sequence: u64,
        creation_event_id: EventId,
        source_set_digest: Digest,
        content_digest: Digest,
        record_digest: Digest,
    ) -> Result<Self, DomainError>;
}
```

Custom `Deserialize` calls `from_parts` to enforce fixed version 1, canonical/scanned text, source order/uniqueness/future-self rules, validation version, and all recomputed digests. It cannot prove external profile/event existence; Task 7 resolves the exact profile namespace and every source event, and Task 12 repeats that proof from the verified stream. Expose immutable read types in production, but no writer command, capability, or service method.

- [ ] **Step 5: Implement roomless scope v1 and whole-item selection**

The following is the exact Rust interface contract; copy the data definitions and implement every declaration ending in `;` in this step:

```text
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum MemoryPurposeScope {
    General,
    Tagged(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryRetrievalScope {
    format_version: u16,
    profile: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    purpose: MemoryPurposeScope,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryRetrievalBudget {
    max_entries: u32,
    max_summaries: u32,
    max_bytes: u64,
    max_sources: u64,
}

pub const MAX_MEMORY_RETRIEVAL_ENTRIES: u32 = 32;
pub const MAX_MEMORY_RETRIEVAL_SUMMARIES: u32 = 8;
pub const MAX_MEMORY_RETRIEVAL_BYTES: u64 = 32_768;
pub const MAX_MEMORY_RETRIEVAL_SOURCES: u64 = 128;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryRetrievalRequest {
    scope: MemoryRetrievalScope,
    budget: MemoryRetrievalBudget,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryKvContextItem {
    entry: MemoryEntryRef,
    display_key: String,
    value: String,
    purpose_tags: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EpisodicContextItem {
    summary: EpisodicSummaryRef,
    qualification: EpisodicQualification,
    label: String,
    body: String,
    purpose_tags: Vec<String>,
    sources: Vec<EpisodicSourceRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemorySnapshotAccounting {
    eligible_entry_count: u64,
    accepted_entry_count: u64,
    omitted_entry_count: u64,
    eligible_summary_count: u64,
    accepted_summary_count: u64,
    omitted_summary_count: u64,
    accepted_byte_count: u64,
    omitted_byte_count: u64,
    accepted_source_count: u64,
    omitted_source_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemorySnapshot {
    scope: MemoryRetrievalScope,
    budget: MemoryRetrievalBudget,
    entries: Vec<MemoryKvContextItem>,
    summaries: Vec<EpisodicContextItem>,
    accounting: MemorySnapshotAccounting,
    snapshot_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemorySnapshotMetadata {
    scope: MemoryRetrievalScope,
    budget: MemoryRetrievalBudget,
    entry_refs: Vec<MemoryEntryRef>,
    summary_refs: Vec<EpisodicSummaryRef>,
    accounting: MemorySnapshotAccounting,
    snapshot_digest: Digest,
}

impl MemoryKvContextItem {
    pub fn from_entry(entry: &MemoryEntryVersion) -> Result<Self, DomainError>;
    pub fn entry(&self) -> &MemoryEntryRef;
    pub fn display_key(&self) -> &str;
    pub fn value(&self) -> &str;
    pub fn purpose_tags(&self) -> &[String];
}

impl MemorySnapshotAccounting {
    pub fn eligible_entry_count(&self) -> u64;
    pub fn accepted_entry_count(&self) -> u64;
    pub fn omitted_entry_count(&self) -> u64;
    pub fn eligible_summary_count(&self) -> u64;
    pub fn accepted_summary_count(&self) -> u64;
    pub fn omitted_summary_count(&self) -> u64;
    pub fn accepted_byte_count(&self) -> u64;
    pub fn omitted_byte_count(&self) -> u64;
    pub fn accepted_source_count(&self) -> u64;
    pub fn omitted_source_count(&self) -> u64;
}

impl MemorySnapshot {
    pub fn scope(&self) -> &MemoryRetrievalScope;
    pub fn budget(&self) -> &MemoryRetrievalBudget;
    pub fn entries(&self) -> &[MemoryKvContextItem];
    pub fn summaries(&self) -> &[EpisodicContextItem];
    pub fn accounting(&self) -> &MemorySnapshotAccounting;
    pub fn snapshot_digest(&self) -> &Digest;
}

impl EpisodicContextItem {
    pub fn from_summary(summary: &EpisodicSummary) -> Result<Self, DomainError>;
    pub fn summary(&self) -> &EpisodicSummaryRef;
    pub fn qualification(&self) -> EpisodicQualification;
    pub fn label(&self) -> &str;
    pub fn body(&self) -> &str;
    pub fn purpose_tags(&self) -> &[String];
    pub fn sources(&self) -> &[EpisodicSourceRef];
}

pub fn select_snapshot<E, S>(
    request: &MemoryRetrievalRequest,
    entries: E,
    summaries: S,
) -> Result<MemorySnapshot, DomainError>
where
    E: IntoIterator<Item = Result<MemoryKvContextItem, DomainError>>,
    S: IntoIterator<Item = Result<EpisodicContextItem, DomainError>>;

pub(crate) struct MemorySnapshotBuilder {
    request: MemoryRetrievalRequest,
    entries: Vec<MemoryKvContextItem>,
    summaries: Vec<EpisodicContextItem>,
    accounting: MemorySnapshotAccounting,
}

impl MemorySnapshotBuilder {
    pub(crate) fn new(request: MemoryRetrievalRequest) -> Result<Self, DomainError>;
    pub(crate) fn consider_entry(
        &mut self,
        item: MemoryKvContextItem,
    ) -> Result<(), DomainError>;
    pub(crate) fn consider_summary(
        &mut self,
        item: EpisodicContextItem,
    ) -> Result<(), DomainError>;
    pub(crate) fn finish(self) -> Result<MemorySnapshot, DomainError>;
}
```

Provide constructors and validating custom deserializers:

```text
impl MemoryPurposeScope {
    pub fn tagged(tags: Vec<String>) -> Result<Self, DomainError>;
}

impl MemoryRetrievalScope {
    pub fn new(
        profile: &AgentProfileVersion,
        purpose: MemoryPurposeScope,
    ) -> Result<Self, DomainError>;

    pub fn validate_against(
        &self,
        profile: &AgentProfileVersion,
    ) -> Result<(), DomainError>;

    pub fn profile(&self) -> &AgentProfileVersionRef;
    pub fn namespace_id(&self) -> MemoryNamespaceId;
    pub fn purpose(&self) -> &MemoryPurposeScope;

    pub(crate) fn from_parts(
        format_version: u16,
        profile: AgentProfileVersionRef,
        namespace_id: MemoryNamespaceId,
        purpose: MemoryPurposeScope,
    ) -> Result<Self, DomainError>;
}

impl MemoryRetrievalBudget {
    pub fn new(
        max_entries: u32,
        max_summaries: u32,
        max_bytes: u64,
        max_sources: u64,
    ) -> Result<Self, DomainError>;
    pub fn max_entries(&self) -> u32;
    pub fn max_summaries(&self) -> u32;
    pub fn max_bytes(&self) -> u64;
    pub fn max_sources(&self) -> u64;
}

impl Default for MemoryRetrievalBudget {
    fn default() -> Self {
        Self::new(32, 8, 32_768, 128).expect("compiled retrieval defaults are valid")
    }
}

impl MemoryRetrievalRequest {
    pub fn new(
        scope: MemoryRetrievalScope,
        budget: MemoryRetrievalBudget,
    ) -> Result<Self, DomainError>;
    pub fn scope(&self) -> &MemoryRetrievalScope;
    pub fn budget(&self) -> &MemoryRetrievalBudget;
}

impl MemorySnapshot {
    pub fn metadata(&self) -> MemorySnapshotMetadata;
}

impl MemorySnapshotMetadata {
    pub fn scope(&self) -> &MemoryRetrievalScope;
    pub fn budget(&self) -> &MemoryRetrievalBudget;
    pub fn entry_refs(&self) -> &[MemoryEntryRef];
    pub fn summary_refs(&self) -> &[EpisodicSummaryRef];
    pub fn accounting(&self) -> &MemorySnapshotAccounting;
    pub fn snapshot_digest(&self) -> &Digest;
}
```

Require `format_version == 1`; derive namespace from the exact profile for fresh scopes; require Tagged to contain 1–8 unique canonical non-`general` tags; and reject every budget component above 32/8/32,768/128 rather than clamping. Scope deserialization calls `from_parts` for context-free format/tag/namespace-shape validation; the application/repository boundary then resolves the exact profile reference and calls `validate_against` to reject namespace substitution before selection. Compute candidate cost from `canonical_json_bytes(item).len()`. Continue after an oversized candidate. Never slice content or source lists. Increment all ten accounting fields with `checked_add` and return the stable overflow error on failure. Digest the complete scope, budget, accepted references/order, and all accounting fields. Expose read-only accessors. Custom `Deserialize` for context items and a full snapshot recomputes canonical byte/source counts, reference ordering, and the digest. Metadata deserialization can validate only its structural counts, hard bounds, canonical ordering, and digest; Task 9 replay must reload its exact immutable refs and recompute actual byte/source accounting before accepting it.

- [ ] **Step 6: Rerun GREEN**

```bash
cargo test --test memory_episodic_contract --test memory_retrieval_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/memory src/domain/error.rs tests/memory_episodic_contract.rs tests/memory_retrieval_contract.rs
git commit -m "feat: add deterministic hybrid retrieval"
```

### Task 6: Add Schema Version 4 and Preserve Every Schema-v3 Row

**Files:**

- Create: `migrations/0004_hybrid_memory.sql`
- Modify: `src/persistence/migrations.rs`
- Modify: `src/persistence/database.rs`
- Modify: `tests/migration_contract.rs`
- Modify: `tests/support/mod.rs`
- Create: `tests/memory_migration_contract.rs`

**Interfaces:**

- Consumes: ordered migrations 1–3 and schema-v3 ledger rows; `ImmediateTransaction::transaction(&self) -> &rusqlite::Transaction<'_>`; the existing migration-boundary parser/fault hook; Tasks 1–5 actor/record wire shapes; and all existing command-receipt capability strings.
- Produces: `LATEST_SCHEMA_VERSION: u32 = 4`; `Migration { version: 4, sql: include_str!("../../migrations/0004_hybrid_memory.sql") }`; rebuilt `approval_records`, `command_receipts`, and `command_event_refs`; the seven exact tables, indexes, and triggers in Step 4; resolver actor columns; capability strings `memory_read`, `memory_preview`, `memory_mutate`, `memory_propose`, and `memory_resolve`; and `Database::v4_migration_boundaries() -> Vec<&'static str>`.

- [ ] **Step 1: Write the failing fresh-install and exact-upgrade contracts**

Assert a fresh database reaches exactly v4; all required strict tables, indexes, composite foreign keys, and triggers exist; immutable tables reject update/delete; current tables accept transactional replacement only with exact immutable refs; active proposal/source constraints work; approval requesters accept only `(human,NULL)`, `(system,NULL)`, or `(agent,canonical profile UUID)`; memory approvals reject `cancelled` and non-Human terminal resolvers; and reopening is idempotent.

Build a real schema-v3 fixture by applying migrations 1–3, populate representative event, projection, profile, skill, approval, receipt, audit, and command-event-ref rows, upgrade, then compare every pre-v4 byte/value. Include a terminal non-memory approval whose legacy `resolution_event_id` is `NULL` and prove that value remains exactly `NULL`. Update the generic migration inventory from 3 to 4, change the “future migration” fixture from version 4 to version 5, and add all v4 objects/columns to its exact assertions.

```rust
#[test]
fn v3_terminal_non_memory_approval_keeps_null_resolution_event() {
    let db = schema_v3_fixture_with_terminal_legacy_approval();
    let before = approval_row_bytes(&db, LEGACY_APPROVAL_ID);
    migrate_to_latest(&db).unwrap();
    assert_eq!(approval_row_bytes(&db, LEGACY_APPROVAL_ID), before.with_null_resolver_columns());
    assert_eq!(schema_version(&db), 4);
}
```

- [ ] **Step 2: Run migration contracts to verify RED**

Run:

```bash
cargo test --test memory_migration_contract --test migration_contract
```

Expected RED: the version assertion reports 3 instead of 4 and the v4 objects do not exist.

- [ ] **Step 3: Rebuild approvals and receipts losslessly**

Use this exact SQL in `migrations/0004_hybrid_memory.sql`. It starts from the
actual schema-v3 column sets. The approval copy explicitly writes `NULL` only
for the two new resolver columns, so every pre-v4 value—including a terminal
non-memory row's nullable `resolution_event_id`—is preserved verbatim.

```sql
DROP INDEX approval_records_status_idx;
-- migration-boundary: drop_approval_records_status_idx

ALTER TABLE approval_records RENAME TO approval_records_v3;
-- migration-boundary: rename_approval_records_v3

CREATE TABLE approval_records (
    approval_id TEXT PRIMARY KEY,
    action_kind TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    object_version INTEGER NOT NULL CHECK (object_version > 0),
    object_digest TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'accepted', 'rejected', 'expired', 'cancelled')
    ),
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    resolved_at_ms INTEGER,
    resolution_kind TEXT,
    resolution_event_id TEXT REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    resolution_actor_kind TEXT,
    resolution_actor_id TEXT,
    CHECK (expires_at_ms IS NULL OR expires_at_ms > created_at_ms),
    CHECK (
        (resolution_actor_kind IS NULL AND resolution_actor_id IS NULL)
        OR (
            resolution_actor_kind IN ('human', 'system')
            AND resolution_actor_id IS NULL
        )
        OR (
            resolution_actor_kind = 'agent'
            AND resolution_actor_id IS NOT NULL
            AND typeof(resolution_actor_id) = 'text'
            AND length(CAST(resolution_actor_id AS BLOB)) = 36
            AND resolution_actor_id = lower(resolution_actor_id)
            AND substr(resolution_actor_id, 9, 1) = '-'
            AND substr(resolution_actor_id, 14, 1) = '-'
            AND substr(resolution_actor_id, 19, 1) = '-'
            AND substr(resolution_actor_id, 24, 1) = '-'
            AND length(replace(resolution_actor_id, '-', '')) = 32
            AND replace(resolution_actor_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        (
            status = 'pending'
            AND resolved_at_ms IS NULL
            AND resolution_kind IS NULL
        )
        OR (
            status <> 'pending'
            AND resolved_at_ms IS NOT NULL
            AND resolution_kind IS NOT NULL
        )
    )
) STRICT;
-- migration-boundary: create_approval_records

INSERT INTO approval_records (
    approval_id,
    action_kind,
    object_kind,
    object_id,
    object_version,
    object_digest,
    actor_kind,
    actor_id,
    status,
    created_at_ms,
    expires_at_ms,
    resolved_at_ms,
    resolution_kind,
    resolution_event_id,
    resolution_actor_kind,
    resolution_actor_id
)
SELECT
    approval_id,
    action_kind,
    object_kind,
    object_id,
    object_version,
    object_digest,
    actor_kind,
    actor_id,
    status,
    created_at_ms,
    expires_at_ms,
    resolved_at_ms,
    resolution_kind,
    resolution_event_id,
    NULL,
    NULL
FROM approval_records_v3
ORDER BY rowid;
-- migration-boundary: copy_approval_records_v3

DROP TABLE approval_records_v3;
-- migration-boundary: drop_approval_records_v3

CREATE INDEX approval_records_status_idx
ON approval_records (status, created_at_ms);
-- migration-boundary: create_approval_records_status_idx

CREATE TRIGGER approval_records_identity_guard
BEFORE UPDATE ON approval_records
WHEN NEW.approval_id IS NOT OLD.approval_id
  OR NEW.action_kind IS NOT OLD.action_kind
  OR NEW.object_kind IS NOT OLD.object_kind
  OR NEW.object_id IS NOT OLD.object_id
  OR NEW.object_version IS NOT OLD.object_version
  OR NEW.object_digest IS NOT OLD.object_digest
  OR NEW.actor_kind IS NOT OLD.actor_kind
  OR NEW.actor_id IS NOT OLD.actor_id
  OR NEW.created_at_ms IS NOT OLD.created_at_ms
  OR NEW.expires_at_ms IS NOT OLD.expires_at_ms
BEGIN
    SELECT RAISE(ABORT, 'approval_identity_immutable');
END;
-- migration-boundary: create_approval_records_identity_guard

CREATE TRIGGER approval_records_requester_insert_guard
BEFORE INSERT ON approval_records
WHEN NOT (
    (NEW.actor_kind IN ('human', 'system') AND NEW.actor_id IS NULL)
    OR (
        NEW.actor_kind = 'agent'
        AND NEW.actor_id IS NOT NULL
        AND typeof(NEW.actor_id) = 'text'
        AND length(CAST(NEW.actor_id AS BLOB)) = 36
        AND NEW.actor_id = lower(NEW.actor_id)
        AND substr(NEW.actor_id, 9, 1) = '-'
        AND substr(NEW.actor_id, 14, 1) = '-'
        AND substr(NEW.actor_id, 19, 1) = '-'
        AND substr(NEW.actor_id, 24, 1) = '-'
        AND length(replace(NEW.actor_id, '-', '')) = 32
        AND replace(NEW.actor_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
)
BEGIN
    SELECT RAISE(ABORT, 'approval_requester_invalid');
END;
-- migration-boundary: create_approval_records_requester_insert_guard

CREATE TRIGGER approval_records_pending_insert_guard
BEFORE INSERT ON approval_records
WHEN NEW.status = 'pending'
 AND (
     NEW.resolution_event_id IS NOT NULL
     OR NEW.resolution_actor_kind IS NOT NULL
     OR NEW.resolution_actor_id IS NOT NULL
 )
BEGIN
    SELECT RAISE(ABORT, 'approval_pending_resolution_invalid');
END;
-- migration-boundary: create_approval_records_pending_insert_guard

CREATE TRIGGER approval_records_memory_insert_guard
BEFORE INSERT ON approval_records
WHEN NEW.action_kind = 'memory_mutation'
 AND NOT (
     NEW.actor_kind = 'agent'
     AND NEW.actor_id IS NOT NULL
     AND NEW.object_kind = 'memory_proposal'
     AND NEW.object_version = 1
     AND typeof(NEW.object_digest) = 'text'
     AND length(CAST(NEW.object_digest AS BLOB)) = 64
     AND NEW.object_digest = lower(NEW.object_digest)
     AND NEW.object_digest NOT GLOB '*[^0-9a-f]*'
     AND NEW.expires_at_ms IS NULL
     AND NEW.status IN ('pending', 'accepted', 'rejected', 'expired')
     AND (
         NEW.status = 'pending'
         OR (
             NEW.resolution_actor_kind = 'human'
             AND NEW.resolution_actor_id IS NULL
             AND NEW.resolution_event_id IS NOT NULL
             AND NEW.resolution_kind = NEW.status
         )
     )
 )
BEGIN
    SELECT RAISE(ABORT, 'memory_approval_invalid');
END;
-- migration-boundary: create_approval_records_memory_insert_guard

CREATE TRIGGER approval_records_transition_guard
BEFORE UPDATE ON approval_records
WHEN NOT (
    OLD.status = 'pending'
    AND OLD.resolved_at_ms IS NULL
    AND OLD.resolution_kind IS NULL
    AND OLD.resolution_event_id IS NULL
    AND OLD.resolution_actor_kind IS NULL
    AND OLD.resolution_actor_id IS NULL
    AND NEW.status IN ('accepted', 'rejected', 'expired', 'cancelled')
    AND NEW.resolved_at_ms IS NOT NULL
    AND NEW.resolution_kind IS NOT NULL
    AND NEW.resolution_actor_kind IS NOT NULL
    AND (
        (NEW.actor_kind IN ('human', 'system') AND NEW.actor_id IS NULL)
        OR (
            NEW.actor_kind = 'agent'
            AND NEW.actor_id IS NOT NULL
            AND typeof(NEW.actor_id) = 'text'
            AND length(CAST(NEW.actor_id AS BLOB)) = 36
            AND NEW.actor_id = lower(NEW.actor_id)
            AND substr(NEW.actor_id, 9, 1) = '-'
            AND substr(NEW.actor_id, 14, 1) = '-'
            AND substr(NEW.actor_id, 19, 1) = '-'
            AND substr(NEW.actor_id, 24, 1) = '-'
            AND length(replace(NEW.actor_id, '-', '')) = 32
            AND replace(NEW.actor_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    )
    AND (
        NEW.action_kind <> 'memory_mutation'
        OR (
            NEW.actor_kind = 'agent'
            AND NEW.object_kind = 'memory_proposal'
            AND NEW.object_version = 1
            AND typeof(NEW.object_digest) = 'text'
            AND length(CAST(NEW.object_digest AS BLOB)) = 64
            AND NEW.object_digest = lower(NEW.object_digest)
            AND NEW.object_digest NOT GLOB '*[^0-9a-f]*'
            AND NEW.expires_at_ms IS NULL
            AND NEW.status IN ('accepted', 'rejected', 'expired')
            AND NEW.resolution_actor_kind = 'human'
            AND NEW.resolution_actor_id IS NULL
            AND NEW.resolution_event_id IS NOT NULL
            AND NEW.resolution_kind = NEW.status
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'approval_transition_invalid');
END;
-- migration-boundary: create_approval_records_transition_guard

CREATE TRIGGER approval_records_terminal_insert_guard
BEFORE INSERT ON approval_records
WHEN NEW.status <> 'pending' AND NEW.resolution_actor_kind IS NULL
BEGIN
    SELECT RAISE(ABORT, 'approval_terminal_resolver_missing');
END;
-- migration-boundary: create_approval_records_terminal_insert_guard

CREATE TRIGGER approval_records_no_delete
BEFORE DELETE ON approval_records BEGIN
    SELECT RAISE(ABORT, 'approval_records_immutable');
END;
-- migration-boundary: create_approval_records_no_delete

DROP TRIGGER command_event_refs_no_update;
-- migration-boundary: drop_command_event_refs_no_update_v4

DROP TRIGGER command_event_refs_no_delete;
-- migration-boundary: drop_command_event_refs_no_delete_v4

DROP INDEX command_event_refs_event_idx;
-- migration-boundary: drop_command_event_refs_event_idx_v4

ALTER TABLE command_event_refs RENAME TO command_event_refs_v3;
-- migration-boundary: rename_command_event_refs_v3

DROP TRIGGER command_receipts_no_update;
-- migration-boundary: drop_command_receipts_no_update_v4

DROP TRIGGER command_receipts_no_delete;
-- migration-boundary: drop_command_receipts_no_delete_v4

ALTER TABLE command_receipts RENAME TO command_receipts_v3;
-- migration-boundary: rename_command_receipts_v3

CREATE TABLE command_receipts (
    command_id TEXT PRIMARY KEY,
    command_fingerprint TEXT NOT NULL CHECK (
        typeof(command_fingerprint) = 'text'
        AND length(CAST(command_fingerprint AS BLOB)) = 64
        AND instr(command_fingerprint, char(0)) = 0
        AND command_fingerprint NOT GLOB '*[^0-9a-f]*'
    ),
    request_json TEXT NOT NULL CHECK (json_valid(request_json)),
    capability TEXT NOT NULL CHECK (capability IN (
        'help_read', 'status_read', 'setup_status_read', 'audit_read',
        'agent_profile_read', 'agent_profile_create', 'agent_profile_preview',
        'agent_profile_activate', 'skill_read', 'skill_create', 'skill_version',
        'skill_assign', 'skill_unassign', 'memory_read', 'memory_preview',
        'memory_mutate', 'memory_propose', 'memory_resolve', 'shutdown',
        'discussion_run', 'mcp_use', 'engineering_job_run', 'git_merge',
        'git_push', 'finance_recommendation'
    )),
    policy_decision TEXT NOT NULL CHECK (policy_decision IN (
        'granted', 'denied', 'denied_by_default', 'approval_required'
    )),
    outcome_json TEXT NOT NULL CHECK (json_valid(outcome_json))
) STRICT;
-- migration-boundary: create_command_receipts_v4

INSERT INTO command_receipts (
    command_id,
    command_fingerprint,
    request_json,
    capability,
    policy_decision,
    outcome_json
)
SELECT
    command_id,
    command_fingerprint,
    request_json,
    capability,
    policy_decision,
    outcome_json
FROM command_receipts_v3
ORDER BY rowid;
-- migration-boundary: copy_command_receipts_v3

CREATE TRIGGER command_receipts_no_update
BEFORE UPDATE ON command_receipts BEGIN
    SELECT RAISE(ABORT, 'command receipts are immutable');
END;
-- migration-boundary: create_command_receipts_no_update_v4

CREATE TRIGGER command_receipts_no_delete
BEFORE DELETE ON command_receipts BEGIN
    SELECT RAISE(ABORT, 'command receipts are immutable');
END;
-- migration-boundary: create_command_receipts_no_delete_v4

CREATE TABLE command_event_refs (
    command_id TEXT NOT NULL REFERENCES command_receipts(command_id),
    event_ordinal INTEGER NOT NULL CHECK (event_ordinal >= 0),
    event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    PRIMARY KEY (command_id, event_ordinal)
) STRICT;
-- migration-boundary: create_command_event_refs_v4

INSERT INTO command_event_refs (command_id, event_ordinal, event_id)
SELECT command_id, event_ordinal, event_id
FROM command_event_refs_v3
ORDER BY command_id, event_ordinal;
-- migration-boundary: copy_command_event_refs_v3

CREATE UNIQUE INDEX command_event_refs_event_idx
ON command_event_refs (event_id);
-- migration-boundary: create_command_event_refs_event_idx_v4

CREATE TRIGGER command_event_refs_no_update
BEFORE UPDATE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;
-- migration-boundary: create_command_event_refs_no_update_v4

CREATE TRIGGER command_event_refs_no_delete
BEFORE DELETE ON command_event_refs BEGIN
    SELECT RAISE(ABORT, 'command event refs are immutable');
END;
-- migration-boundary: create_command_event_refs_no_delete_v4

DROP TABLE command_event_refs_v3;
-- migration-boundary: drop_command_event_refs_v3

DROP TABLE command_receipts_v3;
-- migration-boundary: drop_command_receipts_v3
```

The rebuilt table retains every schema-v3-admissible requester/action shape and
the old pending/terminal check during the copy; otherwise a syntactically valid
legacy row could make the upgrade lossy. The stricter requester, clean-pending,
and memory-mutation rules are installed as `BEFORE INSERT` triggers only after
that deterministic copy. The resolver-pair check is safe on all copied rows
because both new columns are explicitly `NULL`. New terminal rows must name a
resolver, and the update guard permits one pending-to-terminal transition only.
For memory mutations the insert/transition guards additionally require the
exact Agent requester and proposal binding, a Human resolver, a non-null event,
and `resolution_kind = status`. Negative tests exercise every invalid actor
pair, memory binding, transition, and post-transition mutation.

- [ ] **Step 4: Create constrained memory tables**

Create the two parent-key indexes first. They let SQLite enforce exact pinned
profile/namespace and source-event references rather than relying on repository
joins alone:

```sql
CREATE UNIQUE INDEX agent_profile_versions_memory_ref_idx
ON agent_profile_versions (
    profile_id,
    profile_version_id,
    version,
    content_digest,
    memory_namespace_id
);
-- migration-boundary: create_agent_profile_versions_memory_ref_idx

CREATE UNIQUE INDEX event_stream_memory_source_ref_idx
ON event_stream (sequence, event_id, event_type, event_digest);
-- migration-boundary: create_event_stream_memory_source_ref_idx
```

Create the immutable KV history and rebuildable current pointer. Store the
query-critical columns beside canonical record JSON, retain tombstones as
current pointers, and make the exact entry reference a parent key for proposal
expectations:

```sql
CREATE TABLE memory_entry_versions (
    memory_namespace_id TEXT NOT NULL CHECK (
        typeof(memory_namespace_id) = 'text'
        AND length(CAST(memory_namespace_id AS BLOB)) = 36
        AND memory_namespace_id = lower(memory_namespace_id)
        AND substr(memory_namespace_id, 9, 1) = '-'
        AND substr(memory_namespace_id, 14, 1) = '-'
        AND substr(memory_namespace_id, 19, 1) = '-'
        AND substr(memory_namespace_id, 24, 1) = '-'
        AND length(replace(memory_namespace_id, '-', '')) = 32
        AND replace(memory_namespace_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    entry_id TEXT NOT NULL CHECK (
        typeof(entry_id) = 'text'
        AND length(CAST(entry_id AS BLOB)) = 36
        AND entry_id = lower(entry_id)
        AND substr(entry_id, 9, 1) = '-'
        AND substr(entry_id, 14, 1) = '-'
        AND substr(entry_id, 19, 1) = '-'
        AND substr(entry_id, 24, 1) = '-'
        AND length(replace(entry_id, '-', '')) = 32
        AND replace(entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    entry_version_id TEXT NOT NULL UNIQUE CHECK (
        typeof(entry_version_id) = 'text'
        AND length(CAST(entry_version_id AS BLOB)) = 36
        AND entry_version_id = lower(entry_version_id)
        AND substr(entry_version_id, 9, 1) = '-'
        AND substr(entry_version_id, 14, 1) = '-'
        AND substr(entry_version_id, 19, 1) = '-'
        AND substr(entry_version_id, 24, 1) = '-'
        AND length(replace(entry_version_id, '-', '')) = 32
        AND replace(entry_version_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    version INTEGER NOT NULL CHECK (version >= 1),
    predecessor_version_id TEXT,
    display_key TEXT NOT NULL CHECK (
        length(CAST(display_key AS BLOB)) BETWEEN 1 AND 96
        AND instr(display_key, char(0)) = 0
    ),
    normalized_key TEXT NOT NULL CHECK (
        length(CAST(normalized_key AS BLOB)) >= 1
        AND instr(normalized_key, char(0)) = 0
    ),
    state TEXT NOT NULL CHECK (state IN ('present', 'deleted')),
    value_text TEXT,
    value_bytes INTEGER NOT NULL CHECK (value_bytes >= 0),
    purpose_tags_json BLOB NOT NULL CHECK (
        typeof(purpose_tags_json) = 'blob'
        AND json_valid(CAST(purpose_tags_json AS TEXT))
        AND json_type(CAST(purpose_tags_json AS TEXT)) = 'array'
        AND json_array_length(CAST(purpose_tags_json AS TEXT)) BETWEEN 0 AND 8
    ),
    created_by_kind TEXT NOT NULL,
    created_by_id TEXT,
    created_at_ms INTEGER NOT NULL,
    accepted_proposal_id TEXT,
    accepted_proposal_version INTEGER,
    accepted_proposal_digest TEXT,
    plaintext_validation_version INTEGER NOT NULL CHECK (
        plaintext_validation_version = 1
    ),
    creation_event_sequence INTEGER NOT NULL CHECK (creation_event_sequence >= 1),
    creation_event_id TEXT NOT NULL REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    content_digest TEXT NOT NULL CHECK (
        typeof(content_digest) = 'text'
        AND length(CAST(content_digest AS BLOB)) = 64
        AND content_digest = lower(content_digest)
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_digest TEXT NOT NULL CHECK (
        typeof(record_digest) = 'text'
        AND length(CAST(record_digest AS BLOB)) = 64
        AND record_digest = lower(record_digest)
        AND record_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_json BLOB NOT NULL CHECK (
        typeof(record_json) = 'blob'
        AND json_valid(CAST(record_json AS TEXT))
    ),
    PRIMARY KEY (entry_id, version),
    UNIQUE (entry_id, entry_version_id),
    UNIQUE (
        memory_namespace_id,
        normalized_key,
        entry_id,
        entry_version_id,
        version,
        state,
        content_digest
    ),
    CHECK (
        (version = 1 AND predecessor_version_id IS NULL)
        OR (version > 1 AND predecessor_version_id IS NOT NULL)
    ),
    CHECK (
        (state = 'present'
         AND value_text IS NOT NULL
         AND value_bytes = length(CAST(value_text AS BLOB))
         AND value_bytes BETWEEN 1 AND 4096)
        OR (state = 'deleted'
            AND value_text IS NULL
            AND value_bytes = 0
            AND json_array_length(CAST(purpose_tags_json AS TEXT)) = 0)
    ),
    CHECK (created_by_kind = 'human' AND created_by_id IS NULL),
    CHECK (
        (accepted_proposal_id IS NULL
         AND accepted_proposal_version IS NULL
         AND accepted_proposal_digest IS NULL)
        OR (accepted_proposal_id IS NOT NULL
            AND accepted_proposal_version = 1
            AND accepted_proposal_digest IS NOT NULL
            AND length(CAST(accepted_proposal_digest AS BLOB)) = 64
            AND accepted_proposal_digest = lower(accepted_proposal_digest)
            AND accepted_proposal_digest NOT GLOB '*[^0-9a-f]*')
    ),
    FOREIGN KEY (entry_id, predecessor_version_id)
        REFERENCES memory_entry_versions(entry_id, entry_version_id)
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        memory_namespace_id,
        normalized_key,
        accepted_proposal_id,
        accepted_proposal_version,
        accepted_proposal_digest
    ) REFERENCES memory_proposals (
        memory_namespace_id,
        normalized_key,
        proposal_id,
        version,
        content_digest
    ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_memory_entry_versions

CREATE INDEX memory_entry_versions_history_idx
ON memory_entry_versions (memory_namespace_id, normalized_key, version DESC);
-- migration-boundary: create_memory_entry_versions_history_idx

CREATE TRIGGER memory_entry_versions_identity_guard
BEFORE INSERT ON memory_entry_versions
WHEN EXISTS (
    SELECT 1
    FROM memory_entry_versions existing
    WHERE (existing.memory_namespace_id = NEW.memory_namespace_id
           AND existing.normalized_key = NEW.normalized_key
           AND existing.entry_id <> NEW.entry_id)
       OR (existing.entry_id = NEW.entry_id
           AND (existing.memory_namespace_id <> NEW.memory_namespace_id
                OR existing.normalized_key <> NEW.normalized_key))
)
BEGIN
    SELECT RAISE(ABORT, 'memory_entry_identity_mismatch');
END;
-- migration-boundary: create_memory_entry_versions_identity_guard

CREATE TRIGGER memory_entry_versions_predecessor_guard
BEFORE INSERT ON memory_entry_versions
WHEN NEW.version > 1 AND NOT EXISTS (
    SELECT 1
    FROM memory_entry_versions predecessor
    WHERE predecessor.memory_namespace_id = NEW.memory_namespace_id
      AND predecessor.normalized_key = NEW.normalized_key
      AND predecessor.entry_id = NEW.entry_id
      AND predecessor.entry_version_id = NEW.predecessor_version_id
      AND predecessor.version = NEW.version - 1
)
BEGIN
    SELECT RAISE(ABORT, 'memory_entry_predecessor_mismatch');
END;
-- migration-boundary: create_memory_entry_versions_predecessor_guard

CREATE TRIGGER memory_entry_versions_no_update
BEFORE UPDATE ON memory_entry_versions BEGIN
    SELECT RAISE(ABORT, 'memory_entry_versions_immutable');
END;
-- migration-boundary: create_memory_entry_versions_no_update

CREATE TRIGGER memory_entry_versions_no_delete
BEFORE DELETE ON memory_entry_versions BEGIN
    SELECT RAISE(ABORT, 'memory_entry_versions_immutable');
END;
-- migration-boundary: create_memory_entry_versions_no_delete

CREATE TABLE current_memory_entries (
    memory_namespace_id TEXT NOT NULL,
    normalized_key TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    entry_version_id TEXT NOT NULL UNIQUE,
    version INTEGER NOT NULL CHECK (version >= 1),
    state TEXT NOT NULL CHECK (state IN ('present', 'deleted')),
    content_digest TEXT NOT NULL,
    PRIMARY KEY (memory_namespace_id, normalized_key),
    FOREIGN KEY (
        memory_namespace_id,
        normalized_key,
        entry_id,
        entry_version_id,
        version,
        state,
        content_digest
    ) REFERENCES memory_entry_versions (
        memory_namespace_id,
        normalized_key,
        entry_id,
        entry_version_id,
        version,
        state,
        content_digest
    ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_current_memory_entries

CREATE INDEX current_memory_entries_list_idx
ON current_memory_entries (memory_namespace_id, state, normalized_key, entry_id);
-- migration-boundary: create_current_memory_entries_list_idx
```

Create immutable proposals and one immutable terminal resolution per proposal.
The expected-entry composite foreign key is skipped only for explicit absence;
set/delete candidate shapes and exact profile/namespace provenance are enforced
in the schema:

```sql
CREATE TABLE memory_proposals (
    proposal_id TEXT PRIMARY KEY CHECK (
        typeof(proposal_id) = 'text'
        AND length(CAST(proposal_id AS BLOB)) = 36
        AND proposal_id = lower(proposal_id)
        AND substr(proposal_id, 9, 1) = '-'
        AND substr(proposal_id, 14, 1) = '-'
        AND substr(proposal_id, 19, 1) = '-'
        AND substr(proposal_id, 24, 1) = '-'
        AND length(replace(proposal_id, '-', '')) = 32
        AND replace(proposal_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    version INTEGER NOT NULL CHECK (version = 1),
    proposer_profile_id TEXT NOT NULL,
    proposer_profile_version_id TEXT NOT NULL,
    proposer_profile_version INTEGER NOT NULL CHECK (proposer_profile_version >= 1),
    proposer_profile_digest TEXT NOT NULL CHECK (
        typeof(proposer_profile_digest) = 'text'
        AND length(CAST(proposer_profile_digest AS BLOB)) = 64
        AND proposer_profile_digest = lower(proposer_profile_digest)
        AND proposer_profile_digest NOT GLOB '*[^0-9a-f]*'
    ),
    memory_namespace_id TEXT NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('set', 'delete')),
    display_key TEXT NOT NULL CHECK (
        length(CAST(display_key AS BLOB)) BETWEEN 1 AND 96
        AND instr(display_key, char(0)) = 0
    ),
    normalized_key TEXT NOT NULL CHECK (
        length(CAST(normalized_key AS BLOB)) >= 1
        AND instr(normalized_key, char(0)) = 0
    ),
    expected_kind TEXT NOT NULL CHECK (
        expected_kind IN ('absent', 'present', 'deleted')
    ),
    expected_entry_id TEXT,
    expected_entry_version_id TEXT,
    expected_entry_version INTEGER,
    expected_entry_digest TEXT,
    candidate_value TEXT,
    candidate_value_bytes INTEGER,
    candidate_purpose_tags_json BLOB,
    rationale TEXT NOT NULL CHECK (
        length(CAST(rationale AS BLOB)) BETWEEN 0 AND 512
        AND instr(rationale, char(0)) = 0
    ),
    plaintext_validation_version INTEGER NOT NULL CHECK (
        plaintext_validation_version = 1
    ),
    created_at_ms INTEGER NOT NULL,
    creation_event_sequence INTEGER NOT NULL CHECK (creation_event_sequence >= 1),
    creation_event_id TEXT NOT NULL REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    approval_id TEXT NOT NULL UNIQUE REFERENCES approval_records(approval_id)
        DEFERRABLE INITIALLY DEFERRED,
    content_digest TEXT NOT NULL CHECK (
        typeof(content_digest) = 'text'
        AND length(CAST(content_digest AS BLOB)) = 64
        AND content_digest = lower(content_digest)
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_digest TEXT NOT NULL CHECK (
        typeof(record_digest) = 'text'
        AND length(CAST(record_digest AS BLOB)) = 64
        AND record_digest = lower(record_digest)
        AND record_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_json BLOB NOT NULL CHECK (
        typeof(record_json) = 'blob'
        AND json_valid(CAST(record_json AS TEXT))
    ),
    UNIQUE (proposal_id, version, content_digest),
    UNIQUE (proposal_id, version, content_digest, approval_id),
    UNIQUE (
        memory_namespace_id,
        normalized_key,
        proposal_id,
        version,
        content_digest
    ),
    CHECK (
        (expected_kind = 'absent'
         AND expected_entry_id IS NULL
         AND expected_entry_version_id IS NULL
         AND expected_entry_version IS NULL
         AND expected_entry_digest IS NULL)
        OR (expected_kind IN ('present', 'deleted')
            AND expected_entry_id IS NOT NULL
            AND expected_entry_version_id IS NOT NULL
            AND expected_entry_version >= 1
            AND expected_entry_digest IS NOT NULL
            AND length(CAST(expected_entry_digest AS BLOB)) = 64
            AND expected_entry_digest = lower(expected_entry_digest)
            AND expected_entry_digest NOT GLOB '*[^0-9a-f]*')
    ),
    CHECK (
        (operation = 'set'
         AND candidate_value IS NOT NULL
         AND candidate_value_bytes = length(CAST(candidate_value AS BLOB))
         AND candidate_value_bytes BETWEEN 1 AND 4096
         AND candidate_purpose_tags_json IS NOT NULL
         AND typeof(candidate_purpose_tags_json) = 'blob'
         AND json_valid(CAST(candidate_purpose_tags_json AS TEXT))
         AND json_type(CAST(candidate_purpose_tags_json AS TEXT)) = 'array'
         AND json_array_length(CAST(candidate_purpose_tags_json AS TEXT)) BETWEEN 0 AND 8)
        OR (operation = 'delete'
            AND expected_kind = 'present'
            AND candidate_value IS NULL
            AND candidate_value_bytes IS NULL
            AND candidate_purpose_tags_json IS NULL)
    ),
    FOREIGN KEY (
        proposer_profile_id,
        proposer_profile_version_id,
        proposer_profile_version,
        proposer_profile_digest,
        memory_namespace_id
    ) REFERENCES agent_profile_versions (
        profile_id,
        profile_version_id,
        version,
        content_digest,
        memory_namespace_id
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (
        memory_namespace_id,
        normalized_key,
        expected_entry_id,
        expected_entry_version_id,
        expected_entry_version,
        expected_kind,
        expected_entry_digest
    ) REFERENCES memory_entry_versions (
        memory_namespace_id,
        normalized_key,
        entry_id,
        entry_version_id,
        version,
        state,
        content_digest
    ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_memory_proposals

CREATE INDEX memory_proposals_pending_order_idx
ON memory_proposals (memory_namespace_id, created_at_ms, proposal_id);
-- migration-boundary: create_memory_proposals_pending_order_idx

CREATE TRIGGER memory_proposals_no_update
BEFORE UPDATE ON memory_proposals BEGIN
    SELECT RAISE(ABORT, 'memory_proposals_immutable');
END;
-- migration-boundary: create_memory_proposals_no_update

CREATE TRIGGER memory_proposals_no_delete
BEFORE DELETE ON memory_proposals BEGIN
    SELECT RAISE(ABORT, 'memory_proposals_immutable');
END;
-- migration-boundary: create_memory_proposals_no_delete

CREATE TABLE memory_proposal_resolutions (
    proposal_id TEXT PRIMARY KEY,
    proposal_version INTEGER NOT NULL CHECK (proposal_version = 1),
    proposal_content_digest TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('accepted', 'rejected', 'expired')),
    approval_id TEXT NOT NULL UNIQUE,
    resolved_by_kind TEXT NOT NULL CHECK (resolved_by_kind = 'human'),
    resolved_by_id TEXT CHECK (resolved_by_id IS NULL),
    resolved_at_ms INTEGER NOT NULL,
    resolution_event_sequence INTEGER NOT NULL CHECK (resolution_event_sequence >= 1),
    resolution_event_id TEXT NOT NULL REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    resolution_json BLOB NOT NULL CHECK (
        typeof(resolution_json) = 'blob'
        AND json_valid(CAST(resolution_json AS TEXT))
    ),
    UNIQUE (proposal_id, status, resolution_event_id),
    FOREIGN KEY (
        proposal_id,
        proposal_version,
        proposal_content_digest,
        approval_id
    ) REFERENCES memory_proposals (
        proposal_id,
        version,
        content_digest,
        approval_id
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (approval_id) REFERENCES approval_records(approval_id)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_memory_proposal_resolutions

CREATE INDEX memory_proposal_resolutions_event_idx
ON memory_proposal_resolutions (resolution_event_id, proposal_id);
-- migration-boundary: create_memory_proposal_resolutions_event_idx

CREATE TRIGGER memory_proposal_resolutions_no_update
BEFORE UPDATE ON memory_proposal_resolutions BEGIN
    SELECT RAISE(ABORT, 'memory_proposal_resolutions_immutable');
END;
-- migration-boundary: create_memory_proposal_resolutions_no_update

CREATE TRIGGER memory_proposal_resolutions_no_delete
BEFORE DELETE ON memory_proposal_resolutions BEGIN
    SELECT RAISE(ABORT, 'memory_proposal_resolutions_immutable');
END;
-- migration-boundary: create_memory_proposal_resolutions_no_delete

CREATE TABLE current_memory_proposal_status (
    proposal_id TEXT PRIMARY KEY,
    proposal_version INTEGER NOT NULL CHECK (proposal_version = 1),
    proposal_content_digest TEXT NOT NULL,
    memory_namespace_id TEXT NOT NULL,
    normalized_key TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'accepted', 'rejected', 'expired')
    ),
    resolution_event_id TEXT,
    created_at_ms INTEGER NOT NULL,
    CHECK (
        (status = 'pending' AND resolution_event_id IS NULL)
        OR (status <> 'pending' AND resolution_event_id IS NOT NULL)
    ),
    FOREIGN KEY (
        memory_namespace_id,
        normalized_key,
        proposal_id,
        proposal_version,
        proposal_content_digest
    ) REFERENCES memory_proposals (
        memory_namespace_id,
        normalized_key,
        proposal_id,
        version,
        content_digest
    ) DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (proposal_id, status, resolution_event_id)
        REFERENCES memory_proposal_resolutions(
            proposal_id,
            status,
            resolution_event_id
        ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_current_memory_proposal_status

CREATE INDEX current_memory_proposals_pending_idx
ON current_memory_proposal_status (
    memory_namespace_id,
    status,
    created_at_ms ASC,
    proposal_id ASC
);
-- migration-boundary: create_current_memory_proposals_pending_idx

CREATE INDEX current_memory_proposals_all_idx
ON current_memory_proposal_status (
    memory_namespace_id,
    created_at_ms DESC,
    proposal_id ASC
);
-- migration-boundary: create_current_memory_proposals_all_idx
```

Create immutable episodic summaries and their ordered exact source references.
The insert trigger requires sources to be appended contiguously, strictly by
event sequence, and before the summary-recorded event:

```sql
CREATE TABLE episodic_summaries (
    summary_id TEXT PRIMARY KEY CHECK (
        typeof(summary_id) = 'text'
        AND length(CAST(summary_id AS BLOB)) = 36
        AND summary_id = lower(summary_id)
        AND substr(summary_id, 9, 1) = '-'
        AND substr(summary_id, 14, 1) = '-'
        AND substr(summary_id, 19, 1) = '-'
        AND substr(summary_id, 24, 1) = '-'
        AND length(replace(summary_id, '-', '')) = 32
        AND replace(summary_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    version INTEGER NOT NULL CHECK (version = 1),
    memory_namespace_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    profile_version_id TEXT NOT NULL,
    profile_version INTEGER NOT NULL CHECK (profile_version >= 1),
    profile_content_digest TEXT NOT NULL,
    label TEXT NOT NULL CHECK (
        length(CAST(label AS BLOB)) BETWEEN 1 AND 128
        AND instr(label, char(0)) = 0
    ),
    body TEXT NOT NULL CHECK (
        length(CAST(body AS BLOB)) BETWEEN 1 AND 8192
        AND instr(body, char(0)) = 0
    ),
    purpose_tags_json BLOB NOT NULL CHECK (
        typeof(purpose_tags_json) = 'blob'
        AND json_valid(CAST(purpose_tags_json AS TEXT))
        AND json_type(CAST(purpose_tags_json AS TEXT)) = 'array'
        AND json_array_length(CAST(purpose_tags_json AS TEXT)) BETWEEN 0 AND 8
    ),
    source_count INTEGER NOT NULL CHECK (source_count BETWEEN 1 AND 128),
    plaintext_validation_version INTEGER NOT NULL CHECK (
        plaintext_validation_version = 1
    ),
    created_at_ms INTEGER NOT NULL,
    creation_event_sequence INTEGER NOT NULL UNIQUE CHECK (
        creation_event_sequence >= 1
    ),
    creation_event_id TEXT NOT NULL UNIQUE REFERENCES event_stream(event_id)
        DEFERRABLE INITIALLY DEFERRED,
    source_set_digest TEXT NOT NULL CHECK (
        typeof(source_set_digest) = 'text'
        AND length(CAST(source_set_digest AS BLOB)) = 64
        AND source_set_digest = lower(source_set_digest)
        AND source_set_digest NOT GLOB '*[^0-9a-f]*'
    ),
    content_digest TEXT NOT NULL CHECK (
        typeof(content_digest) = 'text'
        AND length(CAST(content_digest AS BLOB)) = 64
        AND content_digest = lower(content_digest)
        AND content_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_digest TEXT NOT NULL CHECK (
        typeof(record_digest) = 'text'
        AND length(CAST(record_digest AS BLOB)) = 64
        AND record_digest = lower(record_digest)
        AND record_digest NOT GLOB '*[^0-9a-f]*'
    ),
    record_json BLOB NOT NULL CHECK (
        typeof(record_json) = 'blob'
        AND json_valid(CAST(record_json AS TEXT))
    ),
    UNIQUE (
        summary_id,
        version,
        memory_namespace_id,
        profile_id,
        profile_version_id,
        profile_version,
        profile_content_digest,
        creation_event_sequence,
        creation_event_id,
        source_set_digest,
        content_digest
    ),
    FOREIGN KEY (
        profile_id,
        profile_version_id,
        profile_version,
        profile_content_digest,
        memory_namespace_id
    ) REFERENCES agent_profile_versions (
        profile_id,
        profile_version_id,
        version,
        content_digest,
        memory_namespace_id
    ) DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_episodic_summaries

CREATE INDEX episodic_summaries_list_idx
ON episodic_summaries (memory_namespace_id, created_at_ms DESC, summary_id ASC);
-- migration-boundary: create_episodic_summaries_list_idx

CREATE TABLE episodic_summary_sources (
    summary_id TEXT NOT NULL REFERENCES episodic_summaries(summary_id)
        DEFERRABLE INITIALLY DEFERRED,
    source_ordinal INTEGER NOT NULL CHECK (source_ordinal BETWEEN 0 AND 127),
    event_sequence INTEGER NOT NULL CHECK (event_sequence >= 1),
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (
        length(CAST(event_type AS BLOB)) >= 1
        AND instr(event_type, char(0)) = 0
    ),
    event_digest TEXT NOT NULL CHECK (
        typeof(event_digest) = 'text'
        AND length(CAST(event_digest AS BLOB)) = 64
        AND event_digest = lower(event_digest)
        AND event_digest NOT GLOB '*[^0-9a-f]*'
    ),
    PRIMARY KEY (summary_id, source_ordinal),
    UNIQUE (summary_id, event_id),
    UNIQUE (summary_id, event_sequence),
    FOREIGN KEY (event_sequence, event_id, event_type, event_digest)
        REFERENCES event_stream(sequence, event_id, event_type, event_digest)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;
-- migration-boundary: create_episodic_summary_sources

CREATE INDEX episodic_summary_sources_event_idx
ON episodic_summary_sources (event_id, summary_id);
-- migration-boundary: create_episodic_summary_sources_event_idx

CREATE TRIGGER episodic_summary_sources_order_guard
BEFORE INSERT ON episodic_summary_sources
WHEN NOT EXISTS (
        SELECT 1
        FROM episodic_summaries summary
        WHERE summary.summary_id = NEW.summary_id
    )
 OR NEW.source_ordinal <> (
        SELECT COUNT(*)
        FROM episodic_summary_sources existing
        WHERE existing.summary_id = NEW.summary_id
    )
 OR NEW.source_ordinal >= (
        SELECT summary.source_count
        FROM episodic_summaries summary
        WHERE summary.summary_id = NEW.summary_id
    )
 OR NEW.event_sequence >= (
        SELECT summary.creation_event_sequence
        FROM episodic_summaries summary
        WHERE summary.summary_id = NEW.summary_id
    )
 OR (
        NEW.source_ordinal > 0
        AND NEW.event_sequence <= (
            SELECT MAX(existing.event_sequence)
            FROM episodic_summary_sources existing
            WHERE existing.summary_id = NEW.summary_id
        )
    )
BEGIN
    SELECT RAISE(ABORT, 'episodic_summary_source_order_mismatch');
END;
-- migration-boundary: create_episodic_summary_sources_order_guard

CREATE TRIGGER episodic_summaries_no_update
BEFORE UPDATE ON episodic_summaries BEGIN
    SELECT RAISE(ABORT, 'episodic_summaries_immutable');
END;
-- migration-boundary: create_episodic_summaries_no_update

CREATE TRIGGER episodic_summaries_no_delete
BEFORE DELETE ON episodic_summaries BEGIN
    SELECT RAISE(ABORT, 'episodic_summaries_immutable');
END;
-- migration-boundary: create_episodic_summaries_no_delete

CREATE TRIGGER episodic_summary_sources_no_update
BEFORE UPDATE ON episodic_summary_sources BEGIN
    SELECT RAISE(ABORT, 'episodic_summary_sources_immutable');
END;
-- migration-boundary: create_episodic_summary_sources_no_update

CREATE TRIGGER episodic_summary_sources_no_delete
BEFORE DELETE ON episodic_summary_sources BEGIN
    SELECT RAISE(ABORT, 'episodic_summary_sources_immutable');
END;
-- migration-boundary: create_episodic_summary_sources_no_delete
```

Repository code must still compare canonical JSON with every mirrored column,
verify each creation event's sequence/type/payload binding, require each
summary's stored source count to equal its source rows before commit, and match
the generic approval row to the exact proposal/resolution. Those correlations
cannot be expressed as row-local SQLite `CHECK` constraints.

- [ ] **Step 5: Expose v4 migration boundaries for fault injection**

```rust
impl Database {
    #[doc(hidden)]
    pub fn v4_migration_boundaries() -> Vec<&'static str> {
        let migrations = ordered();
        let migration = migrations
            .iter()
            .find(|migration| migration.version == 4)
            .expect("schema v4 migration is registered");
        let mut boundaries = migration_boundary_names(migration.sql);
        boundaries.push("schema_migration_record");
        boundaries
    }
}
```

- [ ] **Step 6: Write and run the all-or-nothing rollback contract**

For every returned boundary, inject failure through the real migration transaction and assert schema version, ledger, inventory, and all v3 data remain byte-identical with no partial v4 object.

```bash
cargo test --test memory_migration_contract migration_failure_at_every_v4_boundary_rolls_back
```

- [ ] **Step 7: Rerun GREEN**

```bash
cargo test --test memory_migration_contract --test migration_contract --test agent_profile_migration_contract --test skill_migration_contract
```

- [ ] **Step 8: Commit**

```bash
git add migrations/0004_hybrid_memory.sql src/persistence/migrations.rs src/persistence/database.rs tests/memory_migration_contract.rs tests/migration_contract.rs tests/support/mod.rs
git commit -m "feat: add hybrid memory schema"
```

### Task 7: Persist and Authenticate Entries, Proposals, Approvals, and Summaries

**Files:**

- Create: `src/persistence/memory_repository.rs`
- Modify: `src/persistence/mod.rs`
- Modify: `src/persistence/database.rs`
- Modify: `src/domain/error.rs`
- Create: `tests/memory_persistence_contract.rs`
- Create: `tests/memory_integrity_contract.rs`

**Interfaces:**

- Consumes: `ImmediateTransaction::transaction(&self) -> &rusqlite::Transaction<'_>`; schema-v4 tables from Task 6; `MemoryEntryVersion`, `MemoryProposal`, `MemoryProposalResolution`, `ApprovalRecord`, `EpisodicSummary`, `MemoryProjection`, `MemoryRetrievalRequest`; `canonical_json_bytes<T: Serialize>(&T) -> Result<Vec<u8>, DomainError>`; and `MemorySnapshotBuilder::{new, consider_entry, consider_summary, finish}`.
- Produces: `PersistenceError::Capacity`; every `MemoryRepository` method with the exact parameter/return types in Steps 3 and 5; `MemoryEntryHistoryPage`, `MemoryEntryListRecord`, `MemoryEntriesPage`, `MemoryProposalListRecord`, `MemoryProposalsPage`, `EpisodicSummaryListRecord`, and `EpisodicSummariesPage` with the exact field types in Step 3.

- [ ] **Step 1: Write failing repository round-trip and immutability tests**

Cover exact/idempotent insert, same-ID mismatch, independent-column tampering, canonical JSON mismatch, digest mismatch, normalized-key collision, predecessor substitution, namespace/profile mismatch, proposal/approval mismatch, requester actor kind/ID substitution, second resolution, source identity/order mismatch, SQL-before-materialization limits, current tombstone pointer, stable sort orders, pending/all filters, and transaction rollback.

```rust
#[test]
fn existing_immutable_row_must_match_every_authenticated_column() {
    let fixture = memory_repository_fixture();
    fixture.insert_entry().unwrap();
    fixture.tamper_entry_column("content_digest", other_digest());
    let error = fixture.load_current_entry().unwrap_err();
    assert_eq!(error.code(), "memory_row_mismatch");
}
```

- [ ] **Step 2: Run repository contracts to verify RED**

Run:

```bash
cargo test --test memory_persistence_contract --test memory_integrity_contract
```

Expected RED: compilation fails because `MemoryRepository` does not exist.

- [ ] **Step 3: Implement exact immutable-row codecs and lookups**

The following is the exact Rust interface contract; append `Capacity` to the existing error enum, copy the data definitions, and implement every declaration ending in `;` in this step:

```text
// Append to the existing PersistenceError enum in database.rs.
pub enum PersistenceError {
    Capacity,
}

pub struct MemoryRepository;

impl MemoryRepository {
    pub fn insert_entry_version(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        entry: &MemoryEntryVersion,
    ) -> Result<(), PersistenceError>;

    pub fn replace_current_entry(
        tx: &ImmediateTransaction<'_>,
        entry: &MemoryEntryVersion,
    ) -> Result<(), PersistenceError>;

    pub fn load_current_entry(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
    ) -> Result<Option<MemoryEntryVersion>, PersistenceError>;

    pub fn load_entry_history(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
        limit: u16,
    ) -> Result<MemoryEntryHistoryPage, PersistenceError>;

    pub fn list_current_entries(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        limit: u16,
    ) -> Result<MemoryEntriesPage, PersistenceError>;

    pub fn load_entry_version(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
        version: ObjectVersion,
    ) -> Result<Option<MemoryEntryVersion>, PersistenceError>;

    pub fn insert_proposal_with_approval(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        proposal: &MemoryProposal,
        approval: &ApprovalRecord,
    ) -> Result<(), PersistenceError>;

    pub fn resolve_proposal(
        tx: &ImmediateTransaction<'_>,
        resolution_sequence: u64,
        resolution: &MemoryProposalResolution,
        approval: &ApprovalRecord,
    ) -> Result<(), PersistenceError>;

    pub fn list_proposals(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        filter: MemoryProposalFilter,
        limit: u16,
    ) -> Result<MemoryProposalsPage, PersistenceError>;

    pub fn load_proposal(
        tx: &ImmediateTransaction<'_>,
        proposal_id: MemoryProposalId,
    ) -> Result<Option<(MemoryProposal, MemoryProposalStatus, Option<MemoryProposalResolution>)>, PersistenceError>;

    pub fn count_pending_proposals(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
    ) -> Result<u64, PersistenceError>;

    pub fn count_active_entries(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
    ) -> Result<u64, PersistenceError>;

    pub fn load_pending_proposals_for_key(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
    ) -> Result<Vec<MemoryProposal>, PersistenceError>;

    pub fn load_memory_approval(
        tx: &ImmediateTransaction<'_>,
        approval_id: ApprovalId,
    ) -> Result<Option<ApprovalRecord>, PersistenceError>;

    pub fn validate_projection(
        tx: &ImmediateTransaction<'_>,
        projection: &MemoryProjection,
    ) -> Result<(), PersistenceError>;

    pub(crate) fn validate_entry_context(
        tx: &ImmediateTransaction<'_>,
        entry: &MemoryEntryVersion,
    ) -> Result<(), PersistenceError>;

    pub(crate) fn validate_proposal_context(
        tx: &ImmediateTransaction<'_>,
        proposal: &MemoryProposal,
    ) -> Result<(), PersistenceError>;

    pub(crate) fn validate_summary_context(
        tx: &ImmediateTransaction<'_>,
        summary: &EpisodicSummary,
    ) -> Result<(), PersistenceError>;

    pub(crate) fn insert_episodic_summary(
        tx: &ImmediateTransaction<'_>,
        creation_sequence: u64,
        summary: &EpisodicSummary,
    ) -> Result<(), PersistenceError>;

    pub fn list_episodic_summaries(
        tx: &ImmediateTransaction<'_>,
        namespace: MemoryNamespaceId,
        limit: u16,
    ) -> Result<EpisodicSummariesPage, PersistenceError>;

    pub fn load_episodic_summary(
        tx: &ImmediateTransaction<'_>,
        summary_id: EpisodicSummaryId,
    ) -> Result<Option<EpisodicSummary>, PersistenceError>;
}

pub struct MemoryEntryHistoryPage {
    pub versions: Vec<MemoryEntryVersion>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

pub struct MemoryEntryListRecord {
    pub entry: MemoryEntryRef,
    pub display_key: String,
    pub purpose_tags: Vec<String>,
    pub value_bytes: u64,
    pub created_at_ms: i64,
}

pub struct MemoryEntriesPage {
    pub records: Vec<MemoryEntryListRecord>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

pub struct MemoryProposalListRecord {
    pub proposal: MemoryProposalRef,
    pub proposer: AgentProfileVersionRef,
    pub operation: MemoryProposalOperationKind,
    pub display_key: String,
    pub status: MemoryProposalStatus,
    pub created_at_ms: i64,
}

pub struct MemoryProposalsPage {
    pub records: Vec<MemoryProposalListRecord>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

pub struct EpisodicSummaryListRecord {
    pub summary: EpisodicSummaryRef,
    pub label: String,
    pub purpose_tags: Vec<String>,
    pub source_count: u64,
    pub created_at_ms: i64,
}

pub struct EpisodicSummariesPage {
    pub records: Vec<EpisodicSummaryListRecord>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}
```

Follow the exact-row comparison pattern already used for skills: decode canonical JSON into validated domain values, recompute content/record digests, compare every mirrored column, and accept an existing row only if it is byte-identical. Encode and decode approval requester actors with the same strict Human/System/Agent kind-ID matrix as events; require the proposal Agent ID to equal the pinned proposer profile. Map failures to stable content-free error codes.

At this layer, `validate_entry_context` loads the exact predecessor (when present), requires same namespace/logical entry/key, and exact version adjacency; `validate_proposal_context` resolves the exact proposer profile, derives the namespace, and verifies the approval requester/object binding; `validate_summary_context` resolves the exact profile namespace and every source sequence/ID/type/digest against committed event rows. The insert and every decode/load path call the matching validator before returning a record. Canonical record JSON, mirrored columns, and self-digests must also agree. Task 8 adds the typed payloads that correlate those rows to authoritative envelope content, and Task 12 performs the full verified-stream reconciliation; Task 7 must not claim payload authentication before those contracts exist.

- [ ] **Step 4: Add bounded current/proposal/episodic queries**

Implement exact reference lookup and deliberate detail; `LIMIT 100` page queries with separate checked `COUNT(*)`; entry history newest first; pending proposals oldest first; all proposals newest first; summaries newest first. Keep values/rationale/body out of summary row types but return them for deliberate detail types.

Use explicit stable SQL orderings rather than post-query sorting:

```sql
SELECT v.entry_version_id, v.display_key, v.purpose_tags_json,
       v.value_bytes, v.created_at_ms
FROM current_memory_entries AS c
JOIN memory_entry_versions AS v
  ON v.memory_namespace_id = c.memory_namespace_id
 AND v.normalized_key = c.normalized_key
 AND v.entry_id = c.entry_id
 AND v.entry_version_id = c.entry_version_id
WHERE c.memory_namespace_id = ?1 AND v.state = 'present'
ORDER BY c.normalized_key ASC, c.entry_id ASC
LIMIT ?2;

SELECT p.proposal_id
FROM current_memory_proposal_status AS c
JOIN memory_proposals AS p ON p.proposal_id = c.proposal_id
WHERE c.memory_namespace_id = ?1 AND (?2 = 'all' OR c.status = 'pending')
ORDER BY
  CASE WHEN ?2 = 'pending' THEN p.created_at_ms END ASC,
  CASE WHEN ?2 = 'all' THEN p.created_at_ms END DESC,
  p.proposal_id ASC
LIMIT ?3;

SELECT summary_id
FROM episodic_summaries
WHERE memory_namespace_id = ?1
ORDER BY created_at_ms DESC, summary_id ASC
LIMIT ?2;
```

- [ ] **Step 5: Add streamed snapshot construction inside the transaction**

The following is the exact Rust interface contract; implement the declaration body in this step:

```text
impl MemoryRepository {
    pub fn build_snapshot(
        tx: &ImmediateTransaction<'_>,
        request: &MemoryRetrievalRequest,
    ) -> Result<MemorySnapshot, PersistenceError>;
}
```

Issue deterministic ordered SQL passes for tagged matches and untagged fallback. Order each KV group by normalized key, object version, then stable entry ID; order each summary group by creation time descending, then summary ID. Select each tag-intersecting record once even when multiple requested tags match. Validate current pointers and every immutable/source reference while each row is visited. Feed one candidate at a time to `MemorySnapshotBuilder`; do not collect all eligible rows. Finish complete content materialization before the caller appends the metadata event and receipt in the same transaction.

- [ ] **Step 6: Rerun GREEN and repository regressions**

```bash
cargo test --test memory_persistence_contract --test memory_integrity_contract --test agent_profile_persistence_contract --test skill_persistence_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/persistence src/domain/error.rs tests/memory_persistence_contract.rs tests/memory_integrity_contract.rs
git commit -m "feat: persist and authenticate hybrid memory"
```

### Task 8: Define Memory Commands, Events, Views, Capabilities, and Reduction

**Files:**

- Modify: `src/policy/capability.rs`
- Modify: `src/app/command.rs`
- Modify: `src/app/event.rs`
- Modify: `src/app/outcome.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/app/service.rs`
- Modify: `src/audit/mod.rs`
- Modify: `src/recovery/reducer.rs`
- Modify: `src/persistence/projection_repository.rs`
- Modify: `src/ui/command/renderer.rs`
- Modify: `src/ui/tui/controller.rs`
- Create: `tests/memory_event_contract.rs`
- Create: `tests/memory_application_contract.rs`
- Modify: `tests/projection_contract.rs`

**Interfaces:**

- Consumes: `ApplicationService::execute(&mut self, envelope: CommandEnvelope) -> Result<CommandOutcome, AppError>`; `reduce(state: &mut ProjectionState, event: &EventEnvelope) -> Result<ReducerEffect, RecoveryError>`; and the exact `AgentProfileVersionRef`, `ExpectedMemoryEntryState`, `MemoryEntryDraft`, `MemoryEntryVersion`, `MemoryEntryRef`, `MemoryProposal`, `MemoryProposalRef`, `MemoryProposalResolution`, `MemoryRetrievalRequest`, `MemorySnapshot`, `MemorySnapshotMetadata`, `ApprovalRecord`, and `MemoryProjection` types defined in Tasks 1–7.
- Produces: `Capability::{MemoryRead, MemoryPreview, MemoryMutate, MemoryPropose, MemoryResolve}`; the fourteen `ApplicationCommand` variants and fifteen `ApplicationEvent` variants shown below with their complete field types; the twelve `CommandView` variants and payload structs shown below; `MemoryProposalStatusRef { proposal: MemoryProposalRef, status: MemoryProposalStatus }`; `ProjectionState.memory: MemoryProjection`; and exhaustive `reduce`, command renderer, and TUI outcome match arms for every new event/view.

- [ ] **Step 1: Write failing command/event/view compatibility tests**

Assert deny-unknown-field tagged serde; exact five-capability mapping; actor×command matrix vocabulary; all stable event kinds; one-primary-object rules; metadata-only read events containing references/counts/digests but no KV values, proposal candidate/rationale, episodic labels/bodies, or source lists; audit payloads containing no raw memory prose at all; complete immutable records in mutation events; unique proposal expirations sorted by proposal ID and targeting the same logical key; and `EVENT_SCHEMA_VERSION == 1` with unchanged legacy event bytes.

```rust
#[test]
fn memory_read_event_contains_refs_but_not_plaintext() {
    let envelope = seal_memory_entries_listed_fixture();
    let json = serde_json::to_string(&envelope).unwrap();
    assert!(json.contains(&entry_ref_fixture().entry_version_id().to_string()));
    assert!(!json.contains("private investment thesis"));
    assert_eq!(envelope.event_schema_version, 1);
}
```

- [ ] **Step 2: Run command/event compatibility contracts to verify RED**

Run:

```bash
cargo test --test memory_event_contract --test memory_application_contract
```

Expected RED: compilation fails because the application vocabulary has no memory variants.

- [ ] **Step 3: Add the five exact capabilities and durable commands**

```rust
// Append these variants to the existing Capability enum.
pub enum Capability {
    MemoryRead,
    MemoryPreview,
    MemoryMutate,
    MemoryPropose,
    MemoryResolve,
}

// Append these variants to the existing ApplicationCommand enum.
pub enum ApplicationCommand {
    SetMemoryEntry {
        profile: AgentProfileVersionRef,
        expected: ExpectedMemoryEntryState,
        candidate: MemoryEntryDraft,
        review_token: MemoryReviewToken,
        review_digest: Digest,
    },
    DeleteMemoryEntry {
        profile: AgentProfileVersionRef,
        expected: MemoryEntryRef,
        review_token: MemoryReviewToken,
        review_digest: Digest,
    },
    ProposeMemoryMutation {
        proposer: AgentProfileVersionRef,
        expected: ExpectedMemoryEntryState,
        operation: MemoryProposalOperation,
        rationale: String,
    },
    ApproveMemoryProposal {
        proposal: MemoryProposalRef,
        approval_id: ApprovalId,
        expected_approval_status: ApprovalStatus,
        expected_entry: ExpectedMemoryEntryState,
        review_token: MemoryReviewToken,
        review_digest: Digest,
    },
    RejectMemoryProposal {
        proposal: MemoryProposalRef,
        approval_id: ApprovalId,
        expected_approval_status: ApprovalStatus,
        expected_entry: ExpectedMemoryEntryState,
        review_token: MemoryReviewToken,
        review_digest: Digest,
    },
    ListMemoryEntries { selector: AgentProfileSelector },
    ShowMemoryEntry { selector: AgentProfileSelector, display_key: String },
    ShowMemoryEntryHistory { selector: AgentProfileSelector, display_key: String },
    ShowMemoryEntryVersion {
        selector: AgentProfileSelector,
        display_key: String,
        version: ObjectVersion,
    },
    ListMemoryProposals {
        selector: AgentProfileSelector,
        filter: MemoryProposalFilter,
    },
    ShowMemoryProposal { proposal_id: MemoryProposalId },
    ListEpisodicSummaries { selector: AgentProfileSelector },
    ShowEpisodicSummary { summary_id: EpisodicSummaryId },
    BuildMemorySnapshot { request: MemoryRetrievalRequest },
}
```

Canonicalize nested memory payloads before fingerprinting. Map reads/snapshot to `MemoryRead`, direct edits to `MemoryMutate`, internal creation to `MemoryPropose`, and approve/reject to `MemoryResolve`. `MemoryPreview` is passive only.

Add two content-free staged application errors and their exact codes in `src/app/mod.rs`:

```rust
pub enum AppError {
    MemoryCommandNotImplemented,
    WrongMemoryCommandDispatcher,
}

// Add these arms to AppError::code().
AppError::MemoryCommandNotImplemented => "memory_command_not_implemented",
AppError::WrongMemoryCommandDispatcher => "wrong_memory_command_dispatcher",
```

Task 8 adds explicit service match arms that return `MemoryCommandNotImplemented` for all fourteen new commands so the vocabulary commit compiles without pretending the handlers exist. Tasks 9–11 replace every staged arm, and Task 11 removes `MemoryCommandNotImplemented` plus its code arm. The private Task 9 read dispatcher retains `WrongMemoryCommandDispatcher` for an impossible non-read call.

- [ ] **Step 4: Add typed outcomes and one-event payloads**

Use these names consistently across service and both interfaces:

```text
MemoryEntriesView
MemoryEntryView
MemoryEntryHistoryView
MemoryEntryVersionView
MemoryProposalsView
MemoryProposalView
EpisodicSummariesView
EpisodicSummaryView
MemoryEntryMutationView
MemoryProposalCreatedView
MemoryProposalResolutionView
MemorySnapshotView
```

Append this exact view mapping to the existing enum:

```rust
pub enum CommandView {
    MemoryEntries(MemoryEntriesView),
    MemoryEntry(MemoryEntryView),
    MemoryEntryHistory(MemoryEntryHistoryView),
    MemoryEntryVersion(MemoryEntryVersionView),
    MemoryProposals(MemoryProposalsView),
    MemoryProposal(MemoryProposalView),
    EpisodicSummaries(EpisodicSummariesView),
    EpisodicSummary(EpisodicSummaryView),
    MemoryEntryMutation(MemoryEntryMutationView),
    MemoryProposalCreated(MemoryProposalCreatedView),
    MemoryProposalResolution(MemoryProposalResolutionView),
    MemorySnapshot(MemorySnapshotView),
}
```

Define the view payloads explicitly around exact refs and bounded counts:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntrySummary {
    pub entry: MemoryEntryRef,
    pub display_key: String,
    pub purpose_tags: Vec<String>,
    pub value_bytes: u64,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntriesView {
    pub profile: AgentProfileVersionRef,
    pub namespace_id: MemoryNamespaceId,
    pub entries: Vec<MemoryEntrySummary>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryView {
    pub profile: AgentProfileVersionRef,
    pub entry: MemoryEntryVersion,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryHistoryView {
    pub profile: AgentProfileVersionRef,
    pub current: MemoryEntryRef,
    pub versions: Vec<MemoryEntryHistorySummary>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryVersionView {
    pub profile: AgentProfileVersionRef,
    pub entry: MemoryEntryVersion,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalsView {
    pub profile: AgentProfileVersionRef,
    pub namespace_id: MemoryNamespaceId,
    pub filter: MemoryProposalFilter,
    pub proposals: Vec<MemoryProposalSummary>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProfileIdentityView {
    pub profile: AgentProfileVersionRef,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalView {
    pub proposal: MemoryProposal,
    pub status: MemoryProposalStatus,
    pub resolution: Option<MemoryProposalResolution>,
    pub current_entry: ExpectedMemoryEntryState,
    pub proposer_is_historical: bool,
    pub proposer_identity: MemoryProfileIdentityView,
    pub namespace_owner_identity: MemoryProfileIdentityView,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodicSummariesView {
    pub profile: AgentProfileVersionRef,
    pub namespace_id: MemoryNamespaceId,
    pub summaries: Vec<EpisodicSummaryListItem>,
    pub total_count: u64,
    pub returned_count: u64,
    pub omitted_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodicSummaryView {
    pub summary: EpisodicSummary,
    pub qualification: EpisodicQualification,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryMutationView {
    pub entry: MemoryEntryRef,
    pub expired_proposals: Vec<MemoryProposalRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalCreatedView {
    pub proposal: MemoryProposalRef,
    pub approval_id: ApprovalId,
    pub status: MemoryProposalStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalResolutionView {
    pub resolution: MemoryProposalResolution,
    pub entry: Option<MemoryEntryRef>,
    pub expired_proposals: Vec<MemoryProposalRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemorySnapshotView {
    pub snapshot: MemorySnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryHistorySummary {
    pub entry: MemoryEntryRef,
    pub display_key: String,
    pub created_at_ms: i64,
    pub accepted_proposal: Option<MemoryProposalRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalSummary {
    pub proposal: MemoryProposalRef,
    pub proposer: AgentProfileVersionRef,
    pub operation: MemoryProposalOperationKind,
    pub display_key: String,
    pub status: MemoryProposalStatus,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodicSummaryListItem {
    pub summary: EpisodicSummaryRef,
    pub label: String,
    pub purpose_tags: Vec<String>,
    pub source_count: u64,
    pub created_at_ms: i64,
}
```

These strict list structs deliberately omit the full KV value, proposal rationale/value, and episodic body.

Mutation variants:

```rust
MemoryEntrySet {
    entry: MemoryEntryVersion,
    expired_proposals: Vec<MemoryProposalResolution>,
},
MemoryEntryDeleted {
    entry: MemoryEntryVersion,
    expired_proposals: Vec<MemoryProposalResolution>,
},
MemoryProposalCreated {
    proposal: MemoryProposal,
    approval: ApprovalRecord,
},
MemoryProposalAccepted {
    resolution: MemoryProposalResolution,
    entry: MemoryEntryVersion,
    expired_proposals: Vec<MemoryProposalResolution>,
},
MemoryProposalRejected { resolution: MemoryProposalResolution },
EpisodicSummaryRecorded { summary: EpisodicSummary }, // fixture-only
```

Add metadata-only list/detail/read event variants and `MemorySnapshotBuilt { metadata: MemorySnapshotMetadata }`. Snapshot metadata pins exact accepted entry/summary/source references and all counts, but the receipt outcome retains the complete deliberately requested content.

Use these exact metadata shapes for read events:

```rust
MemoryEntriesListed {
    profile: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    entries: Vec<MemoryEntryRef>,
    total_count: u64,
    returned_count: u64,
    omitted_count: u64,
},
MemoryEntryShown {
    profile: AgentProfileVersionRef,
    entry: MemoryEntryRef,
},
MemoryEntryHistoryShown {
    profile: AgentProfileVersionRef,
    current: MemoryEntryRef,
    versions: Vec<MemoryEntryRef>,
    total_count: u64,
    returned_count: u64,
    omitted_count: u64,
},
MemoryEntryVersionShown {
    profile: AgentProfileVersionRef,
    entry: MemoryEntryRef,
},
MemoryProposalsListed {
    profile: AgentProfileVersionRef,
    filter: MemoryProposalFilter,
    proposals: Vec<MemoryProposalStatusRef>,
    total_count: u64,
    returned_count: u64,
    omitted_count: u64,
},
MemoryProposalShown {
    proposal: MemoryProposalRef,
    status: MemoryProposalStatus,
    resolution: Option<MemoryProposalResolution>,
},
EpisodicSummariesListed {
    profile: AgentProfileVersionRef,
    summaries: Vec<EpisodicSummaryRef>,
    total_count: u64,
    returned_count: u64,
    omitted_count: u64,
},
EpisodicSummaryShown { summary: EpisodicSummaryRef },
MemorySnapshotBuilt { metadata: MemorySnapshotMetadata },
```

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposalStatusRef {
    pub proposal: MemoryProposalRef,
    pub status: MemoryProposalStatus,
}
```

It contains no candidate or rationale text.

- [ ] **Step 5: Reduce memory events into a compatible `ProjectionState`**

```rust
// Add this field to the existing ProjectionState.
pub struct ProjectionState {
    #[serde(default, skip_serializing_if = "MemoryProjection::is_empty")]
    pub memory: MemoryProjection,
}
```

Apply events to a clone, validate embedded record event ID/time/actor against the envelope, primary object identity, predecessor/current state, proposal pending→terminal transitions, accepted proposal association, and every sibling expiration before assigning the clone. Read events never change memory state. Keep the memory field absent from canonical bytes and digest material while empty. Add exhaustive `CommandView` arms to the existing text renderer and TUI outcome applicator now: render safe IDs/counts/status for text mode and retain a safe generic TUI message without opening a Memory pane. Tasks 15 and 17 replace those minimal arms with full view hydration after their state types exist.

- [ ] **Step 6: Prove legacy, empty-v4, and nonempty-v4 projection bytes**

```bash
cargo test --test memory_event_contract --test memory_projection_contract --test projection_contract --test memory_application_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/policy/capability.rs src/app src/audit/mod.rs src/recovery/reducer.rs src/persistence/projection_repository.rs src/ui/command/renderer.rs src/ui/tui/controller.rs tests/memory_event_contract.rs tests/memory_application_contract.rs tests/projection_contract.rs
git commit -m "feat: define hybrid memory application contracts"
```

### Task 9: Implement Bounded Reads, Snapshot Materialization, and Historical Receipt Replay

**Files:**

- Modify: `src/app/service.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/audit/mod.rs`
- Modify: `tests/support/mod.rs`
- Create: `tests/memory_application_read_contract.rs`
- Create: `tests/memory_receipt_contract.rs`

**Interfaces:**

- Consumes: `ApplicationService::execute(&mut self, envelope: CommandEnvelope) -> Result<CommandOutcome, AppError>`; `AgentProfilesProjection::resolve_reference(&self, reference: &AgentProfileVersionRef) -> Result<&AgentProfileVersion, DomainError>`; `MemoryRepository::{list_current_entries, load_current_entry, load_entry_history, load_entry_version, list_proposals, load_proposal, list_episodic_summaries, load_episodic_summary, build_snapshot}` with the exact Task 7 parameter and return types; and Task 8's exact memory command/event/view variants.
- Produces: private `enforce_memory_actor_command(actor: &Actor, command: &ApplicationCommand) -> Result<(), AppError>`; private `prepare_memory_read(tx: &ImmediateTransaction<'_>, profiles: &AgentProfilesProjection, command: &ApplicationCommand) -> Result<(ApplicationEvent, CommandView), AppError>`; private `prepare_memory_snapshot(tx: &ImmediateTransaction<'_>, profiles: &AgentProfilesProjection, request: &MemoryRetrievalRequest) -> Result<(ApplicationEvent, CommandView), AppError>`; and private `materialize_memory_view(tx: &ImmediateTransaction<'_>, command: &ApplicationCommand, event: &EventEnvelope, projection_at_event: &ProjectionState) -> Result<CommandView, AppError>`.

- [ ] **Step 1: Write failing read, retrieval, replay, and redaction tests**

Cover selector-to-exact-profile resolution, namespace isolation, deliberate not-found, all SQL list bounds/orders/counts, tombstone visibility only in history/version, proposal filters, historical proposer warning, episodic metadata/detail labels, source validation, snapshot deterministic selection, full plaintext outcome, metadata-only event/audit, and one receipt/event per read.

Receipt tests must prove same-command byte-exact replay without policy, profile/current-state read, retrieval reselection, IDs, clocks, or writes; changed actor/request/budget conflicts; and snapshot replay returns the historical accepted content after current memory changes.

```rust
#[test]
fn snapshot_replay_uses_historical_refs_after_current_memory_changes() {
    let mut app = memory_app_fixture();
    let command = build_snapshot_command();
    let first = app.execute(command.clone()).unwrap();
    app.accept_another_edit_for_same_key();
    let writes_before = app.write_count();
    let replay = app.execute(command).unwrap();
    assert_eq!(replay, first);
    assert_eq!(app.write_count(), writes_before);
}
```

- [ ] **Step 2: Run read and replay contracts to verify RED**

Run:

```bash
cargo test --test memory_application_read_contract --test memory_receipt_contract
```

Expected RED: execution returns `memory_command_not_implemented`.

- [ ] **Step 3: Add actor guard and default local memory grants**

Implement private `enforce_memory_actor_command(actor: &Actor, command: &ApplicationCommand) -> Result<(), AppError>` with this exhaustive contract: Human is accepted for the eight deliberate reads, `BuildMemorySnapshot`, `SetMemoryEntry`, `DeleteMemoryEntry`, `ApproveMemoryProposal`, and `RejectMemoryProposal`; `Actor::Agent(id)` is accepted only for `ProposeMemoryMutation` whose `proposer.profile_id() == id`; System and every other actor/command pairing return the existing content-free capability-denied error for that command's mapped memory capability. No arm may inspect or copy raw memory prose into the error.

Run same-command receipt lookup first. For a fresh command, run the actor guard before policy-dependent work, review reservation, IDs, clock, or mutation. Extend `PhaseZeroPolicy.rules` from `[PolicyRule; 14]` to `[PolicyRule; 19]` and append grants for all five capabilities; explicit denial still wins. Agent is permitted only for a proposal whose exact proposer has the same stable profile ID; System gets no memory path.

- [ ] **Step 4: Materialize bounded reads through `MemoryRepository`**

Each read begins an immediate transaction, resolves and validates exact profile/namespace or typed object ID, queries at most the bound, constructs a safe metadata event, reduces the cloned projection, writes safe audit/receipt, and commits. Deliberate list outcomes may carry bounded display keys/tags/labels but not full KV values, proposal candidate values/rationale, or episodic bodies. Detail outcomes may carry the deliberately requested plaintext. Read events remain reference/count/digest-only, while audit metadata carries no raw memory prose.

Implement private `prepare_memory_read(tx: &ImmediateTransaction<'_>, profiles: &AgentProfilesProjection, command: &ApplicationCommand) -> Result<(ApplicationEvent, CommandView), AppError>`. It is the only dispatcher for the eight deliberate read variants, so its return type keeps the event/view pair inseparable.

Its exhaustive arms map `ListMemoryEntries`→`MemoryEntriesListed`/`CommandView::MemoryEntries`, `ShowMemoryEntry`→`MemoryEntryShown`/`MemoryEntry`, `ShowMemoryEntryHistory`→`MemoryEntryHistoryShown`/`MemoryEntryHistory`, `ShowMemoryEntryVersion`→`MemoryEntryVersionShown`/`MemoryEntryVersion`, `ListMemoryProposals`→`MemoryProposalsListed`/`MemoryProposals`, `ShowMemoryProposal`→`MemoryProposalShown`/`MemoryProposal`, `ListEpisodicSummaries`→`EpisodicSummariesListed`/`EpisodicSummaries`, and `ShowEpisodicSummary`→`EpisodicSummaryShown`/`EpisodicSummary`. Any non-read variant returns `AppError::WrongMemoryCommandDispatcher` without including input text.

- [ ] **Step 5: Build snapshots wholly inside the command transaction**

Resolve the exact profile reference and namespace, invoke streamed repository selection, finish all content/reference validation, then append `MemorySnapshotBuilt` and store the full `MemorySnapshotView` in the receipt before committing. Do not add a snapshot table or public UI command.

```rust
fn prepare_memory_snapshot(
    tx: &ImmediateTransaction<'_>,
    profiles: &AgentProfilesProjection,
    request: &MemoryRetrievalRequest,
) -> Result<(ApplicationEvent, CommandView), AppError> {
    let profile = profiles.resolve_reference(request.scope().profile())?;
    request.scope().validate_against(profile)?;
    let snapshot = MemoryRepository::build_snapshot(tx, request)?;
    let metadata = snapshot.metadata();
    Ok((
        ApplicationEvent::MemorySnapshotBuilt { metadata },
        CommandView::MemorySnapshot(MemorySnapshotView { snapshot }),
    ))
}
```

- [ ] **Step 6: Materialize replay only from the authenticated event and immutable refs**

Implement private `materialize_memory_view(tx: &ImmediateTransaction<'_>, command: &ApplicationCommand, event: &EventEnvelope, projection_at_event: &ProjectionState) -> Result<CommandView, AppError>`. The helper accepts exactly one authenticated receipt event, matches every memory command to its corresponding event variant, validates primary object/reference/count metadata against the canonical command, and returns the view reconstructed from the event plus immutable rows proven by that event. A command/event mismatch, zero events, or more than one event returns the existing content-free receipt-integrity error.

On replay, hydrate snapshot items from the exact accepted refs in event metadata; never run current retrieval selection. Reconstruct a proposal-created pending outcome from its event even if the live approval is now terminal. Require exactly one receipt event and validate its primary object/metadata against the request.

- [ ] **Step 7: Rerun GREEN and legacy receipt regressions**

```bash
cargo test --test memory_application_read_contract --test memory_receipt_contract --test skill_receipt_contract --test application_contract
```

- [ ] **Step 8: Commit**

```bash
git add src/app src/audit/mod.rs tests/support/mod.rs tests/memory_application_read_contract.rs tests/memory_receipt_contract.rs
git commit -m "feat: add exact memory reads and replay"
```

### Task 10: Implement Passive Human Preview and Direct Mutations

**Files:**

- Modify: `src/app/service.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/runtime/mod.rs`
- Modify: `src/audit/mod.rs`
- Modify: `tests/support/mod.rs`
- Create: `tests/memory_application_mutation_contract.rs`
- Create: `tests/memory_audit_contract.rs`

**Interfaces:**

- Consumes: `MemoryReviewRegistry::reserve_direct(&self, command_id: CommandId, token: MemoryReviewToken, supplied: &MemoryEditReviewBinding) -> Result<ReservedMemoryReview<'_>, DomainError>`; `ReservedMemoryReview::{command_id, token, consume, release, invalidate}`; `MemoryRepository::{load_current_entry, count_active_entries, load_pending_proposals_for_key, insert_entry_version, replace_current_entry, resolve_proposal}` with the exact Task 7 types; and `ApplicationService::execute(&mut self, envelope: CommandEnvelope) -> Result<CommandOutcome, AppError>`.
- Produces: `MemoryEditPreview::{NoChange(MemoryNoChange), Review(MemoryEditReview)}`; `ApplicationService::preview_memory_set(&self, selector: AgentProfileSelector, candidate: MemoryEntryDraft) -> Result<MemoryEditPreview, AppError>`; `ApplicationService::preview_memory_delete(&self, selector: AgentProfileSelector, display_key: String) -> Result<MemoryEditPreview, AppError>`; `ApplicationService::cancel_memory_review(&self) -> Result<(), AppError>`; matching `CommandExecutor` methods taking `&mut self` and returning `AppError`; matching `RuntimeClient` methods taking `&self` and returning `RuntimeError`; and private `prepare_direct_memory_mutation(tx: &ImmediateTransaction<'_>, projection: &ProjectionState, ids: &dyn IdGenerator, clock: &dyn Clock, actor: &Actor, command_id: CommandId, command: &ApplicationCommand, reserved: &ReservedMemoryReview<'_>) -> Result<ApplicationEvent, AppError>`.

- [ ] **Step 1: Write failing no-op, review, mutation, and audit tests**

Test set/create/overwrite/recreate and delete; explicit expected refs; two-stage confirmation; identical set and missing/tombstoned delete no-op; zero review token/event/receipt/audit/approval/ID/clock/write for no-op; direct edit creates no approval; stale state does not rebase or become no-op; Human-only token use; exact plaintext acknowledgment; the 1,024-active-entry boundary for absent/tombstone→present but not overwrite/delete; one event/receipt; immutable history growing by exactly one version per accepted mutation; sorted sibling expirations; and audit exclusion of all raw memory prose.

```rust
#[test]
fn identical_preview_is_a_zero_side_effect_noop() {
    let mut app = memory_app_with_present_entry();
    let before = app.all_side_effect_counters();
    let result = app.preview_memory_set(profile_selector(), identical_draft()).unwrap();
    assert_eq!(result, MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent));
    assert_eq!(app.all_side_effect_counters(), before);
    assert_eq!(app.registered_memory_reviews(), 0);
}
```

- [ ] **Step 2: Run direct-mutation contracts to verify RED**

Run:

```bash
cargo test --test memory_application_mutation_contract --test memory_audit_contract
```

Expected RED: compilation fails because passive preview methods are absent.

- [ ] **Step 3: Expose passive preview and cancellation through the service/runtime boundary**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryEditPreview {
    NoChange(MemoryNoChange),
    Review(MemoryEditReview),
}
```

Add these exact calls, mirroring the existing profile/skill request/reply delegation:

| Owner | Exact signature |
|---|---|
| `ApplicationService` | `preview_memory_set(&self, selector: AgentProfileSelector, candidate: MemoryEntryDraft) -> Result<MemoryEditPreview, AppError>` |
| `ApplicationService` | `preview_memory_delete(&self, selector: AgentProfileSelector, display_key: String) -> Result<MemoryEditPreview, AppError>` |
| `ApplicationService` | `cancel_memory_review(&self) -> Result<(), AppError>` |
| `CommandExecutor` | `preview_memory_set(&mut self, selector: AgentProfileSelector, candidate: MemoryEntryDraft) -> Result<MemoryEditPreview, AppError>` |
| `CommandExecutor` | `preview_memory_delete(&mut self, selector: AgentProfileSelector, display_key: String) -> Result<MemoryEditPreview, AppError>` |
| `CommandExecutor` | `cancel_memory_review(&mut self) -> Result<(), AppError>` |
| `RuntimeClient` | `preview_memory_set(&self, selector: AgentProfileSelector, candidate: MemoryEntryDraft) -> Result<MemoryEditPreview, RuntimeError>` |
| `RuntimeClient` | `preview_memory_delete(&self, selector: AgentProfileSelector, display_key: String) -> Result<MemoryEditPreview, RuntimeError>` |
| `RuntimeClient` | `cancel_memory_review(&self) -> Result<(), RuntimeError>` |

Preview authorizes `MemoryPreview`, reads authoritative state, detects no-op before registration, and never generates durable IDs/times or writes. Cancellation removes the one process-local memory review and performs no durable work.

- [ ] **Step 4: Execute confirmed Human set/delete inside one immediate transaction**

After replay/actor/policy checks, reserve the exact review in `execute_locked` and keep the returned `ReservedMemoryReview` local alive through append, mirrors, projection, audit, receipt, and commit. Then re-resolve profile namespace and current entry, verify candidate and expected state, and call `MemoryRepository::count_active_entries` in the same transaction. Reject an absent/tombstone→present transition when the count is already 1,024; overwrite and delete do not consume another slot. Only then allocate IDs/time in this exact order: for an absent-key set, new stable `MemoryEntryId`, new `MemoryEntryVersionId`, event ID, timestamp; for overwrite/recreate/delete, new `MemoryEntryVersionId`, event ID, timestamp while reusing the stable entry ID. Derive `next_sequence = projection.last_sequence + 1` with checked arithmetic. Use that same event ID/time/sequence in the immutable record and envelope; never generate a second creation event ID.

Load every other pending same-key proposal and create Human-caused expiry resolutions only when the new current state makes its expected state stale. Sort by proposal ID. Append one `MemoryEntrySet` or `MemoryEntryDeleted` event, reconcile all rows/approvals/pointers/projection/audit/receipt, commit, then consume the review.

Implement private `prepare_direct_memory_mutation(tx: &ImmediateTransaction<'_>, projection: &ProjectionState, ids: &dyn IdGenerator, clock: &dyn Clock, actor: &Actor, command_id: CommandId, command: &ApplicationCommand, reserved: &ReservedMemoryReview<'_>) -> Result<ApplicationEvent, AppError>`. It accepts only `Actor::Human` plus `SetMemoryEntry` or `DeleteMemoryEntry`. Before any allocation it requires `reserved.command_id() == command_id` and `reserved.token()` to equal the command variant's review token; `reserve_direct` has already compared the complete private binding while acquiring the ownership guard. It then calls the Task 3 entry constructor with the allocated event ID/time, selects and sorts stale siblings, and returns exactly one `MemoryEntrySet` or `MemoryEntryDeleted` payload. The existing transaction pipeline appends that event once and invokes `MemoryRepository::insert_entry_version`, `replace_current_entry`, and `resolve_proposal` for its embedded records before projection/audit/receipt storage. Only the caller invokes `reserved.consume()` after a successful commit; on error it explicitly releases or invalidates according to Step 5 before the guard drops.

- [ ] **Step 5: Implement review failure lifecycle**

Recoverable transactional failures release only the matching reservation for exact retry. Candidate change, action change, state staleness, cross-flow reuse, shutdown, service finish, terminal failure, and explicit cancel invalidate it. Ensure no successful receipt replay requires the old process-local token still to exist.

```rust
enum MemoryReviewFailureDisposition { Release, Invalidate }

fn memory_review_failure_disposition(error: &AppError) -> MemoryReviewFailureDisposition {
    match error {
        AppError::Persistence(PersistenceError::Contention)
        | AppError::Persistence(PersistenceError::Capacity)
        | AppError::Persistence(PersistenceError::QueryFailed) => MemoryReviewFailureDisposition::Release,
        _ => MemoryReviewFailureDisposition::Invalidate,
    }
}
```

- [ ] **Step 6: Rerun GREEN**

```bash
cargo test --test memory_application_mutation_contract --test memory_audit_contract --test agent_profile_review_race_contract --test skill_application_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/app src/runtime/mod.rs src/audit/mod.rs tests/support/mod.rs tests/memory_application_mutation_contract.rs tests/memory_audit_contract.rs
git commit -m "feat: add reviewed human memory mutations"
```

### Task 11: Implement Internal Agent Proposals and Exact Human Resolution

**Files:**

- Modify: `src/app/service.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/runtime/mod.rs`
- Modify: `src/persistence/memory_repository.rs`
- Modify: `src/audit/mod.rs`
- Modify: `tests/support/mod.rs`
- Create: `tests/memory_proposal_application_contract.rs`

**Interfaces:**

- Consumes: `MemoryReviewRegistry::reserve_resolution(&self, command_id: CommandId, token: MemoryReviewToken, supplied: &MemoryResolutionReviewBinding) -> Result<ReservedMemoryReview<'_>, DomainError>`; `ApprovalRecord::resolve(&self, status: ApprovalStatus, actor: Actor, resolved_at_millis: i64) -> Result<ApprovalRecord, ApprovalError>`; `MemoryRepository::{insert_proposal_with_approval, count_pending_proposals, load_proposal, load_memory_approval, load_current_entry, count_active_entries, load_pending_proposals_for_key, insert_entry_version, replace_current_entry, resolve_proposal}` with the exact Task 7 types; and Task 10's review-failure lifecycle.
- Produces: the exact `MemoryProposalResolutionReview` struct below; `ApplicationService::preview_memory_proposal_approval(&self, proposal: MemoryProposalRef) -> Result<MemoryProposalResolutionReview, AppError>`; `ApplicationService::preview_memory_proposal_rejection(&self, proposal: MemoryProposalRef) -> Result<MemoryProposalResolutionReview, AppError>`; matching `CommandExecutor` methods taking `&mut self` and returning `AppError`; matching `RuntimeClient` methods taking `&self` and returning `RuntimeError`; and private `prepare_memory_resolution(tx: &ImmediateTransaction<'_>, projection: &ProjectionState, ids: &dyn IdGenerator, clock: &dyn Clock, actor: &Actor, command_id: CommandId, command: &ApplicationCommand, reserved: &ReservedMemoryReview<'_>) -> Result<ApplicationEvent, AppError>`.

- [ ] **Step 1: Write failing proposal creation and resolution tests**

Cover Human/System producer rejection, Agent actor/profile mismatch, exact historical profile provenance, no-effect rejection, pending cap inside transaction, one proposal + one pending approval with `expires_at == None` + zero entry mutations at creation, action-specific resolution reviews, non-Human resolution rejection, exact approval/proposal/current-state binding, accept once, reject without mutation, historical proposer warning, direct/accepted sibling expiry, and no memory cancellation route or wall-clock expiry.

```rust
#[test]
fn proposal_creates_one_nonexpiring_pending_approval_and_no_entry() {
    let mut app = memory_app_fixture();
    let outcome = app.execute(proposal_command()).unwrap();
    let approval = app.approval(outcome.approval_id()).unwrap();
    assert_eq!(approval.status(), ApprovalStatus::Pending);
    assert_eq!(approval.expires_at_millis(), None);
    assert_eq!(app.memory_entry_version_count(), 0);
    assert_eq!(app.event_count_for(outcome.command_id()), 1);
}
```

- [ ] **Step 2: Run proposal lifecycle contracts to verify RED**

Run:

```bash
cargo test --test memory_proposal_application_contract
```

Expected RED: execution returns `memory_command_not_implemented`.

- [ ] **Step 3: Implement the internal producer path**

Accept `ProposeMemoryMutation` only through the typed application service. Do not add it to parser, TUI, help, or any production caller. Resolve the exact proposer profile version, require `Actor::Agent(id)` with `id == proposer.profile_id()`, derive its namespace, re-resolve expected current state, reject no-effect candidates, and check pending count under `BEGIN IMMEDIATE`.

Generate proposal ID, approval ID, event ID, and one timestamp only after all checks. Build an exact `ApprovalRecord`:

```rust
ApprovalRecord::builder(ApprovalAction::MemoryMutation)
    .approval_id(approval_id)
    .object(proposal.object_ref()?)
    .actor(Actor::Agent(proposer.profile_id()))
    .created_at_millis(now)
    .build()?
```

Append one `MemoryProposalCreated` containing proposal and pending approval; reconcile event, proposal, approval, current status, audit, and receipt atomically. Do not touch current entries.

- [ ] **Step 4: Add passive approve/reject previews**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryProposalResolutionReview {
    pub action: MemoryResolutionAction,
    pub proposal: MemoryProposal,
    pub approval_id: ApprovalId,
    pub expected_approval_status: ApprovalStatus,
    pub expected_entry: ExpectedMemoryEntryState,
    pub proposer_is_historical: bool,
    pub proposer_identity: MemoryProfileIdentityView,
    pub namespace_owner_identity: MemoryProfileIdentityView,
    pub plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
    pub review_token: MemoryReviewToken,
    pub review_digest: Digest,
}
```

Add `preview_memory_proposal_approval` and `preview_memory_proposal_rejection` at all three boundaries with the same ownership pattern as Task 10: `ApplicationService` takes `&self` and returns `AppError`; `CommandExecutor` takes `&mut self` and returns `AppError`; `RuntimeClient` takes `&self` and returns `RuntimeError`. Each method takes exactly one `MemoryProposalRef` and returns `MemoryProposalResolutionReview`.

Resolve immutable proposal, pending approval, namespace owner, pinned historical proposer, current profile, and current entry. Materialize the exact proposer's display name from its pinned immutable version and the namespace owner's display name from the stable profile's current version. When they differ, set `proposer_is_historical` and retain both identities for fallback/TUI review. Bind approve versus reject into the one-use review digest. Profile-version changes alone do not stale the proposal.

- [ ] **Step 5: Commit acceptance/rejection and all derived transitions in one event**

Acceptance revalidates every binding under the transaction before allocating anything. `execute_locked` holds the resolution `ReservedMemoryReview` across the entire transaction, exactly as Task 10 does for a direct review. Before an absent/tombstone→present acceptance, call `MemoryRepository::count_active_entries` and reject a count of 1,024; overwrite and delete do not consume another slot. An accepted set that expected absence allocates a stable `MemoryEntryId`, `MemoryEntryVersionId`, event ID, and timestamp; accepted overwrite/recreate/delete allocates only a new `MemoryEntryVersionId`, event ID, and timestamp while reusing the logical entry ID. It creates the next Human-authored entry with `accepted_proposal`, resolves the selected approval as accepted, expires all newly stale sibling proposals and their approvals in proposal-ID order, and commits one `MemoryProposalAccepted` event. Rejection allocates only event ID/time after revalidation and resolves only selected proposal/approval in one `MemoryProposalRejected` event. Set `resolution_actor_kind='human'` and `resolution_actor_id=NULL` with the terminal approval transition.

After the proposal and resolution handlers replace the last staged service arms, remove `AppError::MemoryCommandNotImplemented` and its `code()` arm. Add a compile-time exhaustive service test that reaches all fourteen memory command variants; `WrongMemoryCommandDispatcher` remains only for the private read helper's defensive branch.

Implement private `prepare_memory_resolution(tx: &ImmediateTransaction<'_>, projection: &ProjectionState, ids: &dyn IdGenerator, clock: &dyn Clock, actor: &Actor, command_id: CommandId, command: &ApplicationCommand, reserved: &ReservedMemoryReview<'_>) -> Result<ApplicationEvent, AppError>`. The approve arm returns `MemoryProposalAccepted { resolution, entry, expired_proposals }`; the reject arm returns `MemoryProposalRejected { resolution }`. Both require Human, require `reserved.command_id() == command_id` and `reserved.token()` to equal the command's token after `reserve_resolution` has validated the full private binding, and require the exact pending approval/current entry state before their first ID or clock call. The caller consumes only after commit and otherwise releases/invalidates without allowing the RAII guard to leave the state reserved.

- [ ] **Step 6: Rerun GREEN with receipt and audit coverage**

```bash
cargo test --test memory_proposal_application_contract --test memory_receipt_contract --test memory_audit_contract --test policy_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/app src/runtime/mod.rs src/persistence/memory_repository.rs src/audit/mod.rs tests/support/mod.rs tests/memory_proposal_application_contract.rs
git commit -m "feat: add memory proposal approval lifecycle"
```

### Task 12: Reconcile Hybrid Memory and Approvals From Verified Events

**Files:**

- Modify: `src/persistence/memory_repository.rs`
- Modify: `src/persistence/projection_repository.rs`
- Modify: `src/persistence/database.rs`
- Modify: `src/recovery/reducer.rs`
- Modify: `src/recovery/coordinator.rs`
- Modify: `tests/support/mod.rs`
- Create: `tests/memory_recovery_contract.rs`
- Modify: `tests/memory_integrity_contract.rs`

**Interfaces:**

- Consumes: `reduce(state: &mut ProjectionState, event: &EventEnvelope) -> Result<ReducerEffect, RecoveryError>`; verified `&[EventEnvelope]`; `MemoryProjection`; `MemoryRepository::{validate_entry_context, validate_proposal_context, validate_summary_context}` with the exact Task 7 types; and immutable proposal approval evidence carried by Task 8 events.
- Produces: `ExpectedMemoryRecords` and `ExpectedMemoryApproval` with the exact fields below; `expected_memory_records(events: &[EventEnvelope]) -> Result<ExpectedMemoryRecords, RecoveryError>`; `reconcile_verified_memory(tx: &rusqlite::Transaction<'_>, events: &[EventEnvelope], expected_projection: &MemoryProjection) -> Result<(), RecoveryError>`; and test-only `record_test_episodic_summary(fixture: &mut PersistentFixture, profile: AgentProfileVersionRef, label: String, body: String, purpose_tags: Vec<String>, source_event_ids: Vec<EventId>) -> Result<EpisodicSummaryRef, AppError>`.

- [ ] **Step 1: Write failing backfill, tamper, and rebuild contracts**

Test missing proven entry/proposal/resolution/summary/source rows; missing pending and terminal memory approvals; altered or extra immutable rows; altered requester/resolver actor kind or ID; altered profile/namespace/predecessor/approval/source identity; extra memory approval; unrelated approval preservation; missing/corrupt current pointers/status; atomic failure before reconciliation commit; repeated recovery byte/digest identity; recovery creating no snapshot event/receipt; replay after restart returning the receipt's exact historical materialized snapshot after current memory changes without policy, profile/current-state reads, reselection, IDs, clocks, or writes; and legacy streams with the exact empty compatible projection.

```rust
#[test]
fn recovery_without_snapshot_rebuilds_memory_and_creates_no_command_artifacts() {
    let fixture = recovered_memory_fixture_without_projection_snapshot();
    let event_count = fixture.event_count();
    let receipt_count = fixture.receipt_count();
    fixture.run_recovery().unwrap();
    assert_eq!(fixture.current_entry_ref(), Some(entry_ref_fixture()));
    assert_eq!(fixture.event_count(), event_count);
    assert_eq!(fixture.receipt_count(), receipt_count);
}

#[test]
fn snapshot_receipt_replay_after_restart_returns_historical_content_without_new_work() {
    let mut app = persistent_memory_app_fixture();
    let command = build_snapshot_command();
    let historical = app.execute(command.clone()).unwrap();
    app.accept_another_edit_for_same_key();
    app = app.restart_and_recover().unwrap();
    let before = app.all_side_effect_counters();
    let replay = app.execute(command).unwrap();
    assert_eq!(replay, historical);
    assert_eq!(app.all_side_effect_counters(), before);
}
```

- [ ] **Step 2: Run recovery contracts to verify RED**

Run:

```bash
cargo test --test memory_recovery_contract --test memory_integrity_contract
```

Expected RED: the rebuilt current pointer or approval assertion fails.

- [ ] **Step 3: Derive complete expected memory evidence from the verified stream**

```rust
pub(crate) struct ExpectedMemoryRecords {
    pub entries: BTreeMap<MemoryEntryVersionId, (u64, MemoryEntryVersion)>,
    pub proposals: BTreeMap<MemoryProposalId, (u64, MemoryProposal)>,
    pub resolutions: BTreeMap<MemoryProposalId, (u64, MemoryProposalResolution)>,
    pub summaries: BTreeMap<EpisodicSummaryId, (u64, EpisodicSummary)>,
    pub approvals: BTreeMap<ApprovalId, ExpectedMemoryApproval>,
}

pub(crate) struct ExpectedMemoryApproval {
    pub record: ApprovalRecord,
    pub resolution_event_id: Option<EventId>,
    pub resolution_actor: Option<Actor>,
}
```

Implement `pub(crate) expected_memory_records(events: &[EventEnvelope]) -> Result<ExpectedMemoryRecords, RecoveryError>`. Derive proposal approvals from creation events and their exact terminal state from accepted/rejected/embedded expiry payloads. Validate event envelope actor/time/id and profile/source refs while deriving. Do not trust current projection or mirrors as evidence.

- [ ] **Step 4: Reconcile immutable rows and memory-only approvals**

Implement `pub(crate) reconcile_verified_memory(tx: &rusqlite::Transaction<'_>, events: &[EventEnvelope], expected_projection: &MemoryProjection) -> Result<(), RecoveryError>`. Insert a wholly missing row only when authenticated events prove its exact bytes. Require every existing expected row to match all columns/canonical bytes. Reject extra immutable memory rows and extra `memory_mutation` approvals. Reconstruct wholly missing pending or terminal expected memory approvals, including resolver actor columns; never update a mismatched existing approval. Ignore and preserve all other approval actions byte-for-byte.

- [ ] **Step 5: Replace only rebuildable memory state**

After immutable reconciliation succeeds, transactionally replace `current_memory_entries` and `current_memory_proposal_status` from `ProjectionState.memory`. Include them in generic projection clearing/rebuild. Never update/delete suspicious immutable evidence.

```sql
DELETE FROM current_memory_entries;
DELETE FROM current_memory_proposal_status;
```

Then insert every entry pointer ordered by `(memory_namespace_id, normalized_key)` and every proposal status ordered by `proposal_id`, using the exact immutable composite keys already validated by `reconcile_verified_memory`.

- [ ] **Step 6: Add a validated test-only episodic fixture**

In test support, first resolve the exact profile reference to its authoritative immutable profile version and derive its namespace. Then canonicalize and validate the label, body, and purpose tags, including `CredentialPatternSetV1`, without reading the clock or generating any durable ID. Begin an immediate transaction, verify all chosen sources already exist and bind sequence/ID/type/digest, and calculate `tail + 1` with checked arithmetic. Only after every profile, namespace, plaintext, tag, and source check succeeds may the fixture allocate the summary ID, exact event ID, and timestamp. Build `EpisodicSummaryRecorded`, assert `EventRepository` assigns that sequence, apply mirrors/projection, and commit. Do not expose a production application command, capability, service method, fallback route, or TUI action.

Implement test-support-only `pub fn record_test_episodic_summary(fixture: &mut PersistentFixture, profile: AgentProfileVersionRef, label: String, body: String, purpose_tags: Vec<String>, source_event_ids: Vec<EventId>) -> Result<EpisodicSummaryRef, AppError>`. It returns the exact persisted summary reference from the single committed `EpisodicSummaryRecorded` event and is compiled only for tests.

- [ ] **Step 7: Rerun GREEN and all prior recovery suites**

```bash
cargo test --test memory_recovery_contract --test memory_integrity_contract --test agent_profile_recovery_contract --test skill_recovery_contract --test recovery_contract
```

- [ ] **Step 8: Commit**

```bash
git add src/persistence src/recovery tests/support/mod.rs tests/memory_recovery_contract.rs tests/memory_integrity_contract.rs
git commit -m "feat: recover hybrid memory from verified events"
```

### Task 13: Prove Atomicity, Concurrency, Isolation, and Exact Retry Semantics

**Files:**

- Modify: `src/app/service.rs`
- Modify: `src/persistence/database.rs`
- Modify: `src/persistence/memory_repository.rs`
- Modify: `tests/support/mod.rs`
- Create: `tests/memory_atomicity_contract.rs`
- Create: `tests/memory_concurrency_contract.rs`
- Create: `tests/memory_isolation_contract.rs`

**Interfaces:**

- Consumes: Task 10's exact `prepare_direct_memory_mutation` contract and Task 11's exact `prepare_memory_resolution` contract; the existing `CommandTransactionHook: Send + Sync`; `PersistentFixture::raw_database_snapshot(&self) -> RawDatabaseSnapshot`; and two independently opened `ApplicationService` instances over one state directory.
- Produces: the exact `MemoryFaultBoundary` enum below; the seven default memory hook methods below on `CommandTransactionHook`; `PersistenceError::Capacity`; and deterministic atomicity, retry, edit/edit, approve/approve, approve/reject, profile-v1→active-v2 same-namespace, cross-namespace isolation, and copy-without-memory contract tests.

- [ ] **Step 1: Write the failing transaction-boundary matrix**

Snapshot raw rows for the event, projection, memory immutable/current, proposal, resolution, episodic, approval, audit, receipt, and command-event-ref tables. Inject one failure after each relevant boundary: review reservation, event append, proposal insert, approval insert/transition, entry insert, resolution insert, current entry/status replacement, whole projection store, audit append, receipt store, and pre-commit. Also inject SQLite full/capacity failure during immutable insert. Assert every table returns to the identical pre-command state and the recoverable exact review can retry.

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryFaultBoundary {
    ReviewReservation,
    EventAppend,
    ProposalInsert,
    ApprovalWrite,
    EntryInsert,
    ResolutionInsert,
    CurrentStateUpdate,
    ProjectionStore,
    AuditAppend,
    ReceiptStore,
    PreCommit,
    SqliteCapacity,
}

impl MemoryFaultBoundary {
    pub const ALL: &'static [Self] = &[
        Self::ReviewReservation,
        Self::EventAppend,
        Self::ProposalInsert,
        Self::ApprovalWrite,
        Self::EntryInsert,
        Self::ResolutionInsert,
        Self::CurrentStateUpdate,
        Self::ProjectionStore,
        Self::AuditAppend,
        Self::ReceiptStore,
        Self::PreCommit,
        Self::SqliteCapacity,
    ];
}

#[test]
fn every_memory_write_boundary_rolls_back_to_identical_rows() {
    for &boundary in MemoryFaultBoundary::ALL {
        let mut fixture = reviewed_memory_mutation_fixture();
        let before = fixture.raw_database_snapshot();
        fixture.fail_once_at(boundary);
        assert!(fixture.confirm_review().is_err());
        assert_eq!(fixture.raw_database_snapshot(), before);
        assert!(fixture.confirm_review().is_ok());
    }
}
```

- [ ] **Step 2: Run atomicity contracts to verify RED**

Run:

```bash
cargo test --test memory_atomicity_contract
```

Expected RED: compilation fails because memory-specific fault boundaries are absent.

- [ ] **Step 3: Add default-no-op transaction hooks without changing production order**

Insert these exact default methods into the existing public `CommandTransactionHook` trait; do not create a second trait:

```rust
fn before_memory_review_operation(&self, _command_id: CommandId) {}

fn after_memory_review_reservation(
    &self,
    _transaction: &rusqlite::Transaction<'_>,
    _command_id: CommandId,
) -> Result<(), PersistenceError> {
    Ok(())
}

fn after_memory_proposal_insert(
    &self,
    _transaction: &rusqlite::Transaction<'_>,
) -> Result<(), PersistenceError> {
    Ok(())
}

fn after_memory_approval_write(
    &self,
    _transaction: &rusqlite::Transaction<'_>,
) -> Result<(), PersistenceError> {
    Ok(())
}

fn after_memory_entry_insert(
    &self,
    _transaction: &rusqlite::Transaction<'_>,
) -> Result<(), PersistenceError> {
    Ok(())
}

fn after_memory_resolution_insert(
    &self,
    _transaction: &rusqlite::Transaction<'_>,
) -> Result<(), PersistenceError> {
    Ok(())
}

fn after_memory_current_update(
    &self,
    _transaction: &rusqlite::Transaction<'_>,
) -> Result<(), PersistenceError> {
    Ok(())
}
```

Map injected `SQLITE_FULL`/`ErrorCode::DiskFull` and `ErrorCode::TooBig` failures to `PersistenceError::Capacity` without including SQLite text. Treat `Contention`, `Capacity`, and a content-free transactional `QueryFailed` as releasable review failures; stale/domain/integrity/receipt conflicts remain invalidating. Retain generic existing hooks for event, projection, audit, receipt, and pre-commit. Hooks observe real production transactions; tests must not duplicate the write algorithm.

- [ ] **Step 4: Write deterministic race contracts**

Use two independent service instances and barriers around the same pre-transaction/state-validation seams as agent/skill concurrency tests. Prove edit/edit, approve/approve, and approve/reject each yield one winner; the loser has no partial event/receipt/row; matching replay returns the winner; changed content conflicts; and a recoverable failure releases the exact review for retry.

```rust
#[test]
fn concurrent_approve_and_reject_have_one_terminal_winner() {
    let race = proposal_resolution_race(MemoryResolutionAction::Approve, MemoryResolutionAction::Reject);
    let results = race.run();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(race.fixture().resolution_count(), 1);
    assert_eq!(race.fixture().terminal_approval_count(), 1);
}
```

- [ ] **Step 5: Run the race contracts GREEN**

```bash
cargo test --test memory_concurrency_contract --test memory_atomicity_contract
```

- [ ] **Step 6: Write namespace-isolation and copy-without-memory contracts**

Create two profiles sharing the same inert test binding, store overlapping keys, and prove list/detail/history/proposals/episodes/retrieval never cross namespaces. Also write memory under profile v1, activate profile v2 for the same stable profile, prove the exact v2 reference resolves to the unchanged v1 namespace, and prove list/retrieval through v2 returns the v1 memory without copying any row or crossing into another namespace. For duplication, copy only the non-memory `AgentProfileDraft` fields and call the existing profile creation path; assert a fresh namespace and zero memory rows. Do not add a duplicate command or UI.

```rust
#[test]
fn copied_profile_starts_with_a_fresh_empty_namespace() {
    let mut app = memory_app_fixture();
    let source = app.create_profile_with_memory("source", "thesis", "value");
    let draft = copy_profile_draft_with_name(source.profile().to_draft(), "copy");
    let copy = app.create_profile(draft).unwrap();
    assert_ne!(copy.memory_namespace_id(), source.memory_namespace_id());
    assert!(app.list_memory(copy.profile_id()).unwrap().entries.is_empty());
}

#[test]
fn active_profile_v2_retrieves_v1_memory_from_the_same_namespace() {
    let mut app = memory_app_fixture();
    let v1 = app.create_profile_with_memory("source", "thesis", "value");
    let memory_rows = app.count_memory_rows();
    let v2 = app.activate_next_profile_version(v1.profile_id()).unwrap();
    assert_eq!(v2.memory_namespace_id(), v1.memory_namespace_id());
    assert_eq!(app.count_memory_rows(), memory_rows);
    assert_eq!(app.list_memory(v2.exact_reference()).unwrap().entries[0].value(), "value");
    assert_eq!(
        app.build_snapshot_for(v2.exact_reference()).unwrap().entries()[0].value(),
        "value",
    );
}
```

- [ ] **Step 7: Run the namespace-isolation contract GREEN**

```bash
cargo test --test memory_isolation_contract
```

- [ ] **Step 8: Rerun all three GREEN**

```bash
cargo test --test memory_atomicity_contract --test memory_concurrency_contract --test memory_isolation_contract --test agent_profile_concurrency_contract --test skill_concurrency_contract
```

- [ ] **Step 9: Commit**

```bash
git add src/app/service.rs src/persistence/database.rs src/persistence/memory_repository.rs tests/support/mod.rs tests/memory_atomicity_contract.rs tests/memory_concurrency_contract.rs tests/memory_isolation_contract.rs
git commit -m "test: prove memory transaction race safety"
```

### Task 14: Build the Local Memory Editor State Machine

**Files:**

- Create: `src/ui/memory_editor.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/domain/error.rs`
- Create: `tests/memory_editor_contract.rs`

**Interfaces:**

- Consumes: `AgentProfileSelector`, `MemoryEntryDraft`, `MemoryEntryVersion`, `MemoryEditPreview`, `MemoryEditReview`, `MemoryNoChange`, `ApplicationCommand::SetMemoryEntry`, and `DomainError` exactly as defined in Tasks 2, 8, and 10.
- Produces: re-exported `MEMORY_PLAINTEXT_WARNING: &str`; content-free `DomainError::{InvalidMemoryEditorSeed, InvalidMemoryEditorTransition}` variants with codes `invalid_memory_editor_seed` and `invalid_memory_editor_transition`; the exact `MemoryEditorStep`, `MemoryPreviewRequest`, `MemoryEditorEffect`, and `MemoryEditor` field types below; and every constructor, transition, draft, generation, preview, error, and read-only getter signature in the method contract table below.

- [ ] **Step 1: Write failing pure editor-state tests**

Cover create/edit seed behavior; key→value→tags→review order; multiline value handling; exact tag parsing; plaintext warning; changed candidate invalidation; stale preview generation; no-change result; exact set confirmation command; back navigation; safe errors; and no memory prose entering command history merely through editing. Delete review remains a separate Task 10/16/17 flow and does not pass through this set editor.

```rust
#[test]
fn stale_preview_generation_cannot_replace_a_newer_draft() {
    let mut editor = MemoryEditor::for_create(profile_selector());
    editor.submit_line("thesis".to_owned()).unwrap();
    editor.submit_line("older value".to_owned()).unwrap();
    editor.submit_line("analysis".to_owned()).unwrap();
    let first = editor.preview_request().unwrap();
    editor.replace_value("newer value".to_owned()).unwrap();
    assert!(!editor.apply_preview(first.generation, review_fixture()));
    assert_eq!(editor.draft().unwrap().value(), "newer value");
}
```

- [ ] **Step 2: Run editor contract to verify RED**

Run:

```bash
cargo test --test memory_editor_contract
```

Expected RED: compilation fails because `MemoryEditor` is absent.

- [ ] **Step 3: Implement the editor with generation guards**

```rust
pub use crate::memory::MEMORY_PLAINTEXT_WARNING;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryEditorStep { Key, Value, PurposeTags, Review }

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPreviewRequest {
    pub generation: u64,
    pub selector: AgentProfileSelector,
    pub candidate: MemoryEntryDraft,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryEditorEffect {
    None,
    Preview(MemoryPreviewRequest),
    Confirm(ApplicationCommand),
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryEditor {
    selector: AgentProfileSelector,
    seed: Option<MemoryEntryVersion>,
    key: String,
    value: String,
    tags: String,
    step: MemoryEditorStep,
    generation: u64,
    preview: Option<MemoryEditPreview>,
    safe_error_code: Option<&'static str>,
}
```

Implement the inherent `MemoryEditor` methods exactly as follows; each row is an implementation contract, not a declaration-only Rust stub:

| Method | Exact contract |
|---|---|
| `for_create(selector: AgentProfileSelector) -> Self` | Starts at `Key` with empty inputs, generation 0, and no seed/preview/error. |
| `for_set(selector: AgentProfileSelector, entry: MemoryEntryVersion) -> Result<Self, DomainError>` | Seeds canonical key/value/tags from a present entry and starts at `Value`; a tombstone returns `DomainError::InvalidMemoryEditorSeed` before state is installed. |
| `submit_keyboard_line(&mut self, line: &str) -> Result<MemoryEditorEffect, DomainError>` | Rejects embedded newline/control input, delegates the owned text to `submit_line`, and advances exactly one step. |
| `submit_line(&mut self, line: String) -> Result<MemoryEditorEffect, DomainError>` | Canonicalizes the field for the current step; `Key` advances to `Value`, `Value` to `PurposeTags`, and `PurposeTags` installs the draft then returns `Preview(preview_request()?)`; `Review` returns `DomainError::InvalidMemoryEditorTransition`. |
| `back(&mut self) -> MemoryEditorEffect` | Moves one step backward; from `Key` returns `Cancelled`; leaving `Review` clears the review and increments generation. |
| `preview_request(&self) -> Result<MemoryPreviewRequest, DomainError>` | Returns the current generation, cloned selector, and `draft()?`; incomplete state returns `DomainError::InvalidMemoryEditorTransition`. |
| `replace_value(&mut self, value: String) -> Result<(), DomainError>` | Canonicalizes/validates the replacement, increments generation, clears preview/error, and leaves the editor at `PurposeTags`. |
| `apply_preview(&mut self, generation: u64, preview: MemoryEditPreview) -> bool` | Returns `false` without mutation when generation differs; otherwise installs the preview, enters `Review`, clears the safe error, and returns `true`. |
| `clear_review(&mut self)` | Clears preview and safe error, increments generation, and moves `Review` back to `PurposeTags`; otherwise preserves the current input step. |
| `report_error(&mut self, code: &'static str)` | Stores only the stable code, clears preview, increments generation, and never stores raw rejected input. |
| `draft(&self) -> Result<MemoryEntryDraft, DomainError>` | Builds through `MemoryEntryDraft::new(key.clone(), value.clone(), parsed_tags)` so all canonicalization and credential checks are reused. |
| `step(&self) -> MemoryEditorStep` | Returns `step`. |
| `selector(&self) -> &AgentProfileSelector` | Returns `&selector`. |
| `key_input(&self) -> &str` | Returns `&key`. |
| `value_input(&self) -> &str` | Returns `&value`. |
| `tags_input(&self) -> &str` | Returns `&tags`. |
| `generation(&self) -> u64` | Returns `generation`. |
| `preview(&self) -> Option<&MemoryEditPreview>` | Returns `preview.as_ref()`. |
| `safe_error_code(&self) -> Option<&'static str>` | Returns `safe_error_code`. |

The input, generation, preview, and safe-error getters are read-only and are the only way sibling fallback/TUI modules inspect private editor state. Every draft edit increments generation and clears the installed review. A stale generation returns `false` without replacing state.
Tasks 16 and 17 must handle the fallible `for_set` result before installing any
host workflow state and render only `DomainError::code()`, never the entry
payload, when seed validation fails.

- [ ] **Step 4: Rerun GREEN**

```bash
cargo test --test memory_editor_contract
```

- [ ] **Step 5: Commit**

```bash
git add src/ui/memory_editor.rs src/ui/mod.rs tests/memory_editor_contract.rs
git commit -m "feat: add memory editor state machine"
```

### Task 15: Add Exact Fallback Grammar and Deliberate Read Rendering

**Files:**

- Modify: `src/ui/command/parser.rs`
- Modify: `src/ui/command/mod.rs`
- Modify: `src/ui/command/renderer.rs`
- Modify: `src/ui/command/runner.rs`
- Modify: `src/ui/tui/controller.rs`
- Modify: `tests/command_contract.rs`
- Modify: `tests/skill_fallback_contract.rs`
- Create: `tests/memory_fallback_contract.rs`

**Interfaces:**

- Consumes: `parse_fallback_line(input: &[u8]) -> FallbackParsedLine`, `parse_line(input: &[u8]) -> ParsedLine`, `TextRenderer::render_view<W: Write>(view: &CommandView, writer: &mut W) -> io::Result<()>`, Task 8’s eight read `ApplicationCommand` variants, and `AgentProfileSelector`, `MemoryProposalFilter`, `MemoryProposalId`, and `EpisodicSummaryId`.
- Produces: `pub enum MemoryWorkflowCommand { Set { agent: AgentProfileSelector, key: String }, Delete { agent: AgentProfileSelector, key: String }, Approve { proposal_id: MemoryProposalId }, Reject { proposal_id: MemoryProposalId } }`; `FallbackParsedLine::MemoryWorkflow(MemoryWorkflowCommand)`; `ParsedLine::MemoryWorkflow(MemoryWorkflowCommand)`; `pub use parser::MemoryWorkflowCommand`; and private `fn render_memory_view<W: Write>(view: &CommandView, writer: &mut W) -> Option<io::Result<()>>` dispatch. No proposal-create, summary-mutation, or snapshot route is produced.

- [ ] **Step 1: Write failing parser allowlist and renderer tests**

Accept only the eleven documented route forms, existing quoted agent/key syntax, UUID selectors, positive optional history version, pending-default/all proposal filter, and exact proposal/summary UUIDs. The single `/memory history <agent> <key> [version]` form maps to `ShowMemoryEntryHistory` without the optional version and `ShowMemoryEntryVersion` with it. Reject missing/extra/ambiguous/zero-version args, `/memory propose`, summary mutation, snapshot routes, and bare `memory` without retaining raw input.

Test all eight deliberate read/detail views, bounded lists, hostile terminal text escaping, visible `Summary — verify sources`, and absence of memory prose from generic errors/status/audit/help.

```rust
#[test]
fn parser_exposes_only_the_documented_memory_routes() {
    assert!(matches!(
        parse_line(b"/memory list analyst"),
        ParsedLine::Command(ApplicationCommand::ListMemoryEntries { .. })
    ));
    assert!(matches!(
        parse_line(b"/memory set analyst thesis"),
        ParsedLine::MemoryWorkflow(MemoryWorkflowCommand::Set { .. })
    ));
    assert!(matches!(
        parse_line(b"/memory propose analyst thesis"),
        ParsedLine::Command(ApplicationCommand::RejectInput(_))
    ));
    assert!(matches!(
        parse_line(b"/memory snapshot analyst"),
        ParsedLine::Command(ApplicationCommand::RejectInput(_))
    ));
}
```

- [ ] **Step 2: Run fallback parser/render contracts to verify RED**

Run:

```bash
cargo test --test memory_fallback_contract
```

Expected RED: `/memory list` is rejected and `ParsedLine::MemoryWorkflow` is absent.

- [ ] **Step 3: Add typed workflow parsing**

```rust
pub enum MemoryWorkflowCommand {
    Set { agent: AgentProfileSelector, key: String },
    Delete { agent: AgentProfileSelector, key: String },
    Approve { proposal_id: MemoryProposalId },
    Reject { proposal_id: MemoryProposalId },
}

pub enum FallbackParsedLine {
    Command(ApplicationCommand),
    AgentWorkflow(AgentWorkflowCommand),
    SkillWorkflow(SkillWorkflowCommand),
    MemoryWorkflow(MemoryWorkflowCommand),
    Ignored,
}

pub enum ParsedLine {
    Command(ApplicationCommand),
    AgentWorkflow(AgentWorkflowCommand),
    SkillWorkflow(SkillWorkflowCommand),
    MemoryWorkflow(MemoryWorkflowCommand),
    Ignored,
}
```

Map `FallbackParsedLine::MemoryWorkflow(workflow)` to `ParsedLine::MemoryWorkflow(workflow)` in `parse_line`, and publicly re-export `MemoryWorkflowCommand` beside the other workflow types in `ui::command`. Map list/get/history/proposals/proposal/episodes/episode directly to typed commands. Map set/delete/approve/reject only to workflow intents so parsing alone can never commit.

Update every pre-existing exhaustive consumer in the same commit. In `tui::controller::submit_command`, handle `ParsedLine::MemoryWorkflow(_)` by setting the content-free guidance `Open Agents → Memory to edit or resolve memory.` and returning `ControllerEffect::Redraw`; Task 17 replaces that guidance with native nested controls. In `tests/command_contract.rs` and `tests/skill_fallback_contract.rs`, add explicit `MemoryWorkflow(_)` match arms to their classification helpers and assert they are never mistaken for `Command`, `AgentWorkflow`, or `SkillWorkflow`.

- [ ] **Step 4: Render deliberate memory outcomes safely**

Render deliberate lists with bounded, terminal-safe keys/tags/labels while omitting full values, proposal rationale/candidate values, and episodic bodies. Render those exact fields and source material only for deliberate detail or review views after terminal-control escaping and byte bounds. Render mutation/resolution results by IDs, version, status, and digest. Keep help limited to supported fallback routes and clearly mark internal producers unavailable.

```rust
fn render_memory_view<W: Write>(
    view: &CommandView,
    writer: &mut W,
) -> Option<io::Result<()>> {
    match view {
        CommandView::MemoryEntries(value) => Some(render_memory_entries(value, writer)),
        CommandView::MemoryEntry(value) => Some(render_memory_entry(value, writer)),
        CommandView::MemoryEntryHistory(value) => Some(render_memory_history(value, writer)),
        CommandView::MemoryEntryVersion(value) => Some(render_memory_version(value, writer)),
        CommandView::MemoryProposals(value) => Some(render_memory_proposals(value, writer)),
        CommandView::MemoryProposal(value) => Some(render_memory_proposal(value, writer)),
        CommandView::EpisodicSummaries(value) => Some(render_episodic_summaries(value, writer)),
        CommandView::EpisodicSummary(value) => Some(render_episodic_summary(value, writer)),
        CommandView::MemoryEntryMutation(value) => Some(render_memory_mutation(value, writer)),
        CommandView::MemoryProposalCreated(value) => Some(render_proposal_created(value, writer)),
        CommandView::MemoryProposalResolution(value) => Some(render_proposal_resolution(value, writer)),
        CommandView::MemorySnapshot(_) => None,
        _ => None,
    }
}
```

Each referenced renderer has the exact generic form `fn render_<view><W: Write>(view: &<ViewPayload>, writer: &mut W) -> io::Result<()>`; implement all eleven in this step and call `render_memory_view` before the existing non-memory `CommandView` match.

- [ ] **Step 5: Rerun GREEN**

```bash
cargo test --test memory_fallback_contract --test command_contract --test final_fix_command_surface_contract
```

- [ ] **Step 6: Commit**

```bash
git add src/ui/command src/ui/tui/controller.rs tests/memory_fallback_contract.rs tests/command_contract.rs tests/skill_fallback_contract.rs
git commit -m "feat: add memory fallback grammar and views"
```

### Task 16: Add Reviewed Fallback Set/Delete/Approve/Reject Workflows

**Files:**

- Modify: `src/ui/command/runner.rs`
- Modify: `src/ui/command/renderer.rs`
- Modify: `tests/memory_fallback_contract.rs`
- Modify: `tests/fallback_contract.rs`

**Interfaces:**

- Consumes: `MemoryWorkflowCommand`; `MemoryEditor`; `RuntimeClient::preview_memory_set(&self, selector: AgentProfileSelector, candidate: MemoryEntryDraft) -> Result<MemoryEditPreview, RuntimeError>`; `RuntimeClient::preview_memory_delete(&self, selector: AgentProfileSelector, display_key: String) -> Result<MemoryEditPreview, RuntimeError>`; `RuntimeClient::preview_memory_proposal_approval(&self, proposal: MemoryProposalRef) -> Result<MemoryProposalResolutionReview, RuntimeError>`; `RuntimeClient::preview_memory_proposal_rejection(&self, proposal: MemoryProposalRef) -> Result<MemoryProposalResolutionReview, RuntimeError>`; `RuntimeClient::cancel_memory_review(&self) -> Result<(), RuntimeError>`; `RuntimeClient::try_submit(&self, command: ApplicationCommand) -> Result<PendingOutcome, RuntimeError>`; and `MEMORY_PLAINTEXT_WARNING`.
- Produces: `FallbackRunner.memory_workflow: Mutex<Option<MemoryWorkflow>>`; private `enum MemoryWorkflow`; `fn expected_memory_confirmation(action: &str, digest: &Digest) -> String`; `fn confirmation_matches(input: &str, expected: &str) -> bool`; and `fn cancel_registered_memory_review(client: &RuntimeClient, registered: &mut bool) -> Result<(), RuntimeError>`.

- [ ] **Step 1: Write failing workflow, retry, and cleanup tests**

Cover guided set editing from absent/current values, passive no-change, delete detail review, proposal approve/reject review, action-specific exact confirmation, generic `yes`/unrelated command rejection, backpressure retry, stale/terminal reset, and exactly-once review cancellation on explicit cancel, `/quit`, EOF, interrupt, input error, output error, panic-boundary cleanup, and normal teardown.

```rust
#[test]
fn approval_confirmation_cannot_authorize_rejection() {
    let mut runner = fallback_runner_with_pending_proposal();
    runner.handle_line("/memory approve 00000000-0000-0000-0000-000000000123").unwrap();
    let digest = runner.visible_review_digest().unwrap();
    assert!(runner.handle_line(&format!("reject {digest}")).is_err());
    assert_eq!(runner.submitted_command_count(), 0);
    runner.handle_line(&format!("approve {digest}")).unwrap();
    assert_eq!(runner.submitted_command_count(), 1);
}
```

- [ ] **Step 2: Run reviewed fallback workflows to verify RED**

Run:

```bash
cargo test --test memory_fallback_contract
```

Expected RED: the workflow intent returns to the prompt without a review state.

- [ ] **Step 3: Add explicit memory workflow state**

```rust
enum MemoryWorkflow {
    Editing(MemoryEditor),
    SetReview { editor: MemoryEditor, review: MemoryEditReview },
    SetConfirmation {
        editor: MemoryEditor,
        command: ApplicationCommand,
        expected_confirmation: String,
    },
    DeleteReview { review: MemoryEditReview },
    DeleteConfirmation {
        review: MemoryEditReview,
        command: ApplicationCommand,
        expected_confirmation: String,
    },
    ProposalResolutionReview {
        review: MemoryProposalResolutionReview,
    },
    ProposalResolutionConfirmation {
        review: MemoryProposalResolutionReview,
        command: ApplicationCommand,
        expected_confirmation: String,
    },
}
```

Store at most one active memory workflow. Keep approve/reject as different variants or include the action in every state; never store an untyped generic confirmation that can swap actions.

- [ ] **Step 4: Require action-specific confirmation and preserve safe retry**

Derive exact visible phrases from action plus review digest, for example `set <review-digest>`, `delete <review-digest>`, `approve <review-digest>`, and `reject <review-digest>`. Only an exact bounded match submits. Backpressure or a recoverable transaction error retains the exact visible confirmation; stale state, consumed review, command conflict, or terminal application failure cancels it and requires a new preview.

Show `MEMORY_PLAINTEXT_WARNING` in the editor, mutation review, final mutation confirmation, and proposal resolution review/confirmation.

```rust
fn expected_memory_confirmation(action: &str, digest: &Digest) -> String {
    format!("{action} {digest}")
}

fn confirmation_matches(input: &str, expected: &str) -> bool {
    input.as_bytes().len() <= 80 && input == expected
}
```

- [ ] **Step 5: Centralize exactly-once cancellation**

Call `cancel_memory_review()` once whenever a registered memory review leaves the host without a successful commit. Clear local registration before invoking cleanup so nested error/teardown paths cannot double-cancel. Proposal/episode detail reads never register a review.

```rust
fn cancel_registered_memory_review(
    client: &RuntimeClient,
    registered: &mut bool,
) -> Result<(), RuntimeError> {
    if std::mem::take(registered) {
        client.cancel_memory_review()?;
    }
    Ok(())
}
```

- [ ] **Step 6: Rerun GREEN and fallback regressions**

```bash
cargo test --test memory_fallback_contract --test fallback_contract --test skill_fallback_contract --test agent_profile_fallback_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/ui/command/runner.rs src/ui/command/renderer.rs tests/memory_fallback_contract.rs tests/fallback_contract.rs
git commit -m "feat: add reviewed fallback memory workflows"
```

### Task 17: Add an Agents-Owned Memory TUI State Machine

**Files:**

- Modify: `src/ui/tui/model.rs`
- Modify: `src/ui/tui/controller.rs`
- Modify: `src/ui/tui/mod.rs`
- Modify: `tests/tui_navigation_contract.rs`
- Create: `tests/memory_tui_controller_contract.rs`

**Interfaces:**

- Consumes: `MemoryEditor::for_create(selector: AgentProfileSelector) -> Self`; Task 8 `CommandView` memory payloads; `MemoryEditReview`; `MemoryProposalResolutionReview`; `MemoryPreviewRequest`; `MemoryResolutionAction`; memory selectors/IDs; and existing `handle_event(model: &mut TuiModel, event: TuiEvent) -> ControllerEffect` navigation.
- Produces: `AgentsPane::Memory`; `AgentDetailAction`; `MemoryPane`; `MemoryConfirmation`; `MemoryOutcomeIntent`; `MemoryViewState`; `MemoryViewState::open_create_editor(&mut self, selector: AgentProfileSelector) -> Result<(), DomainError>`; `MemoryViewState::begin_pending(&mut self, intent: MemoryOutcomeIntent) -> Result<u64, DomainError>`; `MemoryViewState::clear_pending(&mut self)`; `MemoryViewState::selected_entry_id(&self) -> Result<MemoryEntryId, DomainError>`; `MemoryViewState::apply_matching_view(&mut self, intent: &MemoryOutcomeIntent, view: CommandView) -> bool`; `AgentsViewState::{selected_detail_action, memory}`; thirteen typed memory `ControllerEffect` variants; and `DomainError::{MemoryGenerationOverflow, MemorySelectionUnavailable}`.

- [ ] **Step 1: Write failing nested-navigation and generation-guard tests**

Prove Memory opens from active agent detail under Agents; `NavigationTab` and six-element tab state remain unchanged; no seventh global shortcut exists; Left/Right+Enter selects Assigned Skills or Memory; Esc unwinds exactly one memory layer; bare `q` is inert; modified shortcuts never navigate; editor owns `1234as`; global shortcuts from non-text review/confirmation preserve the pending memory state; selection refreshes by stable ID; and delayed outcomes cannot replace newer selection/draft/review.

```rust
#[test]
fn memory_is_nested_under_agents_without_changing_global_shortcuts() {
    let mut model = agents_detail_model();
    assert_eq!(
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        ),
        ControllerEffect::Redraw,
    );
    assert!(matches!(
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        ),
        ControllerEffect::LoadAgentMemory(_),
    ));
    assert_eq!(model.agents.pane, AgentsPane::Memory);
    assert_eq!(model.active_view, View::Agents);
    assert!(!model.skills.active);

    let before_q = model.clone();
    assert_eq!(
        handle_event(
            &mut model,
            TuiEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        ),
        ControllerEffect::None,
    );
    assert_eq!(model, before_q);
}
```

- [ ] **Step 2: Run nested-navigation contracts to verify RED**

Run:

```bash
cargo test --test memory_tui_controller_contract --test tui_navigation_contract
```

Expected RED: compilation fails because `AgentsPane::Memory` is absent.

- [ ] **Step 3: Add nested memory state without changing global navigation**

```rust
// Append Memory to the existing AgentsPane.
pub enum AgentsPane { List, Detail, History, Editor, Confirmation, Memory }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentDetailAction { AssignedSkills, Memory }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryPane {
    EntryList,
    EntryDetail,
    EntryHistory,
    Editor,
    MutationReview,
    Confirmation,
    Proposals,
    ProposalDetail,
    ProposalResolutionReview,
    EpisodicSummaries,
    EpisodicDetail,
    Result,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryConfirmation {
    pub command: ApplicationCommand,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryOutcomeIntent {
    Entries,
    EntryDetail(MemoryEntryId),
    EntryHistory(MemoryEntryId),
    EntryVersion {
        selector: AgentProfileSelector,
        key: String,
        version: ObjectVersion,
        entry_version_id: MemoryEntryVersionId,
    },
    Proposals,
    ProposalDetail(MemoryProposalId),
    Episodes,
    EpisodeDetail(EpisodicSummaryId),
    Mutation,
    Resolution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryViewState {
    pub profile: Option<MemoryProfileIdentityView>,
    pub namespace_id: Option<MemoryNamespaceId>,
    pub pane: MemoryPane,
    pub entries: Option<MemoryEntriesView>,
    pub entry_detail: Option<MemoryEntryView>,
    pub entry_history: Option<MemoryEntryHistoryView>,
    pub entry_version: Option<MemoryEntryVersionView>,
    pub proposals: Option<MemoryProposalsView>,
    pub proposal_detail: Option<MemoryProposalView>,
    pub episodes: Option<EpisodicSummariesView>,
    pub episode_detail: Option<EpisodicSummaryView>,
    pub selected_entry: usize,
    pub selected_history_version: usize,
    pub selected_proposal: usize,
    pub selected_episode: usize,
    pub entry_scroll: usize,
    pub detail_scroll: usize,
    pub history_scroll: usize,
    pub proposal_scroll: usize,
    pub episode_scroll: usize,
    pub editor: Option<MemoryEditor>,
    pub edit_review: Option<MemoryEditReview>,
    pub resolution_review: Option<MemoryProposalResolutionReview>,
    pub confirmation: Option<MemoryConfirmation>,
    pub review_registered: bool,
    pub generation: u64,
    pub pending_intent: Option<MemoryOutcomeIntent>,
}

impl MemoryViewState {
    pub fn open_create_editor(
        &mut self,
        selector: AgentProfileSelector,
    ) -> Result<(), DomainError> {
        if self.review_registered {
            return Err(DomainError::MemoryReviewUnavailable);
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(DomainError::MemoryGenerationOverflow)?;
        self.editor = Some(MemoryEditor::for_create(selector));
        self.edit_review = None;
        self.resolution_review = None;
        self.confirmation = None;
        self.review_registered = false;
        self.pending_intent = None;
        self.pane = MemoryPane::Editor;
        Ok(())
    }

    pub(crate) fn begin_pending(
        &mut self,
        intent: MemoryOutcomeIntent,
    ) -> Result<u64, DomainError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(DomainError::MemoryGenerationOverflow)?;
        self.pending_intent = Some(intent);
        Ok(self.generation)
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending_intent = None;
    }

    pub(crate) fn selected_entry_id(&self) -> Result<MemoryEntryId, DomainError> {
        self.entries
            .as_ref()
            .and_then(|view| view.entries.get(self.selected_entry))
            .map(|summary| summary.entry.entry_id())
            .ok_or(DomainError::MemorySelectionUnavailable)
    }

    pub(crate) fn apply_matching_view(
        &mut self,
        intent: &MemoryOutcomeIntent,
        view: CommandView,
    ) -> bool {
        let matched = match (intent, view) {
            (MemoryOutcomeIntent::Entries, CommandView::MemoryEntries(value)) => {
                self.entries = Some(value);
                true
            }
            (MemoryOutcomeIntent::EntryDetail(expected), CommandView::MemoryEntry(value))
                if value.entry.reference().entry_id() == *expected =>
            {
                self.entry_detail = Some(value);
                true
            }
            (MemoryOutcomeIntent::EntryHistory(expected), CommandView::MemoryEntryHistory(value))
                if value.current.entry_id() == *expected =>
            {
                self.entry_history = Some(value);
                true
            }
            (MemoryOutcomeIntent::EntryVersion { entry_version_id, .. }, CommandView::MemoryEntryVersion(value))
                if value.entry.reference().entry_version_id() == *entry_version_id =>
            {
                self.entry_version = Some(value);
                true
            }
            (MemoryOutcomeIntent::Proposals, CommandView::MemoryProposals(value)) => {
                self.proposals = Some(value);
                true
            }
            (MemoryOutcomeIntent::ProposalDetail(expected), CommandView::MemoryProposal(value))
                if value.proposal.reference().proposal_id() == *expected =>
            {
                self.proposal_detail = Some(value);
                true
            }
            (MemoryOutcomeIntent::Episodes, CommandView::EpisodicSummaries(value)) => {
                self.episodes = Some(value);
                true
            }
            (MemoryOutcomeIntent::EpisodeDetail(expected), CommandView::EpisodicSummary(value))
                if value.summary.reference().summary_id() == *expected =>
            {
                self.episode_detail = Some(value);
                true
            }
            (MemoryOutcomeIntent::Mutation, CommandView::MemoryEntryMutation(_))
            | (MemoryOutcomeIntent::Resolution, CommandView::MemoryProposalResolution(_)) => true,
            _ => false,
        };
        if matched {
            self.pending_intent = None;
        }
        matched
    }
}

impl Default for MemoryViewState {
    fn default() -> Self {
        Self {
            profile: None,
            namespace_id: None,
            pane: MemoryPane::EntryList,
            entries: None,
            entry_detail: None,
            entry_history: None,
            entry_version: None,
            proposals: None,
            proposal_detail: None,
            episodes: None,
            episode_detail: None,
            selected_entry: 0,
            selected_history_version: 0,
            selected_proposal: 0,
            selected_episode: 0,
            entry_scroll: 0,
            detail_scroll: 0,
            history_scroll: 0,
            proposal_scroll: 0,
            episode_scroll: 0,
            editor: None,
            edit_review: None,
            resolution_review: None,
            confirmation: None,
            review_registered: false,
            generation: 0,
            pending_intent: None,
        }
    }
}
```

`MemoryViewState` owns exact profile/namespace, entry/proposal/episode lists and details, independent selections/scrolls, editor, installed edit or resolution review, confirmation, review-registration flag, pending outcome intent, and monotonically checked generations. Add the content-free `DomainError::{MemoryGenerationOverflow, MemorySelectionUnavailable}` variants with codes `memory_generation_overflow` and `memory_selection_unavailable`; controller and host convert them to the existing safe error presentation.

Add these fields to `AgentsViewState`:

```rust
pub selected_detail_action: AgentDetailAction,
pub memory: MemoryViewState,
```

Initialize `selected_detail_action` to `AssignedSkills`. Left/Right toggles the two actions and Enter opens the selected action even when the agent has zero assigned skills. Preserve both the selector and nested memory state when global tabs change; reset the selector to `AssignedSkills` only when the stable selected profile changes. Keep `NavigationTab` and `tab_states: [TabState; 6]` unchanged.

- [ ] **Step 4: Add typed controller effects and pure transitions**

```rust
// Append these variants to the existing ControllerEffect enum.
pub enum ControllerEffect {
    LoadAgentMemory(AgentProfileSelector),
    LoadMemoryEntry { selector: AgentProfileSelector, key: String },
    LoadMemoryEntryHistory { selector: AgentProfileSelector, key: String },
    LoadMemoryEntryVersion {
        selector: AgentProfileSelector,
        key: String,
        version: ObjectVersion,
        expected_entry_version_id: MemoryEntryVersionId,
    },
    RequestMemorySetPreview(MemoryPreviewRequest),
    RequestMemoryDeletePreview { selector: AgentProfileSelector, key: String, generation: u64 },
    LoadMemoryProposals { selector: AgentProfileSelector, filter: MemoryProposalFilter },
    LoadMemoryProposal(MemoryProposalId),
    RequestMemoryProposalResolutionPreview { proposal: MemoryProposalRef, action: MemoryResolutionAction, generation: u64 },
    LoadEpisodicSummaries(AgentProfileSelector),
    LoadEpisodicSummary(EpisodicSummaryId),
    ExecuteMemory(ApplicationCommand),
    CancelMemoryReview,
}
```

Use arrows/Enter/Esc everywhere. In Agent Detail, Left/Right updates `selected_detail_action` and Enter on `Memory` emits `LoadAgentMemory` regardless of assigned-skill count. From entry list, `c` begins create, `p` opens proposals, and `e` opens episodes. Entry detail exposes edit/delete/history. Proposal detail exposes approve/reject; episode detail is read-only. Do not issue repository calls or durable IDs in controller code.

- [ ] **Step 5: Rerun GREEN and full navigation regressions**

```bash
cargo test --test memory_tui_controller_contract --test tui_navigation_contract --test skill_tui_controller_contract --test agent_profile_tui_controller_contract
```

- [ ] **Step 6: Commit**

```bash
git add src/ui/tui/model.rs src/ui/tui/controller.rs src/ui/tui/mod.rs tests/memory_tui_controller_contract.rs tests/tui_navigation_contract.rs
git commit -m "feat: add agents memory workspace controls"
```

### Task 18: Connect TUI Memory Effects to the Runtime and Cleanup Lifecycle

**Files:**

- Modify: `src/ui/tui/host.rs`
- Modify: `src/ui/tui/mod.rs`
- Modify: `src/ui/tui/model.rs`
- Modify: `src/ui/tui/error.rs`
- Create: `tests/memory_tui_host_contract.rs`

**Interfaces:**

- Consumes: Task 17’s thirteen memory `ControllerEffect` variants and `MemoryViewState`; `RuntimeClient::try_submit(&self, command: ApplicationCommand) -> Result<PendingOutcome, RuntimeError>`; the five passive preview/cancel methods listed in Task 16; `TuiRunner.pending: Option<PendingOutcome>`; and `TuiModel::set_command_in_flight_preserving_navigation(&mut self)`.
- Produces: `TuiRunner::queue_memory_command(&mut self, command: ApplicationCommand, intent: MemoryOutcomeIntent) -> Result<LoopControl, TuiError>`; `TuiRunner::execute_memory_effect(&mut self, effect: ControllerEffect) -> Result<LoopControl, TuiError>`; `fn memory_mutation_intent(command: &ApplicationCommand) -> Result<MemoryOutcomeIntent, TuiError>`; `fn install_set_preview(client: &RuntimeClient, model: &mut TuiModel, request: MemoryPreviewRequest) -> Result<(), TuiError>`; `fn install_delete_preview(client: &RuntimeClient, model: &mut TuiModel, selector: AgentProfileSelector, key: String, generation: u64) -> Result<(), TuiError>`; `fn install_resolution_preview(client: &RuntimeClient, model: &mut TuiModel, proposal: MemoryProposalRef, action: MemoryResolutionAction, generation: u64) -> Result<(), TuiError>`; `fn apply_memory_outcome(state: &mut MemoryViewState, intent: &MemoryOutcomeIntent, generation: u64, view: CommandView) -> bool`; `fn cancel_tui_memory_review_once(client: &RuntimeClient, state: &mut MemoryViewState) -> Result<(), RuntimeError>`; and `TuiError::{UnexpectedControllerEffect, MemoryState(DomainError)}`.

- [ ] **Step 1: Write failing typed-command, retry, and delayed-outcome host tests**

Cover every entry/history/version/proposal/episode read through typed commands; set/delete preview; no-change without registered review; action-specific approve/reject previews; exact confirmation submission; successful targeted refresh; backpressure retention; stale/terminal cleanup; exactly-once cancel on Esc/interrupt/input/draw/runtime failure; and delayed read hydration without navigation theft or protected-state loss.

```rust
#[test]
fn delayed_entry_result_cannot_close_a_newer_editor() {
    let mut state = MemoryViewState::default();
    let expected = entry_id(7);
    let intent = MemoryOutcomeIntent::EntryDetail(expected);
    let old_generation = state.begin_pending(intent.clone()).unwrap();
    state.open_create_editor(profile_selector()).unwrap();

    assert!(!apply_memory_outcome(
        &mut state,
        &intent,
        old_generation,
        CommandView::MemoryEntry(entry_view(expected)),
    ));
    assert_eq!(state.pane, MemoryPane::Editor);
}
```

Place this focused unit test beside `apply_memory_outcome` in `host.rs`. The integration contract drives the public `run_tui_with_screen` with the established blocking `CommandExecutor`/fake screen/event-source pattern to prove that `TuiRunner.pending` retains the receiver until delivery.

- [ ] **Step 2: Run TUI host contracts to verify RED**

Run:

```bash
cargo test --test memory_tui_host_contract
```

Expected RED: the host has no arm for `LoadMemoryEntry`.

- [ ] **Step 3: Route every effect through typed runtime APIs**

Add a private method to the existing `TuiRunner`; do not create a standalone helper that lacks access to `pending`:

```rust
impl TuiRunner {
    fn queue_memory_command(
        &mut self,
        command: ApplicationCommand,
        intent: MemoryOutcomeIntent,
    ) -> Result<LoopControl, TuiError> {
        let generation = self.model.agents.memory.begin_pending(intent)?;
        self.model.set_command_in_flight_preserving_navigation();
        match self.client.try_submit(command) {
            Ok(pending) => {
                self.pending = Some(pending);
                self.model.agents.memory.generation = generation;
                Ok(LoopControl::Continue { redraw: true })
            }
            Err(RuntimeError::Backpressure) => {
                self.model.set_command_in_flight(false);
                self.model.agents.memory.clear_pending();
                self.model.set_message(
                    Severity::Warning,
                    "Another command is running; this memory action remains available to retry.",
                );
                Ok(LoopControl::Continue { redraw: true })
            }
            Err(error) => {
                self.model.set_command_in_flight(false);
                self.model.agents.memory.clear_pending();
                Err(error.into())
            }
        }
    }

    fn execute_memory_effect(
        &mut self,
        effect: ControllerEffect,
    ) -> Result<LoopControl, TuiError> {
        match effect {
            ControllerEffect::LoadAgentMemory(selector) => self.queue_memory_command(
                ApplicationCommand::ListMemoryEntries { selector },
                MemoryOutcomeIntent::Entries,
            ),
            ControllerEffect::LoadMemoryEntry { selector, key } => self.queue_memory_command(
                ApplicationCommand::ShowMemoryEntry { selector, display_key: key },
                MemoryOutcomeIntent::EntryDetail(self.model.agents.memory.selected_entry_id()?),
            ),
            ControllerEffect::LoadMemoryEntryHistory { selector, key } => self.queue_memory_command(
                ApplicationCommand::ShowMemoryEntryHistory { selector, display_key: key },
                MemoryOutcomeIntent::EntryHistory(self.model.agents.memory.selected_entry_id()?),
            ),
            ControllerEffect::LoadMemoryEntryVersion {
                selector,
                key,
                version,
                expected_entry_version_id,
            } => self.queue_memory_command(
                ApplicationCommand::ShowMemoryEntryVersion {
                    selector: selector.clone(),
                    display_key: key.clone(),
                    version,
                },
                MemoryOutcomeIntent::EntryVersion {
                    selector,
                    key,
                    version,
                    entry_version_id: expected_entry_version_id,
                },
            ),
            ControllerEffect::LoadMemoryProposals { selector, filter } => self.queue_memory_command(
                ApplicationCommand::ListMemoryProposals { selector, filter },
                MemoryOutcomeIntent::Proposals,
            ),
            ControllerEffect::LoadMemoryProposal(proposal_id) => self.queue_memory_command(
                ApplicationCommand::ShowMemoryProposal { proposal_id },
                MemoryOutcomeIntent::ProposalDetail(proposal_id),
            ),
            ControllerEffect::LoadEpisodicSummaries(selector) => self.queue_memory_command(
                ApplicationCommand::ListEpisodicSummaries { selector },
                MemoryOutcomeIntent::Episodes,
            ),
            ControllerEffect::LoadEpisodicSummary(summary_id) => self.queue_memory_command(
                ApplicationCommand::ShowEpisodicSummary { summary_id },
                MemoryOutcomeIntent::EpisodeDetail(summary_id),
            ),
            ControllerEffect::ExecuteMemory(command) => {
                let intent = memory_mutation_intent(&command)?;
                self.queue_memory_command(command, intent)
            }
            ControllerEffect::RequestMemorySetPreview(request) => {
                install_set_preview(&self.client, &mut self.model, request)?;
                Ok(LoopControl::Continue { redraw: true })
            }
            ControllerEffect::RequestMemoryDeletePreview { selector, key, generation } => {
                install_delete_preview(&self.client, &mut self.model, selector, key, generation)?;
                Ok(LoopControl::Continue { redraw: true })
            }
            ControllerEffect::RequestMemoryProposalResolutionPreview {
                proposal,
                action,
                generation,
            } => {
                install_resolution_preview(
                    &self.client,
                    &mut self.model,
                    proposal,
                    action,
                    generation,
                )?;
                Ok(LoopControl::Continue { redraw: true })
            }
            ControllerEffect::CancelMemoryReview => {
                cancel_tui_memory_review_once(
                    &self.client,
                    &mut self.model.agents.memory,
                )?;
                Ok(LoopControl::Continue { redraw: true })
            }
            _ => Err(TuiError::UnexpectedControllerEffect),
        }
    }
}
```

`MemoryViewState::selected_entry_id()` resolves the stable selected ID from the bounded list rather than trusting the index after refresh. `memory_mutation_intent` accepts only set/delete as `Mutation` and approve/reject as `Resolution`; other commands return the new content-free `TuiError::UnexpectedControllerEffect`. Add that unit variant and `TuiError::MemoryState(#[from] DomainError)` to `tui::error`, both with content-free display text. The three preview installers call the matching passive runtime methods and install results only when selector/profile/generation match. Presentation code never receives a database or repository handle.

Add one exhaustive memory-effect group to `TuiRunner::apply_effect` that passes all thirteen variants from Task 17 to `self.execute_memory_effect(effect)`. Keep the existing top-level `pending.is_some()` guard, so no second receiver can overwrite `TuiRunner.pending`.

- [ ] **Step 4: Apply outcomes with identity and generation checks**

No-change displays `No change` and leaves `review_registered == false`. A review installs only when selector/profile/generation still match. Successful confirmation clears review state and refreshes the exact profile's entries/proposals without discarding unrelated state. Delayed inactive reads may hydrate cached data but may not select a different item, navigate, close an editor/review, or replace a newer generation.

```rust
fn apply_memory_outcome(
    state: &mut MemoryViewState,
    intent: &MemoryOutcomeIntent,
    generation: u64,
    view: CommandView,
) -> bool {
    if state.generation != generation || state.pending_intent.as_ref() != Some(intent) {
        return false;
    }
    state.apply_matching_view(intent, view)
}
```

In `TuiRunner::poll_pending`, clone `(intent, generation)` from `model.agents.memory` before taking the completed receiver, clone `outcome.view`, and call `apply_memory_outcome` before the generic `apply_outcome`. A mismatch leaves the newer pane/editor/review untouched. A matching read only hydrates its named slot. A matching mutation/resolution clears local review state and returns the exact-profile `LoadAgentMemory` or `LoadMemoryProposals` refresh effect after the completed receiver has been removed; it never selects a different stable ID.

- [ ] **Step 5: Preserve or cancel reviews according to failure class**

Submit through the existing nonblocking/backpressure path. Backpressure retains the exact command/review. A stale or terminal application error calls `cancel_memory_review()` once and preserves the local draft where safe. Extend `run_with_screen` teardown so every host exit cancels an active memory review once before runtime finish.

```rust
fn cancel_tui_memory_review_once(
    client: &RuntimeClient,
    state: &mut MemoryViewState,
) -> Result<(), RuntimeError> {
    if std::mem::take(&mut state.review_registered) {
        client.cancel_memory_review()?;
    }
    Ok(())
}
```

- [ ] **Step 6: Rerun GREEN and host regressions**

```bash
cargo test --test memory_tui_host_contract --test skill_tui_host_contract --test agent_profile_tui_host_contract --test tui_hardening_contract
```

- [ ] **Step 7: Commit**

```bash
git add src/ui/tui/host.rs src/ui/tui/mod.rs src/ui/tui/model.rs tests/memory_tui_host_contract.rs
git commit -m "feat: connect memory workspace to runtime"
```

### Task 19: Render Nested Memory at Narrow, Medium, and Wide Layouts

**Files:**

- Create: `src/ui/tui/views/memory.rs`
- Modify: `src/ui/tui/views/mod.rs`
- Modify: `src/ui/tui/views/agents.rs`
- Modify: `src/ui/tui/render.rs`
- Modify: `src/ui/tui/layout.rs`
- Modify: `src/ui/tui/views/help.rs`
- Create: `tests/memory_tui_render_contract.rs`

**Interfaces:**

- Consumes: `TuiModel`, `AgentsPane::Memory`, `MemoryPane`, `MEMORY_PLAINTEXT_WARNING`, `LayoutMode`, `agent_layout_mode(area: Rect) -> LayoutMode`, `agent_workspace(area: Rect, mode: LayoutMode) -> AgentWorkspaceLayout`, existing safe-text/scroll helpers, and Ratatui `Frame`, `Rect`, and `Theme`.
- Produces: `pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme)` and `pub(super) fn content_height(model: &TuiModel, width: u16) -> u16` in `views::memory`; `pub struct MemoryWorkspaceLayout { pub primary: Rect, pub detail: Option<Rect>, pub context: Option<Rect> }`; `pub fn memory_workspace(area: Rect, mode: LayoutMode) -> MemoryWorkspaceLayout`; and the exhaustive `AgentsPane::Memory` delegation in `views::agents`.

- [ ] **Step 1: Write failing layout, warning, escaping, and navigation tests**

Assert the Agent Detail selector visibly renders `Assigned Skills | Memory` before entry; one/two/three-pane memory layouts; legibility at every supported size including `60x18`; identity/action/Enter/Esc visibility; exact plaintext warning in all edit/review/confirmation/resolution surfaces; hostile-text escaping; source-linked summary label; read-only episodic detail; no global Memory tab/shortcut; unchanged six navigation labels; and no memory prose in routine views.

```rust
#[test]
fn narrow_memory_review_keeps_identity_warning_and_controls_visible() {
    let model = memory_mutation_review_model_with_hostile_text();
    let frame = render_to_string(&model, 60, 18);
    assert!(frame.contains("Agents / Memory"));
    assert!(frame.contains(MEMORY_PLAINTEXT_WARNING));
    assert!(frame.contains("Enter"));
    assert!(frame.contains("Esc"));
    assert!(!frame.contains('\u{1b}'));
}
```

- [ ] **Step 2: Run memory rendering contracts to verify RED**

Run:

```bash
cargo test --test memory_tui_render_contract
```

Expected RED: `views::memory` cannot be resolved.

- [ ] **Step 3: Delegate Agents/Memory rendering without a global view**

When `AgentsPane::Memory`, `agents::render` delegates to `memory::render`; otherwise preserve existing Agents behavior. The header may read `Agents / Memory`, but `View`, `NavigationTab`, `tab_states: [TabState; 6]`, numeric/letter shortcuts, and Skills overlay ownership remain unchanged. Show `Memory input` only when the active memory editor owns the command area.

```rust
pub(super) fn render(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme) {
    if model.agents.pane == AgentsPane::Memory {
        memory::render(frame, area, model, theme);
        return;
    }

    let mode = agent_layout_mode(frame.area());
    let layout = agent_workspace(area, mode);
    if let Some(list) = layout.list {
        render_list(frame, list, model, theme);
    }
    render_active(frame, layout.active, model, theme, layout.list.is_none());
}
```

Also add `AgentsPane::Memory => memory::render(frame, area, model, theme)` to the private `render_active` match so that function stays exhaustive and defensive if it is called directly. All other existing Agents arms and helper bodies remain byte-for-byte unchanged.

- [ ] **Step 4: Implement adaptive panes and bounded safe text**

Use these exact view-module entry points and bodies; the four named helpers below are private to `views::memory` and are implemented in the same step:

```rust
pub(super) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    model: &TuiModel,
    theme: &Theme,
) {
    let layout = memory_workspace(area, agent_layout_mode(frame.area()));
    render_primary(frame, layout.primary, model, theme);
    if let Some(detail) = layout.detail {
        render_detail(frame, detail, model, theme);
    }
    if let Some(context) = layout.context {
        render_context(frame, context, model, theme);
    }
}

pub(super) fn content_height(model: &TuiModel, width: u16) -> u16 {
    let content_width = width.saturating_sub(2).max(1);
    memory_content_lines(model)
        .iter()
        .map(|line| wrapped_line_height(line, content_width))
        .fold(0_u16, u16::saturating_add)
        .max(1)
}
```

Implement these referenced helpers with the exact contracts `fn render_primary(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme)`, `fn render_detail(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme)`, `fn render_context(frame: &mut Frame<'_>, area: Rect, model: &TuiModel, theme: &Theme)`, `fn memory_content_lines(model: &TuiModel) -> Vec<Line<'static>>`, and `fn wrapped_line_height(line: &Line<'_>, width: u16) -> u16`. `memory_content_lines` selects only the active `MemoryPane`; each render helper consumes the same safe, bounded line builders, and `wrapped_line_height` returns the ceiling of escaped display width divided by nonzero `width` with a minimum of one.

Add this exact layout contract to `layout.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryWorkspaceLayout {
    pub primary: Rect,
    pub detail: Option<Rect>,
    pub context: Option<Rect>,
}

pub fn memory_workspace(area: Rect, mode: LayoutMode) -> MemoryWorkspaceLayout {
    match mode {
        LayoutMode::TooSmall | LayoutMode::Narrow => MemoryWorkspaceLayout {
            primary: area,
            detail: None,
            context: None,
        },
        LayoutMode::Medium => {
            let columns = Layout::horizontal([
                Constraint::Percentage(42),
                Constraint::Percentage(58),
            ])
            .split(area);
            MemoryWorkspaceLayout {
                primary: columns[0],
                detail: Some(columns[1]),
                context: None,
            }
        }
        LayoutMode::Wide => {
            let columns = Layout::horizontal([
                Constraint::Percentage(30),
                Constraint::Percentage(44),
                Constraint::Percentage(26),
            ])
            .split(area);
            MemoryWorkspaceLayout {
                primary: columns[0],
                detail: Some(columns[1]),
                context: Some(columns[2]),
            }
        }
    }
}
```

Narrow mode renders one current pane; medium renders entry/list plus active detail/editor; wide adds contextual history/proposal/source information. Apply existing safe text and scroll bounding to every untrusted field. Keep the profile identity, exact key/proposal identity, operation, and controls visible in every review/confirmation before rendering optional prose.

- [ ] **Step 5: Rerun GREEN and render/navigation regressions**

```bash
cargo test --test memory_tui_render_contract --test tui_navigation_contract --test skill_tui_render_contract --test agent_profile_tui_render_contract --test tui_application_contract
```

- [ ] **Step 6: Commit**

```bash
git add src/ui/tui/views src/ui/tui/render.rs src/ui/tui/layout.rs tests/memory_tui_render_contract.rs
git commit -m "feat: render the agents memory workspace"
```

### Task 20: Add Cross-Host and Phase-2 Acceptance Coverage

**Files:**

- Create: `tests/hybrid_memory_acceptance.rs`
- Modify: `tests/support/mod.rs`
- Modify: `tests/tui_navigation_contract.rs`

**Interfaces:**

- Consumes: public application/runtime/fallback/TUI behavior from Tasks 1–19 and existing deterministic test fixtures.
- Produces in `tests/support/mod.rs`: `#[derive(Debug, Clone, PartialEq, Eq)] pub struct HostParityResult { pub command: ApplicationCommand, pub event_payload: ApplicationEvent, pub view: CommandView, pub navigation_labels: [&'static str; 6] }`; `pub fn run_fallback_memory_set_scenario() -> HostParityResult`; `pub fn run_tui_memory_set_scenario() -> HostParityResult`; `#[derive(Debug, Clone, PartialEq, Eq)] pub struct ManualMemoryAcceptanceSeed { pub profile: AgentProfileVersionRef, pub tui_approval_proposal_id: MemoryProposalId, pub tui_rejection_proposal_id: MemoryProposalId, pub fallback_approval_proposal_id: MemoryProposalId, pub fallback_rejection_proposal_id: MemoryProposalId, pub restart_pending_proposal_id: MemoryProposalId, pub summary_id: EpisodicSummaryId }`; and `pub fn seed_manual_memory_acceptance(paths: &AppPaths) -> Result<ManualMemoryAcceptanceSeed, AppError>`. Produces `tests/hybrid_memory_acceptance.rs`; no production interface.

- [ ] **Step 1: Write failing end-to-end acceptance scenarios**

Add deterministic scenarios for:

- fallback and TUI producing equivalent typed direct mutations and deliberate views;
- fallback and TUI resolving the same exact proposal with the same event/outcome semantics;
- two agents never seeing or retrieving one another's memory;
- copied non-memory profile draft fields creating a fresh empty namespace;
- source-linked episodic summaries remaining read-only and visibly qualified;
- command-looking memory remaining inert in editors, renderers, snapshots, and replay;
- audit/error/status/help excluding all memory prose;
- restart/recovery preserving tombstone history, pending proposals, approvals, and deterministic fresh retrieval; and
- every path preserving the six merged global navigation shortcuts.

Name the deterministic retrieval/isolation acceptance case
`bounded_snapshot_is_deterministic_and_never_crosses_namespaces` so the manual
guide can invoke that automated-only path exactly; no production snapshot route
may be introduced for manual inspection.

Also add a root-level ignored test named `seed_manual_acceptance_state`. It is
the sole manual bridge for proposals and summaries because Phase 2 exposes no
production creation route for either. The test must read only the task-specific
`AI_STOCK_FORUM_MEMORY_ACCEPTANCE_STATE_DIR` variable, require its exact path to
equal `AppPaths::discover()?.state_dir()`, and refuse unless that state
directory does not yet exist. Put the decision in a pure private
`validate_manual_acceptance_target(requested: &Path, discovered: &Path, target_exists: bool) -> Result<(), &'static str>`
helper; the ignored test supplies `target_exists` from `symlink_metadata`, and
ordinary tests cover mismatch and pre-existing-path rejection without mutating
process environment variables. Only `ErrorKind::NotFound` maps to `false`; any
other metadata error refuses the seed. The ignored test then calls
`seed_manual_memory_acceptance`, creates a complete runnable synthetic app
state, closes every database handle, prints only stable fixture labels with the
synthetic profile/proposal/summary identifiers, and deliberately leaves the
database for the live acceptance restart. The seed contains five pending
proposals: TUI approval, TUI rejection, fallback approval, fallback rejection,
and restart-preservation. All five use
different normalized keys, and all differ from the manual direct-edit keys
`thesis` and `fallback_thesis`, so neither direct edits nor another proposal's
resolution can expire them as same-key siblings. It also contains one
source-linked episodic summary. Ordinary non-ignored tests cover path mismatch,
pre-existing-directory refusal, exact proposal count/status/key separation, and
handle closure. Never accept a database-file path, overwrite an existing
directory, print seeded prose, or expose this fixture through a production
command, capability, service, or binary.

```rust
#[test]
fn fallback_and_tui_commit_equivalent_typed_memory_mutations() {
    let fallback = run_fallback_memory_set_scenario();
    let tui = run_tui_memory_set_scenario();
    assert_eq!(fallback.command, tui.command);
    assert_eq!(fallback.event_payload, tui.event_payload);
    assert_eq!(fallback.view, tui.view);
    assert_eq!(fallback.navigation_labels, ["1 Overview", "2 Setup", "3 Audit", "4 Help", "a Agents", "s Skills"]);
}
```

- [ ] **Step 2: Run acceptance RED or return defects to their owning task**

Run without changing production code in this task:

```bash
cargo test --test hybrid_memory_acceptance
```

Expected RED: at least one cross-layer acceptance path remains unproved. If a scenario fails, stop Task 20, add the failing assertion to the focused contract named in this routing table, create a focused correction commit, and rerun that owning contract before returning. Do not rewrite or amend already-reviewed history.

| Failure | Owning contract |
|---|---|
| command/event/outcome mismatch | `memory_application_contract` |
| namespace crossover | `memory_isolation_contract` |
| restart or tamper mismatch | `memory_recovery_contract` |
| fallback state/cleanup | `memory_fallback_contract` |
| TUI intent or delayed outcome | `memory_tui_host_contract` |
| layout/escaping/warning | `memory_tui_render_contract` |
| shortcut regression | `tui_navigation_contract` |

Expected before Step 3: all acceptance scenarios pass using only behavior already committed by Tasks 1–19. Do not add a production proposal, summary, or snapshot presentation route.

- [ ] **Step 3: Rerun focused cross-layer regressions**

```bash
cargo test --test hybrid_memory_acceptance \
  --test memory_isolation_contract \
  --test memory_recovery_contract \
  --test memory_fallback_contract \
  --test memory_tui_host_contract \
  --test memory_tui_render_contract \
  --test tui_navigation_contract
```

- [ ] **Step 4: Commit**

```bash
git add tests/hybrid_memory_acceptance.rs tests/support/mod.rs tests/tui_navigation_contract.rs
git commit -m "test: add hybrid memory acceptance coverage"
```

### Task 21: Document, Review, Verify, and Mark Phase 2 Complete

**Files:**

- Modify: `README.md`
- Modify: `architecture.md`
- Create: `docs/testing/phase-2-hybrid-memory.md`
- Modify: `tests/documentation_contract.rs`
- Modify: `phases.md` only after the pre-completion gate passes

**Interfaces:**

- Consumes: every focused Hybrid Memory contract, `tests/hybrid_memory_acceptance.rs`, the approved design, `README.md`, `architecture.md`, and the existing review/verification/release gates.
- Produces: README sections `Phase 2 Hybrid Memory Milestone 3` and `Hybrid Memory fallback commands`; architecture section `Hybrid Memory durability and recovery`; complete `docs/testing/phase-2-hybrid-memory.md`; assertions in `tests/documentation_contract.rs`; and, only after every gate passes, Milestone 3 and the Phase 2 exit marks in `phases.md`. No Rust runtime API is produced.

- [ ] **Step 1: Write the failing durable-documentation contract**

Require README and architecture documentation to describe: local plaintext at every durable copy (entry/proposal/summary rows, events, request/outcome receipts, WAL/journal, detail displays, backups); overwrite/delete history retention; monotonically growing local storage under repeated edits; full-disk/capacity rollback without partial state; credentials prohibition and scanner limits; Agents → Memory navigation; exact fallback grammar; Human review versus proposal approval; source-qualified episodic summaries; bounded deterministic retrieval; recovery; and all explicitly deferred features.

```rust
#[test]
fn durable_docs_disclose_plaintext_history_and_capacity_behavior() {
    let readme = include_str!("../README.md");
    let architecture = include_str!("../architecture.md");
    let manual = include_str!("../docs/testing/phase-2-hybrid-memory.md");
    for required in ["plaintext", "history", "credentials", "Agents → Memory"] {
        assert!(readme.contains(required), "README missing {required}");
    }
    assert!(architecture.contains("BEGIN IMMEDIATE"));
    assert!(manual.contains("60x18"));
}
```

- [ ] **Step 2: Run the documentation contract to verify RED**

Run:

```bash
cargo test --test documentation_contract
```

Expected RED: compilation fails because the manual guide is absent, or an assertion names missing Hybrid Memory text.

- [ ] **Step 3: Add the README operator-facing text**

Insert this complete text before `## Build, run, and test` in `README.md`:

````markdown
## Phase 2 Hybrid Memory Milestone 3

Hybrid Memory is local, per-agent memory nested at **Agents → Memory**. It
supports reviewed Human set, overwrite, delete, proposal approval, and proposal
rejection; bounded entry/history/proposal/episodic reads; and deterministic,
bounded snapshot retrieval for internal application use. Direct Human edits are
reviewed before commit. Agent-authored changes are pending proposals and do not
change memory until a Human approves the exact proposal; rejection records the
decision without changing the entry.

All Hybrid Memory is plaintext at rest. Plaintext can appear in current and
historical entry, proposal, and episodic-summary rows; immutable events;
command request/outcome receipts; SQLite WAL or journal files; deliberate
detail displays; and any database or filesystem backup. Never store passwords,
API keys, access tokens, private keys, seed phrases, session cookies, or other
credentials in memory. The credential scanner is a bounded deny-list and
best-effort guard, not a guarantee that secret material will be detected.

Overwrite and delete create immutable history; they do not erase earlier
plaintext. Repeated edits therefore make local storage grow monotonically.
Capacity or full-disk failure rolls the whole `BEGIN IMMEDIATE` transaction
back: no partial event, memory row, approval, projection, audit row, or receipt
is accepted. Back up and protect the state directory as sensitive plaintext.

Lists deliberately omit full entry values, proposal candidate text and
rationale, and episodic bodies. Detail views reveal the requested record and
escape terminal controls. Episodic detail is read-only and is labelled
`Summary — verify sources`; it keeps exact source references so the summary is
not presented as an unqualified fact. Generic audit, status, help, and error
views contain no memory prose.

### Hybrid Memory fallback commands

The fallback host accepts only these forms; quoted agent names and keys use the
existing command quoting rules:

```text
/memory list <agent>
/memory get <agent> <key>
/memory history <agent> <key> [positive-version]
/memory proposals <agent> [pending|all]
/memory proposal <proposal-uuid>
/memory episodes <agent>
/memory episode <summary-uuid>
/memory set <agent> <key>
/memory delete <agent> <key>
/memory approve <proposal-uuid>
/memory reject <proposal-uuid>
```

Set/delete and approve/reject open local review flows and require the exact
displayed action plus review digest. There is no production `/memory propose`,
summary-mutation, or snapshot command. Snapshot construction is an internal,
source-bounded retrieval interface; it is deterministic for the same verified
state and request.

Recovery verifies the immutable event stream and authenticated mirrors, then
rebuilds only derived current pointers and projections. Existing immutable
rows must match verified events byte-for-byte; missing permitted mirrors may be
recreated, while altered or unexplained immutable rows make startup fail
closed. Inference/chat, automatic memory extraction, semantic/vector search,
embeddings, autonomous proposal generation, remote sync, encryption at rest,
credential vaulting, retention pruning, secure erasure, and production
proposal/summary/snapshot UI routes remain deferred.

Run the focused acceptance test with:

```sh
cargo test --test hybrid_memory_acceptance
```

See [the Phase 2 Hybrid Memory testing guide](docs/testing/phase-2-hybrid-memory.md)
for isolated-state and manual acceptance procedures.
````

- [ ] **Step 4: Add the architecture contract text**

Append this complete section to `architecture.md`:

````markdown
## Hybrid Memory durability and recovery

Hybrid Memory belongs to a stable agent memory namespace. Activating a new
version of the same profile keeps that namespace; creating a different or
copied profile allocates a fresh empty namespace. Every producer and read first
resolves an exact profile-version reference to its authoritative stable
namespace. Deterministic retrieval encodes `memory-scope-v1` with that exact
profile reference, its derived namespace, and either `General` or canonical
`Tagged` purpose scope. Logical-key and proposal identities belong to their
typed commands and records, not to retrieval scope.

The immutable application event stream is the authoritative complete
non-secret memory record. Accepted entry versions, proposals, proposal
resolutions, episodic summaries, approvals, and command request/outcome
receipts are authenticated immutable mirrors used for exact inspection and
idempotent replay. Current-entry and current-proposal-status tables and the
in-memory projection are rebuildable pointers. Generic audit is intentionally
prose-free metadata and is not a second memory record.

Every durable copy is local plaintext: current and historical entry, proposal,
and summary rows; events; request/outcome receipts; SQLite WAL or journal
files; deliberate detail output; and backups. Credentials are prohibited. The
bounded deny-list scanner is defense in depth, not comprehensive secret
detection. Overwrite and tombstone deletion append immutable versions and do
not erase prior plaintext, so repeated edits monotonically increase storage.

All memory mutations use one SQLite `BEGIN IMMEDIATE` transaction. The actor
and capability matrix is exact: a Human may directly set/delete and may
approve/reject; an Agent may only create a proposal for its own exact profile
identity; System cannot mutate or resolve memory. A direct Human review binds
the exact profile, namespace, expected entry state, candidate/action, plaintext
acknowledgement, command, and digest. A proposal-resolution review additionally
binds the immutable proposal, pending approval, current entry state, and
Approve versus Reject action. Review tokens are process-local, one-use, and
held reserved through event, mirror, projection, audit, receipt, and commit.

Proposal creation appends one proposal event and one non-expiring pending
approval without modifying an entry. Approval revalidates the proposal,
approval, namespace, and expected current state, applies the exact candidate,
and expires newly stale same-key siblings in proposal-ID order within one
event. Rejection resolves only the selected proposal and approval. Direct Human
mutation likewise expires proposals made stale by the new current state. A
capacity/full-disk, contention, query, integrity, stale-state, or receipt error
cannot leave a partial event, row, approval, pointer, projection, audit record,
or receipt.

Command replay authenticates the canonical request fingerprint, exactly one
receipt event reference, and the primary object metadata. It reconstructs the
original outcome from that event and its immutable mirrors; it never repeats
current retrieval selection and never needs a process-local review token.
Snapshot retrieval streams deterministically ordered eligible records through
fixed byte/item limits. Exact purpose-tag matches precede bounded untagged
fallbacks; each record appears once; episodic summaries retain and display
their source references.

Startup verifies event sequence/digests, receipts, immutable mirrors, source
references, stable namespace ownership, current pointers, and proposal/
approval coherence. A missing rebuildable pointer is reconstructed. A missing
permitted immutable mirror may be backfilled from its verified event. An
existing immutable row must be byte-equivalent to that event; altered,
conflicting, or unexplained immutable data causes safe startup refusal rather
than repair-by-overwrite. WAL/journal files and backups are treated as
sensitive plaintext throughout recovery and operations.

The production surfaces expose deliberate reads and reviewed Human actions
only. There is no production proposal creator, summary writer, snapshot route,
automatic extraction, semantic/vector search, embeddings, autonomous agent,
remote sync, encryption at rest, vault, pruning, retention policy, or secure
erasure in Phase 2.
````

- [ ] **Step 5: Create the complete manual acceptance guide**

Create `docs/testing/phase-2-hybrid-memory.md` with this content:

````markdown
# Phase 2 Hybrid Memory acceptance guide

Run this guide only against disposable local state. Hybrid Memory, immutable
history, receipts, SQLite WAL/journal files, and backups are plaintext. Use
synthetic text only; never enter credentials or production/private material.

## Automated gate

From the feature worktree, run:

```sh
cargo build --release --locked
cargo test --test hybrid_memory_acceptance
```

The automated fixture is the approved internal harness for creating pending
Agent proposals and source-linked episodic summaries. Phase 2 intentionally has
no production `/memory propose`, summary-write, or snapshot presentation route.

## Disposable state

Never repoint a normal user's home directory and never point this guide at the
normal application state directory.

On Linux, build first so Cargo and rustup retain their normal configuration,
then isolate application data with a task-specific XDG directory:

```sh
memory_smoke_root="$(mktemp -d)"
mkdir -p "$memory_smoke_root/xdg-data"
export XDG_DATA_HOME="$memory_smoke_root/xdg-data"
memory_acceptance_state="$memory_smoke_root/xdg-data/ai-stock-forum"
printf '%s\n' "$memory_acceptance_state"
```

Keep this shell and exact environment for every Linux launch and restart. On
macOS, `BaseDirs` uses the signed-in user's standard Library directory, so run
the live manual steps only from a disposable local macOS user account. Record
that account's exact `~/Library/Application Support/ai-stock-forum` path before
launch. The automated tests use `AppPaths::for_test` and do not require either
OS-level setup.

Before the first live launch, seed the otherwise unavailable proposal and
episodic-summary fixtures into that exact empty application state. On macOS,
set `memory_acceptance_state` to the exact disposable user's recorded path.
The ignored test refuses a path that differs from discovery or already exists:

```sh
AI_STOCK_FORUM_MEMORY_ACCEPTANCE_STATE_DIR="$memory_acceptance_state" \
  cargo test --test hybrid_memory_acceptance seed_manual_acceptance_state \
  -- --ignored --exact --nocapture
```

Record the labelled synthetic profile, five proposal, and summary identifiers,
not their prose. Run this command only once per disposable state directory.

## Direct memory and history

1. Launch `target/release/ai-stock-forum`, press bare `a`, select a synthetic
   agent, open Detail, select `Memory` with Left/Right, and press Enter.
2. Create key `thesis` with value `synthetic value v1` and a synthetic purpose
   tag. Confirm the plaintext warning appears in editor, review, and
   confirmation. Leave review with Esc once and verify no entry exists; repeat
   and confirm the exact displayed `set <review-digest>` action.
3. Open the detail, edit the value to `synthetic value v2`, review, and confirm.
   Open History and verify versions 2 and 1 are newest-first and version 1 still
   contains its original plaintext.
4. Delete the entry, review its exact identity, cancel once, then repeat and
   confirm `delete <review-digest>`. Verify the current list omits it while
   History retains set, overwrite, and tombstone versions. This demonstrates
   that overwrite/delete do not erase immutable plaintext and storage grows.
5. Exit the TUI with `/quit`, then launch the fallback host and complete every
   labelled editor/review prompt. Replace `<agent>` with the printed synthetic
   profile selector and `<displayed-review-digest>` with the digest rendered by
   that exact review:

   ```text
   target/release/ai-stock-forum --command-mode
   /memory set <agent> fallback_thesis
   synthetic fallback value
   analysis
   set <displayed-review-digest>
   /memory list <agent>
   /memory get <agent> fallback_thesis
   /memory history <agent> fallback_thesis
   /quit
   ```

   Verify list output omits the full value; detail/history reveal only the
   deliberately requested data; and fallback and TUI report the same typed
   identity, version, status, and digest fields. Exact cross-host command/event/
   view parity remains an automated acceptance assertion.

## Proposals and episodic summaries

1. Use the five labelled proposal IDs and summary ID printed by the one-time
   seed command. The approved test-only harness already created their pending,
   distinct-key, and source-linked rows in this same disposable state; do not
   run it again.
2. Restart the TUI with the same disposable state. Under Agents → Memory,
   open Proposals, inspect the exact proposer/current-owner identities, select
   the labelled TUI-approval proposal, cancel once, then approve the exact
   displayed digest. Verify one entry mutation and one terminal approval. Use
   the labelled TUI-rejection proposal to verify Reject changes no entry.
3. Exit with `/quit`, launch `target/release/ai-stock-forum --command-mode`, and
   use only the two labelled fallback proposal IDs. For the fallback-approval
   proposal, run `/memory proposal <proposal-uuid>`, then `/memory approve
   <proposal-uuid>`; type `yes` once and verify it cannot commit, then enter the
   exact displayed `approve <review-digest>`. For the fallback-rejection
   proposal, inspect it, run `/memory reject <proposal-uuid>`, type the opposite
   `approve <review-digest>` once and verify it cannot commit, then enter the
   exact displayed `reject <review-digest>`. Inspect `/memory proposals <agent>
   all`, then exit with `/quit`. Do not resolve the labelled restart-pending
   proposal.
4. After fallback exits, relaunch `target/release/ai-stock-forum`, select the
   printed seeded profile under Agents → Memory, and open Episodic Summaries and
   its deliberate detail. Verify it is read-only, labelled `Summary — verify
   sources`, and shows bounded exact source references. Confirm generic Help,
   Status, Audit, and error output contains no entry, proposal, rationale,
   summary, or source prose.

## Isolation, restart, corruption, and capacity

1. Create two synthetic agents and reuse the key `thesis` with different
   values. Verify live list, detail, history, proposals, and episodic-summary
   views never cross namespaces.
2. Run the automated-only snapshot/retrieval assertion; Phase 2 deliberately
   exposes no production snapshot-inspection route:

   ```sh
   cargo test --test hybrid_memory_acceptance \
     bounded_snapshot_is_deterministic_and_never_crosses_namespaces -- --exact
   ```

3. Create a copied profile through the existing profile creator and verify its
   namespace is new and empty. Activate a new version of the original profile
   and verify it retains the original profile's namespace and memory without
   copying memory rows.
4. Leave an uncommitted draft or review, exit with `/quit`, and launch again
   with the same disposable environment. Verify tombstone history, four
   terminal proposal resolutions, the labelled restart-pending proposal, and
   episodic source links are unchanged; the draft and review token do not
   survive restart.
5. Do not hand-edit a live database. Run the isolated recovery contract, which
   creates and corrupts its own test copy, and verify its tamper/startup-refusal
   cases pass without printing memory prose:

   ```sh
   cargo test --test memory_recovery_contract
   ```

6. Run the isolated atomicity contract:

   ```sh
   cargo test --test memory_atomicity_contract
   ```

   Verify its `SqliteCapacity` case proves that raw event, memory, proposal,
   approval, projection, audit, and receipt snapshots are byte-identical before
   and after failure, then proves the exact review can retry. This is the
   deterministic full-disk/capacity acceptance path.

## Layout, shortcuts, and cleanup

1. Exercise Memory at `60x18`, a medium terminal, and a wide terminal. Verify
   one/two/three-pane layouts remain legible and always show agent identity,
   exact action, warning where required, Enter, and Esc.
2. From non-text memory panes verify bare `1`, `2`, `3`, `4`, `a`, and `s`
   preserve the merged six global destinations. Verify there is no seventh
   Memory tab. In a memory editor, type `1234as` and confirm it remains text.
   Bare `q` is inert everywhere; use `/quit` for normal shutdown.
3. Cancel a registered review with Esc, then exercise `/quit`, EOF/interrupt,
   and the approved failure seam. Verify exactly one cancellation, no mutation,
   terminal restoration, and no plaintext in the safe error line.

After recording results, inspect the exact disposable state path printed or
recorded above. Move only that directory to Trash using the desktop file
manager; keep it if further evidence or restart checks are needed. Do not run a
recursive removal command from this guide.

Do not report Linux, Windows, terminal-resize, corruption, or capacity evidence
unless that exact check actually ran. Automated tests require no credential,
network, provider, model, or live terminal dependency.
````

- [ ] **Step 6: Run documentation and all focused memory contracts GREEN**

```bash
cargo test --test documentation_contract
cargo test --test memory_actor_contract \
  --test memory_normalization_contract \
  --test memory_entry_contract \
  --test memory_review_contract \
  --test memory_proposal_contract \
  --test memory_episodic_contract \
  --test memory_retrieval_contract \
  --test memory_migration_contract \
  --test memory_persistence_contract \
  --test memory_integrity_contract \
  --test memory_event_contract \
  --test memory_projection_contract \
  --test memory_application_contract \
  --test memory_application_read_contract \
  --test memory_application_mutation_contract \
  --test memory_proposal_application_contract \
  --test memory_receipt_contract \
  --test memory_audit_contract \
  --test memory_recovery_contract \
  --test memory_atomicity_contract \
  --test memory_concurrency_contract \
  --test memory_isolation_contract \
  --test memory_editor_contract \
  --test memory_fallback_contract \
  --test memory_tui_controller_contract \
  --test memory_tui_host_contract \
  --test memory_tui_render_contract \
  --test hybrid_memory_acceptance
```

- [ ] **Step 7: Use `superpowers:requesting-code-review` and resolve every finding**

Request a spec-to-implementation review covering actor compatibility, schema/recovery integrity, plaintext disclosure, one-event atomicity, exact replay, UI route exclusions, and navigation regressions. For each finding, first reproduce it in the owning focused test, then make the smallest correction and rerun that test. Do not proceed with unresolved findings.

- [ ] **Step 8: Use `superpowers:verification-before-completion` and run the pre-completion gate**

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
git diff --check
git diff --no-index --check /dev/null docs/testing/phase-2-hybrid-memory.md || test $? -eq 1
```

Record the exact command outputs. Do not claim completion from an earlier run or a focused subset.

- [ ] **Step 9: Perform the manual isolated-state acceptance procedure**

Follow `docs/testing/phase-2-hybrid-memory.md` in a disposable state directory. Exercise fallback and TUI direct edits, tombstone/history, review cancellation, proposal resolution through the test fixture or approved internal harness, restart, namespace isolation, narrow `60x18` rendering, unchanged shortcuts, and explicit `/quit`. Confirm no credential, network, model, provider, or terminal dependency is required by automated tests.

- [ ] **Step 10: Mark only Milestone 3 and the Phase 2 exit gate complete**

Only after Steps 6–9 pass, update `phases.md` to mark Phase 2 Milestone 3 and the complete Phase 2 exit criteria done. Do not mark any Phase 3 work complete. Rerun:

```bash
cargo test --test documentation_contract --test hybrid_memory_acceptance
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
git diff --no-index --check /dev/null docs/testing/phase-2-hybrid-memory.md || test $? -eq 1
```

- [ ] **Step 11: Commit and verify the branch**

```bash
git add README.md architecture.md phases.md docs/testing/phase-2-hybrid-memory.md tests/documentation_contract.rs
git diff --cached --check
git commit -m "docs: complete phase 2 hybrid memory"
git status --short --branch
git log --oneline --decorate -22
```

Expected: the isolated worktree is clean, all Hybrid Memory commits are on `codex/phase-2-hybrid-memory`, the user's main checkout remains untouched, and Phase 3 remains deferred.
