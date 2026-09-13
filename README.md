# ai-stock-forum

AI Stock Forum is currently a local Rust foundation for a single-process
terminal application. Phase 0 establishes the typed event core, SQLite
persistence, startup/recovery lifecycle, audit inspection, and the fallback
command adapter. Phase 0B adds an interactive full-screen cockpit while
preserving that fallback. Phase 2 Agent Profiles Milestone 1 adds local,
versioned profile management. Phase 2 Declarative Skills Milestone 2 adds an
inert, versioned skill library and exact agent assignments without running
models, agents, or market work. Phase 2 Hybrid Memory Milestone 3 adds local
reviewed memory, durable agent proposals, source-qualified episodic summaries,
and bounded deterministic retrieval without adding inference or chat.

## Sources of truth

- [Architecture](architecture.md)
- [Delivery phases](phases.md)
- [Phase 2 Agent Profile Foundation design](docs/superpowers/specs/2026-09-05-phase-2-agent-profile-foundation-design.md)
- [Phase 2 Agent Profile testing guide](docs/testing/phase-2-agent-profile-foundation.md)
- [Two-pane shell and Agents local testing](docs/testing/two-pane-shell-agents.md)
- [Two-pane TUI design](docs/superpowers/specs/2026-09-12-two-pane-tui-design.md)
- [Phase 2 Declarative Skills design](docs/superpowers/specs/2026-09-05-declarative-skills-design.md)
- [Declarative Skills testing and workflow guide](docs/testing/declarative-skills.md)
- [Phase 2 Hybrid Memory design](docs/superpowers/specs/2026-09-07-phase-2-hybrid-memory-design.md)
- [Phase 2 Hybrid Memory testing guide](docs/testing/phase-2-hybrid-memory.md)
- [Approved design specification](docs/superpowers/specs/2026-08-31-phase-0-rust-foundation-design.md)
- [Phase 0 implementation plan](docs/superpowers/plans/2026-08-31-phase-0-rust-foundation.md)

The architecture and delivery phases are canonical for the current Rust
implementation. Older documents under `docs/superpowers/` are retained as
historical context and are explicitly marked as superseded.

## Try the current TUI

Quit any other running AI Stock Forum instance, then run this from the worktree:

```sh
make dev
```

This normal launch uses the existing local application data. It does not need a
new macOS account or a different home directory. Keep destructive persistence
and recovery acceptance away from normal state; the
[two-pane shell and Agents guide](docs/testing/two-pane-shell-agents.md) covers
the normal usability path, while the older
[Agent Profile Foundation guide](docs/testing/phase-2-agent-profile-foundation.md)
retains its specialized disposable-state procedure.

The shared two-pane shell and complete Agents experience are the current first
redesign slice. Automated gates and production-render previews pass; the
interactive `make dev` acceptance run has not been performed. See the testing
guide for the exact evidence record.

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

Phase 0B introduced native, non-transcript views. The current top navigation has
exactly nine destinations: Home, Chat, Agents, Skills, Connections, Activity,
Setup, Audit, and Help. Chat and Connections honestly remain Phase 3
placeholders. The cockpit requires at least `60x18` terminal cells. Widths from
60 through 99 show one logical pane; widths from 100 show a two-pane list and
workspace. Widths from 120 use the Wide density. Any smaller width or height
uses the TooSmall guidance screen.

| Control | Result |
| --- | --- |
| Bare `1`-`9` | Open Home, Chat, Agents, Skills, Connections, Activity, Setup, Audit, or Help from NAV. |
| `?` | Open Help. |
| `Tab`, `Shift+Tab` | Move focus to the next or previous enabled section, revealing its pane at compact widths. |
| `WASD` | Move through the focused NAV region; shifted uppercase WASD is equivalent. |
| Arrow keys, `PageUp`, `PageDown`, `Home`, `End` | Quiet navigation equivalents or bounded page movement. |
| `Esc` | Go back, cancel, or clear the current interaction. |
| `/` | Focus the command editor with `/` prefilled. |
| Command editor: text, `Enter`, arrows, `Home`, `End`, `Backspace`, `Delete`, `Up`, `Down`, `Tab`, `Shift+Tab`, `Esc` | Edit, submit, recall in-memory history, move focus, or cancel command entry. |
| Agent profile editor in NAV: `WASD`, `Tab`, `Shift+Tab`, `Enter`, `Esc` | Navigate fields and actions; `Enter` deliberately enters TYPE on a text field. |
| Agent profile editor in TYPE: text, `Enter`, `Esc`, `Tab`, `Shift+Tab` | Enter literal single-line text; retain it when returning to NAV or moving to another field. |
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
catalog, never free-form provider or model labels. The Agents TUI presents the
existing readiness states as `Needs connection`, `Connection unavailable`, and
`Bindings configured`. These labels describe configured catalog references
only: they never claim that a provider was contacted, a model can run, or a
fallback was selected.

