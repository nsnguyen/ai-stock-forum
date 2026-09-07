# ai-stock-forum

AI Stock Forum is currently a local Rust foundation for a single-process
terminal application. Phase 0 establishes the typed event core, SQLite
persistence, startup/recovery lifecycle, audit inspection, and the fallback
command adapter. Phase 0B adds an interactive full-screen cockpit while
preserving that fallback. Phase 2 Agent Profiles Milestone 1 adds local,
versioned profile management. Phase 2 Declarative Skills Milestone 2 adds an
inert, versioned skill library and exact agent assignments without running
models, agents, or market work.

## Sources of truth

- [Architecture](architecture.md)
- [Delivery phases](phases.md)
- [Phase 2 Agent Profile Foundation design](docs/superpowers/specs/2026-09-05-phase-2-agent-profile-foundation-design.md)
- [Phase 2 Agent Profile testing guide](docs/testing/phase-2-agent-profile-foundation.md)
- [Phase 2 Declarative Skills design](docs/superpowers/specs/2026-09-05-declarative-skills-design.md)
- [Declarative Skills testing and workflow guide](docs/testing/declarative-skills.md)
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

## Phase 0B Adaptive Cockpit

Phase 0B is a read-only terminal presentation layer over the existing Phase 0
application boundaries. It adds no agent, market, trading, credential, or
network behavior. When both stdin and stdout are terminals, the default launch
opens the Adaptive Cockpit in the alternate screen; otherwise the existing
line-oriented command host is selected automatically.

The cockpit has four native, non-transcript views: Overview, Setup, Audit, and
Help. It requires at least `60x18` terminal cells. At matching height
thresholds it is Narrow from `60x18`, Medium from `80x24`, and Wide from
`120x30`; any smaller width or height uses the TooSmall guidance screen.

| Control | Result |
| --- | --- |
| Bare `1`-`4` | Open Overview, Setup, Audit, or Help from any non-text browsing pane or confirmation. |
| Bare `a` | Open Agents from any non-text browsing pane or confirmation. |
| Bare `s` | Open Skills from any non-text browsing pane or confirmation. |
| `?` | Open Help. |
| `Tab`, `Shift+Tab` | Move focus forward or backward among visible regions. |
| Arrow keys, `PageUp`, `PageDown`, `Home`, `End` | Navigate the focused view or Audit selection. |
| `i`, `Esc` | Open/focus the inspector; then dismiss the inspector or message. |
| `/` | Focus the command editor with `/` prefilled. |
| Command editor: text, `Enter`, arrows, `Home`, `End`, `Backspace`, `Delete`, `Up`, `Down`, `Tab`, `Shift+Tab`, `Esc` | Edit, submit, recall in-memory history, move focus, or cancel command entry. |
| Agent profile editor: `Up`, `Down`, `Enter`, `Esc` | Choose a template, accept the current field, advance, or go back without requiring colon controls. |
| `/quit` | Request the auditable normal shutdown from command entry, including the TooSmall screen. |
| `Ctrl+C` | Request emergency interrupted shutdown from any focus. |

`NO_COLOR=1` disables foreground and background colors while retaining
non-color focus distinction. Mouse capture remains disabled. Only one process
may use a state directory at once; a second process is rejected through the
existing single-instance guard. Bare `q` has no global shortcut behavior and is
ordinary text inside editors. Bare navigation keys are ordinary text while
command, profile, or skill text entry owns input. `/help`, `/status`, `/setup status`, `/audit tail`,
`/audit tail N`, `/quit`, and rejected input continue through the existing parser,
runtime, application, policy, event, audit, and persistence boundaries. Bare
`/audit` is rejected as malformed input.

Each tab retains its own focus, scroll position, unfinished input, selected
pane, editor draft, and pending confirmation while another tab is visible.
Switching tabs never submits or cancels the retained action.

See [the Phase 0B testing guide](docs/phase-0b-testing.md) for the manual
acceptance procedure, fallback behavior, restoration checks, and host-specific
verification record.

## Phase 2 Agent Profiles Milestone 1

Agent Profiles Milestone 1 is complete. It provides pinned Bull, Bear, Chief,
Engineering, and Custom templates; local create, list, detail, edit-review,
activation, and immutable history workflows; schema version 2 persistence;
restart recovery; Adaptive Cockpit views; and command-mode parity.

