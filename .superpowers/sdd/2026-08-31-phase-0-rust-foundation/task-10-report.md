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

## Fix round 2

### Implementation commit

`d2fe8e2e5402da8ec12e6f494f9b4d9e3669d7a6` (`fix: share command service lifecycle across workers`)

Changed implementation/test files:

- `src/app/service.rs`
- `tests/application_contract.rs`
- `tests/migration_contract.rs`
- `tests/support/mod.rs`

### Strict TDD evidence

Focused RED:

- Command: `cargo test --test application_contract`
- Result: compilation failed with `E0599` because `ApplicationService::worker` did not exist. This proved that tests could no longer use the public, lifecycle-unbound `open_peer_for_test` constructor.

Focused diagnostic after the first implementation step:

- Command: `cargo test --test application_contract`
- Result: 24 passed, 1 failed.
- Failure: `transaction_rejects_a_closed_authoritative_session_before_dependencies_or_writes` received `Recovery(InvalidEventRecord)` instead of `LifecycleFinished`. The exact-session gate was therefore moved ahead of full projection loading, while remaining inside the immediate transaction.

Focused GREEN:

- Command: `cargo test --test application_contract`
- Result: 25 passed, 0 failed.
- Command: `cargo test --test migration_contract`
- Result: 17 passed, 0 failed.

Full-suite GREEN:

- Command: `cargo test`
- Result: 150 passed, 0 failed across unit, integration, and documentation targets.

### Lifecycle and worker design

- Bootstrap creates one private `SharedLifecycle` containing the authoritative bootstrap `SessionId` and an `RwLock<LifecyclePhase>`.
- `ApplicationService::worker` is the only peer construction path. It opens another embedded connection while cloning the same private lifecycle, policy, clock, ID generator, and transaction hook. The lifecycle-unbound public test constructor was removed.
- Command execution takes the shared lifecycle read guard and holds it through `BEGIN IMMEDIATE`, exact-session validation, projection validation, receipt replay or command execution, receipt persistence, and commit.
- Immediately after `BEGIN IMMEDIATE`, execution checks that the exact bootstrap session still exists with no end marker. It then performs the full projection load, preserving authoritative-event-ahead-of-projection rejection before receipt, policy, clock, ID, or write work.
- `finish` takes the exclusive lifecycle guard, durably appends/reduces/projects the terminal event through `RecoveryCoordinator::finish_session`, and flips the shared phase to closed only after that transaction succeeds. A failed finish leaves the lifecycle open; every peer created before a successful finish observes closed afterward.
- A closed shared lifecycle and a transaction-visible closed/missing current session both return exactly `AppError::LifecycleFinished` without policy, clock, event-ID, receipt, event, projection, or approval effects.

### Durable receipt and zero-event evidence

- Denied and approval-required outcomes now have full matrices for sequential replay after policy change, same-ID/different-command conflict, barrier-synchronized same-command races, barrier-synchronized conflicting-command races, and both outcome-materialization and receipt-write failures.
- All zero-event paths assert no clock or event-ID calls, no event refs, no events, no projection movement, and no approval rows. Concurrent attempts produce one durable receipt authority and deterministic replay or conflict.
- Receipt replay validation is tested by independently tampering noncanonical request JSON, noncanonical outcome JSON, typed-invalid request JSON, typed-invalid outcome JSON, fingerprint, capability, policy decision, event-ref/outcome identity, ordinal continuity, and reference encoding. Every case returns exactly `AppError::Persistence(PersistenceError::InvalidEventRecord)` before dependencies or further mutation.
- Rejected-input tests query both durable `request_json` and `outcome_json` and prove the raw secret is absent, in addition to the existing event and debug-output assertions.
- Outcome materialization remains entirely pre-commit. Replay performs no fallible work after commit, and stale authoritative events ahead of projection rows still produce typed recovery rejection with rollback and zero dependency use.

### Exact schema pinning and compatibility

- The migration SQL was unchanged in fix round 2 because the approved normalized receipt schema already satisfies the stronger contract.
- Tests now compare normalized full DDL for `command_receipts` and `command_event_refs`, in addition to exact columns, primary keys, foreign keys, semantic indexes, immutability triggers, and ordered-reference constraints.
- Fingerprints accept exactly 64 lowercase hexadecimal characters. Tests reject uppercase, nonhex, shorter, and longer values.
- Every valid capability and policy-decision value is accepted; uppercase and extra values are rejected.
- Phase 0 remains prerelease. Development databases created before the durable-receipt migration checksum change may fail checksum validation and are not a supported released format.

