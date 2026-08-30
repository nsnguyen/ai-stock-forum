# AI Stock Forum — Agent Platform Architecture

**Status:** Approved version 1 architecture, including the full-screen TUI and
live-participation design

**Updated:** 2026-08-30

**Delivery roadmap:** [phases.md](phases.md)

This is the canonical architecture for this repository. It replaces the earlier
Python/FastAPI/React and Hermes-first designs. Those files remain useful as
history, but they are not implementation instructions for the Rust version.

## 1. Product in plain language

AI Stock Forum is being rebuilt as a local, full-screen terminal agent platform
where the user creates AI agent profiles and participates with them in live,
structured discussions. An agent profile can have its own:

- name, specialty, personality, and instructions;
- direct model-provider binding;
- versioned skills;
- persistent memory;
- approved MCP access; and
- optional engineering-runtime binding.

Finance is the first domain pack, not the platform boundary. A Bull and a Bear
can use the same or different models from the approved initial direct-provider
set: OpenAI, Anthropic, or xAI. The same core should later support engineering,
research, or other domains without rebuilding orchestration, permissions,
memory, or auditing.

The experience is inspired by a Grok-like multi-agent group discussion, but the
platform does not copy or depend on Grok's implementation. The user watches one
visible speaker at a time, may queue a message while that speaker finishes, and
can address the Chief, one agent, or the whole room. Agents first form
independent views, then challenge relevant claims, and finally produce a bounded
synthesis. Agreement is not forced. A split decision, insufficient-evidence
result, or evidence-backed `No Trade` is valid.

The user is the product's root authority. This means the user controls agents,
permissions, jobs, and approvals; it does **not** mean the program runs as the
operating-system `root` user.

## 2. Version 1 boundary

Version 1 is:

- one local Rust executable with an integrated event-driven application core;
- single-user and terminal-only, with a full-screen TUI as the primary client
  and a line-oriented command mode as a fallback and test adapter;
- provider-neutral at the core, with OpenAI, Anthropic, and xAI direct-provider
  adapters as the only discussion-agent inference paths;
- an internal, user-approved MCP marketplace with per-agent grants;
- a bounded multi-agent discussion coordinator and Chief of Staff;
- a safe engineering-job runner for Codex CLI or Claude Code;
- a finance pack for evidence-backed swing-trade research; and
- recommendation and code-change support only, with explicit human approvals.

Version 1 is not:

- a web, mobile, or remote application, nor an application with concurrently
  attached clients;
- a background daemon that keeps rooms spending after the TUI exits;
- a cloud or multi-user service;
- a broker or order-execution system;
- a community marketplace for unreviewed MCP servers or executable skills;
- Hermes or any other external discussion-agent runtime;
- a virtual-machine manager;
- an unrestricted coding-agent host; or
- an autonomous system allowed to merge or push code.

## 3. Terms that must stay separate

Several similar words refer to different things. Keeping them separate prevents
permissions and implementation details from leaking across boundaries.

| Term | Meaning | Example |
|---|---|---|
| **Agent profile** | Versioned platform configuration; not an always-running process | `Bear Analyst` |
| **Inference backend** | Normalized interface for model calls; only direct-provider implementations exist in version 1 | OpenAI, Anthropic, xAI |
| **Engineering runtime** | A coding CLI launched for a bounded repository job | Codex CLI, Claude Code |
| **Skill** | Versioned instructions and resources; never a permission grant | `earnings-quality-v1` |
| **MCP entry** | Approved metadata and launch/connection definition for a tool server | read-only GEX server |
| **MCP grant** | Permission for one profile to request an approved MCP entry | Bear may request GEX |
| **MCP activation** | A short-lived connection and selected schemas for one turn/job | GEX tools loaded for turn 18 |
| **Room** | A bounded, coordinator-mediated multi-agent workflow | `Analyze GOOGL swing trade` |
| **Human participant** | The user as a first-class room actor with authority above the Chief | queues `@Bear challenge the catalyst` |
| **Room event** | One ordered durable fact used for rendering, audit, and recovery | committed message or budget override |
| **Room checkpoint** | Rebuildable safe-boundary snapshot that accelerates resume | next speaker and pending queue |
| **TUI** | Primary full-screen presentation adapter; never the owner of business rules | transcript, roster, evidence, approvals |
| **Application command** | Typed request from the TUI or fallback command adapter | `QueueHumanMessage` |
| **Application event** | Typed result emitted after application validation and persistence | `HumanMessageQueued` |
| **Safe turn boundary** | Durable point after one visible turn finishes and before another starts | committed agent response |
| **PartialMessage** | An interrupted provider stream that remains visible but is excluded from debate context | response interrupted by timeout or power loss |
| **PartialRoomResult** | A deterministic room-level result built from committed material when normal model synthesis cannot finish | deadline expires before synthesis completes |
| **Closing allowance** | Token, cost, time, and turn capacity reserved inside the finite room cap for final synthesis | one bounded synthesis call |
| **Domain pack** | Compile-time Rust module containing domain prompts, schemas, and validators | Finance pack |
| **StructuredProcessRunner** | Rust child-process supervisor; not an AI model or sandbox | launches `codex exec` |
| **Worktree** | Git change isolation for one job; not a security boundary | job branch checkout |

A single agent profile may have both an inference binding and an engineering
binding. In version 1, inference always binds directly to a provider API; the
optional engineering binding selects Codex CLI or Claude Code. The user chooses
both, and the agent may not change either binding itself. Hermes and Herdr are
different external projects, and both are deferred beyond version 1.

## 4. Non-negotiable safety invariants

These rules apply in every phase and cannot be weakened by a prompt, skill,
Chief of Staff, provider, MCP response, or engineering-runtime output.

1. The user can inspect, interrupt, or cancel any room or job.
2. Explicit denial wins over every grant.
3. Skills and personality never grant tools, secrets, filesystem access, or
   network access.
4. Only user-approved internal MCP entries can be granted; only granted entries
   can be selected; selection still passes policy checks.
5. MCP output, web content, model output, repository text, and retrieved memory
   are untrusted data. They never supply authority or executable policy.
6. Coding jobs require both a strict built-in runtime sandbox and an isolated
   Git worktree. If sandbox enforcement is unavailable, the job does not start.
7. Version 1 has no VM mode, no unrestricted mode, and no permissive fallback.
8. A merge and a push require two different, one-shot user approvals. Approving
   a merge never approves a push.
9. Provider and runtime credentials are retrieved through a secret broker and
   are never stored in SQLite, prompts, transcripts, or job logs.
10. Finance recommendations pass deterministic validation before presentation.
    If maximum loss cannot be proven within the supported model, the result is
    ineligible rather than guessed.
