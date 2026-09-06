# Phase 2 Agent Profile Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the first Phase 2 vertical slice: users can create, inspect, edit, version, and activate safe agent profiles from both the fallback terminal and the full-screen adaptive cockpit, with deterministic validation, immutable history, crash-safe persistence, and explicit edit confirmation.

> **Final design correction (2026-09-05):** The design specification is
> authoritative over older examples below. The completed implementation uses
> canonicalized persisted drafts with recursively strict serde, rejects tabs in
> every forbidden profile field, preserves empty descriptions, normalizes and
> deduplicates tags, and pins exact canonical bytes and SHA-256 digests for all
> five templates. Bindings are typed catalog references with `Unbound`,
> `BindingUnavailable`, and `Ready` states; they are not free-form provider or
> model labels. Read, create, preview-edit, and activate-revision are four
> separate deny-wins capabilities. Canonical commands use `/agent`, accept
> name-or-ID selectors and short template names, and support exact historical
> version inspection. Creation requires `create`; revision activation requires
> `activate <review-digest>`. Input and output bounds apply before event,
> outcome, and receipt metadata construction, and oversized or invalid UTF-8
> lines reach authoritative `RejectInput` handling. Recoverable errors retain
> editor and confirmation state. The TUI handles resize, exact restoration,
> unconditional `q` in Too Small, and stepwise `Esc`. Schema v2 independently
> constrains mirrored fields, pins the active digest, enforces globally unique
> per-profile memory namespaces across history, reports history divergence with
> a dedicated error, and is rollback-tested after every ordered v2 schema-change/object boundary.
> Migration 0002 may be amended while Phase 2 is prerelease: exact schema-v1
> upgrades remain supported, while databases created by intermediate Phase 2
> builds must be recreated.

**Architecture:** Add an agents domain with bounded validated values, pinned templates, canonical profile versions, deterministic digests, semantic diffs, and an active-profile projection. Accepted mutations flow through the existing application command, event, reducer, receipt, SQLite, and audit boundaries. Edit preview remains process-local and non-durable; activation carries the candidate again and proves it matches a one-use review token. SQLite stores an append-only mirror of immutable profile-version events plus a rebuildable active pointer.

**Tech Stack:** Rust 1.98, serde/serde_json, sha2, uuid, rusqlite, unicode-normalization 0.1.24, unicode-casefold 0.2.0, ratatui, crossterm, proptest, existing application/event/persistence/recovery infrastructure.

**Spec:** [Phase 2 Agent Profile Foundation Design](../specs/2026-09-05-phase-2-agent-profile-foundation-design.md)

## Global Constraints

- Preserve every Phase 0 and Phase 1 behavior and keep the existing numbered cockpit navigation keys 1 through 4.
- Use test-driven development for every behavior change: write the named failing test, run only that focused test, implement the minimum behavior, then rerun it.
- Use immutable AgentProfileVersion values. Never update or delete an accepted version.
- Treat agent_profile_versions as an immutable event mirror. Backfill missing expected rows only; fail closed on mismatched or unexpected rows.
- Treat active_agent_profiles as a disposable projection that recovery may rebuild from verified events.
- Keep unaccepted draft prose and edit candidates out of events, command receipts, audit summaries, logs, and SQLite.
- Never derive authorization from role, personality, specialty, tags, instructions, template choice, or provider bindings.
- Keep the global event schema version at 1. Add new tagged event kinds without making existing Phase 0 events unreadable.
- Validate all limits in UTF-8 bytes and reject control characters, bidi overrides, NUL, unsafe line separators, and terminal escape bytes before persistence or rendering.
- Use compatibility normalization, full non-Turkic Unicode case folding, and whitespace folding for the active display-name uniqueness key.
- Keep template provenance immutable and informational. A later built-in template revision must not mutate an existing profile.
- Keep every accepted mutation atomic with its event, projection update, immutable mirror row, active pointer, audit record, and command receipt.
- Keep the presentation editor local to the current process. It must not introduce a durable draft entity.
- Do not add skills, memory retrieval, model execution, rooms, debates, market data, or provider calls in this milestone.
- Commit after every task using the exact commit message shown.
- Before claiming completion, use superpowers:requesting-code-review and superpowers:verification-before-completion.

---

## Task 1: Add Agent Profile Vocabulary, Unicode Identity, and Pinned Templates

**Files:**
- Modify: Cargo.toml
- Modify: Cargo.lock
- Modify: src/agents/mod.rs
- Create: src/agents/normalization.rs
- Create: src/agents/template.rs
- Create: src/agents/profile.rs
- Modify: src/domain/id.rs
- Modify: src/domain/error.rs
- Create: tests/agent_profile_domain_contract.rs

- [ ] **Step 1: Write failing identity and template contract tests**

Create tests/agent_profile_domain_contract.rs with focused cases for:

    use ai_stock_forum::agents::{
        builtin_profile_templates, normalize_profile_name_key, AgentProfileDraft,
        AgentRole, ProfileTemplateId,
    };

    #[test]
    fn active_name_key_uses_nfkc_casefold_and_whitespace_fold() {
        assert_eq!(
            normalize_profile_name_key("  Maße\tDesk  ").unwrap(),
            normalize_profile_name_key("MASSE desk").unwrap(),
        );
        assert_eq!(
            normalize_profile_name_key("Kelvin").unwrap(),
            normalize_profile_name_key("kelvin").unwrap(),
        );
        assert_eq!(
            normalize_profile_name_key("Ａｌｐｈａ").unwrap(),
            normalize_profile_name_key("alpha").unwrap(),
        );
        assert_eq!(
            normalize_profile_name_key("Cafe\u{301}").unwrap(),
            normalize_profile_name_key("Café").unwrap(),
        );
    }

    #[test]
    fn builtin_templates_are_pinned_and_complete() {
        let templates = builtin_profile_templates();
        assert_eq!(templates.len(), 5);
        assert_eq!(
            templates.iter().map(|item| item.role).collect::<Vec<_>>(),
            vec![
                AgentRole::Bull,
                AgentRole::Bear,
                AgentRole::Chief,
                AgentRole::Engineering,
                AgentRole::Custom,
            ],
        );
        assert!(templates.iter().all(|item| item.version.get() == 1));
        assert!(templates.iter().all(|item| !item.digest.as_str().is_empty()));
    }

