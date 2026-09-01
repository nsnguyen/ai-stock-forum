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