11. Version 1 cannot place, stage, or transmit broker orders.
12. Only one room may auto-run at a time. Closing or losing the TUI requests a
    pause, cancels or joins background work, and starts no new turn or tool
    activation. During a handled graceful shutdown, only the already-visible
    response may drain to its existing deadline in the foreground process; no
    discussion continues through a background service.
13. User messages never silently truncate the visible speaker. They enter a
    durable queue and receive priority at the next safe turn boundary unless the
    user invokes the separate explicit cancel action.
14. Only the user may change an active room's round, time, token, or cost budget.
    Every override is explicit, finite, and audited; version 1 has no unlimited
    room mode.
15. Every metered provider or MCP operation requires a conservative reservation
    that fits the remaining finite allowance before it starts. The room's
    closing allowance is reserved inside—not added beyond—the accepted cap.
16. The coordinator may not start the next visible turn until the prior message,
    queue decision, budget state, and checkpoint boundary are durable.
17. A `PartialMessage` is never evidence or debate context. The user must choose
    retry or skip before the room can continue.

## 5. System overview

Every Mermaid diagram in this document uses a top-to-bottom layout and an
explicit high-contrast grey palette. This keeps the diagrams narrow enough for
phone scrolling and readable on a dark document background.

```mermaid
---
config:
  theme: base
  themeCSS: "svg { background-color: #2B2F36 !important; }"
  themeVariables:
    darkMode: true
    background: "#2B2F36"
    primaryColor: "#414750"
    primaryTextColor: "#F8F9FA"
    primaryBorderColor: "#AAB2BD"
    secondaryColor: "#343A40"
    secondaryTextColor: "#F8F9FA"
    secondaryBorderColor: "#9AA4AF"
    tertiaryColor: "#4B525C"
    tertiaryTextColor: "#FFFFFF"
    tertiaryBorderColor: "#C5CCD3"
    lineColor: "#D0D7DE"
    textColor: "#F8F9FA"
    edgeLabelBackground: "#343A40"
    clusterBkg: "#30353C"
    clusterBorder: "#8C959F"
---
flowchart TB
    User["User<br/>root product authority"] --> Tui["Primary full-screen Rust TUI"]
    User -.-> Command["Fallback command mode"]
    Tui --> Boundary["Typed UI commands and application events"]
    Command --> Boundary
    Boundary --> App["Application service"]
    App -.->|"typed events"| Boundary
    App --> Core["Policy engine · Chief · rooms<br/>profiles · skills · memory<br/>engineering jobs · domain packs"]
    Core --> Boundaries["Durable and controlled boundaries<br/>SQLite events · projections · checkpoints<br/>adapters · finance evidence · risk validation"]
    Boundaries --> ModelTools["Direct provider and MCP paths<br/>OpenAI / Anthropic / xAI<br/>approved MCP servers"]
    Boundaries --> Coding["Engineering runtime path<br/>Codex CLI / Claude Code"]
    Coding --> Isolation["Strict runtime sandbox<br/>per-job Git worktree"]
```

The platform is a **modular monolith**: one core application process and
executable, plus only the child processes it explicitly supervises. It is
divided into modules with explicit interfaces. This is easier to build, test,
and audit than local microservices. The TUI, fallback command mode, application
services, room coordinator, and persistence all live in that process; version 1
has no local daemon or socket-separated frontend/backend deployment.

The process is event-driven internally. Presentation adapters submit typed
commands to the application service. The service validates authority, commits
state, and publishes typed application events. The TUI renders projections of
those events and may never mutate repositories, call providers, or grant
permissions directly. This boundary keeps streaming responsive and makes the
room engine testable without a real terminal.

### Suggested Rust layout

```text
ai-stock-forum/
├── Cargo.toml
├── src/
│   ├── main.rs                 # process startup only
│   ├── app/                    # use cases and command handlers
│   ├── ui/
│   │   ├── tui/                # full-screen renderer, input, view models
│   │   └── command/            # fallback command parser and text renderer
│   ├── policy/                 # capabilities, grants, denials, approvals
│   ├── agents/                 # profiles, executions, normalized messages
│   ├── rooms/                  # bounded discussion state machine
│   ├── providers/              # inference contract and direct API adapters
│   ├── runtimes/               # Codex and Claude engineering CLI adapters
│   ├── skills/                 # manifests, versions, relevance loading
│   ├── memory/                 # KV and bounded episodic retrieval
│   ├── mcp/                    # catalog, grants, broker, schema loading
│   ├── jobs/                   # runner, worktrees, diff, promotion gates
│   ├── domains/
│   │   └── finance/            # evidence, GEX projection, risk validation
│   ├── persistence/            # SQLite repositories and migrations
│   ├── audit/                  # typed append-only event recording
│   └── recovery/               # replay, checkpoints, interruption decisions
├── tests/                      # cross-module and acceptance tests
├── migrations/                # ordered SQLite migrations
└── docs/
```

Start with Rust modules. Extract a separate crate only when an interface needs
independent compilation or stronger dependency control. Domain packs are
compiled into the binary in version 1; arbitrary dynamic code plugins are not.
The `providers` module owns a normalized inference-backend contract so rooms do
not depend on vendor payloads. Version 1 implements that contract only with
direct APIs. A future external runtime such as Hermes could implement the same
boundary after a separate design review without changing room orchestration.

## 6. Authority and policy model

Every sensitive action is expressed as a typed capability, for example:

```text
mcp.invoke(entry=gex-readonly, tool=get_levels)
memory.propose(agent=bear)
job.start(runtime=codex, repository=/path/to/repo)
git.merge(job=J42, target=main, source_commit=abc123)
git.push(remote=origin, ref=main, commit=def456)
```

Effective authority is the intersection of all applicable allow-lists, minus
all applicable denials:

```text
compiled capability
∩ platform-approved resource
∩ user grant
∩ agent-profile grant
∩ room or job scope
∩ short-lived activation lease
∩ sandbox enforcement
− any explicit denial
```

```mermaid
---
config:
  theme: base
  themeCSS: "svg { background-color: #2B2F36 !important; }"
  themeVariables:
    darkMode: true
    background: "#2B2F36"
    primaryColor: "#414750"
    primaryTextColor: "#F8F9FA"
    primaryBorderColor: "#AAB2BD"
    secondaryColor: "#343A40"
    secondaryTextColor: "#F8F9FA"
    secondaryBorderColor: "#9AA4AF"
    tertiaryColor: "#4B525C"
    tertiaryTextColor: "#FFFFFF"
    tertiaryBorderColor: "#C5CCD3"
    lineColor: "#D0D7DE"
    textColor: "#F8F9FA"
    edgeLabelBackground: "#343A40"
    clusterBkg: "#30353C"
    clusterBorder: "#8C959F"
---
flowchart TB
    Request["Agent or Chief requests action"] --> Compiled{"Capability exists?"}
    Compiled -->|no| Deny["Deny and audit"]
    Compiled -->|yes| Approved{"Resource approved?"}
    Approved -->|no| Deny
    Approved -->|yes| Granted{"User + profile grant?"}
    Granted -->|no| Deny
    Granted -->|yes| Scoped{"Within room/job scope?"}
    Scoped -->|no| Deny
    Scoped -->|yes| Sandbox{"Sandbox / lease valid?"}
    Sandbox -->|no| Deny
    Sandbox -->|yes| Execute["Execute and audit"]
```