Add boundary tests for 64-byte display names, 256-byte descriptions, 64-byte primary specialties, five 48-byte tags, 1,024-byte personality, 4,096-byte instructions, empty optional bindings, duplicate folded tags, whitespace-only values, ANSI escape input, NUL, C0/C1 controls, Unicode bidi overrides, U+2028, and U+2029.

- [ ] **Step 2: Run the focused domain contract and confirm RED**

Run:

    cargo test --test agent_profile_domain_contract

Expected: compilation fails because the agents types and normalization API do not exist.

- [ ] **Step 3: Add exact domain identifiers and validation errors**

Extend src/domain/id.rs through the existing uuid_id macro with:

    AgentProfileId
    AgentProfileVersionId
    MemoryNamespaceId
    ProfileReviewToken

Add stable DomainError cases for invalid profile fields, unsafe visible text, too many tags, duplicate tags, unknown template, and invalid template provenance. Each error must expose a safe static code and must not embed rejected prose.

- [ ] **Step 4: Add Unicode dependencies**

Add:

    unicode-normalization = "0.1.24"
    unicode-casefold = "0.2.0"

Regenerate Cargo.lock as part of the first focused Cargo invocation after implementation.

- [ ] **Step 5: Implement normalization and bounded profile values**

In src/agents/normalization.rs expose:

    pub fn validate_visible_text(field: ProfileField, value: &str, max_bytes: usize)
        -> Result<(), DomainError>;

    pub fn normalize_profile_name_key(value: &str)
        -> Result<NormalizedProfileName, DomainError>;

    pub fn normalize_tag_key(value: &str)
        -> Result<String, DomainError>;

The name pipeline is:

1. Validate non-empty visible text and reject unsafe code points.
2. Apply NFKC compatibility normalization.
3. Apply full non-Turkic Unicode case folding.
4. Convert every Unicode whitespace run to one ASCII space.
5. Trim leading and trailing folded spaces.
6. Reject an empty result.

In src/agents/profile.rs add serde-enabled types:

    pub enum AgentRole {
        Bull,
        Bear,
        Chief,
        Engineering,
        Custom,
    }

    pub struct AgentProfileDraft {
        pub display_name: String,
        pub description: String,
        pub role: AgentRole,
        pub primary_specialty: String,
        pub specialty_tags: Vec<String>,
        pub personality: String,
        pub instructions: String,
        pub bindings: AgentBindings,
        pub skill_refs: Vec<SkillRef>,
        pub mcp_refs: Vec<McpRef>,
    }

`AgentBindings` contains typed optional inference and engineering references
selected from an injected `BindingCatalog`; it contains no free-form provider,
model, or runtime strings. Add a validating constructor that applies every byte
limit cumulatively, canonicalizes accepted values, rejects folded duplicate
tags, recursively rejects unknown serialized fields, requires empty skill_refs
and mcp_refs for this milestone, and allows bindings to be absent. Readiness is
computed as `Unbound`, `BindingUnavailable`, or `Ready` against role-specific
requirements and catalog availability. Production uses an empty catalog.

- [ ] **Step 6: Implement five immutable built-in templates**

In src/agents/template.rs add:

    pub struct ProfileTemplateId(String);
    pub struct ProfileTemplateVersion(u32);

    pub struct ProfileTemplateProvenance {
        pub template_id: ProfileTemplateId,
        pub template_version: ProfileTemplateVersion,
        pub template_digest: Digest,
    }

    pub struct ProfileTemplate {
        pub id: ProfileTemplateId,
        pub version: ProfileTemplateVersion,
        pub digest: Digest,
        pub role: AgentRole,
        pub suggested_name: &'static str,
        pub description: &'static str,
        pub primary_specialty: &'static str,
        pub specialty_tags: &'static [&'static str],
        pub personality: &'static str,
        pub instructions: &'static str,
    }

Pin version 1 templates with these IDs and intent:

| ID | Suggested name | Role | Specialty | Tags |
| --- | --- | --- | --- | --- |
| builtin.bull | Bull Researcher | Bull | upside research | growth, catalysts |
| builtin.bear | Bear Researcher | Bear | downside research | risk, valuation |
| builtin.chief | Chief Moderator | Chief | evidence synthesis | arbitration, decisions |
| builtin.engineering | Research Engineer | Engineering | research systems | tooling, data-quality |
| builtin.custom | Custom Analyst | Custom | general research | custom |

Compute each template digest from an explicit canonical serializable template payload. copy_to_draft() must copy strings into a new editable draft. Never retain a live reference that could alter an accepted profile.

- [ ] **Step 7: Export the agents domain and rerun GREEN**

Export all public Phase 2 types from src/agents/mod.rs.

Run:

    cargo test --test agent_profile_domain_contract

Expected: all identity, validation, readiness, and pinned-template tests pass.

- [ ] **Step 8: Commit**

    git add Cargo.toml Cargo.lock src/agents src/domain/id.rs src/domain/error.rs tests/agent_profile_domain_contract.rs
    git commit -m "feat: add validated agent profile vocabulary"

---

## Task 2: Build Canonical Versions, Digests, Semantic Diffs, and Domain Properties

**Files:**
- Modify: src/agents/profile.rs
- Create: src/agents/diff.rs
- Modify: src/agents/mod.rs
- Create: tests/agent_profile_version_contract.rs

- [ ] **Step 1: Write failing version and diff tests**

