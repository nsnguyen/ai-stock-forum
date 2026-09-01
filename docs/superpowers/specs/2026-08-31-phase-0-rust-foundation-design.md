# Phase 0 Rust Foundation Design Specification

**Status:** Approved on 2026-08-31

**Canonical inputs:** [architecture.md](../../../architecture.md) and
[phases.md](../../../phases.md)

## 1. Purpose

Phase 0 establishes the smallest durable Rust application core that later
phases can extend without moving business rules into a terminal renderer. It
must accept typed commands, commit typed and redacted events to SQLite, rebuild
projections after restart, and expose a line-oriented fallback interface that
does not require a full-screen terminal.

The implementation replaces the incomplete, uncommitted JSONL prototype. No
JSONL compatibility migration is required because that prototype was never an
approved or released storage contract. Existing prototype data is not deleted.

## 2. User-visible result

Starting the executable in Phase 0 opens fallback command mode. The user can
run:

- `/help`
- `/status`
- `/setup status`
- `/audit tail`
- `/audit tail N`
- `/quit`

Restarting preserves the installation identity and event history. `/setup
status` clearly reports that guided setup is not implemented and that no
configuration has been applied. Unknown, malformed, oversized, and
control-character input cannot panic the process or expose raw input in durable
state.

## 3. Scope

Phase 0 includes:

- Rust toolchain, formatting, linting, testing, and locked-build policy;
- one Rust modular monolith and compile-time boundaries for future modules;
- a fallback command parser and text renderer;
- typed application commands, outcomes, events, errors, identifiers, clocks,
  object references, versions, and digests;
- bounded in-process command and outcome channels;
- SQLite discovery, permissions, migrations, transactions, append-only events,
  and rebuildable projections;
- installation identity and process-session recovery;
- setup draft, configuration version, setup-step outcome, and readiness
  skeletons;
- deny-wins capability evaluation and typed approval-record skeletons;
- deterministic fake clocks and ID generators;
- README correction and explicit superseded warnings on legacy plans; and
- offline unit, integration, recovery, and fallback-adapter tests.

Phase 0 explicitly excludes:

- a full-screen TUI renderer;
- guided setup mutation or configuration application;
- agents, model providers, rooms, discussions, MCP activation, memory, skills,
  engineering jobs, Git promotion, and finance behavior;
- credentials, secret input, network calls, subscriptions, and paid services;
- Python, Node, browser, daemon, VM, broker, merge, or push behavior; and
- migration or deletion of data created by the unapproved JSONL prototype.

## 4. Considered approaches

### 4.1 Canonical vertical slice - selected

Implement the complete Phase 0 boundary through SQLite, projections, fallback
commands, recovery, documentation, and tests. This is the only approach that
satisfies the approved roadmap exit gate while preserving a narrow scope.

### 4.2 Minimal command core - rejected

Implement command parsing and an event file, then defer migrations, recovery,
permissions, and setup/policy skeletons. This is faster initially but fails the
Phase 0 exit gate and would force Phase 1 to replace its foundation.

### 4.3 Broad future-platform scaffolding - rejected

Implement detailed interfaces for agents, rooms, providers, MCP, jobs, and
finance before those behaviors exist. This creates speculative abstractions and
pulls later-phase policy decisions into Phase 0.

## 5. Architecture

The application is one Rust process with explicit internal ports:

```text
line input
    -> fallback parser
    -> bounded typed command port
    -> application service
    -> deny-wins policy check
    -> SQLite transaction
    -> append-only event plus projection update
    -> bounded typed outcome port
    -> fallback text renderer
```

The presentation adapter owns parsing and rendering only. It cannot open the
database, execute migrations, inspect repositories, update projections, or
evaluate policy. The application service is the sole command handler. It
returns typed view data rather than preformatted text.

The application service has a synchronous headless API for focused tests. A
bounded runtime wraps that service for the executable and future TUI. Both
paths execute the same handler; the runtime is not a second business-logic
implementation.

