# AI Stock Forum: Design Specification

**Status:** Draft for final document review; each design section was approved in
conversation

**Date:** 2026-08-08

**Companion:** [`architecture.md`](../../../architecture.md)

## 1. Summary

AI Stock Forum is a local decision-support application for stock and options
swing trades lasting roughly several days to several weeks. A fixed committee
of state-isolated Hermes agents analyzes a shared, timestamped evidence snapshot from
different specialties and conducts one structured challenge round. A completed
analysis produces one of three recommendation outcomes:

1. `TRADE` — one exact, risk-validated plan is recommended;
2. `SPLIT_DECISION` — material disagreement remains and is exposed;
3. `NO_TRADE` — the completed analysis finds that risk or evidence does not
   justify a trade.

`ANALYSIS_INCOMPLETE` is a separate operational result used when a failure
prevents a valid recommendation outcome.

The system may recommend long stock or allowlisted defined-risk options
structures. The user can approve, reject, or watchlist a recommendation, but
version 1 has no ability to place, stage, preview, or transmit an order.

## 2. Product principles

1. **Adversarial analysis, shared facts.** Agents may interpret the evidence
   differently, but important inputs come from a common immutable snapshot.
2. **Evidence outranks personality.** Personalities influence investigative
   style and voice, never citation, schema, tool, or risk requirements.
3. **Judgment is probabilistic; safety is deterministic.** Agents form theses;
   ordinary code validates structures, arithmetic, limits, and state changes.
4. **No trade is a useful answer.** The system is not required to manufacture
   activity. A no-trade result must identify the evidence and thresholds that
   caused it.
5. **Failure is not neutrality.** Missing data or agents yield
   `ANALYSIS_INCOMPLETE`, never a disguised `NO_TRADE`.
6. **Approval is narrow.** Approval applies to an exact plan version and does
   not grant execution authority.
7. **History is append-only.** Later outcomes can evaluate earlier reasoning
   but cannot rewrite what was known at decision time.

## 3. Goals

- Run multiple locally managed Hermes profiles with fixed specialties and
  distinct personalities.
- Use ChatGPT subscription access through Hermes's `openai-codex` OAuth
  provider rather than requiring OpenAI API billing for the initial design.
- Incorporate read-only brokerage, options-chain, GEX, news, filings, and market
  evidence with timestamps and provenance.
- Produce transparent bull, bear, fundamental, technical, options, liquidity,
  and risk analysis.
- Support defined-risk multi-leg strategies rather than limiting output to
  single-leg calls and puts.
- Give the user an exact entry, sizing, management, and exit plan when a trade
  is justified.
- Stream the debate and evidence into a comprehensible local dashboard.
- Persist agent identity, audit history, decisions, and outcome scorecards using
  the approved hybrid-memory design.
- Make every run reconstructable from saved evidence, prompts, outputs, and
  configuration, while reproducing deterministic calculations exactly. LLM
  regeneration is not expected to be byte-for-byte identical.

## 4. Non-goals for version 1

- Autonomous, scheduled, or unattended trading.
- Any broker write capability, including previewing, staging, replacing, or
  cancelling orders.
- Naked short options, uncovered ratio spreads, margin short stock, or any
  structure whose worst-case loss cannot be proved by the engine.
- Intraday scalping, zero-DTE automation, portfolio rebalancing, tax advice, or
  personalized fiduciary advice.
- Multi-user accounts, public hosting, mobile applications, or remote access.
- Free-form peer-to-peer agent messaging.
- Training or fine-tuning models based on trade outcomes.
- Treating a simple majority vote as sufficient evidence for a recommendation.

## 5. User-visible workflow

### 5.1 Request

The user supplies a symbol and may supply a question, directional idea, desired
holding period, or event of interest. The application adds the active risk
profile, account snapshot, open-position context, and market-session metadata.

The request cannot start until required risk limits are configured. Limits are
explicit user settings, not values invented by an agent. They include at least:

- maximum loss per trade as dollars and/or account-equity percentage;
- maximum aggregate open defined risk;
- maximum capital allocation for long stock;
- minimum and maximum days to expiration;
- liquidity thresholds and slippage assumptions;
- earnings and other binary-event policy; and
- permitted strategy templates.

If both dollar and percentage limits are present, the stricter result controls.

### 5.2 Preflight and evidence envelope

The coordinator validates the symbol, account/risk configuration, market-data
availability, required agent health, and provider capacity. It asks the
fundamental, technical, bull, and bear profiles for one bounded round of
role-specific evidence queries. The coordinator executes the union of those
queries through controlled read-only acquisition adapters; profiles do not
receive arbitrary web or MCP access.

An `EvidenceQueryPlan` contains source enums and typed parameters, never an
arbitrary URL or command. Initial source kinds are `SEC_FILING`, `COMPANY_IR`,
`NEWS_SEARCH`, `MACRO_CALENDAR`, `MARKET_DATA`, `OPTION_CHAIN`, and `GEX`. Each
request has a symbol/issuer identifier, bounded search terms, time range,
maximum-result count, and rationale. The coordinator rejects unknown tools,
schemes, hosts, parameters, and sizes.

Search/fetch adapters allow HTTPS only, resolve and re-check DNS, block loopback,
private, link-local, multicast, and cloud-metadata addresses, revalidate every
redirect, cap redirects/response bytes/time, enforce expected content types, and
never forward local credentials or authorization headers. MCP requests use
named read-only operations with strict typed arguments rather than model-supplied
method names.