Every accepted create installs immutable version 1. An accepted edit first uses
a passive, local-only preview that writes no event, receipt, draft, profile row,
or audit record. Activation requires a separate explicit confirmation and
creates the next immutable version; earlier bytes and history remain unchanged.
Preview tokens are process-local, one-use review bindings and do not survive a
restart.

### Keyboard-first profile editor

Press `3` to open Agents and use `W`/`S` to choose an agent. Press `Tab` to move
into the workspace, use `A`/`D` among Profile, Memory, Skills, and History, and
press `Enter` to open the selected choice. Normal browsing never requires a UUID
lookup. The visible Agents actions are `N` New, `E` Edit, and `H` History.

Profile fields begin in NAV. Select a field and press `Enter` deliberately to
enter TYPE. In TYPE, WASD, digits, slash, and a leading colon are literal input;
they do not navigate or invoke editor commands. `Esc` retains the exact text and
returns to NAV on that field. `Tab` retains it and moves to the next field in
NAV. Invalid raw input and its error remain available for correction. Profile
text is normalized to the existing single-line storage contract.

Leaving Agents suspends the draft; only the labeled Discard action abandons it.
Review and durable confirmation are separate steps. Final mutation requires a
deliberate `Enter` on the labeled confirmation action: `D` and a held or
repeated `Enter` cannot perform it. The fallback command-mode editor keeps its
existing colon grammar; that protocol is not the TUI prose parser.

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

Normal use is keyboard-first: press bare `4` to open Skills, navigate with
arrows, and use `Enter` and `Esc` through visible review and confirmation steps. Optional `/skill`
commands open the same typed workflows; mutation shortcuts stage review rather
than writing directly. Bare `q` remains inert and `/quit` remains normal
shutdown. Inference and chat remain deferred to Phase 3. See the
[Declarative Skills testing and workflow guide](docs/testing/declarative-skills.md)
for the exact workflow, isolated local commands, persistence checks, and manual
acceptance checklist.

## Phase 2 Hybrid Memory Milestone 3

Hybrid Memory is local per-agent memory nested at **Agents → Memory**. It
supports reviewed Human set, overwrite, and delete actions; durable Agent
proposals; distinct Human approval or rejection; bounded entry, history,
proposal, and episodic reads; and bounded deterministic retrieval for internal
application use. Direct Human edits are reviewed before commit. Agent-authored
changes become pending proposals: proposal creation is durable, but it cannot
create an accepted entry version without a distinct Human approval. Rejection
records the decision without modifying an entry.

The application does not encrypt Hybrid Memory at rest. Application-managed
records are local plaintext; owner-only permissions are access control, not
encryption. Entry and history rows, proposals and rationales, summaries and
source links, mutation events, request/outcome receipts, SQLite WAL/journal
sidecars, and copied backups can retain plaintext. Deliberate detail views
display plaintext. Never store passwords, API keys, access tokens, private
keys, seed phrases, session cookies, or other credentials in memory. The
bounded credential deny-list is best-effort and cannot prove that text contains
no secret.

Overwrite appends an immutable version. Delete adds a tombstone and is not
secure erasure: earlier plaintext remains in immutable history. Immutable
versions, events, and receipts accumulate monotonically, and repeated edits
consume additional local capacity; SQLite may reuse pages, so physical file
size need not grow monotonically. A capacity or full-disk failure rolls the
whole SQLite `BEGIN IMMEDIATE` transaction back with no partial memory row,
event, approval, projection, audit record, or receipt. Protect the state
directory and every copied backup as sensitive plaintext.

Lists deliberately omit entry values, proposal candidate text and rationale,
and episodic bodies. A deliberate detail view reveals the requested record and
escapes terminal controls. Episodic detail is read-only, is labelled
`Summary — verify sources`, and retains bounded exact source references so the
summary is never presented as an unqualified fact. Generic Help, Status, Audit,
and error views contain no memory prose. Startup verifies the immutable event
stream and authenticated mirrors, reconstructs only permitted missing mirrors
and derived pointers, and must fail closed for altered, conflicting, or
unexplained immutable data.

The global destinations are exactly:

```text
1 Home
2 Chat
3 Agents
4 Skills
5 Connections
6 Activity
7 Setup
8 Audit
9 Help
```

