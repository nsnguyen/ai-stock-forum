# Phase 0 Task 6 Report: SQLite migrations and startup checks

## Implementation commit

`5217515f327b5c34dbfc272d410d0a53356d6906` - `feat: add phase 0 sqlite migrations`

## Changed files

- `Cargo.toml`
- `Cargo.lock`
- `migrations/0001_phase0.sql`
- `src/config/mod.rs`
- `src/persistence/mod.rs`
- `src/persistence/database.rs`
- `src/persistence/migrations.rs`
- `tests/migration_contract.rs`

## TDD evidence

| Stage | Command | Result |
| --- | --- | --- |
| Dependency | `cargo add rusqlite --features bundled` | Added `rusqlite` 0.40.2 with bundled SQLite and updated the lockfile. Initial sandboxed registry access failed DNS resolution; the approved rerun completed. |
| RED | `cargo test --test migration_contract --locked` | Failed as expected with `E0432`: `Database` and `LATEST_SCHEMA_VERSION` were not exported by `ai_stock_forum::persistence`. |
| GREEN | `cargo test --test migration_contract --locked` | Passed: 5 passed, 0 failed. |
| Full suite | `cargo test --workspace --all-targets --locked` | Passed: 51 passed, 0 failed across all current unit and integration targets. |

Two intermediate focused runs informed minimal corrections: the first exposed an internal visibility compilation error for `AppliedMigration`; the second showed that enabling WAL before checking the application ID modified a foreign database. The final implementation performs the compatibility checks before write-capable pragma configuration.

## Migration and startup decisions

- Embedded ordered migration `0001_phase0.sql` creates the specified Phase 0 tables, append-only/immutable triggers, constraints, and indexes exactly once.
- The migration runner bootstraps a strict `schema_migrations` table, records the SHA-256 of each embedded SQL migration, and validates every applied record on reopen.
- SQLite application ID `0x4149_4653` (`AIFS`) and `user_version = 1` are set in the same `BEGIN IMMEDIATE` transaction as migration application.
- Startup rejects foreign application IDs, schemas newer than version 1, checksum mismatches, invalid migration state, and malformed SQLite files with typed stable error codes that do not include filesystem paths or database contents.
- Every opened connection explicitly enables foreign keys, WAL journaling, full synchronous durability, and a 5-second busy timeout; `PRAGMA quick_check(1)` runs after migrations.
- `Database::open` delegates directory/database creation and mode correction to the existing Task 5 `AppPaths::ensure` descriptor-safe boundary. It adds no symlink-following permission operation or path-based permission change.
- No event append/query, audit, projection, recovery, or Task 7 repository behavior was added.

## Concerns

None outstanding for Task 6. The migration test database fixtures are isolated with `tempfile`; future task work should retain the startup compatibility checks before introducing repositories that mutate `event_stream`.

## Fix round 1

### Implementation commit

`bb997022f064900b638b33ab3ea4a15d25851767` - `fix: harden sqlite startup checks`

### Changed files

- `src/config/mod.rs`
- `src/config/paths.rs`
- `src/persistence/database.rs`
- `tests/migration_contract.rs`

### Controller ruling and residual risk

The approved Phase 0 boundary is descriptor-anchored/no-follow creation and
validation from Task 5, a private `0700` state directory and `0600` database,
and `SQLITE_OPEN_NOFOLLOW` for SQLite's terminal database component. The
implementation does not claim same-UID intermediate-component replacement
resistance at SQLite-open time and does not implement a custom SQLite VFS.

Residual risk: a malicious process running as the same OS user could race an
intermediate path and redirect SQLite/WAL operations. A custom SQLite VFS is
outside the approved Phase 0 plan and current `rusqlite` surface.

On macOS, SQLite's no-follow check also rejects the operating system's
`/var -> /private/var` alias. `AppPaths::sqlite_open_path` normalizes only that
well-known alias before SQLite opens; it does not change permissions or claim
to eliminate the residual intermediate-path risk.

### Fix details

- `Database::open` retains Task 5's descriptor-safe path creation/validation,
  opens SQLite with `SQLITE_OPEN_NOFOLLOW | SQLITE_OPEN_EXRESCODE`, and maps
  only SQLite's `SQLITE_CANTOPEN_SYMLINK` extended result to the stable
  `database_terminal_path_rejected` startup error.
- SQLite connection startup reads back and requires `foreign_keys = 1`,
  `journal_mode = wal`, `synchronous = FULL` (SQLite value `2`), and
  `busy_timeout = 5000`; a mismatch returns `database_pragma_mismatch`.
- Applied migration records must be exactly contiguous from `1` through
  `user_version`, with the exact compiled checksum for each version. Checksum,
  hole, ahead-record, and count/version disagreement all return
  `database_migration_state_invalid` before pending migrations run.
- Migration execution remains one immediate transaction. A narrow injected
  failing SQL migration proves that schema creation and application/user version
  updates roll back together.

### TDD and verification evidence

