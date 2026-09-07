# Phase 2 Milestone 3: Hybrid Memory Design

Date: 2026-09-07
Status: Approved for implementation planning on 2026-09-07
Baseline: `origin/main` at `be1c3a6`

## Summary

Phase 2 Milestone 3 adds durable, agent-private memory to the local Rust
terminal application. The milestone completes the third and final Phase 2
slice without adding model execution, provider connections, rooms, or market
behavior.

The memory system has three deliberately separate layers:

1. explicit key/value memory that the user creates and edits;
2. agent-originated set or delete proposals that remain inert until the user
   approves or rejects the exact proposal; and
3. bounded episodic summaries linked to authoritative source events and
   treated only as retrieval aids.

Every accepted key/value mutation creates an immutable version. A rebuildable
projection identifies the latest version of each logical key, including a
tombstone after deletion. Retrieval is deterministic, local, scoped to one
exact agent profile and memory namespace, and constrained by explicit item,
byte, and source-reference budgets.

Memory is stored as plaintext in the existing owner-only local SQLite database.
This milestone does not add encryption or key management. Accepted memory may
appear in immutable mirror rows, append-only event payloads, command request and
outcome receipts, SQLite WAL/journal sidecars, deliberate detail views, and
user-created local or filesystem backups. Tombstones do not securely erase any
earlier copy. Memory is therefore non-secret application data, and every write
review must communicate that boundary before the user confirms it.

## Approved product decisions

The user approved the following decisions during design review:

- Store memory as plaintext in local SQLite for now.
- Continue to treat credentials and API keys as outside the memory model.
- Require a visible review and explicit confirmation for every human set,
  overwrite, and delete operation.
- Treat a direct human edit as a confirmed mutation that needs no separate
  approval record.
- Represent deletion as a new immutable tombstone version and retain history.
- Keep episodic summaries read-only in normal Phase 2 use because completed
  rooms do not exist yet.
- Implement a typed internal proposal-ingestion boundary with synthetic tests;
  do not expose agent proposal creation through the TUI or fallback parser.
- Use deterministic metadata filtering and hard retrieval budgets; do not add
  embeddings, semantic search, SQLite FTS, or model calls.
- Use immutable per-key versions plus rebuildable current projections rather
  than mutable rows or whole-namespace snapshots.
- Place Memory inside the Agents workspace. Do not add a global navigation tab
  or shortcut.
- Integrate only against the merged simple-navigation behavior in which the
  bare shortcuts are `1`, `2`, `3`, `4`, `a`, and `s` and modified shortcuts
  do not navigate.

## Development constraint

All specification, planning, implementation, and review work for this milestone
uses the dedicated `codex/phase-2-hybrid-memory` Git worktree. This process
constraint keeps the user's older dirty checkout and concurrent work isolated;
it is not a product behavior or runtime acceptance criterion.

## Goals

- Give every stable agent profile identity an isolated durable memory
  namespace.
- Let a human create, inspect, edit, delete, and review history for bounded
  plaintext key/value records.
- Prevent a UI action or slash command from mutating memory before the exact
  candidate is reviewed and confirmed.
- Let a future agent execution propose an exact set or delete operation without
  granting that agent authority to make the operation durable.
- Let a human inspect and resolve each proposal exactly once.
- Establish immutable, source-linked episodic-summary records for later room
  integration without fabricating room behavior now.
- Produce deterministic, finite memory snapshots for future context assembly.
- Preserve the existing application-command, policy, event, persistence,
  recovery, audit, receipt, and presentation boundaries.
- Complete the outstanding Phase 2 memory, isolation, approval, migration, and
  retrieval gates while keeping all default tests offline.

## Non-goals

- Encryption at rest, SQLCipher, application-layer encryption, or key rotation.
- Storing, importing, discovering, or managing credentials, tokens, passwords,
  private keys, or provider secrets.
- Direct OpenAI, Anthropic, xAI, or other model-provider calls.
- Agent execution, autonomous memory writes, or automatic proposal approval.
- Room creation, room completion, transcripts, orchestration, or live episodic
  summary production.
- User-authored or user-edited episodic summaries.
- Cross-agent memory sharing, namespace merging, or implicit memory copying.
- Semantic search, embeddings, vector databases, SQLite FTS, or heuristic
  relevance scoring.
- Background retention jobs, automatic history pruning, or permanent deletion.
- Persisting a room or job execution snapshot before a consuming execution
  aggregate exists.
- Phase 3 connection, inference, streaming, or chat behavior.

## Existing baseline and compatibility boundary

The milestone starts from schema version 3 after the completed Agent Profile
Foundation and Declarative Skills milestones.

The accepted baseline already provides:

- stable `MemoryNamespaceId` values embedded in immutable agent profile
  versions;
- a database constraint that prevents one memory namespace from belonging to
  multiple stable profile identities;
- append-only, hash-linked application events;
- canonical command receipts and exact replay/conflict behavior;
- immutable profile and skill mirrors with rebuildable active projections;
- process-local one-use review registries;
- a generic exact-object approval vocabulary and approval table skeleton;
- transactional migrations, immediate write transactions, recovery, and fault
  injection seams;
- typed fallback and TUI application boundaries; and
- merged navigation with Overview, Setup, Audit, Help, Agents, and Skills.

`src/memory/mod.rs` is currently only a module-boundary placeholder. Hybrid
Memory must extend the established structures rather than add a second service,
database, event stream, or UI authority path.

Schema-v1, schema-v2, and schema-v3 databases remain supported inputs to the
ordered migration system. Applying migration v4 must not alter existing event,
profile, or skill bytes.

The baseline intentionally treats one committed application command as one
authoritative application event. Hybrid Memory preserves that invariant. A
memory event may contain a bounded composite result, such as one accepted entry
version plus the proposal resolutions made stale by that version; the milestone
does not generalize the application into multi-event command materialization.

## Terminology

- **Stable profile**: the logical `AgentProfileId` shared by all immutable
  versions of one agent profile.
- **Profile version reference**: the exact profile ID, profile-version ID,
  positive object version, and content digest used by a producer or retrieval.
- **Memory namespace**: the stable `MemoryNamespaceId` owned by exactly one
  stable profile.
- **Logical entry**: one key identity inside one namespace, identified by
  `MemoryEntryId` across all versions, deletions, and recreations.
- **Entry version**: one immutable accepted state of a logical entry.
- **Tombstone**: an immutable entry version whose state is deleted and whose
  value is absent.
- **Proposal**: an immutable agent-authored candidate set or delete operation.
- **Resolution**: the one terminal accepted, rejected, or expired result for a
  memory proposal.
- **Episodic summary**: an immutable, labeled retrieval aid linked to exact
  committed source events. It is not a fact, policy rule, or KV mutation.
- **Memory snapshot**: a deterministic set of exact memory references and
  digests selected under one scope and finite retrieval budget.

## Architecture and module boundaries

The implementation remains in the existing single Rust crate.