Profile bindings are typed references selected only from an application-supplied
catalog, never free-form provider or model labels. Readiness is computed as
`Unbound`, `Binding unavailable`, or `Ready` from the role's required bindings
and the current catalog snapshot. The production Milestone 1 catalog is empty,
so normal profiles remain unbound and display `Not Ready`; deterministic tests
inject catalogs to cover unavailable and ready states. Readiness never contacts
a provider, authorizes execution, or selects a fallback.

Every accepted create installs immutable version 1. An accepted edit first uses
a passive, local-only preview that writes no event, receipt, draft, profile row,
or audit record. Activation requires a separate explicit confirmation and
creates the next immutable version; earlier bytes and history remain unchanged.
Preview tokens are process-local, one-use review bindings and do not survive a
restart.

### Keyboard-first profile editor

In the Adaptive Cockpit, press bare `a` to open Agents, then press `c` to
create a profile. Bare `1`-`4`, `a`, and `s` work from non-text browsing panes
and confirmations; while command, profile, or skill text entry owns input,
those characters remain text.
Use `Up` and `Down` to choose a complete built-in template. Press `Enter` to
accept it, then press `Enter` on each prefilled field to keep its current value
and continue. Typing a replacement before `Enter` saves that replacement and
continues. `Esc` returns to the previous field or cancels from the first step.

On Create review, `Enter` opens a separate confirmation pane and a second
`Enter` creates the profile. On Edit review, the first `Enter` requests the
authoritative preview; after it appears, the next `Enter` opens activation
confirmation and one more `Enter` activates the immutable revision. Specialty
tag and optional binding controls remain available as advanced colon commands,
and all existing editor colon controls remain backward-compatible aliases.

Fallback line-command mode is unchanged: it retains explicit `:next`,
`:review`, `:create`, and `:activate` controls plus its exact typed confirmation
phrases.

The verified event stream is authoritative at recovery. A missing immutable
profile row may be backfilled from its verified event, but an altered row or an
unexpected extra row causes safe startup refusal. Recovery never updates or
deletes suspicious immutable history. Only the active-profile pointer is a
rebuildable projection.

Milestone 1 is prerelease software, so migration `0002_agent_profiles.sql` was
amended to its final shape. Databases at the released schema-v1 boundary upgrade
in place. Databases created by an intermediate Phase 2 development build must be
recreated rather than treated as a supported upgrade source.

Milestone 1 does not execute a model or agent. Skill execution, hybrid memory,
provider connections, model execution, MCP use, rooms, debates, market data,
engineering jobs, and trading behavior remain deferred. See the
[Phase 2 testing guide](docs/testing/phase-2-agent-profile-foundation.md) for
exact isolated TUI and fallback procedures.

## Phase 2 Declarative Skills Milestone 2

Declarative Skills Milestone 2 is complete. It provides four deterministic
built-ins, guided custom creation, immutable version history, exact pinned
agent assignments, deliberate historical assignment, explicit upgrade and
unassign reviews, schema version 3 persistence, recovery, and compact-to-wide
Adaptive Cockpit views. Skills are inert accepted context: they cannot execute
or grant shell, filesystem, Git, MCP, provider, browser, or network access.

Normal use is keyboard-first: press bare `s` to open Skills, navigate with
arrows, and use `Enter` and `Esc` through visible review and confirmation steps. Optional `/skill`
commands open the same typed workflows; mutation shortcuts stage review rather
than writing directly. Bare `q` remains inert and `/quit` remains normal
shutdown. Inference and chat remain deferred to Phase 3. See the
[Declarative Skills testing and workflow guide](docs/testing/declarative-skills.md)
for the exact workflow, isolated local commands, persistence checks, and manual
acceptance checklist.

## Build, run, and test

```bash
cargo build --workspace --locked
make dev
cargo run --locked
cargo run --locked -- --command-mode
printf '/status\n/quit\n' | cargo run --quiet --locked
cargo test --all-targets --all-features
```

`make dev` is a shorthand for `cargo run --locked`. The `--command-mode` launch
form always selects the fallback command host. The piped form demonstrates its
automatic redirected-stdin fallback. The fallback host reads one command per
line. `/quit` ends the session cleanly; end of input and an interrupt also end
the foreground session with an explicit shutdown reason. The default test suite
is deterministic and does not require network access.