The Chief of Staff is an ordinary policy-constrained agent with extra
coordination commands. It may suggest a roster and, only after the user accepts
the bounded proposal, open a room using already allowed agents. It may summarize
results and propose a job. It may not approve its own proposal, install an MCP,
expand a grant, change a secret or runtime, merge, push, or bypass deterministic
validation.

Approvals are typed, immutable, one-shot records. They include the exact object
digest and expire on material change. A generic `yes` is accepted only when the
TUI displays one unambiguous pending action and records what was approved.

An active-room budget override is a typed user command, not an agent capability.
It states the exact new finite round, time, token, or cost limit, is persisted
before it takes effect, and cannot be issued by the Chief or a specialist. Chat
routing also grants no authority: `@Name` targets one pinned profile, `@all`
creates sequential obligations for the pinned roster, and a message with no
mention is routed to the Chief.

## 7. Agent profiles and executions

An agent profile is durable configuration. An execution is one bounded run of a
profile for a turn, room phase, or job.

```text
AgentProfileVersion
├── id, version, display_name, description
├── specialties[]
├── personality
├── operating_instructions
├── inference_binding { direct_provider_connection_id, model }
├── optional_engineering_binding { runtime_id }
├── skill_version_refs[]
├── mcp_grant_refs[]
├── memory_namespace
├── policy_profile
└── created_at, supersedes
```

Editing creates a new immutable version and shows a diff before activation.
Rooms pin profile versions so a later edit cannot silently change an in-flight
discussion. A specialty is fixed for the lifetime of that pinned version; an
agent cannot change its own role during a room. Two profiles may use the same
direct provider connection and model while retaining different instructions,
memory namespaces, skills, and grants.

Version 1 does not launch an external discussion-agent runtime. The normalized
inference contract remains intentionally replaceable, but no Hermes adapter,
Hermes authentication, Hermes profile state, or Hermes qualification work is in
the version 1 implementation scope.

## 8. Full-screen TUI and fallback command mode

The executable opens a full-screen terminal user interface by default. It is
the primary version 1 product surface, not a decorative wrapper around a REPL.
Its normal workspace contains:

- a room header with objective, phase, connection health, and finite remaining
  round/time/token/cost budgets;
- an agent roster showing the one visible speaker plus compact states such as
  `Researching`, `Waiting for MCP`, `Ready`, `Speaking`, and `Unavailable`;
- one shared scrollable transcript containing committed user, Chief, and agent
  messages in coordinator order;
- a collapsible activity view for evidence references, MCP selection/calls,
  approvals, warnings, and errors; and
- a persistent composer that accepts messages, mentions, and action commands
  while provider output is streaming.

Wide terminals may show roster, transcript, and activity side by side. Narrow
terminals collapse secondary regions into tabs while keeping the transcript,
speaker identity, budget warning, and composer usable. The default palette is
high-contrast on dark backgrounds and has a monochrome/no-color mode; meaning
never depends on color alone.

Typed input is persisted and appears immediately as `Queued`. `@Name` targets a
specific pinned agent, `@all` requests ordered responses from the pinned roster,
and unaddressed text goes to the Chief. Ordinary input does not cancel the
current speaker. After that response commits, queued user input has priority
before the coordinator starts another agent turn. Multiple queued messages are
selected in durable event-ID order (FIFO) unless the user explicitly cancels a
pending message. The user also has distinct pause, resume, step-one-turn, and
explicit cancel actions. Automatic continuation is the default.

The same executable exposes a line-oriented fallback command mode for recovery,
automation, accessibility, and headless tests. It submits the same typed
application commands and consumes the same events as the TUI; it is not a
second implementation of business rules. Slash commands are available through
that mode and through the TUI command palette.

| Command | Purpose |
|---|---|
| `/agent create|list|show|edit|history` | Manage versioned agent profiles |
| `/room new|list|show|send|pause|resume|step|cancel` | Run and control discussions |
| `/room retry-partial-message|skip-partial-message` | Resolve an interrupted visible turn |
| `/room budget show|set|extend` | Inspect or apply an exact finite user override |
| `/connection add|list|test|remove` | Manage provider/runtime connection references |
| `/marketplace list|show|approve|revoke` | Manage the internal MCP catalog |
| `/mcp grant|revoke|status` | Manage per-agent MCP eligibility |
| `/skill add|list|show|assign|unassign` | Manage versioned skills |
| `/memory get|set|list|proposals|approve|delete` | Manage durable agent memory |
| `/job start|list|show|cancel|diff` | Manage engineering jobs |
| `/approve show|accept|reject` | Resolve exact pending actions |
| `/audit show|tail|export` | Inspect normalized events and decisions |
| `/settings`, `/help`, `/quit` | Configure, learn, and request safe shutdown |

Plural aliases such as `/skills` and `/agents` may map to the corresponding
`list` commands. `/agent edit` uses a guided editor by default. An optional
`$EDITOR` flow exports a secret-free temporary document, validates it, shows a
field-level diff, and asks before activating the new version.

On `/quit`, the application first records a pause request, cancels and joins
background evidence work and MCP leases, and starts no new work. The TUI remains
visible in `Pausing` while at most the already in-flight visible turn reaches
its existing deadline. The application then commits that response or a
`PartialMessage`, writes the paused checkpoint, restores terminal state, and
exits. If terminal state must be restored immediately after a handled renderer
or terminal failure, the same bounded drain happens in the foreground process;
it never becomes daemon work. A room is not durably `Paused` until background
work is joined and the visible turn is resolved. An abrupt process death or
power loss cannot guarantee a safe boundary; restart replays durable events,
labels recovered deltas `PartialMessage`, and opens the room paused for the
user's retry-or-skip decision.

Both presentation adapters are unprivileged. Business rules live in the
application service, so a later web or mobile client cannot bypass policy by
reimplementing commands.

## 9. Connections, providers, and runtimes

Connections describe how an adapter authenticates. They do not grant an agent
permission to use that adapter.

Supported connection kinds are deliberately distinct:

1. **Direct API connection:** an API key stored by the operating-system secret
   store and referenced from SQLite by an opaque ID.
2. **Engineering-runtime-managed login:** a user signs in through an installed
   Codex CLI or Claude Code runtime. The platform records availability and a
   non-secret account label; it does not copy that runtime's token or convert it
   into a direct-provider API key.
