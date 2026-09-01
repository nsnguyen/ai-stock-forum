# ai-stock-forum

AI Stock Forum is currently a local Rust foundation for a single-process,
line-oriented terminal command host. Phase 0 establishes the typed event core,
SQLite persistence, startup/recovery lifecycle, audit inspection, and the
fallback command adapter. It does not run agents or perform market or trading
work.

## Sources of truth

- [Architecture](architecture.md)
- [Delivery phases](phases.md)
- [Approved design specification](docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md)
- [Phase 0 implementation plan](docs/superpowers/plans/2026-08-31-phase-0-rust-foundation.md)

The architecture and delivery phases are canonical for the current Rust
implementation. Older documents under `docs/superpowers/` are retained as
historical context and are explicitly marked as superseded.

## Phase 0 scope

- Rust `1.98.0`, pinned by `rust-toolchain.toml` and required by `Cargo.toml`.
- One foreground process with a bounded, line-oriented fallback command host.
- Typed commands and application events with append-only, hash-linked audit
  records and replayable projections.
- SQLite persistence with database schema version `1` and event schema version
  `1`, including ordered migrations and startup integrity checks.
- Per-user state discovery, a single-process guard, installation identity, and
  resumable process-session bookkeeping.
- Defensive parsing and rendering for bounded input, malformed commands, and
  audit output.

## Build, run, and test

```bash
cargo build --workspace --locked
cargo run --locked
cargo test --workspace --all-targets --locked
```

The program reads one command per line. `/quit` ends the session cleanly; end
of input and an interrupt also end the foreground session with an explicit
shutdown reason. The default test suite is deterministic and does not require
network access.

## Supported commands

The Phase 0 command surface is:

- `/help`
- `/status`
- `/setup status`
- `/audit tail [limit: 1-100]`
- `/quit`

Audit output defaults to 20 entries and accepts a limit from 1 through 100.
Guided setup is not implemented in Phase 0; `/setup status` reports that state
without applying configuration.

## Storage, security, and privacy

Startup discovers a platform-appropriate per-user application data directory
and stores `ai-stock-forum.sqlite3` there. On Unix, the state directory is
owner-only (`0700`) and the database is a regular owner-only file (`0600`).
The database uses ordered migrations, SQLite integrity checks, an immutable
event stream, and projections rebuilt from that stream.

Phase 0 has no provider, broker, market-data, external-runtime, credential,
OAuth, or network integration. It does not ingest real account exports,
market data, secrets, or personal profiles. Raw rejected command input is not
stored as a transcript; rejection records use a category, byte count, bounded
safe token where available, and an input digest.

## Startup and sessions

On startup the application creates or resumes its local state, applies the
known migrations, validates the database, acquires the process guard, ensures
an installation identity exists, and starts a new process session. If an older
session has no terminal event, the application records that interruption once
and prints a warning for the next run. `/status` reports `Installation: ready`
and `Session: active` during a healthy session.

There is no Phase 0 daemon or background service. The process returns success
for normal command-host completion and returns the failure exit code for
startup, runtime, or UI errors. Error output is intentionally summarized and
does not expose local paths or sensitive values.

## Platform status

The current verification context is Unix/macOS. Windows-specific source paths
and static contract coverage are present, but Windows runtime verification has
not been performed for this phase and is not claimed here.

## Explicit non-goals

Phase 0 does not include the full-screen TUI, agent orchestration, model
providers, live data, network access, credential entry, OAuth, MCP, external
runtimes, broker connectivity, order placement, trading recommendations,
financial calculations, guided setup application, web or mobile clients,
multi-user access, remote access, or an autonomous/background service.

## Quality gates

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --locked
cargo build --workspace --locked
```