Cover:

    #[test]
    fn canonical_profile_digest_is_independent_of_json_map_order() { ... }

    #[test]
    fn version_one_has_no_predecessor_and_edit_increments_once() { ... }

    #[test]
    fn semantic_diff_lists_only_changed_fields_in_fixed_order() { ... }

    #[test]
    fn unchanged_candidate_is_rejected() { ... }

    proptest! {
        #[test]
        fn accepted_profile_round_trips_without_digest_drift(draft in valid_profile_drafts()) {
            ...
        }
    }

Assert this fixed diff order:

    DisplayName
    Description
    Role
    PrimarySpecialty
    SpecialtyTags
    Personality
    Instructions
    Bindings

Assert diff values are safe structured old/new values for confirmation UI, but are not formatted into audit text.

- [ ] **Step 2: Run the focused contract and confirm RED**

    cargo test --test agent_profile_version_contract

Expected: missing AgentProfileVersion and ProfileFieldDiff APIs.

- [ ] **Step 3: Implement immutable profile versions**

Add:

    pub struct AgentProfileVersion {
        pub profile_id: AgentProfileId,
        pub profile_version_id: AgentProfileVersionId,
        pub version: ObjectVersion,
        pub content_digest: Digest,
        pub display_name: String,
        pub normalized_name: NormalizedProfileName,
        pub description: String,
        pub role: AgentRole,
        pub primary_specialty: String,
        pub specialty_tags: Vec<String>,
        pub personality: String,
        pub instructions: String,
        pub bindings: AgentBindings,
        pub skill_refs: Vec<SkillRef>,
        pub mcp_refs: Vec<McpRef>,
        pub memory_namespace_id: MemoryNamespaceId,
        pub default_policy_ref: String,
        pub template_provenance: Option<ProfileTemplateProvenance>,
        pub created_at_ms: i64,
        pub supersedes: Option<AgentProfileVersionId>,
    }

Expose constructors:

    pub fn create(
        profile_id: AgentProfileId,
        profile_version_id: AgentProfileVersionId,
        memory_namespace_id: MemoryNamespaceId,
        created_at_ms: i64,
        draft: AgentProfileDraft,
        provenance: Option<ProfileTemplateProvenance>,
    ) -> Result<Self, DomainError>;

    pub fn next_version(
        current: &AgentProfileVersion,
        new_version_id: AgentProfileVersionId,
        created_at_ms: i64,
        draft: AgentProfileDraft,
    ) -> Result<Self, DomainError>;

Creation sets version 1, no supersedes value, and the fixed policy reference profile-default/v1. Editing preserves profile ID, memory namespace, policy reference, and template provenance; increments exactly once; and points supersedes to the current version ID.

- [ ] **Step 4: Define the canonical digest payload**

Hash canonical JSON for all semantic and lineage fields except content_digest itself. Include the explicit string role representation, normalized name, ordered tags, bindings, empty skill/MCP arrays, profile ID, version ID, object version, memory namespace, policy reference, provenance, timestamp, and predecessor. Do not digest serde debug output or a HashMap.

- [ ] **Step 5: Implement semantic diffing**

In src/agents/diff.rs add:

    pub enum ProfileField { ... }

    pub struct ProfileFieldDiff {
        pub field: ProfileField,
        pub before: ProfileFieldValue,
        pub after: ProfileFieldValue,
    }

    pub fn diff_profile(
        current: &AgentProfileVersion,
        candidate: &AgentProfileDraft,
    ) -> Result<Vec<ProfileFieldDiff>, DomainError>;

Use typed ProfileFieldValue variants rather than concatenated display strings. A zero-field diff returns DomainError::AgentProfileUnchanged.

- [ ] **Step 6: Rerun GREEN**

    cargo test --test agent_profile_version_contract

Expected: examples and property tests pass with deterministic digests and ordering.

- [ ] **Step 7: Commit**

    git add src/agents tests/agent_profile_version_contract.rs
    git commit -m "feat: add immutable agent profile versions"

---

## Task 3: Add Profile Events and Deterministic Projection Reduction

**Files:**
- Modify: src/app/event.rs
- Create: src/agents/projection.rs
- Modify: src/agents/mod.rs
- Modify: src/recovery/reducer.rs
- Create: tests/agent_profile_event_contract.rs

- [ ] **Step 1: Write failing event compatibility tests**

Test canonical encode/decode for:

    ApplicationEvent::AgentProfileCreated {
        profile: profile_v1,
    }

    ApplicationEvent::AgentProfileVersionActivated {
        profile: profile_v2,
        previous_version_id: profile_v1.profile_version_id,
    }

Also assert:

- Existing Phase 0 event fixtures still decode with schema version 1.
- Unknown event kinds fail safely.
- A profile event with an altered content digest fails reduction.
- Duplicate active normalized names fail reduction.
- Version 2 without a matching active version 1 fails reduction.
- A valid create then edit yields one active version and two historical versions.

- [ ] **Step 2: Run the event contract and confirm RED**

    cargo test --test agent_profile_event_contract

Expected: new event variants and profile projection do not exist.

- [ ] **Step 3: Extend the schema-v1 tagged event union**

Add the two profile mutation variants to ApplicationEvent. Keep EVENT_SCHEMA_VERSION equal to 1. Serialize with explicit stable event-kind tags and canonical payloads; do not rely on Rust variant debug names.

- [ ] **Step 4: Implement AgentProfilesProjection**

Add:

    pub struct AgentProfilesProjection {
        versions_by_id: BTreeMap<AgentProfileVersionId, AgentProfileVersion>,
        active_by_profile: BTreeMap<AgentProfileId, AgentProfileVersionId>,
        active_name_index: BTreeMap<NormalizedProfileName, AgentProfileId>,
    }

Expose deterministic read methods for active list sorted by folded name then profile ID, active lookup, history sorted by version, and exact version lookup.

The reducer must verify:

- Recomputed content_digest equals the event value.
- Create is version 1 with no predecessor and unused IDs.
- Activation advances exactly one version from the currently active version.
- previous_version_id and supersedes both equal the current active version.
- Profile identity, memory namespace, policy reference, and provenance remain unchanged.
- No other active profile owns the normalized name.
- Replaying the same event sequence through recovery produces byte-equivalent state.

- [ ] **Step 5: Integrate profile reduction into recovery**

Extend the existing aggregate projection/reducer rather than creating a second event scan. Profile events update AgentProfilesProjection; existing events leave it unchanged.

- [ ] **Step 6: Rerun GREEN**

    cargo test --test agent_profile_event_contract

Expected: new and legacy event compatibility tests pass.

- [ ] **Step 7: Commit**

    git add src/app/event.rs src/agents src/recovery/reducer.rs tests/agent_profile_event_contract.rs
    git commit -m "feat: reduce agent profile events"

---

## Task 4: Add SQLite Schema v2 with an Immutable Version Mirror

**Files:**
- Create: migrations/0002_agent_profiles.sql
- Modify: src/persistence/migrations.rs
- Create: tests/agent_profile_migration_contract.rs

- [ ] **Step 1: Write failing migration tests**

Assert:

- A fresh database reaches schema version 2.
- A schema-v1 fixture upgrades without changing existing event, receipt, audit, installation, setup, or session rows.
- Fault injection after every ordered v2 schema-change/object boundary, including both receipt
  rebuild copies and every table, index, trigger, and migration-version record creation, preserves
  all schema-v1 rows and the exact schema inventory with no partial v2 object or version record.
- Foreign keys are effective.
- agent_profile_versions rejects UPDATE and DELETE.
- active_agent_profiles allows transactional replacement during recovery.
- normalized_name has a unique binary index.
- Running startup twice is idempotent.

- [ ] **Step 2: Run the migration contract and confirm RED**

    cargo test --test agent_profile_migration_contract

Expected: schema remains version 1 and profile tables are absent.

- [ ] **Step 3: Add migration 0002_agent_profiles.sql**

Create STRICT tables with explicit constraints:

Create STRICT immutable-version and active-pointer tables. The immutable table
stores independently constrained mirror columns for every query-critical field
as well as canonical payload bytes; persistence compares both representations.
The active row pins profile ID, exact version ID and number, normalized name, and
content digest through a composite deferred foreign key. Enforce active-name
uniqueness and prevent one memory namespace from being used by two stable
profiles across historical versions. Add immutable UPDATE/DELETE triggers only
to `agent_profile_versions`.

- [ ] **Step 4: Register ordered migration version 2**

Set LATEST_SCHEMA_VERSION to 2 and register 0001 followed by 0002. Preserve the current all-or-nothing migration transaction and required pragma checks. Route test-only post-boundary fault injection through that real transaction runner rather than duplicating migration SQL or execution logic.

- [ ] **Step 5: Rerun GREEN**

    cargo test --test agent_profile_migration_contract

Expected: fresh install, schema-v1 upgrade, boundary-by-boundary rollback, immutability, and idempotency tests pass.

- [ ] **Step 6: Commit**

    git add migrations/0002_agent_profiles.sql src/persistence/migrations.rs tests/agent_profile_migration_contract.rs
    git commit -m "feat: add immutable agent profile storage"

---

## Task 5: Materialize Versions and Reconcile Recovery Without Rewriting History

**Files:**
- Create: src/persistence/agent_profile_repository.rs
- Modify: src/persistence/mod.rs
- Modify: src/persistence/projection_repository.rs
- Modify: src/recovery/coordinator.rs
- Modify: src/recovery/reducer.rs
- Modify: src/app/mod.rs
- Create: tests/agent_profile_persistence_contract.rs
- Create: tests/agent_profile_recovery_contract.rs

- [ ] **Step 1: Write failing repository and recovery tests**

Repository tests must cover append, duplicate exact insert idempotency, duplicate mismatch rejection, history ordering, active pointer replacement, and transaction rollback.

Recovery tests must cover:

- Missing immutable mirror row is backfilled from a verified event.
- Existing row with altered payload, digest, event sequence, or identity refuses startup.
- Unexpected extra immutable row refuses startup.
- Missing, stale, or corrupt active pointer is rebuilt from verified events.
- Active rebuild does not update or delete immutable rows.
- Recovery interruption rolls back active-pointer replacement and any missing-row backfill.
- Legacy databases with no profile events remain valid and empty.

- [ ] **Step 2: Run focused contracts and confirm RED**

    cargo test --test agent_profile_persistence_contract
    cargo test --test agent_profile_recovery_contract

Expected: repository APIs and profile reconciliation are missing.

- [ ] **Step 3: Implement immutable mirror repository**

Expose transaction-scoped functions:

    pub fn insert_expected_version(
        tx: &rusqlite::Transaction<'_>,
        event_sequence: i64,
        profile: &AgentProfileVersion,
    ) -> Result<(), PersistenceError>;

    pub fn load_all_versions(
        conn: &rusqlite::Connection,
    ) -> Result<Vec<StoredAgentProfileVersion>, PersistenceError>;

    pub fn replace_active_profiles(
        tx: &rusqlite::Transaction<'_>,
        projection: &AgentProfilesProjection,
    ) -> Result<(), PersistenceError>;

insert_expected_version must compare every stored column and payload byte when the logical key already exists. Exact equality is accepted; any mismatch returns database_agent_profile_history_mismatch.

- [ ] **Step 4: Add fail-closed recovery reconciliation**

From the verified event stream, derive an ordered expected immutable-row set.

Reconcile with these exact rules:

1. Expected row absent from SQLite: insert it.
2. Expected row present and byte-equivalent: keep it.
3. Expected row present but different: return RecoveryError::AgentProfileHistoryMismatch.
4. SQLite row has no corresponding verified event: return RecoveryError::UnexpectedAgentProfileHistory.
5. Only after immutable reconciliation succeeds, replace active_agent_profiles in the same transaction from the reduced projection.