```text
src/memory/
├── mod.rs          public domain boundary and re-exports
├── normalization.rs bounded key, value, tag, label, and summary text
├── entry.rs        immutable KV versions, references, and tombstones
├── proposal.rs     proposed mutations and terminal resolution rules
├── episodic.rs     immutable summaries and ordered source references
├── projection.rs   current-key and proposal-status deterministic state
├── retrieval.rs    scoped budgets, selection, omissions, and snapshots
└── review.rs       process-local human edit review registry

src/persistence/
└── memory_repository.rs

src/ui/
├── memory_editor.rs
├── command/        parser, renderer, and fallback workflow integration
└── tui/
    └── views/memory.rs

migrations/
└── 0004_hybrid_memory.sql
```

The existing application, policy, audit, recovery, and presentation modules
gain memory variants and integration code. Presentation modules may keep drafts,
focus, selection, and process-local confirmation state, but they never read or
write a repository directly.

`ProjectionState` gains one `MemoryProjection` field containing only current
entry references (including tombstones) and proposal status/reference metadata;
it never embeds KV values, rationale, labels, summary bodies, or source lists.
Immutable history remains in the event stream and reconciled mirrors. The field
implements `Default`/`is_empty`, defaults during deserialization, and is skipped
from persisted/digest material while empty so legacy projection bytes remain
stable.

The core durable write path is unchanged:

```text
presentation adapter or internal producer
            ↓ typed operation
application service and actor/policy validation
            ↓
immediate SQLite transaction
            ↓
append one event → reduce → reconcile immutable rows → rebuild pointers
            ↓
materialize typed outcome and safe audit entry → write receipt → commit
```

## Typed identifiers

The domain adds distinct UUID-backed identifiers:

- `MemoryEntryId`
- `MemoryEntryVersionId`
- `MemoryProposalId`
- `MemoryReviewToken`
- `EpisodicSummaryId`

These identifiers cannot be interchanged with agent, profile-version, skill,
approval, command, event, or namespace identifiers.

The milestone also adds `AgentProfileVersionRef`, containing:

- `profile_id`;
- `profile_version_id`;
- positive object version; and
- exact profile content digest.

The reference must resolve to the authoritative immutable profile version, and
its memory namespace is always derived from that version. No memory command or
retrieval request accepts a caller-supplied namespace as an alternative source
of authority.

### Digest separation

All digests are lowercase SHA-256 over versioned canonical JSON:

- an entry content digest covers display/normalized key, state, value, and
  purpose tags;
- a proposal content digest covers proposer reference, derived namespace,
  operation, key, expected state, candidate, and rationale;
- a summary content digest covers owning namespace, pinned profile reference,
  label, body, tags, and complete ordered source references; and
- each record digest additionally covers its typed IDs, fixed or monotonic
  object version, predecessor/provenance links, creation actor/time, creation
  event sequence/ID, plaintext-validation version, and content digest as
  applicable.

An event digest is not a record-digest input, avoiding a cycle; recovery instead
validates the record's creation event ID and the event envelope's own digest
chain independently. Object references bind typed ID, positive version, and
content digest, while mirror reconciliation also requires the exact record
digest and canonical record bytes.

## Key/value model and validation

### Entry draft

A `MemoryEntryDraft` contains:

- `display_key`;
- plaintext `value`;
- zero or more `purpose_tags`.

The separate `MemoryEditReview` carries the explicit plaintext/non-secret
acknowledgment and the final confirmation proves it was accepted. That
acknowledgment is not semantic memory content. The review states that plaintext
history survives overwrite and deletion and may also be retained by local
backups.

Canonicalization applies before byte-limit checks:

- CRLF and CR normalize to LF in multiline values.
- Leading and trailing whitespace is removed from keys and tags.
- Internal key whitespace folds to one ASCII space.
- Key identity uses the existing NFKC plus case-fold comparison rules.
- Tags use the existing normalized tag comparison rules and canonical sort.
- NUL, ESC, C0/C1 control characters other than normalized value newlines, and
  bidi override/isolate controls are rejected.
- Tabs are rejected so terminal layout and canonical bytes remain stable.

Limits are:

- display key: 1 through 96 UTF-8 bytes;
- value: 1 through 4,096 UTF-8 bytes;
- purpose tags: at most 8;
- each purpose tag: 1 through 32 UTF-8 bytes;
- active logical keys per namespace: at most 1,024; and
- pending proposals per namespace: at most 256.

Duplicate normalized purpose tags are rejected. Two display keys that normalize
to the same identity name the same logical entry inside a namespace. The display
form stored on a later version may change while the normalized identity remains
stable.

The normalized tag `general` is reserved for the explicit untagged retrieval
scope and cannot be stored as a purpose tag. This avoids treating a literal tag
and the untagged fallback as the same purpose accidentally.

Memory values are inert text. Command-looking strings, Markdown, URLs, file
paths, and tool-like instructions receive no authority and are never executed.

### Non-secret boundary

The editor and confirmation surface state that memory is stored as plaintext
local data, is copied into append-only history and command receipts, is not
erased by a tombstone, and must not contain credentials. The memory command
surface has no secret-import field and never receives values from a credential
broker.

Validation rejects reserved credential-oriented keys, including normalized
separator and plural variants of `password`, `passphrase`, `api key`, `access
token`, `refresh token`, `session token`, `private key`, `secret`, and
`credential`. One deterministic high-confidence scanner also rejects known
credential material in every plaintext-bearing memory field: KV values,
proposal values and rationale, and episodic labels and bodies. Its versioned
`CredentialPatternSetV1` rejects any substring matching one of these exact
ASCII-oriented rules after control-safe canonicalization:

- case-insensitive `-----BEGIN ` followed by optional `RSA `, `DSA `, `EC `,
  or `OPENSSH ` and then `PRIVATE KEY-----`;
- case-insensitive `authorization`, optional ASCII space, `:`, optional ASCII
  space, case-insensitive `bearer`, at least one ASCII space, then 16 through
  the enclosing field's maximum characters from `[A-Za-z0-9._~+/=-]`; or
- case-insensitive prefix `sk-`, `sk-ant-`, or `xai-` followed by 20 through
  the enclosing field's maximum characters from `[A-Za-z0-9_-]`, delimited by
  non-token characters or field boundaries.

Matching occurs before any event, receipt, or row is written. Every accepted
entry version, proposal, and summary stores `plaintext_validation_version = 1`
in its canonical record and record digest. Recovery validates a record with its
stored pattern set, not the newest set; unknown versions fail closed. Future
pattern changes require a new version plus golden tests and do not retroactively
invalidate authenticated history. These rules are defense in depth, not a claim
that arbitrary prose can be perfectly classified. The plaintext warning and
the rule that credentials stay outside memory remain authoritative.

No memory value, proposal, or summary is copied into a connection secret field.
Future connection features use opaque secret references outside this aggregate.

## Immutable memory entries

