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

## Fix round 1 of 5

Status: complete and green.

### Findings addressed

- Capability serde now explicitly maps semantic `AgentSkillAssign` and
  `AgentSkillUnassign` to authoritative schema-v3 wires `skill_assign` and
  `skill_unassign`. The legacy `agent_skill_*` spellings are rejected and no execute
  capability exists.
- Independent application instances now expose upgrade and unassignment preview forwarding,
  allowing separate valid review registries/connections to exercise real conflicting commits.
- Receipt replay coverage now includes create, version activation, assignment, upgrade, and
  unassignment with the same command identity.
- Review coverage binds creation/version candidates and assignment operation, exact ref,
  profile identity, and expected active profile version. It also covers consumed-token reuse,
  same-command replay, and incorrect exact refs for upgrade/unassign.
- Concurrency coverage retains the skill-version race and adds independent assignment,
  upgrade, and unassignment races.

### RED and pass-immediately evidence

Capability serde RED:

```text
cargo test --test skill_audit_contract assignment_capability_serde_uses_schema_v3_wire_values
```

Observed: 0 passed, 1 failed. `AgentSkillAssign` serialized as
`"agent_skill_assign"` instead of `"skill_assign"`. After explicit serde renames: 1 passed,
0 failed.

Receipt replay matrix:

```text
cargo test --test skill_receipt_contract fix_round_one_replay_matrix
```

After correcting a test-only selector import, the behavioral contract passed immediately:
1 passed, 0 failed. This records evidence added for existing shared receipt behavior rather
than claiming a production defect.

Review/exact-ref matrix:

```text
cargo test --test skill_application_contract fix_round_one_review_and_exact_refs
```

After correcting test-only exact-ref acquisition and non-overlapping diagnostic command IDs,
the behavioral contract passed immediately: 1 passed, 0 failed. Existing review reservation
and rollback behavior already met these findings.

Concurrency API RED:

```text
cargo test --test skill_concurrency_contract fix_round_one_agent_skill_races
```

Observed compile RED: independent services had assignment preview but no upgrade or
unassignment preview methods. The minimum production change added those two forwarding
methods. After test-only ownership and valid-draft fixture corrections, all three behavioral
races passed: 3 passed, 0 failed.

### Durable snapshot evidence

For every create/version/assign/upgrade/unassign replay, the test captures the original typed
outcome and a durable snapshot containing exact skill detail/history outcomes, exact agent
detail/history outcomes where applicable, active profile pointer rows, mutation event JSON,
receipt records, event IDs carried by the typed outcome/receipt relationship, and row totals
for events, receipts, event references, and immutable profile versions. Same-identity replay
returns an equal typed outcome and an equal post-replay snapshot.

Every rejected review or exact-ref command compares before/after snapshots containing exact
typed skill/profile pointers and histories, active profile rows, mutation payloads, and all
relevant durable row totals. Rejections return exactly `SkillReviewMismatch`,
`SkillReviewUnavailable`, `StaleAgentProfileVersion`, or `SkillNotAssigned` as appropriate,
without durable change.

Each agent mutation race snapshots durable totals before launch and proves one typed winner,
one `StaleAgentProfileVersion` loser, exactly one new immutable profile version, one event,
one receipt, and one event reference. It also verifies the sole active profile pointer equals
the winner's exact profile-version ID and that the sole mutation payload embeds that ID.

### Final verification

Task 5 focused command:

```text
cargo test --test skill_application_contract \
  --test skill_event_contract \
  --test skill_concurrency_contract \
  --test skill_receipt_contract \
  --test skill_audit_contract
```

Result: 15 passed, 0 failed across 5 test binaries.

Affected policy/audit/application/reducer/renderer/controller regression command:

```text
cargo test --test agent_profile_tui_host_contract \
  --test agent_profile_application_contract \
  --test fallback_fix_round_contract \
  --test agent_profile_hardening_contract \
  --test agent_profile_service_final_fix_contract \
  --test fallback_contract \
  --test application_contract \
  --test tui_application_contract \
  --test policy_contract \
  --test final_fix_application_contract \
  --test runtime_contract \
  --test projection_contract \
  --test agent_profile_recovery_contract \
  --test agent_profile_tui_controller_contract \
  --test tui_hardening_contract
```

Result: 174 passed, 0 failed across 15 test binaries.

Combined fix-round verification: 189 passed, 0 failed across 20 test binaries.

### Fix-round self-review

- Final diff is limited to `src/policy/capability.rs`, `src/app/service.rs`, four existing
  Task 5 focused test files, and this report.
- `git diff --check` is clean.
- Production delta is minimal: two serde attributes and two independent-service forwarding
  methods; transaction, audit, event, renderer, controller, migration, and recovery behavior
  are unchanged.
- Tests use accepted repository/application reads for exact refs rather than constructing
  domain refs or inspecting implementation-private state.
- No execute capability, Task 6 recovery work, slash parsing, Task 7 workflow, Task 8 state,
  unrelated formatting, push, PR, or merge was added.
