# Phase 0 Task 5 Report

Implementation commit: `4d4b3d7bc3a24d531784059ffad3e49220c06b9a`

## Changed files

- `Cargo.toml`
- `Cargo.lock`
- `src/config/mod.rs`
- `src/config/paths.rs`
- `tests/config_contract.rs`

## Test evidence

RED command:

```text
cargo test --test config_contract --locked
```

Result: after fetching the newly locked dependency, compilation failed as
expected because `AppPaths` and `StartupError` did not exist in `config`.

GREEN command:

```text
cargo test --test config_contract --locked
```

Result: PASS, 7 tests passed.

Full-suite command:

```text
cargo test --workspace --all-targets --locked
```

Result: PASS, 42 tests passed across the workspace with no failures.

## Decisions

- `AppPaths::discover` uses `directories::BaseDirs::data_dir()` and the
  `ai-stock-forum` child; tests use only explicit `for_test` roots.
- `ensure` creates or validates the state directory, applies Unix mode `0700`,
  creates or validates the database file, and applies Unix mode `0600`.
- `symlink_metadata` plus regular-type checks reject symlinks, directories,
  and special files before permission changes.
- `StartupError` is typed and exposes only stable safe messages; filesystem
  paths, environment values, and file contents are not included.

## Concerns

- The initial dependency fetch and first RED attempt required network approval
  because the sandbox could not resolve crates.io; the required RED command was
  rerun successfully after the locked crates were fetched.
- `ensure` creates an empty database path to establish its owner-only mode;
  SQLite opening, schema work, and database startup remain intentionally out
  of scope for Task 5.
