# Task 5 Report: Declarative Skill Application Integration

## Status

Complete and green. Task 5 adds the application command, preview/review, typed outcome,
event, policy, audit, transaction, idempotency, and concurrency contracts for declarative
skills. It does not add recovery reconciliation, slash parsing, or interactive skill UI
states.

Accepted base: `21e34dac1a6bf88689b62bd1a30fb784949745bf`.

## Command and review shapes

The application command surface now contains:

- `CreateSkill { skill_id, candidate, review_token, review_digest }`
- `ActivateSkillVersion { skill_id, expected_active_version_id, candidate, review_token, review_digest }`
- `ListSkills`
- `ShowSkill { selector }`
- `ShowSkillHistory { selector }`
- `ShowSkillVersion { selector, version }`
- `AssignAgentSkill { profile_id, expected_active_profile_version_id, skill, review_token, review_digest }`
- `UpgradeAgentSkill { profile_id, expected_active_profile_version_id, expected, replacement, review_token, review_digest }`
- `UnassignAgentSkill { profile_id, expected_active_profile_version_id, expected, review_token, review_digest }`

`SkillSelector` accepts an exact `SkillId` or a user-provided name normalized through the
accepted skill canonicalizer. `AgentSkillAssignmentOperation` distinguishes assign,
upgrade, and unassign and carries exact immutable `SkillVersionRef` values.

Skill edits and assignments use a dedicated reservation-style `SkillReviewRegistry`.
Preview binds the canonical candidate or exact assignment operation, the expected active
skill/profile version, and a digest. Commit reserves the token before durable work,
releases it on rollback/commit failure, and consumes it after a successful commit. A
reserved token cannot authorize a competing command and a consumed token is single-use.

Creation rejects pre-existing history for the requested `SkillId`, requires normalized-name
availability, and creates only object version 1. Version activation verifies the expected
active `SkillVersionId`, rechecks normalized-name availability, inserts the immutable next
version, and moves the active pointer in the same immediate transaction.

Assignment commit validates the selected exact skill version, checks the expected active
profile version, distinguishes assignment from explicit upgrade, and requires the exact
currently assigned ref for upgrade/unassign. It creates one immutable profile version and
moves the active profile pointer atomically. The 16-skill domain limit is surfaced as the
stable `AgentSkillLimitExceeded` application error.

## Outcome shapes

Typed `CommandView` variants are:

- `SkillCreated(SkillCreatedView)` and `SkillVersionActivated(SkillVersionActivatedView)`, exposing IDs, object version, predecessor where applicable, and content digest.
- `Skills(SkillsView)`, exposing bounded summaries and total/returned/truncated counts.
- `Skill(SkillView)` and `SkillVersion(SkillView)`, which may return the full accepted `SkillDraft` to the direct caller plus exact ref, timestamp, provenance, and predecessor.
- `SkillHistory(SkillHistoryView)`, exposing exact refs, timestamps, predecessor IDs, active version ID, and bounded counts.
- `AgentSkillAssigned`, `AgentSkillUpgraded`, and `AgentSkillUnassigned`, each carrying the resulting profile/version IDs, predecessor profile version, object version, and resulting profile content digest.

Outcome materialization validates command/event identity, selectors and requested versions,
predecessors, accepted repository rows, active pointers, and canonical candidates before
returning a typed result.

## Event shapes and compatibility

The existing event schema version remains unchanged. New stable event kinds cover skill
creation, version activation, list/detail/history/version reads, and assignment,
upgrade, and unassignment.

Skill read events contain only exact refs, versions, digests, display-name/provenance
summaries, predecessor IDs, and counts. They never contain instructions or resource bodies.
Create/version events likewise carry mutation metadata, while accepted content remains in
the immutable skill repository.

Assignment events embed the accepted immutable `AgentProfileVersion`, matching the existing
agent-profile authoritative-event reduction pattern. This is the narrow compatibility
exception authorized in the scope ruling: `recovery/reducer.rs` reduces the embedded profile
so normal operation and replay preserve resulting profile history/pointer state. It does not
reconcile skills, rebuild the skill repository, or implement Task 6 sequencing.

## Capabilities

The semantic capabilities are narrowly:

- `SkillRead`
- `SkillCreate`
- `SkillVersion`
- `AgentSkillAssign`
- `AgentSkillUnassign`

Their persisted schema-v3 values are exactly `skill_read`, `skill_create`, `skill_version`,
`skill_assign`, and `skill_unassign`. There is no skill execute capability. The audit
contract proves unrelated/execution-like values are rejected.

## Transactions, concurrency, and receipts

Every skill/profile mutation runs in one SQLite immediate transaction containing immutable
row insertion, active-pointer movement, event append, projection persistence, outcome
materialization, audit-visible event data, and exactly one command receipt. Any error rolls
the transaction back and releases the review reservation.