Every evidence item receives a trust tier: `PRIMARY`, `REPUTABLE_SECONDARY`, or
`UNVERIFIED`. A material catalyst or numeric claim must be supported by a
primary source or corroborated by two independent reputable sources. Unverified
evidence can generate a disclosed question but cannot independently satisfy a
recommendation criterion. Conflicting sources remain visible rather than being
silently merged.

The coordinator then freezes one immutable `EvidenceEnvelope` with two
partitions. Its `MarketEvidenceSnapshot` contains:

- underlying quote and market session;
- option-chain quotes, greeks, volume, open interest, and quote timestamps;
- volatility measurements and term/skew context when available;
- GEX levels and source metadata;
- company events, earnings timing, filings, and cited news;
- macro events relevant to the intended holding window;
- every source's retrieval time, observed time, publication time when known,
  provider name, locator, and content hash; and
- the risk-policy and calculation versions active for the run.

Its sensitive `AccountRiskSnapshot` contains account equity, relevant positions,
open orders, and existing/reserved risk. Raw account data is available only to
the coordinator and deterministic risk engine. LLM roles receive the shared
market projection plus the minimum derived constraints needed for their job,
such as a per-trade loss budget; no role receives both open-web access and raw
account data.

Before sizing, every live position and open order must normalize into a
supported conservative exposure model. Unsupported adjusted contracts,
multi-expiration combinations, uncovered shorts, unknown deliverables, or
unparseable orders cause `ANALYSIS_INCOMPLETE` and set remaining approval
capacity to zero. The system does not ignore an exposure merely because version
1 cannot model it. This check runs again during the pre-approval refresh.

The frozen envelope never changes. Specialists cannot fetch supplemental
evidence during their independent briefs. If a role identifies material missing
evidence, the coordinator either returns `ANALYSIS_INCOMPLETE` or builds a new
envelope and restarts every dependent brief. Materially changed quotes likewise
require a new envelope and recommendation version.

### 5.3 Independent briefs

The coordinator sends every specialist the same frozen market snapshot, a
least-privilege role projection, and a fixed role mandate. First-pass calls run
with bounded concurrency and no agent sees another agent's initial conclusion.
No live external tools are available during this phase. This avoids early
anchoring and timing-dependent inputs.

Each mandatory brief must include:

- a concise conclusion and calibrated confidence;
- observations separated from interpretations;
- evidence-item references for every material factual claim;
- assumptions and invalidation conditions;
- the strongest evidence against its own conclusion;
- missing-data warnings; and
- role-specific structured fields.

Invalid responses receive at most the configured limited retry count. A missing
mandatory brief after retries results in `ANALYSIS_INCOMPLETE`.

### 5.4 Candidate construction

After first-pass briefs, the options strategist constructs at most three total
candidates, counting `NO_TRADE` as a candidate. For this responsibility it acts
as the trade-structure strategist and may recommend cash-funded long stock as a
simpler baseline. A trade candidate must be fully specified rather than
described generically.

The strategist may choose a simpler structure when it better expresses the
thesis. Complexity earns no preference by itself.

### 5.5 Cross-examination

The coordinator creates targeted challenges, such as sending the bull thesis to
the bear or a proposed spread to the risk/liquidity role. Each challenged role
gets exactly one rebuttal round. New factual claims still require evidence-item
references.

There is no open-ended group chat. When the challenge round ends, the record is
closed and sent to deterministic validation.

### 5.6 Risk gate and repair

The deterministic risk engine validates each trade candidate. A repairable
failure may be returned once to the options strategist with explicit rejection
codes and constraints. The repaired plan is validated from scratch once. A
candidate that still fails is ineligible for recommendation.

The risk/liquidity agent can warn about risks beyond the deterministic model but
cannot waive an engine failure or change a calculation.

### 5.7 Moderation

The neutral moderator receives only the saved briefs, rebuttals, validated
candidates, deterministic calculations, and dissent record. It must return one
of the three recommendation outcomes defined in this specification.

The moderator weighs evidence quality, relevance, freshness, uncertainty, and
role calibration. It does not use a simple vote count. It cannot create a new
trade, change legs, add facts, or override the risk engine.

A `SPLIT_DECISION` explains the unresolved issue and may display validated
alternatives. It is not directly approval-eligible. Selecting an alternative
creates a new exact recommendation version that must pass the risk gate.

### 5.8 Human decision

For an approval-eligible `TRADE`, the user can:

- approve the exact recommendation version;
- reject it with an optional reason; or
- add it to a watchlist with conditions for fresh analysis.

`NO_TRADE`, `SPLIT_DECISION`, and `ANALYSIS_INCOMPLETE` can be acknowledged or
watchlisted but are not approval-eligible. No decision path invokes a broker
write tool.

## 6. Fixed agent specialties

### 6.1 Fundamental and catalyst analyst

This agent is a patient, numbers-first investigator that distrusts unsupported
narratives. It examines business quality, reported results, valuation context,
earnings, guidance, filings, company-specific news, macro sensitivity, and
scheduled events. It must distinguish reported facts, consensus estimates, and
its own interpretation.

### 6.2 Technical and GEX analyst

This agent is tactical, visually minded, and precise about levels without
pretending they are certainties. It examines multi-timeframe trend,
support/resistance, volume, realized and implied volatility regime, options
positioning, dealer-gamma context, and user-provided GEX levels. GEX is
contextual evidence, not a stand-alone trade signal.