### Scope and concerns

- No runtime queue, terminal host, external action, broker/network behavior, credential execution, or approval-row creation was added.
- The shared lifecycle is process-local by design, while the exact current-session row is revalidated transactionally for durable authority on every command.

## Fix round 3 (final)

### Implementation commit

`654cf5b7b8e91cf4aa2cb4c2df39dace2bc8b907` (`fix: serialize user command IDs with shutdown`)

Changed implementation/test files:

- `migrations/0001_phase0.sql`
- `src/app/mod.rs`
- `src/app/service.rs`
- `tests/application_contract.rs`
- `tests/migration_contract.rs`
- `tests/support/mod.rs`

### Strict TDD evidence

Focused RED application contract:

- Command: `cargo test --test application_contract`
- Result: compilation failed because the public restricted `ApplicationWorker` type and the deterministic lifecycle race-hook boundaries did not exist.

Focused RED migration contract:

- Command: `cargo test --test migration_contract`
- Result: 15 passed and 2 failed.
- `command_receipt_fingerprint_and_enumerated_domains_reject_every_extra_form` demonstrated that the old CHECK accepted an embedded-NUL fingerprint.
- `migration_records_and_complete_schema_are_exact` demonstrated that the exact DDL lacked text typing, UTF-8 byte length, and NUL rejection.

Focused implementation diagnostic:

- Command: `cargo test --test application_contract`
- Result: 26 passed and 1 failed only because the new race test initially counted one ID for `finish_session`; the reviewed recovery contract allocates both terminal event and correlation IDs. Expectations were corrected to the exact existing dependency contract without changing lifecycle behavior.

Focused GREEN:

- Command: `cargo test --test application_contract`
- Result: 27 passed, 0 failed.
- Command: `cargo test --test migration_contract`
- Result: 17 passed, 0 failed.

Full-suite GREEN:

- Command: `cargo test`
- Result: 152 passed, 0 failed across unit, integration, and documentation targets.

### Lifecycle race and API design

- `ApplicationService` is now exclusively the owner type. It owns `BootstrapState`, so `finish`, `installation_id`, and `session_id` are total owner-only methods with no optional-state panic path.
- `ApplicationService::worker` returns the public, restricted `ApplicationWorker`, which exposes only `execute_user` and `execute`. It cannot finish the session or access owner identity methods.
- Owner and worker delegate execution to one private `CommandExecutor`; no lifecycle or idempotency logic is duplicated.
- `execute_user` invokes the pre-read test boundary, acquires one shared lifecycle read guard, validates open state, invokes the post-read boundary, allocates command and correlation IDs, and moves that same guard into the complete transactional execution path. It never releases and reacquires the guard.
- `finish` invokes the deterministic pre-write test boundary and then takes the lifecycle write guard. Therefore it either closes first, causing exact `LifecycleFinished` rejection before user-command IDs, or waits until an already-started command has committed and released its read guard.
- The barrier-only race test proves both orderings without sleeps. Command-first consumes five IDs total: command ID, command correlation ID, command event ID, finish event ID, and finish correlation ID. Finish-first consumes only the two finish IDs; the rejected user command consumes zero IDs, policy calls, receipts, or command events.
- Workers created before finish still share the private lifecycle token and reject after finish, while all transaction-visible session, stale projection, receipt replay, rollback, and privacy checks remain unchanged.

### Fingerprint schema decision

- The receipt fingerprint CHECK now requires `typeof(command_fingerprint) = 'text'`.
- It requires exactly 64 UTF-8 bytes through `length(CAST(command_fingerprint AS BLOB)) = 64`, avoiding SQLite text-length ambiguity at embedded NUL.
- It independently rejects embedded NUL with `instr(command_fingerprint, char(0)) = 0`.
- It retains the lowercase ASCII hexadecimal domain check with `NOT GLOB '*[^0-9a-f]*'`.
- Exact DDL tests and insertion tests cover valid 64-byte lowercase hex, `64a + NUL + tail`, `63a + NUL`, uppercase, nonhex, multibyte, short, long, and 64-byte BLOB values.
- Because Phase 0 is prerelease and `0001_phase0.sql` changed in place, its checksum changed again. Development databases created against earlier Task 10 checksums can fail with `database_migration_state_invalid` and remain unsupported released formats.

### Scope

No runtime queue, terminal host, broker/network behavior, credential execution, external action, or approval execution was added.