`make dev` and `cargo run --locked` are normal application launches and use the
user's standard persistent app-state directory described below. They are
appropriate for normal use, but must not be used for destructive acceptance
experiments; use the isolated-state procedures in
[the Phase 0B testing guide](docs/phase-0b-testing.md) for those checks.

## Supported commands

Each supported command form has one typed application effect. `/agent` is the
canonical profile prefix in both hosts; command mode also accepts the equivalent
bare `agent` alias for compatibility:

| Form | Output/effect | Continuation |
| --- | --- | --- |
| `/help` | Outputs `Available commands:` followed by `/help`, `/status`, `/setup status`, `/audit tail [limit: 1-100]`, and `/quit`; commits `HelpViewed`. | Continues. |
| `/status` | Outputs exactly `Installation: ready` and `Session: active`; commits `StatusViewed`. | Continues. |
| `/audit tail` | Outputs `Audit tail (limit 20):` plus the selected entries or `No audit entries.`; commits `AuditTailViewed(limit=20)`. | Continues. |
| `/audit tail N` | Outputs `Audit tail (limit N):` plus the selected entries or `No audit entries.` for `N` from 1 through 100; commits `AuditTailViewed(limit=N)`. | Continues. |
| `/setup status` | Outputs exactly `Setup: not started` and `Guided setup is not implemented in Phase 0.` on a fresh installation; commits `SetupStatusViewed`. | Continues. |
| `/agent create` | Lists the five pinned templates and enters the local guided creator after selection. | Continues until confirmation or cancel. |
| `/agent create <bull\|bear\|chief\|engineering\|custom>` | Copies the exact pinned template into a local guided creator. | Continues until confirmation or cancel. |
| `/agent list` | Lists at most 100 active profiles with role, specialty, readiness, version, and ID. | Continues. |
| `/agent show <name-or-id>` | Shows the accepted active profile fields and immutable metadata. | Continues. |
| `/agent edit <name-or-id>` | Loads the active version into a local editor, previews an authoritative diff, and requires explicit activation confirmation. | Continues until confirmation or cancel. |
| `/agent history <name-or-id> [version]` | Shows bounded newest-first metadata, or the exact immutable version with complete accepted content and predecessor diff. | Continues. |
| `/skill list` or `/skills` | Lists the bounded local skill library with active exact versions and provenance. | Continues. |
| `/skill add` | Opens the local guided creator and stages review before version 1 can be confirmed. | Continues until confirmation or cancel. |
| `/skill show <name-or-id> [version]` | Shows the active or requested exact historical skill version. | Continues. |
| `/skill assign <skill> <agent> [version]` | Stages an assign or upgrade review for the displayed exact version; it does not mutate directly. | Continues until confirmation or cancel. |
| `/skill unassign <skill> <agent>` | Stages an unassign review for the agent's exact pin; it does not mutate directly. | Continues until confirmation or cancel. |
| `/quit` | Outputs exactly `Shutting down.`; commits `ShutdownRequested` and ends the session with `UserQuit`. | Ends normally. |

In fallback line-command mode, creation requires the exact phrase `create` and
revision activation requires `activate <review-digest>`. The Adaptive Cockpit
uses a separate visible confirmation pane where `Enter` confirms the pending
Create or Activate action. Recoverable submission errors retain the same draft,
review, and confirmation so the user can retry without reconstructing work.

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

For Phase 2 Milestones 1 and 2, profile and skill instructions are stored
locally as accepted content and appear only in their explicit detail/editor
views. Do not enter API keys or credentials; Phase 2 has no secret-storage or
provider-connection workflow. Generic audit entries and errors omit profile or
skill prose, provider material, and rejected hostile text.

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

Automated and static coverage exercises the supported platform paths. Live
terminal verification is host-specific and must be recorded separately. The
Phase 2 testing guide defines the exact record; no unperformed host action may
be inferred from automated coverage.
Windows runtime verification has not been performed for this milestone.

## Explicit non-goals

Phase 2 Milestones 1 and 2 do not add agent orchestration, skill execution,
hybrid memory, model execution, model providers, live or market data, rooms,
debates, network access, credential entry, OAuth, MCP, external runtimes,
broker connectivity, order placement, trading recommendations, financial
calculations, guided setup application, web or mobile clients, multi-user
access, remote access, or an autonomous/background service.

## Quality gates

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
# Retained Phase 0 compatibility gates:
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --locked
cargo build --workspace --locked
```