| Stage | Command | Result |
| --- | --- | --- |
| RED | `cargo test --lib --test migration_contract --locked` | Failed as expected before the new seams existed: missing `verify_connection_pragmas`, `open_with_before_sqlite_open`, and `run_migrations_with`. |
| GREEN | `cargo test --lib --test migration_contract --locked` | Passed: 3 internal persistence tests and 12 migration contract tests, 0 failures. |
| Full suite | `cargo test --workspace --all-targets --locked` | Passed: 61 tests, 0 failures. |

The focused tests cover deterministic terminal replacement at the SQLite-open
boundary, pragma mismatch/readback, exact schema columns, constraints, foreign
keys, indexes, immutable triggers, migration row/checksum and inconsistent
states, immediate-transaction rollback, and Unix database mode correction.

## Fix round 2

### Implementation commit

`1cf3ac0401f2943f21393f2cef3b2f3f2625eb36` - `test: assert exact sqlite schema contract`

### Changed files

- `tests/migration_contract.rs`

### Scope

Added a deterministic exact-schema contract rather than production behavior.
The test rejects application-object drift and verifies all non-internal table,
index, and trigger objects; every column's name, declared type, nullability,
default, and primary-key position; every foreign-key parent/from/to and action;
every explicit index's uniqueness, ordered key columns, and normalized
definition; every immutable trigger's target, operation, and normalized
definition; and `STRICT` table declarations.

Fresh isolated databases behaviorally reject every specified CHECK, UNIQUE,
foreign-key, and immutable-trigger operation. The `installation_id` uniqueness
case temporarily bypasses its singleton CHECK only to make that otherwise
unreachable unique constraint independently observable. SQL normalization is
limited to case and whitespace for definitions SQLite exposes only as text.

### TDD and verification evidence

| Stage | Command | Result |
| --- | --- | --- |
| RED | `cargo test --test migration_contract task_six_schema_contract_is_exact_and_every_constraint_is_enforced --locked` | Failed as expected because the exact catalog and exhaustive behavioral assertion helpers did not exist. |
| GREEN | `cargo test --test migration_contract --locked` | Passed: 13 migration contract tests, 0 failures. |
| Full suite | `cargo test --workspace --all-targets --locked` | Passed: 62 tests, 0 failures. |

## Fix round 3 (final allowed round)

### Implementation commit

`44a40172205f0aa2a8b73bba330059f9dad3d8ee` - `test: strengthen sqlite schema oracle`

### Changed files

- `tests/migration_contract.rs`

### Scope

The schema oracle now derives the complete semantic index set from `PRAGMA
index_list` and `PRAGMA index_xinfo`, including SQLite-generated autoindexes.
It compares origin, uniqueness, partial flag, and ordered key columns for every
application table while intentionally ignoring generated index names. A focused
fixture with an extra anonymous `UNIQUE` constraint proves the oracle rejects
drift that the prior `sqlite_%` catalog filter could not observe.

Accepted-value tests now exercise every member of each enumerated CHECK domain:
setup state and path, setup-step status, capability-readiness status, and every
approval status with the required pending/terminal companion fields. Existing
negative probes remain in place. Index and trigger SQL checks now compare a
punctuation-aware token sequence that ignores whitespace and case but preserves
semantic punctuation and quoted literals.

### TDD and verification evidence

| Stage | Command | Result |
| --- | --- | --- |
| RED | `cargo test --test migration_contract semantic_index_oracle_detects_an_extra_autoindex_constraint --locked` | Failed as expected because the semantic-index and accepted-domain helpers did not exist. |
| GREEN | `cargo test --test migration_contract --locked` | Passed: 15 migration contract tests, 0 failures. |
| Full suite | `cargo test --workspace --all-targets --locked` | Passed: 64 tests, 0 failures. |

An initial combined-filter invocation used a literal `|` and ran zero tests;
it was immediately replaced by the complete migration-contract command above,
which executed all 15 focused tests.

## Round 3 review correction

### Implementation commit

`734436fdb34616b3ca5ccb628d4017c9967620f3` - `test: preserve sql literal case`

### Changed files

- `tests/migration_contract.rs`

### Correction

The punctuation-aware SQL tokenizer previously lowercased complete quoted
string-literal tokens, which made semantically distinct SQLite literals compare
equal. It now preserves quoted tokens verbatim, including doubled-quote escape
handling, while continuing to lowercase unquoted SQL words/identifiers and
ignore whitespace outside tokens. The regression proves different literal case
remains distinct and keyword/identifier case plus punctuation whitespace remain
equivalent. No production schema or behavior changed.

### TDD and verification evidence

| Stage | Command | Result |
| --- | --- | --- |
| RED | `cargo test --test migration_contract sql_token_normalization_preserves_literal_case_but_ignores_sql_formatting --locked` | Failed as expected: differently cased quoted literals normalized to the same token. |
| GREEN | `cargo test --test migration_contract --locked` | Passed: 16 migration contract tests, 0 failures. |
| Full suite | `cargo test --workspace --all-targets --locked` | Passed: 65 tests, 0 failures. |
