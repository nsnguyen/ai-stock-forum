# ai-stock-forum

AI Stock Forum is currently a local Rust foundation for a single-process,
line-oriented terminal command host. Phase 0 establishes the typed event core,
SQLite persistence, startup/recovery lifecycle, audit inspection, and the
fallback command adapter. It does not run agents or perform market or trading
work.

## Sources of truth

- [Architecture](architecture.md)
- [Delivery phases](phases.md)
- [Approved design specification](docs/superpowers/specs/2026-08-31-phase-0-rust-foundation-design.md)
- [Phase 0 implementation plan](docs/superpowers/plans/2026-08-31-phase-0-rust-foundation.md)

The architecture and delivery phases are canonical for the current Rust
implementation. Older documents under `docs/superpowers/` are retained as
historical context and are explicitly marked as superseded.

## Phase 0 scope

- Rust `1.98.0`, pinned by `rust-toolchain.toml` and required by `Cargo.toml`.
- One foreground process with a bounded, line-oriented fallback command host.
- Typed commands and application events with append-only, hash-linked audit
  records, durable command receipts, and replayable projections.
- SQLite persistence with database schema version `1` and event schema version `1`,
  including ordered migrations and startup integrity checks. For audit and
  projections, events remain authoritative; receipts are durable
  command-idempotency evidence.
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

Each supported CLI form has one typed application effect:

| Form | Output/effect | Continuation |
| --- | --- | --- |
| `/help` | Outputs `Available commands:` followed by `/help`, `/status`, `/setup status`, `/audit tail [limit: 1-100]`, and `/quit`; commits `HelpViewed`. | Continues. |
| `/status` | Outputs exactly `Installation: ready` and `Session: active`; commits `StatusViewed`. | Continues. |
| `/audit tail` | Outputs `Audit tail (limit 20):` plus the selected entries or `No audit entries.`; commits `AuditTailViewed(limit=20)`. | Continues. |
| `/audit tail N` | Outputs `Audit tail (limit N):` plus the selected entries or `No audit entries.` for `N` from 1 through 100; commits `AuditTailViewed(limit=N)`. | Continues. |
| `/setup status` | Outputs exactly `Setup: not started` and `Guided setup is not implemented in Phase 0.` on a fresh installation; commits `SetupStatusViewed`. | Continues. |
| `/quit` | Outputs exactly `Shutting down.`; commits `ShutdownRequested` and ends the session with `UserQuit`. | Ends normally. |

Rejected input is also audited as a typed event and the command host continues.
Rejected full lines are not stored verbatim.
Fatal startup, runtime, or UI failures emit one safe summary and use the
failure exit code instead of continuing the command host.

## Storage, security, and privacy

The application uses `directories::BaseDirs` to discover its per-user data
directory. The exact default locations are:

| Platform | State directory | Database | Lock |
| --- | --- | --- | --- |
| macOS | `~/Library/Application Support/ai-stock-forum/` | `~/Library/Application Support/ai-stock-forum/ai-stock-forum.sqlite3` | `~/Library/Application Support/ai-stock-forum/phase0-bootstrap.lock` |
| Linux/XDG | `$XDG_DATA_HOME/ai-stock-forum/` or `~/.local/share/ai-stock-forum/` when `XDG_DATA_HOME` is unset | `$XDG_DATA_HOME/ai-stock-forum/ai-stock-forum.sqlite3` or `~/.local/share/ai-stock-forum/ai-stock-forum.sqlite3` | `$XDG_DATA_HOME/ai-stock-forum/phase0-bootstrap.lock` or `~/.local/share/ai-stock-forum/phase0-bootstrap.lock` |
| Windows | `%APPDATA%\ai-stock-forum\` | `%APPDATA%\ai-stock-forum\ai-stock-forum.sqlite3` | `%APPDATA%\ai-stock-forum\phase0-bootstrap.lock` |

The lock filename is `phase0-bootstrap.lock` on every platform.

On Unix, the state directory is owner-only (`0700`) and the database and lock
are regular owner-only files (`0600`). The database uses ordered migrations,
SQLite integrity checks, an immutable event stream, immutable command receipts
and ordered command-event references, and projections rebuilt from the event
stream. In Phase 0, events remain authoritative for audit and projections;
receipts are durable command-idempotency evidence.

Privacy warning: users must not enter secrets; Phase 0 has no supported secret, credential, or profile workflow.

On rejection, a bounded escaped first token, category, exact byte count, and SHA-256 digest may be persisted. Audit rendering may show the category, bounded safe token, and byte count; the digest and rejected full line are not rendered.

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
and static contract coverage are present, but the non-Unix process guard
currently refuses startup; Windows runtime verification has not been performed
for this phase and is not claimed here.

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
