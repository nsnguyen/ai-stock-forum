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
