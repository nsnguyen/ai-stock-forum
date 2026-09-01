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