## 6. Module boundaries

The initial layout is:

```text
src/
|-- main.rs                    process startup and exit status only
|-- lib.rs                     public module boundary
|-- app/                       commands, service, outcomes, ports
|-- runtime/                   bounded command/outcome worker
|-- ui/
|   |-- command/               line parser, runner, text renderer
|   `-- tui/                   documented placeholder boundary
|-- domain/                    IDs, clocks, actors, versions, digests, errors
|-- config/                    application-directory discovery
|-- persistence/               SQLite connection, migrations, repositories
|-- audit/                     redacted audit event views and queries
|-- recovery/                  event verification and projection rebuild
|-- setup/                     inert setup/readiness contract skeletons
|-- policy/                    capabilities, deny-wins rules, approvals
|-- agents/                    documented placeholder boundary
|-- rooms/                     documented placeholder boundary
|-- providers/                 documented adapter boundary
|-- runtimes/                  documented adapter boundary
|-- skills/                    documented placeholder boundary
|-- memory/                    documented placeholder boundary
|-- mcp/                       documented adapter boundary
|-- jobs/                      documented placeholder boundary
`-- domains/
    `-- finance/               documented placeholder boundary

migrations/                    ordered embedded SQLite migrations
tests/                         cross-module and acceptance tests
```

Placeholder modules contain only module documentation or boundary traits that
Phase 0 actually consumes. They do not define speculative domain models.

## 7. Typed command boundary

### 7.1 Commands

`ApplicationCommand` contains:

- `ShowHelp`
- `ShowStatus`
- `ShowSetupStatus`
- `ShowAuditTail { limit }`
- `RejectInput { category, safe_token, byte_length, input_digest }`
- `RequestShutdown`

The fallback parser maps complete input into one of these variants. A rejected
input remains a typed application command so rejection and redaction behavior
are owned and audited by the application service.

### 7.2 Grammar

- Leading and trailing whitespace is ignored.
- Internal command separators are one or more Unicode whitespace characters.
- Blank lines are ignored by the presentation adapter and create no event.
- `/audit tail` defaults to 20 records.
- `/audit tail N` accepts an unsigned decimal integer from 1 through 100.
- Missing, extra, or invalid arguments produce a typed `Malformed` rejection.
- Unknown command names produce a typed `Unknown` rejection.
- Input above 4096 bytes produces a typed `Oversized` rejection after the
  adapter discards the remainder of that line.
- Invalid UTF-8 produces a typed `InvalidEncoding` rejection.
- Control characters are never reflected literally in rendered output.
- `/quit` is the only command alias for shutdown in Phase 0.

The parser never places the complete rejected line in `ApplicationCommand`.
For an unknown command it retains only a bounded, escaped command token, the
original byte length, and a SHA-256 digest.

### 7.3 Outcomes and rendering

`CommandOutcome` contains the command and correlation IDs, committed events,
one typed view model, and a shutdown disposition. View variants cover help,
status, setup status, audit tail, input rejection, and shutdown.

The renderer converts those variants into plain text. Repository records and
raw event payload JSON are not presentation types.

## 8. Bounded runtime

The default runtime uses bounded request and outcome queues with capacity 32.
Capacity is injectable so tests can exercise backpressure with capacity 1.

Each request contains a `CommandEnvelope` and a one-command response path. One
application worker serializes writes. The worker sends an outcome only after
the corresponding SQLite transaction commits. If the transaction fails, it
returns a typed error and publishes no committed event.

Phase 0 does not add Tokio. A small standard-thread runtime or bounded channel
crate is sufficient. Phase 1 may replace the host mechanics while retaining
the command, event, service, and projection contracts.

## 9. Domain primitives

Production and deterministic test implementations are provided for:

- `Clock`, returning UTC Unix epoch milliseconds;
- `IdGenerator`, returning stable UUID-form identifiers;
- `InstallationId`, `SessionId`, `CommandId`, `EventId`, `CorrelationId`,
  `CausationId`, `ApprovalId`, and setup/configuration IDs;
- `ObjectVersion`, a positive integer newtype;
- `Sha256Digest`, validated as lowercase hexadecimal; and
- `Actor`, limited in Phase 0 to `Human` and `System`.

IDs are opaque and never derived from process IDs, paths, or timestamps. Tests
inject ordered IDs and timestamps. Production IDs come from an operating-system
random source.

## 10. Event model

The normalized event envelope contains:

- global sequence number;
- event ID and event schema version;
- typed event name and typed redacted payload;
- actor;
- UTC timestamp;
- correlation ID and optional causation ID;
- optional object kind, ID, version, and digest;
- previous event digest; and
- current event digest.

The current digest is SHA-256 over a versioned canonical representation of all
preceding immutable fields, including the previous digest. Sequence numbers and
the digest chain are allocated and written inside one transaction. Startup
verifies contiguous sequence numbers, payload decoding, event schema support,
and the complete digest chain before trusting projections.

Phase 0 event variants are:

- `InstallationInitialized`
- `ProcessSessionStarted`
- `PreviousSessionInterrupted`
- `HelpViewed`
- `StatusViewed`
- `SetupStatusViewed`
- `AuditTailViewed`
- `CommandRejected`
- `ShutdownRequested`
- `ProcessSessionEnded`
- `ProjectionRebuilt`

Event payloads are closed typed structures. They cannot accept arbitrary maps.
Unknown input payloads contain only the rejection category, safe token, byte
length, and digest.

One input command receives one correlation ID. All events committed for that
command share it. Startup and recovery work use explicit system correlations.

## 11. SQLite design

SQLite is the only Phase 0 source of truth. The first migration creates:

- `schema_migrations`;
- `event_stream`;
- append-only update/delete rejection triggers for `event_stream`;
- `installation_projection`;
- `process_session_projection`;
- `projection_metadata`;
- `setup_drafts`;
- `installation_configuration_versions`;
- `active_installation_configuration`;
- `setup_step_outcomes`;
- `capability_readiness`;
- `approval_records`; and
- constraints and indexes needed by the Phase 0 queries.

Setup and approval tables establish identifiers, schema versions, immutable
version links, state columns, exact object/review digests, actor fields, and
timestamps. Phase 0 exposes no command that inserts a setup draft, applies a
configuration, changes readiness, or accepts an approval.

`/setup status` returns `NotStarted` when no setup rows exist. It may append the
redacted `SetupStatusViewed` audit event, but it must not create a draft,
configuration version, active pointer, step outcome, readiness row, or approval.

## 12. Migrations and database startup

Migrations are ordered SQL files embedded in the binary. Every migration has a
compiled checksum. Startup:

1. discovers or creates the application state directory;
2. opens SQLite without deleting or recreating existing files;
3. enables foreign keys and the selected durability pragmas;
4. rejects a foreign SQLite application ID or schema newer than the binary;
5. verifies checksums for applied migrations;
6. applies each pending migration in its own immediate transaction;
7. updates the SQLite application and user versions atomically;
8. runs a bounded integrity check; and
9. verifies the authoritative event stream.

A failed migration rolls back. A checksum mismatch, unsupported event version,
newer schema, malformed database, or corrupt authoritative event produces a
typed startup error. Startup never guesses, skips an event, or silently creates
a replacement database.

## 13. Configuration discovery and permissions

Production paths use the operating system's application-data base directory
with an `ai-stock-forum` child:

- macOS: `~/Library/Application Support/ai-stock-forum`;
- Linux: `$XDG_DATA_HOME/ai-stock-forum`, falling back to
  `~/.local/share/ai-stock-forum`; and
- Windows: the roaming application-data directory plus `ai-stock-forum`.

Tests inject an explicit temporary path through library constructors; the
production binary does not need a test-only public command.

On Unix, the state directory is created or corrected to mode `0700`, and the
database is created or corrected to mode `0600`. SQLite sidecar files remain
inside the owner-only directory. Failure to establish required permissions is a
startup error.

The database filename is `ai-stock-forum.sqlite3`. Secrets are not configuration
and no Phase 0 schema or event accepts them.

## 14. Projections and recovery

The event stream is authoritative. Projections record the last applied event
sequence and digest. Startup compares those markers with the verified event
stream.

- A missing or stale projection is rebuilt transactionally from sequence one.
- A projection whose marker does not match the event chain is discarded and
  rebuilt.
- A corrupt projection is recoverable because it is not authoritative.
- A corrupt authoritative event stops recovery.
- The completed rebuild appends a `ProjectionRebuilt` event and advances the
  projection marker through that event.

Installation bootstrap creates exactly one installation ID. Reopening the same
database reuses it.

Every process run has a session projection. Startup checks the latest session:

- if it ended, startup begins a new session normally;
- if it has no end event, startup appends `PreviousSessionInterrupted`, invokes
  registered recovery hooks, and then begins the new session; and
- if recovery hooks fail, startup stops with an inspectable error.

`/quit`, clean input EOF, and handled process interruption append
`ProcessSessionEnded` with an explicit reason. The implementation never relies
on `Drop` alone to claim a clean shutdown.

Phase 0 recovery hooks receive the interrupted session ID and a transaction but
perform no later-phase workflow mutation. Future phases register their own
typed reconciliation logic at this boundary.

## 15. Policy and approval skeletons

The capability vocabulary includes safe Phase 0 capabilities for help, status,
setup-status read, audit read, and shutdown, plus named families for future
discussion, MCP, engineering, Git promotion, and finance actions.

Policy evaluation accepts explicit grants and denials and follows these rules:

1. any matching denial returns `Denied`;
2. otherwise, a matching grant returns `Granted`; and
3. absence of both returns `DeniedByDefault`.

Skills, personality, input text, event payloads, and future adapter output are
not policy sources.

`ApprovalRecord` binds an approval ID, action kind, exact object identity,
object version and digest, actor, status, creation time, optional expiry, and
resolution metadata. Phase 0 persists the schema and validates the type but
provides no sensitive approval action.

## 16. Audit behavior

Application events are the normalized append-only audit source. The audit
module maps typed events to typed, redacted audit rows and view models. `/audit
tail` reads the latest committed events in sequence order and includes the
current `AuditTailViewed` event.

Audit output shows sequence, timestamp, actor, event kind, correlation ID, and a
safe typed summary. It does not render raw payload JSON. If event persistence
fails, the command fails and no dependent state change commits.

## 17. Error model

Expected failures use typed error enums grouped into:

- startup and configuration discovery;
- migration and compatibility;
- persistence and transaction;
- event integrity and recovery;
- command validation;
- policy denial; and
- runtime/channel lifecycle.

Every user-facing error has a stable code and safe message. Internal sources
remain available to diagnostics without placing raw input, environment values,
credential material, or unredacted payloads in normal output.

User input, missing files, migration failures, database errors, policy denial,
and channel closure are not panic conditions.

## 18. Documentation transition

The README identifies `architecture.md` and `phases.md` as canonical, explains
that Phase 0 is the active Rust slice, documents the fallback commands, and
lists the quality gates.

The following historical files receive a top-level `SUPERSEDED - DO NOT
EXECUTE` warning that links to the canonical architecture and roadmap:

- `docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md`;
- `docs/superpowers/plans/2026-08-09-ai-stock-forum-roadmap.md`;
- `docs/superpowers/plans/2026-08-09-phase-1-deterministic-risk-core.md`; and
- `docs/superpowers/plans/2026-08-15-phase-0a-workspace-contract-foundation.md`.

The repository's existing ignore rules are preserved. Rust `target` output and
local application-state paths are added without replacing unrelated Python,
editor, test, or worktree rules.

## 19. Toolchain and dependency policy

Before code work, install Rust 1.98.0, the official stable release current on
2026-08-31, because the current machine has no Rust compiler or Cargo. Pin
`1.98.0` in `rust-toolchain.toml` and the matching `rust-version` package field.
Include `rustfmt` and `clippy` components.

`Cargo.lock` is committed. Runtime dependencies are limited to what Phase 0
uses directly: SQLite, serialization, structured errors, hashing, UUIDs,
platform directories, and bounded channels or signal handling where the
standard library is insufficient. SQLite may be bundled to avoid requiring a
separate system installation. Default tests remain offline after dependencies
are fetched.

## 20. Testing strategy

### 20.1 Unit tests

- complete command grammar and malformed-argument matrix;
- 4096-byte boundary, oversized lines, invalid UTF-8, control characters, and
  large unknown tokens;
- stable ID and fake-clock behavior;
- canonical event digest and digest-chain fixtures;
- event serialization round trips and unsupported schema versions;
- reducer behavior for every Phase 0 event;
- deny-wins, grant, and denied-by-default policy cases; and
- typed approval validation.

### 20.2 Persistence and migration tests

- fresh database and all required tables, indexes, and triggers;
- migration idempotency and checksum validation;
- rejection of a newer schema and foreign application ID;
- malformed/corrupt SQLite error path;
- append-only update/delete rejection;
- event and projection atomicity under an injected transaction failure;
- owner-only directory and database modes on Unix; and
- no setup/configuration/readiness/approval rows after setup-status reads.

### 20.3 Recovery tests

- stable installation identity across restart;
- clean session end does not report interruption;
- missing session end produces one interruption event on restart;
- stale, missing, and corrupt projections rebuild to the same state;
- event sequence gaps, payload corruption, and digest mismatch stop recovery;
  and
- rebuilt projection digest equals direct reduction of the authoritative stream.

### 20.4 Application and adapter tests

- every supported command follows parser -> application -> repository -> event
  -> projection -> typed view -> renderer;
- events for one command share its correlation ID;
- `/audit tail` default, explicit limit, lower bound, upper bound, and ordering;
- unknown and malformed input remain recoverable and redacted;
- `/setup status` reports `NotStarted` without inventing configuration;
- `/quit`, EOF, and handled interruption record clean termination; and
- scripted fallback sessions run without a real terminal.

### 20.5 Quality gates

The completed phase must pass:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --locked
cargo build --workspace --locked
```

No default test may require a network connection, provider account, paid
subscription, broker, terminal renderer, Python, Node, or browser.

## 21. Exit-gate traceability

| Roadmap exit condition | Required evidence |
|---|---|
| Canonical README and superseded legacy docs | Documentation tests/review and explicit file inventory |
| Fresh install and migration | Migration integration tests |
| Restart preserves installation identity/history | Recovery integration tests |
| Corrupt/incompatible database paths | Startup failure integration tests |
| Typed presentation -> app -> repository flow | Application and adapter acceptance tests |
| Events rebuild identical projections | Reducer/recovery parity tests |
| `/setup status` does not apply configuration | Application plus database assertions |
| Unknown/malformed input cannot panic | Parser corpus and fallback acceptance tests |
| Formatting, lint, tests, and build pass | Recorded quality-gate output |
| Offline Rust-only execution | Dependency inventory and offline test run |

## 22. Completion definition

Phase 0 is complete only when:

- every behavior in this specification is implemented test-first;
- all quality gates pass from a clean locked dependency graph;
- the fallback executable demonstrates the required commands and restart;
- the roadmap exit-gate table is backed by passing tests;
- no deferred subsystem behavior is present; and
- the implementation receives a final code review against this specification.