3. **Local MCP connection:** an approved executable or endpoint definition with
   pinned metadata and no embedded secret values.

Direct OpenAI API access for a discussion agent and ChatGPT-managed Codex login
for an engineering job are separate connection types. The same rule applies to
Anthropic API keys and Claude Code login. A runtime-managed login is never
offered as a discussion-agent inference binding, and the platform never
impersonates a subscription or extracts its credentials.

All adapters emit normalized events such as:

```text
Started, TextDelta, ToolRequested, ToolResult, UsageReported,
Completed, TimedOut, Cancelled, Failed
```

Provider-specific payloads are retained only when useful for debugging and are
redacted before storage. The orchestration layer consumes normalized events and
validated output schemas, not vendor-specific transcript shapes.

Provider text deltas may update the current draft in the TUI, and background
provider or MCP work may update agent status concurrently. Those events do not
publish extra speakers. Only the room coordinator can promote one completed
draft into the ordered shared transcript, so the user never sees two agents
speaking simultaneously.

## 10. Chief of Staff and room discussion flow

The Chief listens to the user, asks only necessary questions, and converts a
request into a bounded room proposal: objective, roster, evidence needs,
available MCP categories, maximum rounds, time budget, token budget, and cost
budget. The proposal also shows the closing allowance reserved inside those
finite limits. The user may edit, accept, interrupt, or cancel it.

Agents do not have an unrestricted peer network. The coordinator routes typed
messages and owns ordering, deadlines, checkpoints, and the audit trail. Any
number of rooms may be saved, paused, or completed, but only one room may be
auto-running. Starting or resuming another first requests a safe-boundary pause
of the current room.

Agents may gather evidence and call granted MCPs concurrently, represented only
by status and activity events. Conversational inference launches serially for
the coordinator-selected speaker, so the visible stream is the actual response,
not a delayed reveal of a hidden parallel conversation. First-pass independence
is preserved by building every specialist's first turn from the same sealed
pre-round transcript/evidence snapshot and withholding earlier specialists'
first-pass output from later first-pass contexts until the round closes.
Automatic continuation is the default. This independence guarantee applies to
an uninterrupted first-pass batch. If the user intervenes before that batch
finishes, the intervention closes the batch at the next safe boundary; unstarted
first-pass obligations are invalidated and replanned after the routed human turn
from a newly sealed snapshot. This gives the user's input immediate effect
without silently feeding one specialist's first pass into another specialist's
supposedly independent turn.

The user is a first-class room actor. A message entered during a visible response
is durably queued without interrupting that response. At the next safe boundary,
queued human input takes priority: `@Name` routes to one agent, `@all` creates a
stable ordered list of agent response obligations, and an unaddressed message is
handled by the Chief, which may answer, redirect a specialist, or revise the
agenda within existing permissions and budgets. Multiple human messages are
handled in durable event-ID order unless the user explicitly cancels one.
Explicit cancel is a separate action and may terminate the in-flight provider
request.

```mermaid
---
config:
  theme: base
  themeCSS: "svg { background-color: #2B2F36 !important; }"
  themeVariables:
    darkMode: true
    background: "#2B2F36"
    primaryColor: "#414750"
    primaryTextColor: "#F8F9FA"
    primaryBorderColor: "#AAB2BD"
    secondaryColor: "#343A40"
    secondaryTextColor: "#F8F9FA"
    secondaryBorderColor: "#9AA4AF"
    tertiaryColor: "#4B525C"
    tertiaryTextColor: "#FFFFFF"
    tertiaryBorderColor: "#C5CCD3"
    lineColor: "#D0D7DE"
    textColor: "#F8F9FA"
    edgeLabelBackground: "#343A40"
    clusterBkg: "#30353C"
    clusterBorder: "#8C959F"
---
flowchart TB
    Ask["User asks Chief"] --> Proposal["Chief proposes objective, roster,<br/>evidence, finite limits, and closing allowance"]
    Proposal --> Accept["User accepts,<br/>edits, or cancels"]
    Accept --> Pin["After acceptance: pin profiles, skills,<br/>memory, models, policy, and MCP entries"]
    Pin --> Research["Agents gather evidence concurrently<br/>no conversational response yet"]
    Research --> Boundary["At each safe boundary:<br/>process FIFO human input first"]
    Boundary --> Replan["If the user intervened:<br/>close the first-pass batch and<br/>replan from a new sealed snapshot"]
    Replan --> Select["Coordinator selects one visible speaker"]
    Select --> Reserve["Atomically reserve the maximum<br/>authorized call usage"]
    Reserve --> Work["If it fits: stream one speaker,<br/>reconcile usage, commit, checkpoint,<br/>then repeat at the next boundary"]
    Work --> Stop["When a working call cannot fit:<br/>queued input pauses AwaitingExtension;<br/>otherwise enter the closing path"]
    Stop --> Choice["In AwaitingExtension, the user<br/>extends finitely or chooses close"]
    Choice --> Closing["Use only the pre-reserved<br/>closing allowance"]
    Closing --> Synthesis["Attempt one bounded synthesis;<br/>on failure build PartialRoomResult<br/>without another model call"]
    Synthesis --> Validate["Run domain validator<br/>when applicable"]
    Validate --> Result["Persist and show evidence,<br/>dissent, risks, and next action"]
```

The accepted room budget is finite in round, active-run time, token, and cost
dimensions. Before work begins, each dimension is partitioned into a working
allowance and a minimum closing allowance; the closing allowance is inside the
accepted total, never extra capacity. A proposal that cannot fund its declared
closing path does not start. Paused time does not consume active-run time.

Before every metered provider or MCP operation, the budget ledger reserves its
maximum authorized round slot, input/output tokens, cost under pinned pricing,
and deadline window. The operation does not start unless every reservation fits
its appropriate allowance. Actual reported usage is reconciled afterward and
unused capacity is released; if trustworthy usage is unavailable, the full
reservation remains charged. This rule also prevents concurrent background work
from oversubscribing the same room budget.

The TUI continuously displays total, working, reserved, and consumed amounts.
Only the user may replace or extend the limits, and every override must be a new
explicit finite value. If working allowance is exhausted while human input is
queued, the room enters `AwaitingExtension` rather than dropping the message or
spending more. The user may extend it finitely or close the room; closing leaves
the queued message visibly recorded as unhandled and uses only the existing
closing allowance. With no queued input, the room moves directly to the closing
path.

Normal synthesis uses the pre-reserved closing allowance. If that bounded call
fails, times out, or cannot produce a valid result, the coordinator makes no
further model call and builds a `PartialRoomResult` deterministically from
already committed structured claims, dissent, evidence, and failure records.
A `PartialRoomResult` is never silently presented as a complete consensus; in
the finance pack it cannot become an eligible trade. The synthesizer may not
erase dissent, invent consensus, or use majority vote as a substitute for
evidence. For finance, deterministic validation follows successful synthesis.

