# Phase 0 Task 13 Report

## Commit

Original Task 13 documentation and contract-test commit:

`eca7622f75134ab0a483c1cbb457587ac0bddd3e`

Original evidence report commit:

`9acd4cb58fd0e4099a92573c4cc6ecc98a6f659c`

Fix round 1 implementation commit:

`270fa4f47f52f01abd871e6ff44358e7c8e3ba57`

## Changed files

- `README.md`
- `docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md`
- `docs/superpowers/plans/2026-08-09-ai-stock-forum-roadmap.md`
- `docs/superpowers/plans/2026-08-09-phase-1-deterministic-risk-core.md`
- `docs/superpowers/plans/2026-08-15-phase-0a-workspace-contract-foundation.md`
- `tests/documentation_contract.rs`
- `.superpowers/sdd/2026-08-31-phase-0-rust-foundation/task-13-report.md`

## TDD evidence

### RED

Command:

`cargo test --test documentation_contract --locked`

Result: expected failure. Both contract tests failed: the stale README did not
contain `phases.md`, and the four legacy documents did not contain the required
`SUPERSEDED - DO NOT EXECUTE` warning.

### GREEN

Command:

`cargo test --test documentation_contract --locked`

Result: pass, `2 passed; 0 failed`.

### Full suite

Command:

`cargo test --workspace --all-targets --locked`

Result: pass. All unit and integration test targets completed successfully
with no failures.

## Warning coverage

Each of the four named legacy documents begins with the source-text warning
`SUPERSEDED - DO NOT EXECUTE`, identifies the document as describing the retired
Python/React/Hermes architecture, and links to the canonical `architecture.md`
and active `phases.md`. Historical content after each inserted banner was
preserved.

## Concerns

- Windows-specific source paths and static contract coverage exist, but Windows
  runtime verification was not performed and is not claimed.
- Guided setup, the full-screen TUI, agent/provider integrations, live data,
  network and credential integrations, broker connectivity, trading behavior,
  and other later-phase capabilities remain explicitly deferred.
- The documentation contract checks the required precedence markers and core
  README anchors; the full suite supplies the implementation behavior evidence.

## Fix round 1 evidence

The strengthened contract was written first and run before documentation
updates.

### RED

Command:

`cargo test --test documentation_contract --locked`

Result: expected failure, `1 passed; 2 failed`. The new README canonical-link
assertion and canonical-design receipt-inventory assertion failed because the
old documentation had not yet been updated; the existing legacy-banner test
passed.

### GREEN

Command:

`cargo test --test documentation_contract --locked`

Result: pass, `3 passed; 0 failed`.

### Full suite

Command:

`cargo test --workspace --all-targets --locked`

Result: pass. All unit and integration test targets completed successfully
with no failures.

The README now has the exact canonical design link, platform-specific state,
database, and lock locations, schema versions, six CLI forms with effects and
continuation, precise privacy behavior, receipt/event roles, and non-goals.
The canonical design now records immutable command receipts and ordered event
references in schema version 1 before release, including the pre-receipt
development-database compatibility ruling. The contract rejects the stale
approved-spec link, absolute local paths, and prototype run directions.

## Fix round 2 evidence

Fix round 2 implementation commit:

`a85c49fe3582250954384f2bfaa0d4d7348e7cdc`

### RED

Command:

`cargo test --test documentation_contract --locked`

Result: expected failure, `2 passed; 1 failed`. The strengthened README
contract rejected the old `%LOCALAPPDATA%` path before the documentation was
corrected.

### GREEN

Command:

`cargo test --test documentation_contract --locked`

Result: pass, `3 passed; 0 failed`.

### Full suite

Command:

`cargo test --workspace --all-targets --locked`

Result: pass. All unit and integration test targets completed successfully
with no failures.

Round 2 documents the implemented Windows locations under `%APPDATA%`, not
`%LOCALAPPDATA%`. It states the exact privacy split: persisted rejection
metadata may include a bounded escaped first token, category, byte count, and
SHA-256 digest; audit rendering may show only the category, bounded safe token,
and byte count, never the digest or rejected full line; users must not enter
secrets. The contract asserts every complete CLI behavior row, including
`SetupStatusViewed` and exact `Shutting down.` output for `/quit`.