`MemoryEntryVersion` contains:

- memory namespace ID;
- stable entry ID;
- unique entry-version ID;
- positive monotonically increasing object version;
- optional exact predecessor version ID;
- canonical display key and normalized key;
- `MemoryEntryState`, either `Present` or `Deleted`;
- plaintext value and canonical purpose tags only when present;
- creation actor and time;
- optional accepted proposal reference;
- plaintext validation version;
- preallocated creation event ID; and
- canonical content and record digests.

`MemoryEntryRef` contains the namespace ID, entry ID, entry-version ID, object
version, normalized key, state, and content digest. It is sufficient to detect
namespace substitution, stale writes, version substitution, and unexpected
deletion state.

Rules:

- Version 1 has no predecessor. Every later version points to version `n - 1`.
- A set on an absent never-before-seen key creates version 1 with a new stable
  entry ID.
- An overwrite requires the exact current reference and creates the next
  present version.
- A delete requires the exact current present reference and creates the next
  tombstone version.
- Recreating a deleted key requires the exact tombstone reference and creates
  the next present version with the same stable entry ID.
- Setting identical canonical content is a typed no-op, not a new version.
- Deleting an already deleted or never-created key is a typed no-op.
- Existing versions are never updated or deleted.
- The current projection retains the exact tombstone pointer, while normal list
  views filter deleted keys.
- History remains available newest first and is bounded before cloning or
  rendering records.
- Every accepted entry version in Milestone 3 has creation actor Human. An
  approved agent proposal is preserved separately through its exact accepted
  proposal reference; it does not make the agent the mutation actor.

The content and record digests follow the separation above. Readiness,
selection state, and presentation metadata are not digest inputs.

No-op detection occurs during passive preview. An identical set returns
`NoChange::IdenticalContent`; a delete of a missing or tombstoned key returns
`NoChange::AlreadyAbsent`. Neither result registers a review token nor creates a
command, event, receipt, audit entry, ID, timestamp, or database write. If state
changes after a real review was registered, confirmation fails as stale rather
than being silently converted to a no-op.

Immutable versions, events, and receipts grow monotonically in Milestone 3.
There is deliberately no total-history quota or pruning command in this local
release. Item-size, active-key, pending-proposal, query, and retrieval bounds
limit individual operations and in-memory work, while a full disk or SQLite
capacity failure rolls the transaction back without partial state. The UI and
documentation must disclose that repeated edits retain plaintext history and
consume local storage.

## Human edit review

Human set, overwrite, recreate, and delete workflows are two-stage operations.

The passive preview operation:

- resolves the exact current profile and memory entry state;
- canonicalizes and validates the candidate;
- computes an ordered before/after diff;
- binds the operation, actor, profile reference, namespace, expected current
  entry reference or expected absence, and candidate digest;
- registers a process-local one-use `MemoryReviewToken`; and
- writes no event, receipt, draft, row, audit entry, clock value, or generated
  durable ID.

The confirmation command repeats all exact binding data. Confirmation succeeds
only for `Actor::Human`, the owning command flow, the same candidate, and the
same authoritative current entry state.

Cancellation, candidate edits, replacement previews, normal shutdown, terminal
failure, and application-service finish invalidate the relevant token. A
recoverable transactional failure releases the reservation so the exact visible
confirmation can be retried; a successful commit consumes it permanently.

Direct human mutations do not create an `ApprovalRecord`. The confirmed review
is still explicit and auditable, but it is not an agent proposal approval.

## Agent-originated proposals

### Producer identity

The existing actor model gains `Actor::Agent(AgentProfileId)`. Existing Human
and System enum serialization and event digests remain byte-for-byte valid. The
canonical database representation is:

| Actor | `actor_kind` | `actor_id` |
|---|---|---|
| Human | `human` | `NULL` |
| System | `system` | `NULL` |
| Agent | `agent` | canonical stable profile UUID |

Any other kind/ID combination fails decoding. Event digest material continues
to use a null actor ID for legacy Human and System events and includes the
canonical profile UUID for Agent events. Event-envelope serde adds only the new
Agent variant; it does not rename or reshape the two legacy variants. The same
actor encoding rules apply to approval records. Round-trip, legacy-golden, and
actor-ID substitution tests cover the event repository, event digest, receipts,
audit renderer, and every exhaustive actor match.

Each proposal separately embeds an exact `AgentProfileVersionRef`. The service
requires the actor profile ID to equal the reference's stable profile ID and
resolves the reference before accepting the proposal. This makes the stable
actor identity queryable while preserving exact version provenance.

The proposal command is available only through an internal typed application
boundary. Neither the fallback parser nor the TUI can construct it. Phase 2 has
no production caller; deterministic tests inject synthetic agent actors. Phase
3 or the future room service may call the same boundary after its own approved
design.

### Proposal shape

`MemoryProposal` contains:

- proposal ID;
- immutable object version `1`;
- exact proposer profile reference;
- derived memory namespace;
- operation: `Set` or `Delete`;
- canonical display and normalized key;
- candidate plaintext value and purpose tags for `Set`;
- expected current entry reference or explicit expected absence;
- bounded plaintext rationale, at most 512 UTF-8 bytes;
- plaintext validation version;
- creation time and preallocated event ID;
- exact approval ID; and
- canonical content and record digests.

`MemoryProposalRef` contains proposal ID, fixed object version `1`, and exact
content digest. Approval binding, review binding, events, and views use this
reference instead of an unversioned UUID.

Proposal key, value, tags, and rationale use the same control-safety and
canonicalization rules as direct human drafts. Proposal creation may durably
create only the proposal and its pending exact approval; it may not create or
change a memory entry.

A set proposal may expect absence, an exact present version, or an exact
tombstone. A delete proposal must expect an exact current present version.
Proposal creation rejects an identical set and every delete that is already a
no-op; an agent cannot consume approval capacity with a candidate that has no
effect at creation time.

The proposal limit is checked inside the write transaction. Resolved proposals
do not count toward the pending cap.

### Approval and rejection

`ApprovalAction` gains `MemoryMutation`. Proposal creation writes a pending
approval record whose `ObjectRef` is the exact proposal ID, object version `1`,
and content digest. The approval requester actor is the exact Agent actor from
the proposal event, and `expires_at` is absent because this milestone has no
wall-clock proposal expiry. The proposal row has one unique approval ID, and
repository and recovery checks enforce a one-to-one match among the proposal,
approval object reference, and any terminal resolution.

Only `Actor::Human` may resolve it. The review surface shows:

- proposer display name and exact profile version;
- operation and namespace owner;
- canonical key;
- current state;
- proposed value and tags for a set;
- rationale;
- expected-current reference; and
- proposal digest.

Approval and rejection are themselves explicitly confirmed presentation
workflows.