Never clear agent_profile_versions from projection_repository. Add only active_agent_profiles to the rebuildable clear-and-write path.

- [ ] **Step 5: Add safe public error mapping**

Expose stable safe codes for mirror mismatch, unexpected history, invalid profile payload, and active projection rebuild failure. Do not include profile prose or raw payload bytes in Display output.

- [ ] **Step 6: Rerun GREEN**

    cargo test --test agent_profile_persistence_contract
    cargo test --test agent_profile_recovery_contract

Expected: all append-only and recovery rules pass.

- [ ] **Step 7: Commit**

    git add src/persistence src/recovery src/app/mod.rs tests/agent_profile_persistence_contract.rs tests/agent_profile_recovery_contract.rs
    git commit -m "feat: reconcile durable agent profile history"

---

## Task 6: Add Commands, Reads, Capabilities, Outcomes, and One-Use Edit Review

**Files:**
- Modify: src/app/command.rs
- Modify: src/app/outcome.rs
- Modify: src/app/service.rs
- Modify: src/app/event.rs
- Modify: src/app/mod.rs
- Modify: src/policy/capability.rs
- Create: src/agents/review.rs
- Modify: src/agents/mod.rs
- Modify: tests/support/mod.rs
- Create: tests/agent_profile_application_contract.rs

- [ ] **Step 1: Write failing application behavior tests**

Cover:

- Create from a fully edited template copy activates version 1.
- Create with unbound typed references succeeds and returns `Unbound` / `Not
  Ready`; injected catalogs cover unavailable and role-specific ready states.
- Case-folded active-name collision is rejected.
- List, show, and history return deterministic structured views.
- Preview returns ordered field diffs, a token, and a review digest without writing an event, receipt, audit row, profile row, or command history entry.
- A newer preview invalidates the older token.
- Cancel and shutdown invalidate the token.
- Activation rejects a changed candidate, changed base, changed digest, wrong profile, and reused token.
- Successful activation creates version 2 and preserves version 1.
- Role and profile prose do not add capabilities.
- Denied create/edit/read operations use the existing policy boundary.
- User-supplied profile prose never appears in generic audit summary text.

- [ ] **Step 2: Run the application contract and confirm RED**

    cargo test --test agent_profile_application_contract

Expected: commands, views, capabilities, and review APIs are missing.

- [ ] **Step 3: Add explicit capabilities**

Add four distinct capabilities: profile read, profile create, profile edit
preview, and profile revision activation. Map every operation to exactly one
required capability. Do not infer any capability from AgentRole or content.

- [ ] **Step 4: Add mutation and read commands**

Extend ApplicationCommand with:

    CreateAgentProfile {
        draft: AgentProfileDraft,
        template_provenance: Option<ProfileTemplateProvenance>,
    }

    ActivateAgentProfileVersion {
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate: AgentProfileDraft,
        review_token: ProfileReviewToken,
        review_digest: Digest,
    }

    ListAgentProfiles
    ShowAgentProfile { selector: AgentProfileSelector }
    ShowAgentProfileHistory { selector: AgentProfileSelector }
    ShowAgentProfileVersion {
        selector: AgentProfileSelector,
        version: ObjectVersion,
    }

Accepted create data may be persisted in its command receipt because it is no longer a draft. Edit preview must not be an ApplicationCommand and must not enter receipt storage.

Add CommandView variants with typed profile summaries, a detailed profile, immutable history metadata, created confirmation, and activated confirmation.

- [ ] **Step 5: Implement a bounded process-local review registry**

In src/agents/review.rs define:

    pub struct ProfileEditPreview {
        pub profile_id: AgentProfileId,
        pub expected_active_version_id: AgentProfileVersionId,
        pub diffs: Vec<ProfileFieldDiff>,
        pub review_token: ProfileReviewToken,
        pub review_digest: Digest,
    }

    enum ReviewState {
        Available(PendingProfileReview),
        Reserved {
            command_id: CommandId,
            review: PendingProfileReview,
        },
    }

    struct PendingProfileReview {
        token: ProfileReviewToken,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate_digest: Digest,
        review_digest: Digest,
    }

The registry holds at most one review. It stores no candidate strings and no diff values. review_digest is the canonical hash of profile ID, base version ID, candidate digest, and typed ordered diff metadata.

- [ ] **Step 6: Add passive preview to ApplicationService**

Expose:

    pub fn preview_agent_profile_edit(
        &self,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate: AgentProfileDraft,
    ) -> Result<ProfileEditPreview, AppError>;

    pub fn cancel_agent_profile_edit(&self);

Preview reads the active profile, validates the expected base and candidate, computes the diff and digests, invalidates any earlier review, and issues a fresh token from IdGenerator. It performs no database write.

- [ ] **Step 7: Integrate token reservation with command receipts and transactions**

For ActivateAgentProfileVersion:

1. Start the transaction and check for an exact command receipt replay first.
2. Recompute candidate and review digests from the command payload.
3. Atomically reserve the matching Available token for this command ID.
4. Recheck active version and normalized-name uniqueness inside the transaction.
5. Append the activation event, reduce, mirror, update active pointer, audit, and receipt.
6. Commit.
7. Mark the token consumed by removing it.
8. On pre-commit failure, release the reservation only for the same command ID.
9. A competing command cannot reserve the token.
10. An exact committed command replay returns its receipt without consulting the consumed token.

- [ ] **Step 8: Materialize profile outcomes and safe read events**

Create and activation events carry the full canonical AgentProfileVersion. Read events, if the existing service audits reads through events, carry only profile ID, result count, and active version IDs. Generic audit summaries contain event kind, IDs, version numbers, readiness, and result counts only.

- [ ] **Step 9: Rerun GREEN**

    cargo test --test agent_profile_application_contract