Receipt lookup precedes review reservation for reviewed mutations. An exact command replay
returns the stored serialized typed `CommandOutcome`; it does not need the consumed review
token and does not append another event, write another immutable version/profile, move a
pointer, or insert another receipt. Reusing a command ID with a different canonical request
returns stable `IdempotencyConflict`.

The concurrency contract runs two independent workers against one expected active skill
version. SQLite serialization plus the in-transaction expected-pointer check yields exactly
one committed version/event/pointer winner and one stable stale-version loser. Assignment
operations use the same immediate-transaction boundary and independently check the expected
active profile version before constructing the next immutable profile.

## Audit redaction evidence

`AuditEntry::from_event` and the text renderer summarize skill activity with typed IDs,
versions, digests, names/provenance, predecessor IDs, and counts. They do not serialize raw
event JSON and do not include skill instructions or resource bodies. The focused tests place
distinctive prose in both fields, execute direct full-content reads, then assert that event
payloads and rendered audit/metadata output do not contain that prose.

## TDD evidence

Initial RED command:

```text
cargo test --test skill_application_contract \
  --test skill_event_contract \
  --test skill_concurrency_contract \
  --test skill_receipt_contract \
  --test skill_audit_contract
```

Observed RED: exit 101 before tests ran, with approximately 67 missing-surface compiler
errors for the new commands, outcomes, events, capabilities, previews, and service behavior.
A focused renderer assertion was added before renderer behavior; its RED compile exposed four
exhaustive consumers. The three out-of-scope consumers were reported and then explicitly
authorized by the follow-up scope ruling before modification.

Final focused GREEN command: the same five-test-target command above.

Final focused result: 9 passed, 0 failed across 5 test binaries.

Directly affected compatibility command:

```text
cargo test --test projection_contract \
  --test agent_profile_recovery_contract \
  --test fallback_contract \
  --test fallback_fix_round_contract \
  --test agent_profile_tui_controller_contract \
  --test tui_hardening_contract
```

Final compatibility result: 85 passed, 0 failed across 6 test binaries.

Combined final evidence: 94 passed, 0 failed across 11 test binaries.

## Scope extensions

The user explicitly authorized these exhaustive compatibility files after the first RED:

- `src/recovery/reducer.rs`: conservative event handling only; assignment events preserve embedded accepted profile reduction, with no skill reconciliation/projection rebuilding.
- `src/ui/command/renderer.rs`: concise typed metadata/outcome rendering; full skill prose is not rendered in audit-style output and no slash workflow was added.
- `src/ui/tui/controller.rs`: safely acknowledges new outcomes and redraws without adding editor/navigation states.

`src/skills/review.rs` was modified under the original pre-authorization to safely represent
assignment review reservations. During implementation an export was momentarily attempted in
`src/skills/mod.rs`, immediately reverted, and the application-facing preview DTO was placed
in the authorized application outcome module. There is no final diff in that unauthorized
file.

## Self-review

- `git status --short` shows only the 11 authorized production files and five required tests; this report is ignored by the repository and is force-added for the checkpoint.
- `git diff --check` is clean.
- Outcome materialization was tightened during self-review to bind read selectors/version numbers and mutation predecessor IDs to committed event metadata.
- Capability wire values were checked against migration 3 and corrected to `skill_assign`/`skill_unassign` while retaining unambiguous semantic Rust variant names.
- Event schema version remains unchanged; no recovery sequencing, slash parser, interactive skill state, main entry point, migration, or unrelated formatting was changed.
- Residual boundary: Task 6 must implement skill reconciliation/repository recovery; this task deliberately only preserves embedded assignment profile reduction.

## Commits

- Base: `21e34dac1a6bf88689b62bd1a30fb784949745bf`.
- Task 5 checkpoint: the commit containing this report (actual SHA reported to the caller after creation).

## Interfaces consumed by Tasks 6-8

Task 6 consumes the stable `ApplicationEvent` variants, `SkillEventSummary`,
`SkillHistoryEventEntry`, embedded assignment profile versions, exact predecessor refs, and
the unchanged event schema version. The current reducer intentionally leaves skill
repository reconciliation to Task 6.

Task 7 consumes `ApplicationCommand`, `SkillSelector`, all typed skill `CommandView` DTOs,
the five capabilities, stable `AppError` variants/codes, and metadata-safe renderer output.
Task 7 must add parsing/workflows rather than changing these application contracts.

Task 8 consumes the public preview methods on `ApplicationService`, `ApplicationWorker`, and
`IndependentApplicationService`, including assignment/upgrade/unassign previews and their
single-use review token/digest. The controller currently treats resulting views as safe
acknowledged outcomes; Task 8 may add dedicated editor/navigation state without changing the
transaction or receipt contracts.
