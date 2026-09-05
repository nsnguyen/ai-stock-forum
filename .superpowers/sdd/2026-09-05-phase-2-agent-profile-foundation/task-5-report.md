# Task 5 Report: Materialize Versions and Reconcile Recovery Without Rewriting History

## Status

Implemented and verified.

## TDD Evidence

### RED

Command:

```text
cargo test --test agent_profile_persistence_contract
```

Result: exit 101. Compilation failed because `insert_expected_version`, `load_all_versions`, `replace_active_profiles`, `PersistenceError::AgentProfileHistoryMismatch`, `PersistenceError::InvalidAgentProfilePayload`, and `AppError::code` did not exist.

Command:

```text
cargo test --test agent_profile_recovery_contract
```

Result: exit 101. Compilation failed because the profile repository functions and the four profile recovery error variants did not exist.

These were the expected feature-missing failures against the approved Task 4 baseline.

### GREEN

The first implementation run reached compilation and exposed two local implementation mistakes: readiness was requested from `AgentProfileVersion` instead of its bindings, and a matched-row index was dereferenced twice. Both focused commands exited 101 before running tests. Those compiler-identified mistakes were corrected without changing the contracts.

Command:

```text
cargo test --test agent_profile_persistence_contract
```

Result: exit 0; 5 passed, 0 failed.

Command:

```text
cargo test --test agent_profile_recovery_contract
```

Result: exit 0; 7 passed, 0 failed.

## Full Suite

Command:

```text
cargo test
```

Result: exit 0; 351 passed, 0 failed, 0 ignored across unit, integration, and documentation test targets.

## Files

- Created `src/persistence/agent_profile_repository.rs`
- Modified `src/persistence/mod.rs`
- Modified `src/persistence/database.rs`
- Modified `src/persistence/event_repository.rs`
- Modified `src/persistence/projection_repository.rs`
- Modified `src/recovery/coordinator.rs`
- Modified `src/recovery/reducer.rs`
- Modified `src/app/mod.rs`
- Modified `src/config/mod.rs`
- Created `tests/agent_profile_persistence_contract.rs`
- Created `tests/agent_profile_recovery_contract.rs`
- Created `.superpowers/sdd/2026-09-05-phase-2-agent-profile-foundation/task-5-report.md`

`database.rs`, `event_repository.rs`, and `config/mod.rs` required focused changes because they own the shared persistence/recovery enums and the exhaustive startup safe-code mapping.

## Self-Review

- Verified events remain authoritative. Reconciliation re-verifies and reloads the event snapshot inside an immediate transaction before deriving expected profile versions.
- Missing expected immutable rows are inserted only after mismatches and unexpected rows have been ruled out.
- Exact existing rows are compared across every stored column and canonical payload byte and remain untouched.
- Mismatched rows return `database_agent_profile_history_mismatch`; unexpected rows return `unexpected_agent_profile_history`.
- Production recovery contains no update, delete, or clear operation for `agent_profile_versions`.
- Active rows are replaced only after immutable reconciliation succeeds, in the same transaction.
- Active replacement failure rolls back both the replacement and any missing immutable-row backfill.
- Projection loading validates materialized active rows against the event-derived projection, and projection digests now cover agent profiles.
- Legacy event streams without profile events reconcile to empty immutable and active profile state.
- Persistence, recovery, startup, and application errors expose stable safe codes with fixed Display text that contains no profile prose or payload bytes.
- No application commands, schema changes, plan changes, or specification changes were added.

## Concerns

None.

## Commit

Commit message: `feat: reconcile durable agent profile history`

This report is included in that commit; the resulting commit hash is returned in the task response.