Their process-local one-use resolution review binds Human actor, owning command
flow, action (`Approve` versus `Reject`), exact `MemoryProposalRef`, approval ID
and pending status, expected current entry state, and review digest. An approve
review can never authorize rejection or vice versa. Candidate replacement,
cross-flow reuse, stale proposal/current state, cancellation, successful
commit, terminal failure, or application-service finish invalidates it. A
recoverable transaction failure releases only the reserved matching review so
the still-visible exact confirmation can retry.

Acceptance revalidates the proposal, pending approval, exact proposal digest,
namespace ownership, and expected current entry state inside one immediate
transaction. It writes one bounded composite `MemoryProposalAccepted` event
containing the accepted resolution, the new present or tombstone entry version,
and expiration resolutions for every other pending proposal targeting the same
logical key whose expected state no longer matches. Embedded expiration records
are sorted by proposal ID. The proposal resolution, all affected approval
statuses, immutable entry mirror, current pointer, event, projection, safe audit
metadata, and receipt commit atomically.

Rejection writes one `MemoryProposalRejected` event containing only the terminal
rejection and approval resolution. It never mutates memory.

Accept, reject, and expire are the only memory-proposal terminal states in this
milestone. Cancellation remains a generic approval vocabulary value but has no
memory-proposal command or event. A second or concurrent resolution returns the
authoritative terminal outcome when the same command is replayed, or a
deterministic conflict for a different command.

Pending proposals survive restart. They have no wall-clock auto-expiry in this
local milestone. Every accepted mutation, whether a direct Human edit or an
approved proposal, expires all other proposals for that logical key whose
expected target state became stale. A direct set/delete event embeds those
sorted expiration resolutions just as an acceptance event does.

Profile-version edits alone do not expire a proposal. A proposal may have been
created by an execution pinned to a then-active version, so its exact historical
proposer remains eligible while the namespace remains owned by the same stable
profile. If that version is no longer active, approval review displays a
prominent `proposer version is historical` warning and both the historical and
current profile version identities. Acceptance never substitutes the current
profile version for the pinned proposer.

## Episodic summaries

`EpisodicSummary` contains:

- summary ID;
- immutable object version `1`;
- owning memory namespace;
- exact pinned profile-version reference;
- canonical label, 1 through 128 UTF-8 bytes;
- plaintext body, 1 through 8,192 UTF-8 bytes;
- zero through 8 purpose tags using the KV tag rules;
- 1 through 128 ordered exact source-event references;
- plaintext validation version;
- creation time, event sequence, and event ID; and
- canonical content and record digests.

`EpisodicSummaryRef` contains summary ID, fixed object version `1`, namespace,
exact pinned profile version reference, creation event sequence and ID,
source-set digest, and content digest. The source-set digest covers the full
ordered source-reference list, so a compact snapshot reference still detects
missing, reordered, or substituted provenance.

Each source reference pins event sequence, event ID, event type, and event
digest. Sources must be unique, strictly ordered by sequence, already committed,
and earlier than the summary-recorded event. A missing, changed, reordered, or
future source fails validation.

Episodic summaries are immutable and never promoted automatically into KV
memory. They are visibly labeled `Summary — verify sources` in every view and
snapshot. Retrieval treats them as context aids, never as policy, authority, or
ground truth.

The repository, reducer, retrieval, recovery, and read-only views are delivered
now. Milestone 3 deliberately has no application command, policy capability, or
service method that records a summary. Domain constructors and an explicit
test-only verified-event fixture exercise storage and replay. A future
completed-room design must add its own typed producer command and authority
before production code can call the boundary. Ordinary Human, Agent, System,
fallback, and TUI paths therefore cannot create or edit summaries in this
milestone, and no synthetic command appears in production help.

Because Phase 2 has no room event kinds, test fixtures may link summaries to
existing earlier committed application events solely to prove ID, digest,
ordering, persistence, retrieval, and tamper checks. The fixture bypasses only
the future semantic requirement that sources belong to a completed room; it
does not bypass source existence, sequence, ID, type, digest, uniqueness, or
ordering validation. Enabling a production writer requires the future room
design to define and enforce the exact eligible room-event allowlist.

## Deterministic retrieval and snapshots

### Scope

`MemoryRetrievalScope` contains:

- scope format version `1`;
- exact agent profile-version reference;
- derived namespace;
- `MemoryPurposeScope`, either `General` or `Tagged` with a nonempty canonical
  set of one through 8 purpose tags.

`MemoryRetrievalRequest` pairs that scope with one finite
`MemoryRetrievalBudget`; scope identity and resource limits are distinct digest
inputs rather than duplicating the budget inside the scope.

Scope format version 1 is explicitly roomless because no Room identity exists
in Phase 2. A later room phase must introduce a new scope format containing an
exact Room reference; it may not reinterpret a version-1 scope or widen its
namespace.

### Budget

The default budget is:

- at most 32 present KV records;
- at most 8 episodic summaries;
- at most 32,768 total canonical UTF-8 bytes; and
- at most 128 episodic source references.

Every request carries exact finite values. Requested values above the compiled
hard maxima are rejected rather than silently clamped. The initial compiled
hard maxima equal the defaults; later phases must explicitly revise the design
before widening them.

Budget arithmetic is checked for overflow. A record is admitted as one complete
unit or omitted; keys, values, labels, summary bodies, and source lists are never
partially sliced into a snapshot.

### Selection and ordering

Retrieval uses no database-dependent hash-map order and no heuristic score.

1. Resolve the exact profile and namespace.
2. Use the consistent SQLite snapshot established by the command's
   `BEGIN IMMEDIATE` transaction.
3. Validate each entry candidate's exact immutable reference and current
   pointer, and each summary candidate's immutable reference and source set.
4. For `Tagged`, select each present KV entry once when its tags intersect the
   requested set, followed by untagged entries as general fallback. For
   `General`, select only untagged entries.
5. Sort each KV group by normalized key, object version, and stable entry ID.
6. Apply the same intersection-once and untagged fallback rule to episodic
   summaries.
7. Sort each summary group by creation time descending and summary ID.
8. Traverse in that order, accepting whole records that fit and recording
   explicit omission counts for all rejected candidates.

An individually oversized candidate is omitted and traversal continues, so one
large earlier record cannot hide every smaller later record. This behavior is
deterministic and reported. Tombstones and records that do not match the scope
are ineligible, not omitted, and never contribute to omission counters.

The byte cost of one candidate is the length of its canonical JSON context
item. A KV context item contains its exact reference, display key, value, and
purpose tags. An episodic context item contains its exact reference, label,
body, purpose tags, and complete ordered source references. The accepted-byte
count is the checked sum of those item lengths; source-reference cost is also
counted separately against its own budget. Exact eligible, accepted, omitted,
omitted-byte, and omitted-source totals use checked `u64` arithmetic. Overflow
returns a stable retrieval-overflow error rather than dropping an accounting
field or clamping a value.

Repository selection streams rows in the specified order and keeps only the
bounded accepted result plus scalar counters; it never clones the entire
episodic history merely to compute exact omissions.