Expected: all create, read, preview, activation, policy, and non-leakage tests pass.

- [ ] **Step 10: Commit**

    git add src/app src/agents src/policy tests/support/mod.rs tests/agent_profile_application_contract.rs
    git commit -m "feat: add agent profile application workflow"

---

## Task 7: Prove Idempotency, Concurrency, Atomicity, and Input Hardening

**Files:**
- Modify: src/app/service.rs
- Modify: src/persistence/agent_profile_repository.rs
- Create: tests/agent_profile_concurrency_contract.rs
- Create: tests/agent_profile_atomicity_contract.rs
- Create: tests/agent_profile_hardening_contract.rs

- [ ] **Step 1: Write failing concurrency tests**

Use barriers and two independent database connections to assert:

- Two creates with names that fold to the same key produce one winner and one deterministic conflict.
- Two edits from the same base produce one version 2 winner and one stale-base rejection.
- Two activations using one review token produce one winner.
- Replaying the winning command ID and exact payload returns the same outcome.
- Reusing a command ID with a changed candidate returns command conflict.
- A failed contender leaves no event, profile row, active pointer, audit row, or receipt.

- [ ] **Step 2: Write failing rollback and hardening tests**

Inject failure after each write boundary already supported by the test hook:

    after_event_append
    after_profile_mirror_insert
    after_active_pointer_update
    after_projection_store
    after_audit_append
    after_receipt_store
    before_commit

For each injection, assert complete rollback and successful retry with the same review token after reservation release.

Add hostile input cases for ESC, OSC, CSI, newline injection, carriage return, tabs where forbidden, NUL, bidi overrides, over-limit multibyte strings, invalid UUIDs, unknown roles, malformed canonical JSON, digest tampering, and oversized fallback lines.

- [ ] **Step 3: Run focused contracts and confirm RED**

    cargo test --test agent_profile_concurrency_contract
    cargo test --test agent_profile_atomicity_contract
    cargo test --test agent_profile_hardening_contract

Expected: at least one race, rollback hook, or input guard is not yet implemented.

- [ ] **Step 4: Tighten transaction and reservation ordering**

Keep normalized-name uniqueness enforced in both reducer logic and SQLite. Map the SQLite unique failure to the same safe active_name_conflict code as the precheck. Ensure token reservation release is scoped to the command ID that acquired it and cannot resurrect an invalidated review.

- [ ] **Step 5: Complete safe parsing and output boundaries**

Reject unsafe input before constructing accepted domain values. Render profile fields only through terminal-safe text helpers. Error Display implementations expose fixed context and identifiers only.

- [ ] **Step 6: Rerun GREEN**

    cargo test --test agent_profile_concurrency_contract
    cargo test --test agent_profile_atomicity_contract
    cargo test --test agent_profile_hardening_contract

Expected: deterministic winner, rollback, retry, and hostile-input tests pass.

- [ ] **Step 7: Commit**

    git add src/app/service.rs src/persistence/agent_profile_repository.rs tests/agent_profile_concurrency_contract.rs tests/agent_profile_atomicity_contract.rs tests/agent_profile_hardening_contract.rs
    git commit -m "test: harden agent profile transactions"

---

## Task 8: Build the Shared Presentation-Local Guided Profile Editor

**Files:**
- Create: src/ui/profile_editor.rs
- Modify: src/ui/mod.rs
- Create: tests/agent_profile_editor_contract.rs

- [ ] **Step 1: Write failing editor state-machine tests**

Cover exact steps:

    Template
    Identity
    Specialty
    Personality
    Instructions
    OptionalBindings
    Review

Cover create and edit mode, forward/back navigation, template copying, multiline personality/instructions, tag add/remove with five-tag cap, provider/model clearing, cancel, review, activation confirmation, validation errors that retain input, terminal resize independence, and zero persistence effects.

Define colon controls:

    :next
    :back
    :show
    :clear
    :tag add <value>
    :tag remove <value>
    :provider <value>
    :model <value>
    :review
    :activate
    :cancel

Normal non-colon lines edit the current text field. In multiline fields, :next ends the field. Unknown colon commands return a local safe editor message.

- [ ] **Step 2: Run the editor contract and confirm RED**

    cargo test --test agent_profile_editor_contract

Expected: profile editor module is missing.

- [ ] **Step 3: Implement a UI-agnostic editor state machine**

Add:

    pub enum ProfileEditorMode {
        Create { provenance: ProfileTemplateProvenance },
        Edit {
            profile_id: AgentProfileId,
            expected_active_version_id: AgentProfileVersionId,
        },
    }

    pub enum ProfileEditorStep { ... }

    pub enum ProfileEditorEffect {
        None,
        PreviewEdit(PreviewEditRequest),
        Execute(ApplicationCommand),
        Cancelled,
    }

    pub struct ProfileEditor {
        mode: ProfileEditorMode,
        step: ProfileEditorStep,
        draft: AgentProfileDraft,
        review: Option<ProfileEditorReview>,
        local_message: Option<SafeUiMessage>,
    }

The editor returns typed effects and never calls persistence. Editing any field after review clears the review token and requires a new preview.

- [ ] **Step 4: Keep prose out of command history**

Expose only safe editor-control summaries to hosts. Raw field lines stay in the editor buffer and are never added to fallback history, TUI command history, tracing, or generic audit data.

- [ ] **Step 5: Rerun GREEN**

    cargo test --test agent_profile_editor_contract

Expected: all state transitions and local-only guarantees pass.

- [ ] **Step 6: Commit**

    git add src/ui/profile_editor.rs src/ui/mod.rs tests/agent_profile_editor_contract.rs
    git commit -m "feat: add guided agent profile editor"

---

## Task 9: Add Fallback Terminal Profile Workflows