### 6.3 Bull advocate

This agent is an energetic opportunity seeker, but not a cheerleader. It builds
the strongest supportable upside case, identifies positive asymmetry, and states
what would confirm or disprove it. It must address the strongest known bearish
evidence.

### 6.4 Bear advocate

This agent is a forensic skeptic and red-team critic, but not a permanent bear.
It builds the strongest supportable downside or avoidance case, challenges
crowded assumptions, and identifies gap, catalyst, valuation, and positioning
risks. It must provide falsifiable objections rather than generic caution.

### 6.5 Options strategist

This agent is a pragmatic structure engineer that prefers the simplest strategy
which expresses the thesis well. It maps the debated distribution, time
horizon, volatility view, and catalysts to allowlisted structures. It supplies
exact legs, quantities, entry assumptions, and lifecycle rules. It cannot
self-certify risk validity.

### 6.6 Risk and liquidity officer

This agent is a blunt capital-preservation officer that makes tradeoffs explicit
instead of using vague warnings. It challenges concentration, gap exposure,
volatility crush, bid/ask quality, open interest, early assignment, pin risk,
event timing, and the practicality of entry and exit. Its narrative assessment
supplements deterministic validation.

### 6.7 Neutral moderator

This agent is a calm, transparent judge that is comfortable with uncertainty.
It synthesizes the closed record, preserves material dissent, compares
candidates, and selects the final outcome. Neutrality means calibrated judgment,
not forcing a compromise.

## 7. Hermes integration

### 7.1 Profile isolation

Each specialty runs as a separate, state-isolated Hermes profile with its own:

- `SOUL.md` role/personality instructions;
- sessions and working state;
- curated role-memory projection;
- capability allowlist and gateway configuration; and
- local gateway port.

Profiles communicate only through the coordinator. The application uses the
Hermes OpenAI-compatible HTTP interface rather than importing Hermes private
implementation modules. At startup and before each run, it checks profile
health and capabilities.

Hermes profiles isolate configuration and state; they are not treated as
filesystem, process, or network security boundaries. In live-data mode the
launcher runs each gateway in a separate rootless Podman container and must:

- disable terminal/shell, filesystem-write, code-execution, process-control,
  cron/scheduling, delegation/MoA, skill or plugin installation, unrestricted
  HTTP, and automatic memory-write capabilities;
- provide only explicit profile configuration/session volumes, never mount the
  host home directory or container-engine socket, sanitize environment variables
  and inherited file descriptors, and use a read-only root filesystem with
  explicit writable state/tmp mounts;
- route outbound HTTPS through an allowlisted proxy limited to the approved
  model-provider endpoints and explicitly approved local gateway endpoints; and
- keep broker credentials and the application's sensitive account artifacts
  unreadable by profile processes.

An unsandboxed developer mode may operate only on recorded or synthetic
fixtures. Account/GEX credentials, live evidence acquisition, and approval are
disabled unless the container and egress controls pass startup verification.

Each analysis creates one explicit Hermes session per `(run_id, role)`. That
session may continue from initial brief to rebuttal inside the same run and is
then archived. Transcripts are never reused as context across runs. Stable
persona policy is immutable configuration; outcome-memory retrieval is selected
and injected by the coordinator.

### 7.2 ChatGPT subscription provider

Hermes profiles use the `openai-codex` provider authorized through ChatGPT OAuth.
The version 1 baseline requires each profile to complete Hermes's supported
device-authorization flow and retain its credential in that profile's private
container volume. A future pinned Hermes version may replace this with an
officially documented credential-owning gateway, but the application never
copies, symlinks, or parses refresh-token or `auth.json` files. The design
assumes all profiles share one account-level capacity pool; it does not assume
that seven profiles provide seven independent quotas.

Every Hermes gateway has a distinct `API_SERVER_KEY`. Gateway keys are supplied
to the coordinator through OS credential storage or launch-time secrets, never
SQLite, prompts, browser payloads, or logs.

The coordinator therefore provides:

- a configurable global concurrency semaphore;
- a per-run provider-call, token, and wall-time budget;
- exactly one model iteration per Hermes request, with native Hermes delegation,
  MoA, compression/summarization, background work, and auxiliary model paths
  disabled;
- per-profile request timeouts;
- bounded retry with jitter for transient provider failures;
- explicit rate-limit and authentication failure events;
- cancellation when a run is superseded; and
- whole-pool backoff when provider reset metadata indicates shared exhaustion.

A pinned-version compatibility test must demonstrate the one-request/one-model-
iteration boundary before live mode is enabled. Evidence projections are sized
deterministically to fit the context window; Hermes may not trigger hidden model
compression. If the boundary or context budget cannot be satisfied, the run
fails explicitly rather than consuming unadmitted auxiliary calls.

A provider limit or OAuth failure produces `ANALYSIS_INCOMPLETE` if mandatory
work cannot finish. The product must not fall back silently to an unapproved
provider or billable API key.

### 7.3 MCP and tool policy

Brokerage, account, option-chain, search, fetch, and GEX acquisition adapters are
owned by the coordinator and are read-only. Research profiles output bounded
query plans during evidence discovery; the coordinator executes them, records
the results, and freezes the union before analysis. Debate profiles have no
external data tools. The moderator receives only the closed record.