### Snapshot

`MemorySnapshot` contains:

- retrieval scope;
- requested budget;
- exact accepted KV context items;
- exact accepted episodic context items;
- accepted byte and source-reference counts;
- exact eligible and omitted counts separately for KV and summaries;
- exact omitted-byte and omitted-source counts; and
- canonical snapshot digest.

The snapshot digest covers every scope, budget, reference, ordering, and
omission field plus the content and source-set digests of every context item. It
does not duplicate profile, KV, or summary prose inside digest metadata.
Selection and content materialization both finish inside that same
`BEGIN IMMEDIATE` transaction before the metadata-only event and complete
receipt are appended. The fully materialized immutable value is returned only
after all exact references validate and the transaction commits.

Milestone 3 returns snapshots as serializable immutable values but does not add
a separately queryable execution-snapshot table. Because `BuildMemorySnapshot`
is a normal read command, its command receipt retains the complete plaintext
outcome for exact replay; the memory plaintext warning and documentation
explicitly disclose that additional durable copy. Phase 3 and later execution
aggregates may add a dedicated pinned snapshot record when they have an approved
consumer.

## Application commands and passive operations

### Durable typed commands

The application command vocabulary gains:

- `SetMemoryEntry`
- `DeleteMemoryEntry`
- `ProposeMemoryMutation` through the internal producer boundary only
- `ApproveMemoryProposal`
- `RejectMemoryProposal`
- `ListMemoryEntries`
- `ShowMemoryEntry`
- `ShowMemoryEntryHistory`
- `ShowMemoryEntryVersion`
- `ListMemoryProposals`
- `ShowMemoryProposal`
- `ListEpisodicSummaries`
- `ShowEpisodicSummary`
- `BuildMemorySnapshot` through an internal typed retrieval boundary only

Read commands continue to flow through the application service and produce
typed outcomes. They never let presentation code reach SQLite directly.

Mutation request shapes are exact and receipt-stable:

- `ExpectedMemoryEntryState` is `Absent`, `Present(MemoryEntryRef)`, or
  `Deleted(MemoryEntryRef)`.
- `SetMemoryEntry` carries exact profile reference, expected state, canonical
  candidate, review token, and review digest.
- `DeleteMemoryEntry` carries exact profile reference, expected present entry
  reference, review token, and review digest.
- `ProposeMemoryMutation` carries exact proposer reference, a typed set/delete
  operation with its expected state and candidate, and rationale. Service-owned
  IDs and time are generated only after authorization and validation.
- `ApproveMemoryProposal` and `RejectMemoryProposal` carry exact proposal
  reference, approval ID, expected pending status, expected current entry state,
  action-specific review token, and review digest.
- Read commands carry a bounded selector or exact typed reference;
  `BuildMemorySnapshot` carries the complete canonical retrieval request.

Every command and nested operation uses tagged, deny-unknown-field serde. The
review digest covers every mutation field except the one-use token itself, and
the command fingerprint covers the complete canonical command request including
actor and token.

### Passive review operations

Preview and cancellation are service/runtime operations rather than durable
application commands, matching the existing profile and skill review patterns:

- preview direct set;
- preview direct delete;
- preview proposal approval;
- preview proposal rejection; and
- cancel memory review.

They allocate no durable ID, timestamp, event, receipt, proposal, approval, or
memory version. Only the later exact confirmation command mutates state.

### Selectors and bounds

Agent selectors accept stable UUID or canonical display name using the existing
profile selector. Memory keys accept a display key and resolve through the
namespace-local normalized identity. Proposal and summary selectors are exact
typed UUIDs; they do not accept ambiguous labels.

List and history operations apply database limits before materializing domain
records. Initial limits are 100 returned rows with exact `u64` total, returned,
and omitted counts. Entry lists contain current present records ordered by
normalized key then entry ID. Entry history contains every present and tombstone
version for one logical key, newest object version first. Episodic summaries are
newest creation time then summary ID.

Proposal list accepts `pending` or `all`; pending is the default. Pending is
oldest creation time then proposal ID, so resolving the visible first page makes
the next pending proposals discoverable even at the 256-item cap. All is newest
creation time then proposal ID. The exact proposal-detail command remains
available for any known ID. Counts exclude no status selected by the filter.

## Policy and actor model

Capabilities are narrow and non-overlapping:

- `MemoryRead`
- `MemoryPreview`
- `MemoryMutate`
- `MemoryPropose`
- `MemoryResolve`

The default local capability policy grants all five memory capabilities. A
central memory actor-command guard then narrows those grants with this mandatory
matrix; both checks must pass and an explicit policy denial still wins.

| Actor | Allowed memory operations in Milestone 3 |
|---|---|
| Human | read, preview, direct mutate, approve, reject |
| Agent whose ID matches the exact proposer | propose only through the internal boundary |
| System | none |

The guard runs for every internal and presentation-originated memory operation,
not just parser routes. After same-command receipt lookup, it rejects an invalid
pair before policy-dependent work, review reservation, clocks, IDs, or mutation.
Therefore Human cannot impersonate an agent producer, Agent cannot use a Human
review token, and Agent or System cannot read, approve, reject, or directly
mutate memory in this milestone.

- A profile's personality, skill text, memory text, provider binding, or role
  never grants a capability.
- The profile ID in an Agent actor must equal the exact proposer profile ID.
- An unknown memory capability remains denied by default, and an explicit denial
  wins.

The v4 migration extends the command-receipt capability constraint using the
same table-rebuild pattern proven by schema v2 and v3. Existing receipt bytes,
event references, fingerprints, and outcomes remain unchanged.

## Events, outcomes, and audit

### Event kinds

Mutation events are:

- `MemoryEntrySet`
- `MemoryEntryDeleted`
- `MemoryProposalCreated`
- `MemoryProposalAccepted`
- `MemoryProposalRejected`
- `EpisodicSummaryRecorded` in verified test fixtures only

Read and retrieval events are:

- `MemoryEntriesListed`
- `MemoryEntryShown`
- `MemoryEntryHistoryShown`
- `MemoryEntryVersionShown`
- `MemoryProposalsListed`
- `MemoryProposalShown`
- `EpisodicSummariesListed`
- `EpisodicSummaryShown`
- `MemorySnapshotBuilt`

The existing event envelope schema remains version 1 because the envelope shape
does not change. New event kinds use strict, tagged, deny-unknown-field payloads.
Legacy events retain identical canonical bytes and digests.

Mutation events carry the complete accepted immutable record needed for replay,
including plaintext memory or proposal content. `MemoryEntrySet` and
`MemoryEntryDeleted` may also carry sorted stale-proposal expiration records.
`MemoryProposalAccepted` carries its resolution, the accepted entry version, and
sorted sibling expirations. `MemoryProposalCreated` carries the exact pending
approval record. `MemoryProposalRejected` carries its terminal resolution.
These bounded composite payloads preserve one command, one event, and one
primary object while allowing every derived row to commit atomically.

