# Phase 0 Task 10 Report

## Implementation commit

`99108a20aea8acea7038f882360fb301f2448fd0` (`feat: add transactional application service`)

## Changed files

- `src/app/service.rs`
- `src/app/mod.rs`
- `src/app/outcome.rs`
- `src/persistence/event_repository.rs`
- `src/persistence/projection_repository.rs`
- `tests/application_contract.rs`
- `tests/support/mod.rs`
- `.superpowers/sdd/2026-08-31-phase-0-rust-foundation/task-10-report.md`

## TDD evidence

### RED

Command:

```text
cargo test --test application_contract --locked
```

Result: exit 101. Compilation failed on the intended missing Task 10 API:
`ApplicationService`, `AuthorizationDecision`, and `CommandPolicy` were unresolved.

### Focused GREEN

Command:

```text
cargo test --test application_contract --locked
```

Result: exit 0; 13 passed, 0 failed. Coverage includes all current command variants,
typed views, exact capability mapping, allow/deny/approval behavior, deterministic
dependency counts, replay and conflict, stale state, rollback, privacy, and finish.

### Full current suite

Command:

```text
cargo test --locked
```

Result: exit 0; 137 passed, 0 failed, with no warnings.

## Idempotency and transaction design

- A command ID is persisted without a new schema object by representing its UUID as
  the event's typed `CausationId`. Recovery-owned events continue to use no causation
  ID, preserving the Task 7 repository boundary.
- The command fingerprint is the submitted actor, correlation ID, typed redacted
  command event, causation ID, and absent object reference. A retry with the same
  fingerprint returns the authoritative event; the same command ID with any changed
  request fact returns `AppError::CommandConflict` before policy, clock, or ID use.
- Outcomes are derived durably from the authoritative event and projection state at
  that event's sequence. Audit tails are bounded through the original event sequence,
  so later events cannot change a replayed outcome.
- Every new command opens one immediate transaction, checks for replay, evaluates the
  injected policy, reloads authoritative projection state, appends the event, reduces,
  stores the projection, and commits. Any error drops the transaction and rolls back
  event, projection, and derived command receipt together.
- Denied, approval-required, replay, and conflict execution with a supplied envelope
  do not consume clocks or IDs and do not mutate. `execute_user` allocates the two IDs
  that define a newly accepted envelope; event clock/ID allocation still occurs only
  after authorization and current projection loading inside `execute`.
- The bootstrap policy uses the reviewed deny-wins evaluator and grants exactly the
  five Phase 0 capabilities. Approval-required decisions remain typed and inert.

## Concerns and scope boundaries

- Phase 0 commands each produce exactly one event. If a future command produces
  multiple events, it must persist one command fingerprint and the complete ordered
  event set within the same immediate transaction before that command is enabled.
- The current schema has no unique constraint on `causation_id`; the process guard and
  immediate transaction serialize application commands, while repository loading
  rejects a corrupt duplicate causation identity deterministically.
- No runtime queue, terminal host, broker, network, credential, or approval execution
  behavior was added.

## Fix round 1: durable command receipts

### Implementation commit

`bae433ff183fe614f879ae068078fde5580944a1` (`fix: persist transactional command receipts`)

### Controller ruling applied

The causation-derived receipt design was replaced. Command identity is now owned by a
durable `command_receipts.command_id` primary key, including commands that commit zero
events because policy denied them or requires approval. Event causation IDs remain in
place for event-level semantics but are no longer the command-idempotency authority.

### Exact schema design

- `command_receipts` stores `command_id`, the lowercase SHA-256
  `command_fingerprint`, canonical `request_json`, typed `capability`, typed
  `policy_decision`, and canonical complete `outcome_json`.
- `command_event_refs` stores zero or more event references under the receipt with a
  non-negative `event_ordinal` and composite primary key
  `(command_id, event_ordinal)`.
- `command_event_refs_event_idx` uniquely assigns each command-created event to one
  receipt. Foreign keys target both `command_receipts.command_id` and
  `event_stream.event_id`.
- Both tables are `STRICT`. JSON validity, fingerprint shape, capability values,
  policy-decision values, and ordinal bounds are database constraints.
- Update/delete triggers make receipts and ordered event references immutable.
- The exact Task 6 schema oracle now covers every new table, column, declared type,
  nullability, primary-key position, foreign key, semantic index, trigger SQL, check,
  enumeration, uniqueness constraint, and immutable path.

### Idempotency and transaction boundary

1. Execution rejects a finished lifecycle before opening a transaction or consulting
   policy, clocks, or IDs.
2. One immediate transaction loads the primary-key receipt first. A canonical matching
   request replays the validated durable typed result; a changed request returns
   `CommandConflict`.
3. With no receipt, the same transaction verifies authoritative event/projection state
   before policy. Stale projection rows are rejected without mutation or dependency use.
4. Granted commands allocate one event ID and one timestamp, append, reduce, persist
   projection, materialize the bounded transaction-visible view, canonicalize the full
   typed outcome, write the receipt and ordered refs, and commit.
5. Denied and approval-required commands materialize canonical typed error outcomes and
   write zero-event receipts in that transaction. They write no event, projection, or
   approval row and consume no clock or generated event ID.
6. After commit, result conversion is infallible; no database read, view construction,
   JSON operation, or other fallible work remains.
7. Receipt loading validates canonical request/outcome JSON, the fingerprint,
   capability/policy facts, contiguous event ordinals, authoritative event envelopes,
   and the complete typed outcome before replay.

Barrier-synchronized tests use two SQLite connections. A same-command race produces
one receipt/effect and two identical outcomes with one policy/clock/event-ID use. A
same-ID conflicting-command race produces one winner and one deterministic
`CommandConflict`, again with one durable receipt/effect.

### TDD and verification evidence

RED schema command:

```text
cargo test --test migration_contract --locked
```

Result: exit 101; 11 passed and 5 failed specifically because `command_receipts` and
`command_event_refs` were absent from the database and exact schema oracle.

RED application command:

```text
cargo test --test application_contract --locked
```

Result: exit 101; compilation failed on the intended missing
`CommandTransactionHook` service contract.

Focused GREEN:

```text
cargo test --test application_contract --locked
cargo test --test migration_contract --locked
```

Results: 19 application tests passed and 16 migration tests passed; 0 failed.

Full current suite:

```text
cargo test --locked
```

Result: 143 passed, 0 failed, with no warnings.

### Prerelease migration compatibility

Phase 0 is prerelease, so migration `0001_phase0.sql` was coherently extended in place.
Its checksum therefore changed. Development databases created before this fix can fail
startup with `database_migration_state_invalid`; those pre-fix databases are not a
supported released format and should be recreated rather than migrated in place.

### Scope

No runtime queue, terminal host, broker, network, credential, approval execution, or
other external-action behavior was added in this fix round.