The GEX MCP and brokerage MCP/server must expose read capabilities only, not
merely hide write tools in the UI. Tool inputs, outputs, timestamps, errors, and
provider identities are recorded. Webpage text and MCP payload strings are
treated as untrusted data and never as instructions that can modify the
workflow, tool policy, or evidence-acquisition scope.

## 8. Deterministic risk engine

### 8.1 Responsibility

The risk engine is a pure, versioned application module. Given identical
candidate, snapshot, account, cost, and risk-policy inputs, it returns an
identical result. It does not predict direction or decide whether the thesis is
persuasive.

### 8.2 Required inputs

- normalized underlying and option legs;
- option contract identifier, deliverable, settlement style, exercise style,
  currency, and multiplier;
- proposed entry limit and side of market;
- saved bid, ask, midpoint, volume, and open interest;
- commissions and conservative slippage configuration;
- account equity, relevant positions, open orders, active approval reservations,
  and aggregate open-risk estimate;
- holding window, expiration, event calendar, and freshness requirements; and
- the versioned user risk policy.

### 8.3 Required calculations and checks

For options, the engine constructs the combined payoff from the actual legs and
evaluates expiration outcomes over all payoff breakpoints and both tails. It
separately stress-tests early assignment, exercise, expiration, and pin
scenarios because expiration payoff alone does not bound every operational cash
or stock obligation.

The operational scenario generator is mandatory and cannot be configured away.
At each relevant lifecycle boundary it enumerates every feasible subset of short
legs assigned and every long-leg exercise/non-exercise state, including
exercise-by-exception, contrary exercise, sequential assignment, simultaneous
assignment, expiration cutoff, and overnight stock/cash states. Configuration
sets rejection thresholds and management deadlines; it cannot remove safety
paths from the scenario set.

The engine must compute or explicitly mark not applicable:

- net debit or credit;
- `expiration_payoff_max_profit` and `expiration_payoff_max_loss` per unit and
  for the proposed quantity, including configured transaction-cost assumptions;
- breakeven points;
- required cash or buying-power estimate;
- maximum gross assignment stock notional, exercise cash requirement, and
  temporary stock quantity under configured stress scenarios;
- dividend, corporate-action, early-assignment, and expiration/pin-risk gates;
- risk/reward measures without hiding unbounded or undefined values;
- permitted quantity based on the strictest active limit;
- quote age, spread width, volume, open interest, and estimated slippage;
- days to expiration and holding-window compatibility;
- earnings and other configured event-policy compliance;
- candidate-only and post-trade portfolio payoff;
- standard-contract, underlying, deliverable, settlement, exercise-style,
  currency, multiplier, and expiration consistency; and
- valid opening actions, protective-leg ratios, and packaged-entry requirements.

Version 1 rejects adjusted or nonstandard contracts. All hedging legs in one
candidate must use the same standard underlying, deliverable, settlement style,
exercise style, currency, multiplier, and expiration. Every `SELL_TO_OPEN`
option must be covered at the necessary ratio by a `BUY_TO_OPEN` option in the
same self-contained candidate. A candidate cannot rely on an existing holding,
a future adjustment, or a `SELL_TO_CLOSE` leg to become bounded.

Every multi-leg candidate is presented as one atomic complex-order package with
a net limit; legging into the position is prohibited by the recommendation.
Short-leg structures must include a mandatory close-before-expiration rule and
must pass account-capacity gates for stressed assignment/exercise exposure.
Configured ex-dividend, low-extrinsic-value, corporate-action, and pin-risk
conditions are hard failures, not warnings that the moderator may waive.

Position sizing follows this shape:

```text
per_trade_budget = min(configured dollar cap,
                       account equity × configured percentage cap,
                       remaining aggregate risk capacity after
                       positions, open orders, and active approvals)

quantity = floor(per_trade_budget /
                 conservative expiration-payoff maximum loss per unit)
```

Further liquidity, gross-assignment-capacity, or allocation limits may reduce
the quantity. If the result is less than one, the candidate fails. An intended
stop loss is never used to claim that an otherwise unbounded structure has
defined risk. For long stock, the engine uses the full purchase debit as
theoretical worst-case loss rather than assuming a stop will fill.

### 8.4 Initial allowlist

The initial UI and schema support:

- cash-funded long stock without margin;
- long calls and long puts;
- vertical debit spreads;
- bounded-risk vertical credit spreads with a protective long leg;
- standard butterflies and iron butterflies; and
- iron condors.

Every candidate must still pass payoff-based bounded-loss validation. Strategy
labels are descriptive, not trusted assertions.

The following are rejected in version 1:

- any naked short option;
- short straddles or strangles;
- uncovered or conditionally uncovered ratio spreads;
- margin short stock;
- structures that rely on a future adjustment to become bounded; and
- calendars, diagonals, or other multiple-expiration structures.

Calendars and diagonals can be considered later only with reviewed models for
path-dependent exposure, expiration transitions, exercise, and assignment.

## 9. Recommendation contract

An approval-eligible recommendation contains:

### Identity and thesis

- symbol, asset name, outcome, directional bias, and recommendation ID/version;
- intended entry window and holding period;
- concise thesis, confidence, key assumptions, and thesis invalidation; and
- evidence-envelope ID and generation timestamp.

### Exact trade specification

- strategy name and whether it is stock or options;
- for each option leg: opening action, quantity, option type, expiration, strike,
  contract multiplier, saved bid/ask, and quote timestamp;
- for stock: side, quantity, saved quote, and cash requirement;
- total quantity, proposed net limit, acceptable entry condition, and a
  do-not-chase boundary; and