Read events carry bounded identity, counts, and digests but not full values or
summary bodies. The event object reference identifies the exact primary
immutable object: entry version for direct mutation, proposal for proposal
creation/resolution, and summary for the test-only summary record. Reducer and
materialization rules validate every embedded reference and reject duplicates,
unsorted expirations, unrelated keys, or inconsistent approval transitions.

Plaintext can still be duplicated in command receipt `request_json` and
`outcome_json`, including a fully materialized snapshot outcome. This is an
intentional consequence of exact replay in the approved local plaintext model,
not audit leakage. The user-facing warning and documentation name every durable
copy and explain that overwrite/delete does not erase it.

The implementation update to `architecture.md` makes its generic "redacted
payload" wording precise: credentials and incidental sensitive transport data
remain forbidden/redacted, while an authoritative memory mutation event retains
the complete user-approved non-secret memory record required for replay. Audit
summaries remain prose-free.

### Outcomes and views

Typed views include:

- bounded entry summaries;
- complete deliberate entry detail;
- bounded immutable history and exact version detail;
- proposal summaries, exact proposal detail, and resolution status;
- episodic summary metadata and deliberate source-linked detail;
- direct mutation and proposal-resolution results; and
- snapshot metadata plus deliberately requested accepted content.

Routine status, overview, and audit snapshots never include memory prose.

### Audit redaction

Audit summaries include only safe fields such as profile ID, namespace ID,
entry or proposal ID, normalized operation, object version, counts, lengths,
status, and digests. They never include raw keys, values, rationale, labels,
summary bodies, purpose tags, or command input.

Deliberate memory detail and confirmation views may show exact plaintext after
terminal-control sanitization. This is a presentation decision, not a generic
log or audit emission.

## SQLite schema v4

Migration `0004_hybrid_memory.sql` creates strict, constrained storage.

### `memory_entry_versions`

The immutable mirror stores independently constrained identity, namespace,
version, predecessor, normalized key, state, content digest, record digest,
canonical record JSON, creation event, and creation time fields. Query-critical
fields are not trusted only from serialized JSON.

Constraints enforce:

- UUID-shaped identifiers;
- positive versions;
- version-1/null-predecessor and later-version/non-null-predecessor shape;
- one stable entry ID for one normalized key in one namespace;
- exact predecessor identity and version adjacency;
- present/value and deleted/no-value state shape;
- canonical lowercase SHA-256 digests;
- valid canonical JSON; and
- a committed creation event reference.

No-update and no-delete triggers make every version immutable.

### `current_memory_entries`

The rebuildable current table maps `(memory_namespace_id, normalized_key)` to an
exact entry-version ID, version, state, and content digest. It references the
immutable mirror through a composite foreign key. A tombstone remains current
but is filtered from active list views.

### `memory_proposals`

The immutable proposal mirror stores the exact proposer profile reference,
namespace, operation, expected target reference/absence, canonical candidate,
rationale, approval ID, content/record digests, creation event, and time.

Set and delete operations have mutually exclusive candidate shapes enforced by
checks. No-update and no-delete triggers preserve the accepted proposal.

### `memory_proposal_resolutions`

One immutable row per resolved proposal stores terminal status (`accepted`,
`rejected`, or `expired`), resolving Human actor, exact approval ID, resolution
event, and time. In Milestone 3 even automatic expiry is caused by the Human's
accepted mutation and records that same actor and event. Unique and foreign-key
constraints prevent a second resolution. Repository checks require the generic
approval row to have action `MemoryMutation`, the exact proposal object
reference, the same terminal status/resolver/time, and no prior transition.

### `current_memory_proposal_status`

This rebuildable projection contains pending and terminal status for bounded
list queries. It pins proposal and optional resolution event identity. Recovery
may replace it only after immutable proposal and resolution reconciliation.

### `episodic_summaries` and `episodic_summary_sources`

The immutable summary table stores independently constrained namespace,
profile reference, label/body metadata, tags, counts, digests, record JSON,
creation event, and time. The ordered source table stores summary ID, ordinal,
event sequence, event ID, event type, and event digest. Foreign keys and
repository validation require exact authoritative source events.

No-update/no-delete triggers apply to both tables.

### Approval and receipt evolution

Migration v4 rebuilds `approval_records` without changing any existing row
value and adds nullable `resolution_actor_kind` and `resolution_actor_id`
columns. Existing pre-v4 rows retain `NULL` in both new columns. New pending
rows also require both columns to be `NULL`; a terminal memory-mutation row
requires the canonical Human resolver shape (`human`, `NULL`) together with
its resolution time, kind, and event. The rebuilt table also gains update
guards so exact requester/object-binding columns never change and status can
transition from pending to one terminal state only once. The terminal
resolution metadata and resolver columns must be populated by that same
transition and cannot subsequently change.

Legacy terminal non-memory approvals remain valid when their pre-v4
`resolution_event_id` is `NULL`; migration v4 neither invents nor rewrites that
historical value. The non-null resolution-event requirement applies only to
new terminal `MemoryMutation` approvals.

The migration rebuilds `command_receipts` to admit the five memory capability
wire values while preserving existing rows and command-event references
exactly. Every approval and receipt copy/rebuild boundary has an
injected-failure test proving full rollback to schema v3.

## Transaction and concurrency semantics

Every durable memory operation uses `BEGIN IMMEDIATE` and the existing command
receipt authority.

The service order is:

1. reject a closed application lifecycle;
2. canonicalize the request without reading clocks or generating IDs;
3. begin the immediate transaction;
4. load and validate any existing command receipt, returning its exact stored
   outcome before repeating authorization or dependencies;
5. verify durable lifecycle and projection state;
6. enforce the memory actor-command matrix, then evaluate capability policy;
7. reserve the matching process-local review when applicable;
8. re-resolve profile, namespace, proposal, approval, and current-entry state;
9. validate expected exact references, semantic effect, and capacity;
10. allocate only the IDs and timestamp required by the accepted operation;
11. append exactly one authoritative event, composite where required;
12. reduce a cloned projection state;
13. reconcile immutable mirrors, approvals, and resolution rows;
14. replace rebuildable current pointers/status;
15. materialize safe outcome and audit entries;
16. write one canonical receipt with its one event reference; and
17. commit before consuming the successful review registration.

Properties:

- Same command ID and same canonical request returns the exact stored outcome
  without generating IDs, reading clocks, or writing again.
- Same command ID with changed content returns `CommandConflict`.
- Two edits against the same expected entry version have one winner and one
  stale loser.
- Concurrent approve/reject attempts have one terminal winner.
- A failed boundary rolls back the event, immutable rows, pointers, resolutions,
  approval status, audits, and receipts together.
- No failure silently rebases a proposal or confirmed human candidate onto a
  newer entry.

## Recovery and integrity

Startup continues to verify the complete event digest chain before trusting any
memory row.

Recovery then:

1. reduces verified events into expected memory entries, proposals,
   resolutions, episodic summaries, and current state;
2. validates every exact profile and source-event reference;
3. inserts an immutable row only when the verified stream proves it is missing;
4. accepts an existing immutable row only when all constrained and canonical
   bytes match;
5. refuses startup for altered rows, identity substitution, unexpected extra
   rows, invalid predecessors, extra resolutions, or invalid source links;
6. rebuilds current entry and proposal-status projections transactionally; and
7. produces the same projection digest on repeated recovery.

For approvals specifically, recovery derives the complete expected memory-only
approval map from `MemoryProposalCreated` and later composite
accepted/rejected/expiry payloads. A wholly missing expected
`ApprovalAction::MemoryMutation` row is reconstructed exactly from authenticated
events, whether its expected state is pending or terminal. An existing row must
already match every binding, actor, status, time, resolution, and event field;
recovery never updates it from pending to terminal or otherwise "fixes" a
mismatch. Any extra memory-mutation approval not proven by the event stream also
fails startup. Rows for all other approval actions are outside this
reconciliation and remain untouched.

Recovery never updates or deletes suspicious immutable history. Only explicitly
rebuildable pointers and status projections may be replaced.

A legacy event stream with no memory events recovers to the exact empty memory
state without changing its prior projection canonical bytes or digest. The
`ProjectionState` deserializer defaults a missing memory field to the empty
projection, and both the persisted projection representation and digest material
skip that field while empty. Nonempty memory uses the new field. Golden tests
cover pre-v4 bytes, empty v4 bytes, and nonempty v4 bytes.

Recovery itself creates no memory snapshot. Running an identical fresh
retrieval command against unchanged recovered state yields the same snapshot
digest; replaying an existing command ID returns the complete historical
snapshot stored in its receipt without rerunning selection, even if current
pointers have since changed.

## Fallback command experience

The supported grammar is:

```text
/memory list <agent>
/memory get <agent> <key>
/memory history <agent> <key> [version]
/memory set <agent> <key>
/memory delete <agent> <key>
/memory proposals <agent> [pending|all]
/memory proposal <proposal-id>
/memory approve <proposal-id>
/memory reject <proposal-id>
/memory episodes <agent>
/memory episode <summary-id>
```

Agent names and keys may be quoted according to the existing bounded parser.
Exact UUID selectors remain supported. Extra, missing, zero-version, unknown,
or ambiguous arguments become typed malformed input with actionable usage and
without retaining raw input.

`/memory set` opens a guided local editor for value and purpose tags. It loads
the current value when present, or starts a new candidate when absent. Review
registers the passive token; a second exact confirmation submits the mutation.
An identical set or already-absent delete displays the passive `No change`
result and never enters confirmation.

`/memory delete`, `approve`, and `reject` load authoritative detail and then show
an exact confirmation. Generic `yes` or an unrelated command cannot confirm a
memory action. Backpressure and recoverable application failures keep the
visible confirmation retryable only while its review remains valid.

`proposal`, `episodes`, and `episode` are deliberate read-only detail paths.
There is no fallback command for proposal creation, summary creation/editing, or
raw snapshot building. The latter remains an internal typed retrieval boundary
in this milestone.

The set editor and final set/delete confirmation both display `Plaintext local
memory — do not store credentials; history is retained after overwrite or
delete`. Proposal approval/rejection review displays the same warning because
the proposal is already retained as plaintext and acceptance may add another
history version.

EOF, interrupt, output failure, explicit cancel, and normal quit cancel any
registered review exactly once before host teardown.

Renderers bound every list before cloning content and visibly escape hostile
text. Routine failure lines contain stable safe codes and no raw memory content.

## TUI experience and navigation compatibility

Memory is an Agents sub-workspace, not a seventh global tab.

From an active agent detail, the existing action selector gains `Memory`. Enter
opens that exact profile's memory while preserving the Agents tab as the owner.
The global shortcuts remain unchanged:

- `1` Overview;
- `2` Setup;
- `3` Audit;
- `4` Help;
- `a` Agents; and
- `s` Skills.

Bare shortcuts navigate only when a text editor does not own input. Modified
shortcuts do not navigate. Switching away preserves memory selection, scroll,
draft, and confirmation state under the same per-tab rules as profiles and
skills.

`MemoryViewState` owns:

- selected profile and namespace identity;
- list/detail/history/proposal/episode selections;
- independent scroll offsets;
- editor stage and local draft;
- current passive review generation;
- pending exact confirmation; and
- delayed-outcome intent and generation guards.

Panes are:

- Entry List;
- Entry Detail;
- Entry History;
- Editor;
- Mutation Review;
- Confirmation;
- Proposals;
- Proposal Detail;
- Proposal Resolution Review;
- Episodic Summaries;
- Episodic Detail; and
- Result.

The TUI exposes the same entry, history, proposal list/detail/resolution, and
episodic list/detail actions as fallback mode. It has no proposal-create,
episodic-create/edit, or snapshot-build action. Its editor, mutation review,
final mutation confirmation, and proposal-resolution review use the same
plaintext-retention warning as fallback mode.

Narrow mode shows one pane at a time. Medium shows the list beside the active
detail/editor. Wide shows list, detail, and contextual history/proposal/source
information. Every review and confirmation keeps profile identity, operation,
key/proposal identity, and Enter/Esc guidance visible at `60x18` and all larger
supported layouts.

Arrow keys move selection, Enter opens or accepts the visible action, and Esc
unwinds exactly one memory layer. Bare `q` remains inert; `/quit` is the normal
shutdown command. Memory text never enters hidden command history merely by
being displayed or edited.

Delayed reads may hydrate inactive memory data but may not replace a newer
selection, steal navigation, discard a draft, or close a protected review or
confirmation.

## Error handling

New failures have stable, content-free error codes. Categories include:

- invalid memory key, value, purpose tag, rationale, or summary;
- credential-oriented key or recognized credential material rejected;
- namespace/profile mismatch;
- entry not found for deliberate read/detail;
- proposal has no semantic effect at creation;
- active-key or pending-proposal capacity reached;
- stale expected entry version;
- invalid, missing, stale, or consumed review;
- proposal not found, already resolved, or stale;
- proposer actor/profile mismatch;
- approval mismatch or non-human resolution;
- invalid episodic source reference;
- immutable memory/proposal/summary integrity mismatch;
- current memory projection rebuild failure; and
- database contention or transactional persistence failure.

Errors never interpolate raw memory keys, values, proposal rationale, summary
text, rejected input, database bytes, or SQL details. Recoverable presentation
errors preserve deliberate local drafts when safe; terminal integrity errors
fail closed.

## Testing strategy

Implementation is test-driven. Each implementation milestone starts with a
failing focused contract and passes its local gate before the next milestone.

### Domain contracts