There is no dedicated Memory destination: Memory stays nested under Agents,
and bare `m` does not navigate there. Modified shortcuts are inert.
While a Memory text editor owns input, typed shortcut characters remain editor
text. `?` remains a Help alias but is not a destination label. Switching away
and back preserves the nested Memory state.

### Hybrid Memory fallback commands

The fallback host accepts exactly these eleven forms, in this order. Quoted
agent names and keys use the existing command quoting rules.

```text
/memory list <agent>
/memory get <agent> <key>
/memory history <agent> <key> [positive-version]
/memory set <agent> <key>
/memory delete <agent> <key>
/memory proposals <agent> [pending|all]
/memory proposal <proposal-id>
/memory approve <proposal-id>
/memory reject <proposal-id>
/memory episodes <agent>
/memory episode <summary-id>
```

Set/delete and approve/reject open local review flows and require the exact
displayed action plus review digest. There is no production `/memory propose`,
summary-write, or snapshot presentation route. Snapshot construction is an
internal source-bounded interface and is deterministic for the same verified
state and request.

### Recovery and deferred Hybrid Memory boundaries

Recovery authenticates the immutable event stream, request/outcome receipts,
and durable mirrors before rebuilding derived current pointers and projections.
An existing immutable row must match its verified event byte-for-byte. A
permitted missing mirror may be recreated, but altered or unexplained immutable
data makes startup fail closed. There is no production proposal creator,
summary writer, or snapshot presentation route. Inference/chat, automatic memory extraction,
semantic/vector search, embeddings, autonomous proposal generation, remote
sync, encryption at rest, credential vaulting, retention pruning, secure
erasure, production proposal creation, summary mutation, and snapshot
presentation remain deferred.

Run `cargo test --test hybrid_memory_acceptance` for the focused automated
acceptance contract. See the [Phase 2 Hybrid Memory testing
guide](docs/testing/phase-2-hybrid-memory.md) for disposable-state and manual
acceptance procedures.

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
| `/help` | Outputs `Available commands:` followed by the complete supported Phase 0, Skills, and Hybrid Memory grammar; explicitly states that internal Memory producers are unavailable; commits `HelpViewed`. | Continues. |
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
| `/memory list <agent>` | Lists bounded current entry metadata without values. | Continues. |
| `/memory get <agent> <key>` | Shows one deliberately requested current entry and its authenticated metadata. | Continues. |
| `/memory history <agent> <key> [positive-version]` | Lists bounded newest-first history metadata or shows one exact immutable version. | Continues. |
| `/memory set <agent> <key>` | Opens the guided value/tag editor and exact Human mutation review. | Continues until confirmation or cancel. |
| `/memory delete <agent> <key>` | Opens an exact Human tombstone review. | Continues until confirmation or cancel. |
| `/memory proposals <agent> [pending\|all]` | Lists bounded proposal metadata for the agent namespace. | Continues. |
| `/memory proposal <proposal-id>` | Shows one deliberately requested proposal and its authenticated state. | Continues. |
| `/memory approve <proposal-id>` | Opens an exact Human proposal-approval review. | Continues until confirmation or cancel. |
| `/memory reject <proposal-id>` | Opens an exact Human proposal-rejection review. | Continues until confirmation or cancel. |
| `/memory episodes <agent>` | Lists bounded episodic-summary metadata without bodies. | Continues. |
| `/memory episode <summary-id>` | Shows one read-only source-qualified episodic summary. | Continues. |
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

Hybrid Memory is also application-managed local plaintext and is not encrypted
at rest. Current and historical entries, proposals and rationales, summaries
and source links, mutation events, receipts, SQLite WAL/journal sidecars,
deliberate detail displays, and copied backups can retain plaintext. Owner-only
filesystem permissions limit access but are not encryption. The bounded
credential deny-list is only a best-effort guard and cannot prove arbitrary
text contains no secret. A deletion adds an immutable tombstone; it does not
securely erase earlier versions. Immutable rows, events, and receipts
accumulate and repeated edits consume capacity even though SQLite may reuse
pages.

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

Phase 2 does not add agent orchestration, skill execution, inference/chat,
model execution, model providers, automatic memory extraction, semantic/vector
search, embeddings, autonomous proposal generation, remote sync, encryption at
rest, credential vaulting, retention pruning, secure erasure, production
proposal/summary/snapshot producer routes, live or market data, rooms, debates,
network access, credential entry, OAuth, MCP, external runtimes, broker
connectivity, order placement, trading recommendations, guided setup
application, web or mobile clients, multi-user access, remote access, or an
autonomous/background service.

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