- for multi-leg options: one atomic complex-order package and an explicit
  prohibition on legging into the trade; and
- explicit instruction to rerun analysis when the plan becomes stale.

### Risk and lifecycle

- deterministic expiration-payoff maximum loss and profit, breakevens,
  buying-power estimate, account-risk percentage, reward/risk representation,
  and calculation version;
- stressed assignment/exercise gross exposure, required account capacity, and
  applicable dividend, corporate-action, and pin-risk gates;
- planned profit-taking, loss/thesis invalidation, time stop, expiration exit,
  event handling, and assignment/exercise management;
- liquidity and slippage warnings; and
- scenario analysis for expected, adverse, and invalidated cases.

Lifecycle exits are recommendations, not guaranteed fills. The displayed
expiration-payoff maximum loss is based on the structure itself, not on the
planned exit succeeding. Operational assignment/exercise exposures are shown
separately and must pass their own capacity gates.

### Evidence and dissent

- supporting evidence references;
- the strongest contrary evidence;
- each specialist's final confidence and unresolved objections;
- data freshness and missing-data disclosures; and
- why this candidate was preferred to the alternatives or why no trade won.

The application serializes the normalized recommendation, evidence-envelope ID,
risk-policy version, and calculation version into a content hash. User approval
binds to that hash.

## 10. Contract conventions

All application and agent contracts are versioned Pydantic models with generated
JSON Schema checked into the implementation. Provider-enforced JSON Schema is
not assumed: the coordinator validates every response itself and may retry with
structured validation errors. It never uses regex or silent coercion to repair a
trade plan.

Normative conventions are:

- every top-level object contains `schema_version` and an immutable UUID;
- timestamps are timezone-aware ISO 8601 UTC values and source observations
  distinguish `observed_at`, `published_at`, and `retrieved_at`;
- money, strikes, prices, greeks, ratios, and percentages serialize as decimal
  strings with an explicit currency or unit rather than binary JSON floats;
- quantities are positive integers and option actions are restricted to
  `BUY_TO_OPEN` and `SELL_TO_OPEN` in new candidates;
- option legs reference immutable `contract_id` and `quote_id` records instead
  of reconstructing symbols from display text;
- evidence citations are arrays of immutable `evidence_item_id` values;
- enums use the uppercase values defined by this document;
- unknown fields are rejected, required fields cannot silently become null, and
  optional/null semantics are defined explicitly in each generated schema; and
- recommendation hashing uses canonical UTF-8 JSON with stable key ordering,
  normalized decimal strings, and no presentation-only fields.

The principal contracts are:

| Contract | Required core fields |
|---|---|
| `AnalysisRequest` | Symbol, user question, horizon, risk-policy version, account-snapshot request |
| `EvidenceQueryPlan` | Allowlisted source enum, typed identifiers/search terms, time bound, result cap, rationale |
| `EvidenceEnvelope` | Shared market snapshot, sensitive account snapshot, role-projection hashes, provenance |
| `EvidenceItem` | Source type/provider/locator, trust tier, timestamps, content hash, normalized observation |
| `SpecialistBrief` | Role, conclusion, confidence, observations, interpretations, evidence IDs, assumptions, invalidations, contrary evidence |
| `CandidatePlan` | Asset type, strategy, self-contained legs, atomic package limit, lifecycle rules, evidence IDs |
| `RiskAssessment` | Exact input hashes, payoff metrics, operational stress exposure, sizing, rule results, engine version |
| `RecommendationVersion` | Outcome, selected validated candidate if any, dissent, snapshot/policy/engine versions, content hash |
| `UserDecision` | Decision enum, recommendation hash, pre-decision refresh ID, reservation ID, timestamp |
| `CapacityReservation` | Expiration-loss, cash/buying-power, temporary shares/notional, concentration, validity/reconciliation state |

The implementation plan may split these contracts into smaller types, but may
not weaken the validation or provenance rules above.

## 11. Persistence and hybrid memory

### 11.1 Application records

SQLite is the version 1 system of record. At minimum the schema represents:

| Record | Purpose |
|---|---|
| `analysis_runs` | Request identity, state, timing, configuration versions, and terminal outcome |
| `evidence_envelopes` | Immutable shared/sensitive partitions, role-projection hashes, and content hash |
| `evidence_items` | Source-backed frozen evidence with provenance |
| `agent_invocations` | Profile, prompt/input hash, structured output, timing, retries, and errors |
| `specialist_briefs` | Validated conclusions, confidence, assumptions, and citations |
| `candidate_plans` | Versioned stock or multi-leg trade candidates |
| `risk_assessments` | Deterministic inputs, outputs, rule results, and engine version |
| `recommendation_versions` | Final normalized outcomes and approval content hashes |
| `user_decisions` | Approve, reject, acknowledge, or watchlist action and timestamp |
| `risk_reservations` | Active multi-dimensional capacity vector, review deadline, release, and reconciliation state |
| `outcome_evaluations` | Later scorecards linked without mutating the original record |
| `system_events` | Append-only state, streaming, tool, timeout, and audit events |

SQLite uses transactions, foreign keys, migrations, and uniqueness constraints
for immutable versions. Large raw source payloads may be stored as content-
addressed local artifacts while the database stores their hashes and metadata.

### 11.2 Memory layers

1. **Stable persona memory:** read-only profile configuration containing the
   fixed specialty, personality, rubric, and capability policy. Outcome data
   cannot rewrite it automatically.