**Files:**
- Modify: src/ui/fallback/parser.rs
- Modify: src/ui/fallback/renderer.rs
- Modify: src/ui/fallback/runner.rs
- Modify: src/ui/fallback/mod.rs
- Create: tests/agent_profile_fallback_contract.rs

- [ ] **Step 1: Write failing fallback transcript tests**

Cover:

    /agent list
    /agent show <name-or-id>
    /agent history <name-or-id> [version]
    /agent create
    /agent create <bull|bear|chief|engineering|custom>
    /agent edit <name-or-id>

Assert create defaults to a template selection screen, edit loads the active
version, guided field lines are not command-history entries, review displays a
field-by-field diff, `:create` requires exact `create`, `:activate` requires
exact `activate <review-digest>`, `Esc` returns one step, recoverable errors
retain the exact workflow, and EOF/cancel leaves no durable draft.

- [ ] **Step 2: Run the fallback contract and confirm RED**

    cargo test --test agent_profile_fallback_contract

Expected: parser rejects agent commands and runner has no editor mode.

- [ ] **Step 3: Extend command parsing without weakening limits**

Parse only the canonical forms above, while retaining the equivalent bare
`agent` aliases in fallback mode. Resolve quoted normalized names or typed IDs.
Keep the maximum cumulative input length and provide numeric limit guidance.
When an editor is active, route input to ProfileEditor before the normal command
parser.

- [ ] **Step 4: Render deterministic safe profile output**

List output columns:

    NAME | ROLE | SPECIALTY | READY | VERSION | ID

Show output includes accepted fields, provenance, bindings readiness, memory namespace ID, policy reference, created time, and digest. History shows version, version ID, predecessor, created time, readiness, and digest. Do not render raw JSON.

- [ ] **Step 5: Wire create and edit flows**

Create flow copies a selected template, allows every copied field to be changed, and submits CreateAgentProfile only after final confirmation. Edit flow requests passive preview, stores the returned review metadata in the editor, and submits ActivateAgentProfileVersion only after explicit confirmation.

- [ ] **Step 6: Rerun GREEN**

    cargo test --test agent_profile_fallback_contract

Expected: parser, transcripts, cancellation, and non-leakage tests pass.

- [ ] **Step 7: Commit**

    git add src/ui/fallback tests/agent_profile_fallback_contract.rs
    git commit -m "feat: add fallback agent profile workflow"

---

## Task 10: Add Adaptive Cockpit Agent Navigation and Controller State

**Files:**
- Modify: src/ui/tui/model.rs
- Modify: src/ui/tui/controller.rs
- Modify: src/ui/tui/mod.rs
- Create: tests/agent_profile_tui_controller_contract.rs

- [ ] **Step 1: Write failing controller tests**

Assert:

- Pressing a outside command entry opens Agents.
- Pressing a while command entry is focused inserts the character a.
- Keys 1, 2, 3, and 4 still open Overview, Setup, Audit, and Help.
- Agents supports list focus, detail focus, editor focus, selection, scrolling, create, edit, history, back, cancel, review, and confirmation.
- Escape unwinds one local layer before exiting the Agents view.
- q quits only when no text editor or confirmation is active.
- Narrow, medium, and wide terminal sizes preserve state while changing layout.
- Resize during an edit loses no draft data.

- [ ] **Step 2: Run the controller contract and confirm RED**

    cargo test --test agent_profile_tui_controller_contract

Expected: View::Agents and profile-editor controller effects are missing.

- [ ] **Step 3: Extend TUI model**

Add View::Agents and:

    pub enum AgentsPane {
        List,
        Detail,
        History,
        Editor,
        Confirmation,
    }

    pub struct AgentsViewState {
        pub selected_profile: usize,
        pub selected_template: usize,
        pub pane: AgentsPane,
        pub list_scroll: usize,
        pub detail_scroll: usize,
        pub history_scroll: usize,
        pub editor: Option<ProfileEditor>,
        pub pending_confirmation: Option<ProfileConfirmation>,
    }

Keep agents state in TuiModel so it survives render and resize cycles.

- [ ] **Step 4: Extend controller effects**

Add typed effects for loading list/detail/history, starting create/edit, requesting preview, executing create/activation, and cancelling review. The controller must not call the application service directly.

- [ ] **Step 5: Preserve input precedence**

Use this order:

1. Active confirmation.
2. Active profile editor.
3. Existing command entry.
4. View-local Agents navigation.
5. Global navigation and quit.

This guarantees a types into command/editor text when text input owns focus.

- [ ] **Step 6: Rerun GREEN**

    cargo test --test agent_profile_tui_controller_contract

Expected: navigation, precedence, and resize-state tests pass.

- [ ] **Step 7: Commit**

    git add src/ui/tui tests/agent_profile_tui_controller_contract.rs
    git commit -m "feat: add cockpit agent profile state"

---

## Task 11: Render the Agents Cockpit and Connect It to Application Snapshots

**Files:**
- Create: src/ui/tui/views/agents.rs
- Modify: src/ui/tui/views/mod.rs
- Modify: src/ui/tui/render.rs
- Modify: src/ui/tui/layout.rs
- Modify: src/ui/tui/host.rs
- Modify: src/app/service.rs
- Create: tests/agent_profile_tui_render_contract.rs
- Create: tests/agent_profile_tui_host_contract.rs

- [ ] **Step 1: Write failing render and host tests**

Render tests cover:

- Narrow layout under 80 columns: one pane with list/detail/editor switching.
- Medium layout from 80 through 119 columns: list plus active detail/editor.
- Wide layout at 120 or more columns: list, detail/editor, and readiness/history inspector.
- Empty state explains how to create the first profile.
- Not Ready is visually distinct without implying failure.
- Active version, role, specialty, tags, provenance, digest, and diff are visible.
- Long safe content wraps and scrolls without layout panic.
- Tiny terminals show the existing minimum-size fallback.
- No raw escape sequence reaches the buffer.