## 11. MCP marketplace and lazy activation

Version 1 has a local internal marketplace, not arbitrary remote installation.
Each entry contains an ID, version, source, digest, transport, launch or endpoint
definition, concise capability tags, risk class, secret references, and a human
review record.

```mermaid
---
config:
  theme: base
  themeCSS: "svg { background-color: #2B2F36 !important; }"
  themeVariables:
    darkMode: true
    background: "#2B2F36"
    primaryColor: "#414750"
    primaryTextColor: "#F8F9FA"
    primaryBorderColor: "#AAB2BD"
    secondaryColor: "#343A40"
    secondaryTextColor: "#F8F9FA"
    secondaryBorderColor: "#9AA4AF"
    tertiaryColor: "#4B525C"
    tertiaryTextColor: "#FFFFFF"
    tertiaryBorderColor: "#C5CCD3"
    lineColor: "#D0D7DE"
    textColor: "#F8F9FA"
    edgeLabelBackground: "#343A40"
    clusterBkg: "#30353C"
    clusterBorder: "#8C959F"
---
flowchart TB
    Cataloged["EntryVersion<br/>Cataloged"] --> Approved["EntryVersion<br/>Approved pinned digest"]
    Approved -->|"user creates"| GrantActive["Grant<br/>Active for one profile"]
    GrantActive -->|"agent requests"| Selected["ActivationLease<br/>Selected for one operation"]
    Selected --> Active["ActivationLease<br/>Active"]
    Selected --> Released["ActivationLease<br/>Released"]
    Active --> Released
    Approved --> EntryRevoked["EntryVersion<br/>Revoked"]
    GrantActive --> GrantRevoked["Grant<br/>Revoked"]
    EntryRevoked -.->|"matching grants"| GrantRevoked
    GrantRevoked -.->|"matching leases"| Released
```

Approval, grants, and activations are separate records and lifecycles:

- **EntryVersion** says whether one exact reviewed entry digest is approved.
- **Grant** says whether one profile may request that entry digest. An approved
  entry can have many grants.
- **ActivationLease** represents one selection/connection for one turn or job.
  A grant can have multiple concurrent leases, and every new lease revalidates
  the current entry and grant state.

Lazy context loading uses a staged handshake:

1. The first prompt sees only a compact index of granted entry IDs, purposes,
   risk labels, and capability tags.
2. The agent requests one entry for a stated purpose.
3. After policy validation, the broker discovers that entry internally and
   returns a bounded summary of only its allowed tools—not their full schemas.
4. The agent selects the relevant tool or tools; the broker then loads only
   those exact schemas for the next model turn.
5. The model makes a typed invocation that the broker validates again.

Discovery caches and audit records are keyed by the approved entry digest and
discovered schema digest. The platform never places every MCP server or every
tool schema in every context window.

The broker owns process lifetime, timeouts, output-size limits, redaction, and
audit events. Read/write effect metadata is explicit per tool, but version 1
only presents and invokes read-only MCP tools. A write-capable schema in an entry
remains unavailable until a later design adds a separate typed capability and
exact approval policy. Finance uses read-only MCPs, including GEX. MCP content
is evidence with provenance, not an instruction that can modify policy or
approve another action.

“Read-only” is an enforced resource boundary, not a trusted label from an MCP.
The broker supplies read-only database/API credentials, read-only filesystem
mounts, and, where required, a mutation-denying network broker. If the platform
cannot independently prevent mutation for the declared resources, the entry
cannot activate as read-only.

Local MCP processes are launched without a shell, receive a minimal environment,
and are limited to manifest-declared filesystem and network resources by a
broker-managed host sandbox. If those declared limits cannot be enforced, the
local entry cannot activate. A marketplace review records what host access the
MCP itself needs; an agent grant never expands it. Before every activation, the
broker revalidates the canonical manifest and every local launch artifact
against its approved digest. Remote entries pin a canonical origin, expected
server identity, and schema digest. A mismatch refuses activation, creates a new
catalog version for review, and leaves old grants unusable.

## 12. Skills, personality, and memory

### Skills

A skill is a versioned declarative bundle: manifest, instructions, examples,
output schema hints, and optional static resources. It cannot contain an
executable permission grant. Agents receive compact manifests first and load
assigned skill content only when relevant. Rooms pin exact skill versions.

### Personality

Personality controls voice, perspective, and reasoning emphasis. It does not
change authority. “Aggressive trader,” “Bull,” or “security expert” never means
the agent receives additional tools or a higher risk limit.

### Memory

Version 1 uses a hybrid model:

- **Explicit KV memory:** durable facts and preferences in an agent-private
  namespace. The user can edit them directly. Agents can only propose changes;
  the user approves or rejects each durable mutation.
- **Bounded episodic summaries:** labeled summaries of completed rooms, linked
  to source event IDs. They are retrieval aids, not policy or ground truth.
- **Append-only audit events:** the full operational history, not automatically
  inserted into prompts.

Retrieval is scoped by agent, room, purpose, and size budget. Private memory is
not shared merely because two profiles use the same provider connection or
model. Cross-agent sharing happens through the room transcript or an explicit
user action and is audited.

### Room context and resume

Room history is not long-term agent memory and does not require a memory grant
to remain recoverable. Before each turn or resumed execution, a provider-neutral
context builder creates a bounded packet for that specific pinned agent from:

- its profile, personality, specialty, operating instructions, relevant skill
  versions, and approved memory snapshot;
- a source-linked structured summary of older committed room events;
- recent committed transcript turns verbatim;
- unresolved claims, questions, response obligations, and dissent;
- relevant evidence and MCP result references; and
- that agent's own previously published position.

The packet never contains another agent's private memory, `PartialMessage`
output, or hidden chain-of-thought. Older summaries retain source event IDs so
the user and context builder can inspect the underlying transcript. Promoting a
room conclusion into durable agent memory remains a separate user-approved
memory proposal.

## 13. Structured engineering jobs

The engineering agent is a normal agent profile with an optional user-selected
engineering binding. Its default may be Codex CLI or Claude Code, and the user
may override that choice for a particular job. The profile and runtime cannot
switch themselves.

### What StructuredProcessRunner is

`StructuredProcessRunner` is ordinary Rust code that supervises a child process.
It receives a typed specification rather than a shell command string:

```text
ProcessSpec
├── approved_runtime_id
├── executable and fixed adapter-owned arguments
├── structured prompt on stdin
├── isolated worktree cwd
├── minimal environment allow-list
├── timeout and output limits
├── expected JSON or JSONL event format
├── approved MCP leases
├── required sandbox capability policy
├── verified sandbox capability attestation
└── cancellation handle
```