2. **Per-run working memory:** the active snapshot, briefs, challenges, and
   candidate context. It is isolated by run and not treated as durable truth.
3. **Outcome memory:** append-only compact scorecards about recommendation,
   decision, realized path, rule adherence, and confidence calibration.

Future prompts may retrieve a small number of relevant outcome summaries using
symbol, regime, strategy, and catalyst tags. They do not inject full historical
transcripts by default. Historical replay fixes an `as_of` boundary so later
prices, outcomes, and revised data cannot leak into the analysis.

## 12. Application architecture

### 12.1 Backend

Python 3.12+, FastAPI, Pydantic, SQLAlchemy, Alembic, and `asyncio` provide:

- request validation and API endpoints;
- a persisted debate state machine;
- Hermes HTTP clients and concurrency control;
- controlled evidence acquisition, role projection, normalization, and
  provenance;
- deterministic risk calculations;
- recommendation versioning and decision recording; and
- Server-Sent Event publication.

The job runner is a bounded in-process asynchronous service for version 1. Run
state is persisted at each transition so a process restart can identify and
safely fail or resume eligible work without duplicating decisions.

### 12.2 Frontend

React, TypeScript, and Vite provide:

- symbol/request entry and risk-profile visibility;
- a live phase timeline and per-agent status;
- specialist cards with claims, confidence, citations, and dissent;
- a shared evidence drawer with timestamps and freshness indicators;
- candidate comparison and deterministic risk details;
- prominent distinction among `NO_TRADE`, `SPLIT_DECISION`, and
  `ANALYSIS_INCOMPLETE`; and
- an exact recommendation review with approval controls enabled only when the
  backend marks the current version eligible.

The default recommendation report is decision-first. Its header shows symbol,
horizon, evidence-envelope ID, data age, outcome, and calibrated confidence. The
main navigation exposes `Overview`, `Debate`, `Evidence`, `Payoff`, and `Audit`.
On wide screens the overview uses two columns:

- the main column contains the moderator thesis, exact packaged legs, payoff and
  operational-exposure metrics, entry/exit/invalidation rules, supporting and
  opposing evidence, and every specialist's stance; and
- the always-visible side column contains deterministic gate results, market/GEX
  levels, freshness, active reservations, and the human-decision controls with
  an explicit “no order is sent” message.

Agent stance counts are transparency metadata, not a vote-based decision score.
`NO_TRADE`, `SPLIT_DECISION`, and `ANALYSIS_INCOMPLETE` retain the same evidence
and audit access but never display an enabled approval control.

SSE is used because updates primarily flow from server to browser. Mutating user
actions remain ordinary authenticated local HTTP requests. The local UI uses an
ephemeral same-site session, rejects unexpected `Host` and `Origin` values,
requires CSRF protection for mutations, and does not enable permissive CORS.

### 12.3 Deployment boundary

The web application and SQLite database run directly on one local machine. In
live-data mode, seven Hermes gateways run in separate rootless Podman containers
and publish distinct loopback ports. The launcher applies and verifies the
mount, environment, file-descriptor, read-only-root, capability, and
provider-egress restrictions specified in section 7.1. The rest of the
application does not require Docker Compose, Kubernetes, Redis, Celery, Kafka,
or a remote database.

Fixture-only development mode may run gateways without containers, but the
backend refuses live account/GEX credentials and disables approval in that mode.

Remote access is a separate design because it introduces user authentication,
TLS, network exposure, secret management, and session-security requirements.

## 13. Workflow states and failure semantics

Run states, recommendation outcomes, and user decisions are separate fields.
Canonical run states are:

```text
CREATED
PREFLIGHT
EVIDENCE_DISCOVERY
SNAPSHOTTING
INDEPENDENT_BRIEFS
CANDIDATE_CONSTRUCTION
CROSS_EXAMINATION
RISK_VALIDATION
REPAIR
MODERATION
AWAITING_DECISION
COMPLETED | ANALYSIS_INCOMPLETE | SUPERSEDED | CANCELLED
```

Recommendation outcomes are `TRADE`, `SPLIT_DECISION`, and `NO_TRADE`. User
decisions are `APPROVED`, `REJECTED`, `WATCHLISTED`, and `ACKNOWLEDGED`, subject
to the eligibility rules in section 5.8. A recorded decision moves an active
run from `AWAITING_DECISION` to `COMPLETED`; the decision itself remains a
separate immutable record.

Only the state machine changes run state. Each transition emits an append-only
event. At most one repair cycle is permitted. Retry counts for transport or
schema errors are bounded and are distinct from the repair cycle.

Mandatory failures include:

- failed live-mode container, egress, gateway-key, or capability verification;
- unavailable or unhealthy required Hermes profiles;
- ChatGPT OAuth or provider capacity failure after bounded retries;
- missing required market/account inputs;
- unsupported or unparseable live positions/open orders;
- invalid evidence-query plans after schema retry;
- stale evidence beyond configured thresholds;
- malformed mandatory briefs after retry;
- inconsistent snapshot or recommendation hashes; and
- internal risk-engine errors.

These produce `ANALYSIS_INCOMPLETE` and disable approval. If all actionable
candidates fail deterministic candidate rules but the evidence and debate
completed, the moderator may return `NO_TRADE` with the risk-engine rejection
codes as evidence.

## 14. Approval integrity

- Only a `TRADE` recommendation that passed the current risk-policy and engine
  versions is approval-eligible.