Host tests cover loading snapshots, create, preview, activation, stale conflict refresh, cancellation, and clean shutdown.

- [ ] **Step 2: Run focused tests and confirm RED**

    cargo test --test agent_profile_tui_render_contract
    cargo test --test agent_profile_tui_host_contract

Expected: Agents view renderer and host effects are missing.

- [ ] **Step 3: Extend presentation snapshot**

Add structured active profile summaries and selected-profile data to PresentationSnapshot through explicit service reads. Do not expose repository rows or raw event JSON.

- [ ] **Step 4: Render the adaptive Agents view**

Use the established cockpit color, border, typography, and focus language. Add Agents as a letter shortcut in navigation while leaving numbered tabs unchanged.

Render:

- Header: Agents, active count, ready count, not-ready count.
- List: selected marker, display name, role, specialty, readiness, version.
- Detail: immutable identity/version metadata and accepted content.
- Editor: current guided step, field guidance, validation message, progress.
- Review: ordered field diffs with before/after labels.
- Confirmation: explicit Create or Activate choice and cancel key.
- Inspector: template provenance, bindings readiness, history metadata.

- [ ] **Step 5: Connect host effects to ApplicationService**

The host translates controller effects into service calls and returns typed outcomes to the model. A stale edit refreshes the detail and displays a safe conflict message; it never auto-rebases or auto-activates.

On TUI shutdown, call cancel_agent_profile_edit before terminal restoration.

- [ ] **Step 6: Rerun GREEN**

    cargo test --test agent_profile_tui_render_contract
    cargo test --test agent_profile_tui_host_contract

Expected: all responsive rendering and host workflow tests pass.

- [ ] **Step 7: Commit**

    git add src/ui/tui src/app/service.rs tests/agent_profile_tui_render_contract.rs tests/agent_profile_tui_host_contract.rs
    git commit -m "feat: render adaptive agent profile cockpit"

---

## Task 12: Complete Acceptance Coverage, Documentation, Review, and Release Gates

**Files:**
- Create: tests/agent_profile_acceptance.rs
- Modify: tests/topology_contract.rs
- Modify: README.md
- Modify: phases.md
- Modify: architecture.md
- Create: docs/testing/phase-2-agent-profile-foundation.md

- [ ] **Step 1: Write the end-to-end acceptance test**

Exercise a real temporary SQLite database and application service:

1. Start from a schema-v1 database and migrate.
2. Create an unbound Bull template copy renamed Research North.
3. Verify it activates as version 1 with `Unbound` / `Not Ready`.
4. List and show it.
5. Preview an edit that changes specialty, tags, personality, and instructions;
   exercise bound readiness separately with an injected typed catalog.
6. Verify no durable write occurred during preview.
7. Activate after explicit simulated confirmation.
8. Verify version 2 remains honestly Unbound in production, injected-catalog
   readiness is role-specific, and version 1 remains byte-stable.
9. Restart the service and verify recovery reproduces the same active state and history.
10. Exercise fallback list/show/history output.
11. Exercise TUI model and renderer at narrow, medium, and wide sizes.
12. Verify audit and errors contain no personality, instructions, provider secret material, or rejected hostile input.

- [ ] **Step 2: Run the acceptance test and confirm RED if integration is incomplete**

    cargo test --test agent_profile_acceptance

Expected: fail only where an integration seam remains; implement that seam without broad refactoring.

- [ ] **Step 3: Update topology and operator documentation**

Document:

- What Phase 2 Milestone 1 includes and explicitly defers.
- Agent create/edit/list/show/history commands.
- Cockpit a shortcut and preserved 1 through 4 keys.
- Ready versus Not Ready.
- Immutable versions and explicit activation.
- Local-only preview behavior.
- Recovery fail-closed behavior for suspicious immutable history.
- Exact local manual testing transcript.
- How to inspect version history without exposing raw SQLite payloads.

Update phases.md to mark only this milestone complete, not all Phase 2 memory and skills work. Update architecture.md with the accepted aggregate and immutable-mirror recovery rule.

- [ ] **Step 4: Run focused acceptance GREEN**

    cargo test --test agent_profile_acceptance

Expected: the complete vertical slice passes.

- [ ] **Step 5: Request independent code review**

Invoke superpowers:requesting-code-review with the approved spec and this plan. Resolve every correctness, security, persistence, concurrency, and regression finding using superpowers:receiving-code-review. Add a regression test before each fix.

- [ ] **Step 6: Run release verification from a clean worktree**

Invoke superpowers:verification-before-completion, then run:

    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets --all-features

Expected: formatting clean, zero Clippy warnings, all legacy and Phase 2 tests pass.

- [ ] **Step 7: Run live terminal smoke tests**

In a real PTY:

1. Launch the release binary against an isolated temporary data directory.
2. Open Agents with a.
3. Create an unbound profile with a short template name, use exact `create`, and
   observe Not Ready.
4. Edit it through both name and ID selectors, inspect the field diff, return
   once with `Esc`, then use exact `activate <review-digest>`.
5. Verify bounded list/detail/history and exact historical-version content plus
   predecessor diff.
6. Resize across narrow, medium, and wide layouts.
7. Resize below the minimum and verify `q` still quits unconditionally while
   `Esc` does not.
8. Confirm alternate screen, cursor, bracketed paste, and terminal modes are restored.
9. Repeat the read workflow in fallback mode.
10. Confirm the second-instance guard still rejects a concurrent process.

- [ ] **Step 8: Commit documentation and acceptance coverage**

    git add tests/agent_profile_acceptance.rs tests/topology_contract.rs README.md phases.md architecture.md docs/testing/phase-2-agent-profile-foundation.md
    git commit -m "docs: complete agent profile foundation"

- [ ] **Step 9: Finish the development branch**

Invoke superpowers:finishing-a-development-branch. Present merge, pull request, keep-branch, and discard-worktree options without changing the user's dirty main checkout.
