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

## Fix round 1

Base: `acdf6717f14b389e388d242aeeb91d2b67811931`

### Changed files

- `Cargo.toml`
- `Cargo.lock`
- `src/config/paths.rs`
- `tests/config_contract.rs`
- `.superpowers/sdd/2026-08-31-phase-0-rust-foundation/task-5-report.md`

### Test evidence

Initial RED command:

```text
cargo test --test config_contract --locked
```

Result: the intermediate-symlink test failed as expected, while the socket
fixture also failed with macOS sandbox `EPERM` before exercising the assertion.

Clean RED command after making the socket fixture skip only on that platform
restriction:

```text
cargo test --test config_contract --locked
```

Result: 10 tests passed and 1 test failed as expected: the existing
check-then-path implementation followed the intermediate symlink.

GREEN command:

```text
cargo test --test config_contract --locked
```

Result: PASS, 11 tests passed.

Full-suite command:

```text
cargo test --workspace --all-targets --locked
```

Result: PASS, 46 tests passed across the workspace.

### Decisions and portability

- Unix state and database handling now walks components through directory
  descriptors using `rustix` `openat`, `mkdirat`, `O_NOFOLLOW`, `fstat`, and
  `fchmod`; no Unix path-based chmod remains after validation.
- Existing and newly created database files are validated and chmodded through
  their opened descriptors, rejecting symlinks, directories, FIFOs, sockets,
  and other special files.
- The macOS system alias `/var -> /private/var` is normalized only for the
  descriptor walk so standard temporary test roots remain usable; all
  user-controlled intermediate symlinks are still rejected.
- No deterministic replacement-race test was added because the public API has
  no operation-boundary seam; descriptor anchoring removes the vulnerable
  path-based chmod boundary without introducing timing-dependent tests.

Fix commit: `2a9074e5fad9712f936a20bc81a12037680342a0`.