- Approval requires the recommendation content hash supplied by the dashboard.
  On click, the backend obtains a fresh read-only quote, account, positions, and
  open-orders snapshot; enforces configured quote/account TTL and materiality
  thresholds; and reruns validation and sizing.
- If refreshed inputs change any material recommendation field, the backend
  supersedes the displayed version and requires the user to review a new version
  instead of accepting the stale hash.
- Approval and capacity reservation occur in one database transaction. Each
  reservation is a vector containing expiration-payoff loss, cash/buying-power
  requirement, temporary share quantity/gross notional, and symbol/sector
  concentration. Active approvals are atomically subtracted in every dimension
  so multiple unexecuted plans cannot spend the same operational capacity.
- Every approval has a validity TTL. Expiry prevents the plan from remaining
  approval-current but does not silently free its reservation. A later read-only
  account refresh may reconcile the reservation to a matching position only
  when the match is unambiguous.
- A user request to mark a plan abandoned triggers a fresh positions/open-orders
  read. The reservation is released only when that read finds no matching or
  ambiguous exposure. If account data is unavailable, stale, or ambiguous, the
  reservation remains counted; version 1 provides no unchecked release override.
- Changing a leg, quantity, entry price, snapshot, account state, risk setting,
  engine version, or relevant evidence creates a new version and invalidates
  approval.
- The approval record stores user action, time, displayed hash, pre-approval
  refresh snapshot, recommendation version, capacity-reservation vector,
  approval-valid-until time, and reservation reconciliation state.
- There is deliberately no broker order-submission request, broker-write client,
  or execution state in version 1.

Watchlists are passive records in version 1. They do not poll, schedule a scan,
or refresh a recommendation automatically.

## 15. Security and trust boundaries

- All application and Hermes HTTP listeners bind to `127.0.0.1` by default.
- OAuth credentials remain in Hermes/provider-managed local credential storage
  and never enter prompts, logs, SQLite, or frontend payloads.
- Brokerage and MCP credentials are similarly excluded from application logs
  and model context unless a narrowly required non-secret field is normalized.
- Hermes profile separation is not a security boundary. Dangerous capabilities
  are disabled, and process/filesystem/network restrictions are enforced by the
  launcher rather than by prompt text.
- External acquisition is coordinator-owned and allowlisted. Brokerage and MCP
  servers expose read-only capabilities at the server layer; profile processes
  do not receive their credentials.
- Every Hermes gateway requires its own bearer key, and the dashboard uses
  same-site session, origin, and CSRF controls even on loopback.
- The application validates all agent output against strict schemas and treats
  it as untrusted until validation succeeds.
- External text is quoted/labeled as evidence and cannot alter system prompts,
  workflow rules, tool permissions, or approval eligibility.
- Logs and exported audit records redact secrets and sensitive account
  identifiers.
- Remote network exposure and broker execution require new threat models and are
  prohibited from being enabled as configuration-only changes.

## 16. Observability

The local dashboard and structured logs expose:

- run state and elapsed time per phase;
- profile health, invocation latency, retry, and rate-limit status;
- evidence age and provider errors;
- validation and deterministic rejection codes;
- global concurrency utilization;
- provider iteration/token/time budgets and whole-pool backoff state;
- active approval validity, reservations, and reconciliation state;
- recommendation/snapshot version changes; and
- user decisions and subsequent outcome-evaluation status.

Prompts and outputs are stored for local audit with secret redaction. The UI
shows human-readable errors without converting infrastructure failures into
market conclusions.

## 17. Testing strategy

### 17.1 Unit tests

- Hand-calculated payoff cases for every allowlisted strategy.
- Boundary and property tests across strikes, credits/debits, quantities,
  multipliers, fees, and risk limits.
- Rejection of uncovered tails, malformed legs, stale quotes, and unsupported
  expiration combinations.
- Rejection of adjusted/nonstandard contracts, mismatched deliverables,
  insufficient protective ratios, non-opening legs, and legged entries.
- Assignment/exercise gross-exposure, ex-dividend, low-extrinsic-value,
  expiration, and pin-risk gates.
- Exhaustive short-assignment-subset and long-exercise/non-exercise lifecycle
  scenario generation that cannot be disabled by configuration.
- Position-size rounding and strictest-limit behavior.
- Recommendation hashing, approval refresh/invalidation, and atomic reservation
  behavior under concurrent approval attempts.
- State-machine transition rules and retry/repair limits.

### 17.2 Contract tests

- Fake Hermes gateways for health, capabilities, structured output, streaming,
  timeout, rate-limit, and authentication behavior.
- Verification that every gateway requires its distinct bearer key and that
  forbidden profile capabilities are absent.
- Pinned-Hermes verification that tool-free `max_iterations=1` requests cannot
  trigger compression, delegation, or other auxiliary provider calls.
- Recorded read-only MCP fixtures and schema-change detection.
- Proof that configured MCP servers expose no broker-write capability.
- Pydantic validation for all specialist, candidate, risk, and moderator output.
- Evidence citation resolution and provenance completeness.
- `EvidenceQueryPlan` rejection of arbitrary URLs/operations plus SSRF, DNS
  rebinding, redirect, response-limit, content-type, and credential-forwarding
  tests.
- Source trust-tier and material-claim corroboration rules.

### 17.3 Workflow integration tests