It launches non-interactive structured modes such as Codex `exec` with JSONL
events or Claude Code print mode with stream JSON. It parses commands, file
changes, tool calls, usage, completion, and errors into normalized events. It
does not build shell strings, scrape an interactive terminal, or treat free-form
text as an authorization instruction.

Runtime events are untrusted telemetry, even when they are valid JSON. The job
service independently derives changed files, diffs, and object IDs from the
worktree. It launches approved checks in a fresh verifier sandbox with no ref
writes, credentials, or general network; repository code never executes inside
the core control-plane process.

The runner is not itself the security boundary. The runtime's strict built-in
sandbox and the platform's policy preflight enforce the boundary; the worktree
only isolates Git changes.

### Job lifecycle and approvals

```mermaid
---
config:
  theme: base
  themeCSS: "svg { background-color: #2B2F36 !important; }"
  themeVariables:
    darkMode: true
    background: "#2B2F36"
    primaryColor: "#414750"
    primaryTextColor: "#F8F9FA"
    primaryBorderColor: "#AAB2BD"
    secondaryColor: "#343A40"
    secondaryTextColor: "#F8F9FA"
    secondaryBorderColor: "#9AA4AF"
    tertiaryColor: "#4B525C"
    tertiaryTextColor: "#FFFFFF"
    tertiaryBorderColor: "#C5CCD3"
    lineColor: "#D0D7DE"
    textColor: "#F8F9FA"
    edgeLabelBackground: "#343A40"
    clusterBkg: "#30353C"
    clusterBorder: "#8C959F"
---
flowchart TB
    Start["User submits repository, base ref,<br/>runtime, scope, and limits"] --> Worktree["Job service creates dedicated<br/>branch and worktree"]
    Worktree --> Preflight["Preflight strict sandbox<br/>and typed policy"]
    Preflight --> Proven{"Required sandbox proven?"}
    Proven -->|no| Refuse["Refuse start and audit reason"]
    Proven -->|yes| Launch["Launch Codex CLI or Claude Code<br/>in structured non-interactive mode"]
    Launch --> Telemetry["Normalize untrusted JSON or JSONL<br/>events, commands, changes, and result"]
    Telemetry --> Verify["Independently derive diff<br/>rerun checks<br/>create platform checkpoint"]
    Verify --> Review["TUI shows checks, diff,<br/>risks, and review summary"]
    Review --> Merge["User approves exact local merge"]
    Merge --> RevalidateMerge["Revalidate and apply<br/>exact merge candidate"]
    RevalidateMerge --> PushProposal["Show merged commit<br/>and exact remote/ref proposal"]
    PushProposal --> Push["User separately approves exact push"]
    Push --> RevalidatePush["Revalidate exact commit,<br/>remote, ref, and push"]
```

### Mandatory sandbox policy

Every runtime adapter returns a machine-checkable capability report for the
exact `(host, runtime version, adapter version)` combination. A flag name or
runtime self-description is not proof. The report covers canonical writable and
readable roots, command-network denial, environment/credential isolation,
bypass and unsandboxed-retry prevention, sanitized Git-view isolation, symlink
and hardlink escape resistance, and child-process containment. Preflight fails
unless every capability required by the job policy is attested and verified by
negative conformance tests.

Attestation pins the canonical identity, digest/signature, and effective
configuration of the runtime binary, adapter build, and operating-system sandbox
backend—not only their version strings. Any identity or configuration change
disables that tuple until it passes qualification again. These components are
part of the trusted enforcement base.

- Use the runtime's strict built-in sandbox with write access limited to the
  dedicated worktree and explicitly required temporary paths.
- Refuse startup if sandbox support or its required fail-closed settings cannot
  be verified on the host.
- Never pass unrestricted, danger, permission-bypass, or unsandboxed-retry
  modes. There is no “try again with fewer restrictions” path.
- Do not use a VM in version 1.
- Give the child a minimal environment. Do not expose Git credentials, signing
  keys, unrelated provider keys, parent process secrets, or broad home-directory
  access.
- Allow only the narrow runtime-owned authentication mechanism established by
  the user's runtime login; never copy its token into the child environment or
  make the rest of the runtime configuration directory writable.
- Permit only the runtime's required inference connection and explicit
  allowlisted network/MCP routes. Sandboxed commands receive no general network
  access.
- Do not expose the actual shared Git metadata to the coding child. Give it only
  a broker-generated, sanitized, read-only Git view needed for status/diff; omit
  remote configuration, credentials, hooks, signing, and unrelated worktree
  metadata. The child may edit files but may not commit or change any ref.
- After the child exits, the trusted job service may create an audited, typed
  checkpoint on the dedicated job branch from the independently derived diff,
  using controlled Git plumbing that constructs the tree from reviewed bytes and
  disables hooks, filters, signing, credential helpers, and user/repository
  configuration side effects.
- Launch jobs with verified owner-death or guardian-process containment. On app
  restart, reconcile recorded processes and terminate surviving children before
  accepting new jobs. If containment cannot be proven, refuse the job.
- On timeout or cancellation, terminate the entire process tree, mark the job
  interrupted, and preserve the worktree for inspection.

The promotion service prepares a candidate commit in a controlled promotion
worktree with hooks, signing, user Git configuration, and credential helpers
disabled. Merge approval pins repository identity, source object ID, target ref
and expected object ID, merge strategy/options, checks digest, policy version,
abort-on-conflict behavior, candidate tree, and candidate commit. The approval
is atomically claimed before the target ref is conditionally updated; restart
reconciliation makes the operation idempotent.

Push approval is created only after the local merge. It pins the exact merged
object ID, canonical remote URL/identity, destination ref, expected remote object
ID, and fast-forward-only exact-lease mode. The approval is separately and
atomically claimed, then revalidated immediately before the single-ref push. The
transport uses an atomic expected-old-object compare-and-swap and independently
proves the new commit descends from that expected object. A broad lease, changed
expected object, or non-fast-forward update is forbidden. Any changed identity,
object, ref, check, configuration, or policy invalidates the relevant approval.
Push permission is never inferred from merge approval.

## 14. Finance domain pack

The finance pack turns the general room system into evidence-backed swing-trade
research for horizons of several days to several weeks.

### Evidence envelope

Every run pins an immutable evidence envelope with:

- symbol and as-of timestamp;
- market prices, option-chain snapshots, liquidity, and source timestamps;
- news, filing, and catalyst references;
- read-only GEX MCP results when requested;
- an explicit current user risk budget and user-provided or read-only account
  capacity snapshot when trade eligibility is requested;
- source IDs, retrieval times, freshness state, and content digests; and
- explicit missing, conflicting, or stale fields.

All independent first passes receive the same core envelope. Specialists may
receive role-relevant projections, but every cited claim points back to source
IDs. Missing MCP data remains missing; an agent may not manufacture a substitute.

### Deterministic risk engine

The risk engine is ordinary Rust, not an LLM. Agents propose a structured trade;
the engine independently checks it.