- Distinct typed IDs and exact references.
- UTF-8 byte boundaries, newline normalization, Unicode normalization, control
  rejection, reserved credential keys, reserved `general` tag, mandatory
  credential-material patterns across every plaintext aggregate, and duplicate
  tags.
- Immutable version creation, predecessor adjacency, no-op detection,
  tombstone, and recreate behavior.
- Canonical content/record digests and serde rejection of malformed states.
- Proposal operation shapes, exact proposer identity, and terminal resolution.
- Episodic label/body/source limits and source ordering.
- `CredentialPatternSetV1` exact positives, near-miss negatives, delimiter and
  field-boundary behavior, persisted validation version, and unknown-version
  rejection.

Property tests cover arbitrary Unicode boundaries, canonical ordering,
overflow-safe budgets, stable digests, and invalid state combinations.

### Migration and persistence contracts

- Fresh schema reaches exactly version 4 with every intended table, index,
  trigger, foreign key, and check.
- Exact schema-v3 fixture upgrades without changing existing rows.
- Every v4 migration boundary rolls back to the identical v3 inventory,
  migration ledger, and data.
- Immutable rows reject update/delete and identity substitution.
- Current pointers accept only exact immutable references.
- Repository round trips and limits validate every independently stored field.
- Approval transitions and receipt capability values accept only canonical
  forms.
- Human/System/Agent actor database shapes round-trip exactly; legacy actor
  digests remain golden, and Agent actor-ID tampering fails recovery.

### Application, approval, and concurrency contracts

- Direct edits write nothing before review confirmation.
- Human confirmation creates one exact version/event/receipt transaction.
- Identical set and absent/tombstoned delete previews return `NoChange` without
  a review, command, event, receipt, audit, ID, clock read, or database write.
- Direct Human set/delete/recreate creates no `ApprovalRecord` row.
- System or Agent cannot use a human review.
- Human cannot call the proposal producer; Agent cannot propose for another
  profile; parser/TUI expose no proposal-create path.
- Proposal creation writes one proposal and pending approval but no memory
  mutation, and rejects a candidate with no semantic effect.
- Human approval applies the exact proposal once; Human rejection never mutates
  memory.
- Agent/System self-approval and changed proposal digests fail closed.
- Approve and reject review tokens cannot authorize the opposite action.
- Direct and proposal-accepted mutations expire every newly stale sibling
  proposal in deterministic order inside the same composite event/transaction.
- Concurrent edit/edit, approve/approve, and approve/reject races have one
  deterministic winner.
- Same-command replay and changed-command conflict preserve existing receipt
  guarantees.
- Fault injection after each proposal, approval, event, immutable mirror,
  current pointer, projection, audit, receipt, and pre-commit boundary proves
  full rollback.

### Recovery and integrity contracts

- Empty legacy streams preserve compatible state.
- Missing proven immutable rows are backfilled.
- Altered or unexpected entry, proposal, resolution, or summary rows stop
  startup without rewriting evidence.
- Missing pending and terminal memory approval rows are reconstructed exactly;
  mismatched or extra memory approval rows fail closed; unrelated approval
  actions remain byte-identical.
- Missing/corrupt current rows rebuild exactly.
- Invalid predecessor, namespace, approval, and source-event references fail
  before recovery mutation.
- Repeated recovery yields a byte-identical projection; an unchanged fresh
  retrieval yields the same snapshot digest, while command replay returns the
  receipt's historical materialized snapshot without reselection.

### Retrieval and isolation contracts

- Two profiles sharing a placeholder provider never retrieve one another's
  memory.
- Profile edits retain their stable namespace. The duplication acceptance
  fixture copies only non-memory draft fields and calls the existing profile
  creation path; the new profile receives a generated namespace and zero
  entries. Milestone 3 does not add a duplicate-profile command or UI action.
- Exact profile references and namespace derivation are mandatory.
- Tag matching, general fallback, stable ordering, whole-record admission,
  omission counters, byte limits, item limits, source limits, and arithmetic
  overflow are deterministic.
- Missing or changed exact references fail rather than silently widen or
  substitute content.
- Command-looking memory remains inert.

### Fallback and TUI contracts

- Parser accepts only the documented grammar and exact selector forms.
- Guided set/delete and approve/reject require distinct review and confirmation
  states.
- For every action both expose, TUI and fallback produce identical typed
  commands, events, views, validation failures, and terminal outcomes.
- Both hosts expose proposal and episodic detail but no proposal-create,
  summary-create/edit, or snapshot-build route.
- Both hosts show the exact plaintext-retention warning in the editor/review and
  final mutation confirmation; proposal resolution shows it before confirmation.
- Navigation preserves the merged `1` through `4`, `a`, and `s` behavior and
  never adds a global memory shortcut.
- Drafts and confirmations survive unrelated tab switches without allowing
  hidden commits.
- Narrow, medium, and wide rendering keeps identity, action, and controls
  visible and safely escapes untrusted content.
- Audit, error, status, and ordinary help views do not leak memory prose.

### Full gate

The milestone must pass:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

No default test requires a terminal, network, provider, paid subscription,
credential, model, room, or real market data.

## Documentation and completion

Implementation updates:

- `README.md` with the completed Hybrid Memory behavior, local-plaintext warning,
  navigation path, fallback commands, and test instructions;
- `phases.md` to mark only Phase 2 Milestone 3 and the complete Phase 2 exit gate
  as complete after every criterion passes;
- `architecture.md` with the accepted concrete memory aggregate, retrieval, and
  approval contracts without weakening later-phase requirements; and
- `docs/testing/phase-2-hybrid-memory.md` with isolated state-directory,
  fallback, TUI, recovery, and manual acceptance procedures.

Phase 2 is complete only when:

- all Milestone 1 and 2 regressions still pass;
- immutable KV history and tombstones recover exactly;
- an agent-authored proposal cannot become memory without a distinct exact
  Human approval;
- namespace isolation and empty-on-duplicate behavior are proven;
- episodic summaries remain source-linked retrieval aids rather than facts;
- retrieval is deterministic and bounded with exact snapshot references;
- TUI and fallback parity is proven;
- migration, corruption, race, rollback, audit-redaction, and hostile-text tests
  pass;
- the complete offline quality gate passes; and
- final code and specification review findings are resolved.

Only after that gate may Phase 3 receive its own approved design and
implementation plan for connections, normalized inference, and single-agent
chat.

## Explicitly deferred after Milestone 3

- Encryption and encrypted backup/recovery.
- Provider credentials and the secret broker.
- Real agent proposal production.
- Automated or bulk proposal approval.
- Completed-room episodic production and room-scoped retrieval enforcement.
- Cross-agent memory sharing or export.
- Background pruning and permanent deletion.
- Semantic, embedding, vector, FTS, or model-ranked retrieval.
- Persisted execution snapshots, prompt assembly, inference, and chat.
- MCP, live rooms, engineering jobs, finance evidence, and trade decisions.

These are later approved phase boundaries, not incomplete Hybrid Memory error
paths.