- Bullish, bearish, neutral, and ambiguous completed debates.
- One invalid candidate repaired successfully and one rejected after repair.
- Missing mandatory role, malformed output, stale data, interrupted provider,
  and unavailable MCP paths.
- Frozen evidence inputs across concurrent briefs and complete brief restart
  when a material evidence envelope changes.
- Role-projection checks proving raw account data never enters an LLM request.
- Unsupported/adjusted/multi-expiration positions and unparseable open orders
  failing preflight instead of being ignored.
- Correct distinction between `NO_TRADE` and `ANALYSIS_INCOMPLETE`.
- Restart/supersession behavior and prevention of duplicate decisions.

### 17.4 Frontend tests

- Live SSE phase and agent-status rendering.
- Evidence navigation, timestamp display, and stale-data warnings.
- Candidate comparison and risk-calculation display.
- Approval disabled for incomplete, split, no-trade, stale, failed-risk, and
  superseded versions.
- Stale-hash rejection when a recommendation changes in another tab or process.
- Approval-time refresh, material-change review, approval-validity expiry, and
  visible reservation/reconciliation state.
- Reservation release requiring a fresh unambiguous positions/open-orders read.
- Live-mode startup refusal when rootless-container mounts, sanitized
  environment/file descriptors, read-only root, gateway keys, or provider-only
  egress controls fail verification.

### 17.5 Historical replay and evaluation

Version 1 includes a fixture-based replay/evaluation harness, not a scheduled
user-facing backtest service. Saved envelopes are replayed under their original
`as_of` boundary with live web, MCP, and account adapters disabled. New LLM
outputs are recorded as nondeterministic comparison runs; saved deterministic
risk inputs must reproduce their original calculations exactly.

Evaluation scores evidence grounding, calibration, rule adherence, dissent
handling, and eventual outcomes. Profit and loss alone does not determine
whether reasoning was sound. Rich dashboards, automated grading, and model
training remain deferred.

## 18. Acceptance criteria

Version 1 is not complete unless all of the following are demonstrably true:

1. Every options recommendation has a payoff-proved finite
   `expiration_payoff_max_loss`, uses a self-contained atomic structure, and
   passes assignment/exercise operational-capacity gates.
2. Identical saved inputs produce identical risk calculations and rule results.
3. A candidate cannot bypass the risk gate through moderator or user-interface
   behavior.
4. Missing mandatory analysis produces `ANALYSIS_INCOMPLETE` and disables
   approval.
5. A completed `NO_TRADE` identifies the evidence, thresholds, or candidate
   failures that justify it.
6. Every material factual claim resolves to timestamped evidence in the saved
   run.
7. The exact legs, quantities, limit, lifecycle rules, and account risk are
   visible before approval.
8. Approval performs a fresh account/quote check; any material plan, snapshot,
   account, policy, or calculation change invalidates the displayed version.
9. The complete debate, tool evidence, calculations, state transitions, and
   human decision can be reconstructed from the audit record.
10. Historical replay cannot access live tools or evidence created after its
    `as_of` time.
11. Active approvals atomically reserve expiration-loss, cash/buying-power,
    temporary-share/notional, and concentration capacity, preventing two
    unexecuted plans from spending the same remaining capacity in any dimension.
12. No application component, acquisition server, or configured Hermes profile
    exposes a brokerage order tool, order-submission request, execution endpoint,
    or broker-write permission.
13. Hermes profiles expose no terminal, filesystem-write, arbitrary-network,
    delegation, scheduling, installation, or automatic-memory-write capability,
    and each live profile runs in a verified rootless container with a read-only
    root, private mounts, sanitized environment/file descriptors, and
    provider-only egress.
14. Raw account equity, positions, orders, credentials, and approval reservations
    never enter an LLM prompt. Browser responses contain only the account fields
    explicitly required for the user's risk review and never contain credentials.
15. Every live position and open order normalizes into the conservative exposure
    model; otherwise approval capacity is zero and the run is incomplete.
16. Evidence-query plans cannot specify arbitrary URLs or operations, and
    material claims satisfy the source-tier/corroboration policy.
17. A reservation cannot be released without a fresh, unambiguous account and
    open-orders read showing that no matching exposure exists.
18. Each profile uses supported device authorization with a private credential
    volume, and the pinned Hermes version proves the one-request/one-model-
    iteration boundary used for provider admission.
19. The supported workflow operates locally without Redis, Kafka, Celery, a
    remote database, or public network exposure.

## 19. Deferred extensions

The following require separate design and approval rather than incremental
configuration changes:

- calendars, diagonals, and other multi-expiration options structures;
- covered-position strategies tied to live inventory;
- remote/mobile access and multi-user authentication;
- scheduled scans and portfolio-level opportunity ranking;
- alternative model providers or paid API fallback;
- paper-trading or broker execution of any kind; and
- richer outcome analytics and agent-calibration policies.

## 20. External interface references

The integration boundary relies on these upstream Hermes capabilities:

- [Hermes profiles](https://hermes-agent.nousresearch.com/docs/user-guide/profiles/)
- [Hermes API server](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/api-server.md)
- [Hermes programmatic integration](https://github.com/nousresearch/hermes-agent/blob/main/website/docs/developer-guide/programmatic-integration.md)
- [Hermes MCP support](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/features/mcp.md)
- [Hermes provider configuration](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/integrations/providers.md)

Upstream behavior and authentication compatibility must be confirmed against the
installed Hermes version during implementation. The application must fail
explicitly rather than silently changing providers or permissions.