For version 1:

- only cash-funded **long** stock is supported; short stock is rejected, and
  loss-to-zero plus fees must fit the explicit current user risk budget;
- option eligibility is a closed enum: `LongCall`, `LongPut`,
  `BullCallDebitSpread`, `BearPutDebitSpread`, `BullPutCreditSpread`,
  `BearCallCreditSpread`, `LongCallButterfly`, `LongPutButterfly`, and
  `IronCondor`;
- option legs must use one underlying, one expiration, a supported unadjusted
  contract style/settlement, an explicit multiplier, integer quantities, and
  the exact ratios defined for that enum;
- the risk model covers expiration payoff, net debit/credit, fees, rounding,
  assignment, exercise, and the account capacity needed for supported interim
  settlement states;
- uncovered short options, unbounded ratio exposure, unsupported multi-expiry
  interactions, or any structure whose worst case cannot be proven are rejected;
- liquidity, freshness, concentration, capacity, and user risk-budget gates are
  deterministic. A missing budget or capacity snapshot cannot be model-defaulted
  and prevents `EligibleTradePlan`; and
- model prose can explain the result but cannot override a failed gate.

The safe rule is mechanical: **finite-looking is not enough; maximum loss must
be proven across the supported state model.** New structures become eligible by
adding tested deterministic support, not by adding a prompt exception.

### Result and approval states

A finance room returns one of:

```text
EligibleTradePlan | NoTrade | SplitDecision | InsufficientEvidence
```

Every result includes evidence references, bullish and bearish claims,
uncertainties, dissent, invalidation conditions, and why the selected state won.
`NoTrade` must cite concrete failed gates or unresolved evidence; it is not an
empty refusal.

An eligible plan contains an immutable plan ID and digest, symbol, long-stock or
named option-structure kind, shares or exact option legs, expiration/strikes,
quantities, entry limit, estimated debit/credit, proven maximum loss, pinned
risk-budget/capacity snapshot, profit/exit plan, time stop, thesis invalidation,
evidence snapshot ID, and policy/engine versions.

User approval records acceptance of that exact recommendation for planning. It
does not execute an order. Approval binds the complete canonical plan digest;
any digest-input change invalidates it. Examples include quote, leg, quantity,
expiration, contract metadata, fees/rounding, maximum loss, explicit risk budget,
capacity snapshot, exit/invalidation rule, evidence snapshot, engine version,
and policy version.

## 15. Persistence, audit, and secrets

SQLite is the local source of truth for configuration and recoverable workflow
state. Ordered migrations manage schema changes. Important records include:

- versioned agents, skills, policy profiles, and runtime bindings;
- connection metadata and opaque secret references;
- MCP entries, review records, versions, grants, and activations;
- KV memory, episodic summaries, and memory proposals;
- rooms, pinned participants, ordered messages, human-routing targets, pending
  queues, evidence references, budgets, checkpoints, and results;
- engineering jobs, normalized events, worktrees, commits, and checks;
- finance plan versions and deterministic validation reports; and
- typed approvals, rejections, cancellations, and promotion outcomes.

Operational events are append-only and have stable IDs, actor, timestamp,
correlation ID, object version/digest, and redacted payload. Mutable views such
as “current agent version” are projections over versioned records.

For a room, the numbered append-only event stream is authoritative. It includes
the proposal and pinned versions, state transitions, user messages and targets,
agent turn starts and completed messages, structured claims and evidence,
provisional stream chunks, MCP activity, response obligations, pause/cancel
requests, budget reservations/reconciliation and user overrides,
`PartialMessage` decisions, synthesis, `PartialRoomResult`, validation, and
failure records. A TUI transcript or status panel is a rebuildable projection
of those events, not a second source of truth.

At every safe turn boundary, the application transactionally records the last
applied event ID plus a rebuildable checkpoint containing room phase, next
speaker, pending user messages, ordered `@all` obligations, remaining finite
budgets, unresolved claims/questions, evidence references, and pinned context
versions. No next provider turn starts until that boundary is durable.

Streaming chunks remain provisional until the coordinator commits a completed
message. Any provider stream interrupted before commit—by timeout, explicit
cancel, handled shutdown, process death, or power loss—is projected as a visible
`PartialMessage` but excluded from summaries, context packets, evidence, and
synthesis. The room opens paused and the user chooses either retry, which creates
a new turn linked to the interrupted attempt, or skip, which records why the
attempt was omitted.

```mermaid
---
config:
  theme: base
  themeCSS: "svg { background-color: #2B2F36 !important; }"
  themeVariables:
    darkMode: true
    background: "#2B2F36"
    primaryColor: "#414750"
    primaryTextColor: "#F8F9FA"
    primaryBorderColor: "#AAB2BD"
    secondaryColor: "#343A40"
    secondaryTextColor: "#F8F9FA"
    secondaryBorderColor: "#9AA4AF"
    tertiaryColor: "#4B525C"
    tertiaryTextColor: "#FFFFFF"
    tertiaryBorderColor: "#C5CCD3"
    lineColor: "#D0D7DE"
    textColor: "#F8F9FA"
    edgeLabelBackground: "#343A40"
    clusterBkg: "#30353C"
    clusterBorder: "#8C959F"
---
flowchart TB
    Input["User, coordinator, provider,<br/>or MCP produces a typed event"] --> Log["Append numbered room event<br/>to SQLite"]
    Log --> Boundary{"Safe turn boundary?"}
    Boundary -->|yes| Checkpoint["Persist rebuildable room checkpoint"]
    Boundary -->|no| Draft["Keep stream chunks provisional"]
    Checkpoint --> Restart["On resume: load checkpoint<br/>and replay later events"]
    Draft --> Restart
    Restart --> PartialMessage{"Incomplete visible turn?"}
    PartialMessage -->|yes| Decision["Label PartialMessage and open paused<br/>user chooses retry or skip"]
    PartialMessage -->|no| Context["Build bounded context<br/>for the next pinned agent"]
    Decision --> Context
    Context --> Next["User resumes one live room"]
```

On normal resume, the application loads the latest valid checkpoint, replays all
later events, verifies sequence and object digests, rebuilds projections, and
then constructs provider-independent agent context. A missing or corrupt
checkpoint is discarded and rebuilt from the event stream. Corrupt authoritative
events stop recovery with an inspectable error; the platform never guesses or
silently drops transcript history.

Secret values live in the operating-system credential store or the owning
runtime's supported login store. SQLite stores only opaque references and safe
labels. Logs redact known secret values, authorization headers, environment
values, and raw credential files. Database and exported audit files use
owner-only filesystem permissions by default.

## 16. Failure behavior

Failing safely is part of the interface:

| Failure | Required behavior |
|---|---|
| Provider timeout or interrupted stream | Stop at its deadline, expose any draft only as `PartialMessage`, exclude it from context, and require retry or skip |
| Malformed normal synthesis or exhausted closing call | Build a clearly labeled `PartialRoomResult` deterministically from committed structured material; make no further model call |
| Graceful TUI exit | Record pause, cancel/join background work, keep `Pausing` visible while only the in-flight turn drains to its existing deadline, checkpoint, restore terminal state, and exit |
| Handled terminal or renderer loss | Restore terminal state promptly, perform the same bounded drain in the foreground process, checkpoint, and exit; never daemonize |
| Abrupt death during a visible response | Recover provisional chunks as `PartialMessage`, open paused, and require user retry or skip |
| Missing or corrupt room checkpoint | Rebuild it from authoritative events; stop with an inspectable error if authoritative events are corrupt |
| Room context cannot be rebuilt within policy | Keep the room paused and explain the missing/corrupt source; never invent a summary or silently omit history |
| MCP unavailable or schema changed | Release lease, mark evidence unavailable, never fabricate |
| Sandbox preflight fails | Do not start the engineering child process |
| Job times out or is cancelled | Runner kills the contained process tree, marks interrupted, preserves worktree |
| App crashes during a job | Owner-death/guardian containment kills children; restart reconciliation marks interrupted and preserves worktree |
| Evidence is stale or internally inconsistent | Fail affected finance gates; usually `NoTrade` or `InsufficientEvidence` |
| Agent/profile/plan changes after approval | Invalidate the approval and request a new exact approval |
| Merge base or source commit changes | Refuse merge until the user reviews a new proposal |
| Merged commit, remote, or destination changes | Refuse push until separately re-approved |
| Audit persistence fails for a sensitive action | Do not perform that action |

There is no silent downgrade from a safer mode to a more permissive one.

## 17. Security model

The primary local threats are prompt injection in evidence/repositories,
malicious or compromised MCP servers, overpowered personalities/skills, coding
agents executing unsafe commands, credential leakage, and ambiguous Git
approval.

The main controls are typed commands and schemas, deny-wins capability checks,
version pinning, lazy MCP activation, provenance, context separation, strict
runtime sandboxes, worktree isolation, minimal environments, secret brokering,
network limits, deterministic finance gates, exact approval digests, and an
append-only audit trail.

The TUI is not a trusted authority boundary. Every action crosses the same typed
application-command and policy checks as fallback command mode. Provider, MCP,
repository, and memory text is rendered as untrusted content: control characters
and terminal escape sequences are removed or visibly escaped so model output
cannot rewrite the screen, spoof an approval, alter the title/clipboard, or
inject input.

Version 1 trusts the local operating-system account, compiled application,
qualified runtime binary and adapter, and operating-system sandbox backend as
enforcement components. It does not claim to defend against an
already-compromised host, malicious kernel, or administrator with access to the
user's files.

## 18. Testing strategy

Implementation phases must include tests at the boundary being introduced:

- unit tests for parsing, state machines, capability intersections, and
  deterministic calculations;
- property tests for finance payoffs, maximum-loss bounds, approval invalidation,
  and deny-wins policy behavior;
- adapter contract tests using fake providers, fake runtimes, and fake MCPs;
- migration and crash-recovery tests with temporary SQLite databases;
- reducer and headless-view-model tests proving the TUI renders only committed
  application state and cannot bypass command policy;
- deterministic terminal snapshots across supported minimum widths, resize,
  dark/high-contrast, and monochrome modes, plus panic/signal terminal-restore
  tests;
- room-ordering tests for one visible speaker, concurrent background status,
  event-ID/FIFO queued-human priority, stable `@all` ordering, Chief-default
  routing, interruption and replanning of sealed first-pass batches,
  auto-continuation, pause/resume/step/cancel, and finite user-only overrides;
- budget-ledger tests for conservative per-call reservation, usage
  reconciliation, unknown-usage charging, concurrent-work exclusion, protected
  closing allowance, `AwaitingExtension`, and deterministic
  `PartialRoomResult` fallback without an extra model call;
- replay tests for one-live-room switching, graceful disconnect, abrupt death,
  `PartialMessage` retry/skip, checkpoint rebuilding, bounded context
  reconstruction, and exclusion of `PartialMessage` output and hidden
  reasoning;
- negative sandbox tests for unauthorized reads/writes, home and credential
  paths, symlink/hardlink escapes, network access, environment leakage, Git
  metadata writes, orphan processes, and permissive fallback attempts;
- release conformance tests for every advertised host, runtime version, and
  adapter version tuple;
- MCP lifecycle/concurrency and activation-time artifact/schema digest tests;
- Git integration tests proving merge and push approvals are separate and exact;
- transcript tests proving independent first passes remain sealed and untrusted
  control sequences cannot affect terminal state; and
- opt-in live smoke tests that require user-provided connections and never run in
  the default test suite.

Tests use synthetic market and account data. No default test requires a paid
provider, subscription, live broker, or real secret.

## 19. Deferred beyond version 1

The following are deliberate extensions, not missing foundation work:

- Hermes or another external discussion-agent runtime adapter;
- Herdr or another general-purpose external terminal supervisor; the narrow
  fail-closed per-job guardian required above is internal version 1 machinery;
- virtual-machine or remote worker execution;
- unrestricted or permission-bypass coding modes;
- web, mobile, or remote-attachment clients;
- background-daemon room execution;
- multiple simultaneously auto-running rooms or simultaneous visible speakers;
- multi-user accounts, remote hosting, and team authorization;
- community MCP installation and arbitrary executable skills;
- dynamically loaded domain-code plugins;
- broker write access, order staging, or order execution;
- automatic runtime switching by an agent;
- unrestricted peer-to-peer agent messaging;
- unsupported cross-expiration option structures; and
- automatic merge, push, deployment, or release.

## 20. External interface references

Implementation should verify current CLI flags against official documentation
rather than treating examples in this document as immutable command lines:

- [OpenAI Codex non-interactive mode](https://developers.openai.com/codex/noninteractive/)
- [Claude Code headless mode](https://code.claude.com/docs/en/headless)
- [Claude Code sandboxing](https://code.claude.com/docs/en/sandboxing)
- [Model Context Protocol specification](https://modelcontextprotocol.io/specification/)
- [Mermaid flowchart direction](https://mermaid.js.org/syntax/flowchart.html)
- [Mermaid theme configuration](https://mermaid.js.org/config/theming.html)

## 21. Documentation precedence

When documents disagree, use this order:

1. the user's latest explicit decision;
2. this architecture;
3. [phases.md](phases.md);
4. a phase-specific implementation plan approved after these documents; then
5. older specifications and plans as historical context only.

The next step after review is to write a detailed implementation plan for Phase
0. Existing Python, React, FastAPI, and Hermes-first plans must not be executed
against this architecture without being rewritten and approved.
