# Phase 1 Deterministic Risk Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a fixture-driven, deterministic validator for cash-funded long
stock and the complete version-1 defined-risk options allowlist, producing exact
payoff, operational stress, and multi-dimensional sizing evidence without any
live dependency.

**Architecture:** A strict versioned contract layer feeds a pure calculation
pipeline: resolve and validate the candidate, prove expiration payoff from its
signed legs, enumerate mandatory exercise/assignment states, apply market and
lifecycle gates, and size the trade against normalized capacity vectors. A thin
`asf-risk` CLI reads JSON fixtures and emits canonical results. No module reads
the clock, environment, filesystem, or network except the CLI's explicit input
and output boundary.

**Tech Stack:** Python 3.12+, uv lockfile, Pydantic 2.13, standard-library
`Decimal`/`argparse`/`json`/`hashlib`/`uuid`, pytest 9, Hypothesis 6, pytest-cov
7, Ruff 0.15, and mypy 2.

## Global Constraints

- Follow test-driven development for every behavior: add one focused failing
  test, run it and confirm the expected failure, add the minimum implementation,
  rerun it, then run the affected suite.
- Use `Decimal` for every price, amount, ratio, percentage, fee, and derived
  financial value. Production code must not use `float`.
- Take `as_of` from `TradeValidationRequest`; never read the wall clock.
- Do not add a database, FastAPI, React, Hermes, OAuth, MCP, browser, broker,
  market-data client, telemetry backend, or container configuration in this
  phase.
- Do not ingest credentials or real account exports. All fixtures are synthetic.
- Derive payoff and boundedness from signed legs. Never trust
  `declared_strategy` as proof.
- Every `SELL_TO_OPEN` leg must be protected inside the same atomic package.
  Existing holdings and future adjustments are irrelevant to boundedness.
- Operational scenario enumeration is unconditional for every options
  candidate. No policy or request field may disable it.
- Keep gross purchase cash, sale proceeds, and shares moved separate; do not net
  away settlement capacity.
- Phase 1 produces a `CapacityReservationDraft`. It does not write or claim an
  atomic durable reservation.
- A stop price never reduces theoretical maximum loss.
- Unsupported or unknown account exposure returns `INCOMPLETE`, never unused
  capacity.
- Commit only after the focused red/green cycle and every test/static check that
  exists at that task boundary pass. Task 14 runs the complete phase-wide gate.

---

## Phase Exit Criteria

- `uv run asf-risk validate examples/fixtures/pass/iron-condor.json` emits a
  byte-stable canonical `PASS` result and exits `0`.
- Every allowlisted structure has a hand-calculated golden result.
- Naked, uncovered-ratio, adjusted, multi-expiration, nonstandard-deliverable,
  legged, mismatched, stale, illiquid, event-blocked, and over-capacity fixtures
  have stable fail-closed statuses and rule codes; stale or invalid mandatory
  evidence is `INCOMPLETE`, while a fully observed unsafe trade is `REJECT`.
- Unsupported committed exposure produces a stable `INCOMPLETE` result.
- Repeated runs of the same input produce identical result IDs, hashes, rule
  ordering, JSON bytes, and schema output.
- Candidate-only and post-trade portfolio payoff are both calculated from
  complete normalized inputs.
- Unit, property, golden, CLI, Ruff, formatting, mypy, and coverage checks pass.

## Exact Repository Shape at Phase Completion

```text
.python-version
pyproject.toml
uv.lock
README.md
schemas/
  v1/
    trade-validation-request.schema.json
    trade-validation-result.schema.json
examples/
  fixtures/
    pass/
      long-stock.json
      long-call.json
      long-put.json
      bull-call-debit.json
      bear-put-debit.json
      bull-put-credit.json
      bear-call-credit.json
      long-call-butterfly.json
      long-put-butterfly.json
      iron-butterfly.json
      iron-condor.json
      asymmetric-iron-condor.json
    reject/
      uncovered-short-call.json
      uncovered-call-ratio.json
      adjusted-contract.json
      mismatched-expiration.json
      legged-package.json
      low-liquidity.json
      event-blocked.json
      insufficient-capacity.json
    incomplete/
      stale-quote.json
      unsupported-account-exposure.json
src/
  ai_stock_forum/
    __init__.py
    cli.py
    contracts/
      __init__.py
      base.py
      decimal_string.py
      v1/
        __init__.py
        enums.py
        market.py
        trade.py
        policy.py
        result.py
        validation.py
    serialization/
      __init__.py
      canonical.py
    risk/
      __init__.py
      engine_version.py
      violations.py
      structure.py
      payoff.py
      scenarios.py
      market_rules.py
      capacity.py
      validator.py
tests/
  conftest.py
  factories.py
  unit/
    test_package.py
    contracts/
      test_decimal_string.py
      test_v1_models.py
      test_schema_exports.py
    serialization/
      test_canonical.py
    risk/
      test_structure_long_options.py
      test_structure_verticals.py
      test_structure_complex.py
      test_payoff_stock.py
      test_payoff_options.py
      test_payoff_complex.py
      test_scenarios.py
      test_market_rules.py
      test_capacity.py
      test_validator.py
  property/
    test_payoff_properties.py
    test_scenario_properties.py
    test_capacity_properties.py
  integration/
    test_cli.py
    test_golden_results.py
  golden/
    long-stock-result.json
    long-call-result.json
    long-put-result.json
    bull-call-debit-result.json
    bear-put-debit-result.json
    bull-put-credit-result.json
    bear-call-credit-result.json
    long-call-butterfly-result.json
    long-put-butterfly-result.json
    iron-butterfly-result.json
    iron-condor-result.json
    asymmetric-iron-condor-result.json
    uncovered-short-call-result.json
    uncovered-call-ratio-result.json
    adjusted-contract-result.json
    mismatched-expiration-result.json
    legged-package-result.json
    low-liquidity-result.json
    event-blocked-result.json
    insufficient-capacity-result.json
    stale-quote-result.json
    unsupported-account-exposure-result.json
```

## Contract and Calculation Decisions

### Top-level request

`TradeValidationRequest` is the only validator input:

```python
class TradeValidationRequest(V1Model):
    schema_version: Literal["1.0"]
    request_id: UUID
    as_of: AwareDatetime
    candidate: Candidate
    market: MarketSnapshot
    costs: CostAssumptions
    operational_context: OperationalContext
    policy: RiskPolicy
    account: AccountRiskSnapshot
```

`Candidate` is a discriminated union on `asset_type`:

```python
Candidate = Annotated[
    StockCandidate | OptionCandidate,
    Field(discriminator="asset_type"),
]
```

`StockCandidate` contains a UUID, symbol, `LONG_STOCK`, positive requested share
quantity, immutable underlying `quote_id`, USD limit price, and
`cash_funded: bool`. The raw contract permits `false` so the rule layer can
return a stable domain rejection; only `true` can pass.

`OptionCandidate` contains a UUID, symbol, declared strategy, positive requested
package quantity, a nonempty tuple of `OptionLeg` references, one `PackageLimit`,
boolean `atomic_package`, boolean `legging_allowed`, the planned holding-window
end, and lifecycle rules including `intended_exit` (`CLOSE` or
`HOLD_TO_EXPIRATION`). These booleans remain syntactically representable so
unsafe values receive domain rule results; passing requires an atomic package
with legging disabled. `close_before_expiration_days` is optional in the wire
model, but the rule layer requires a positive value and `intended_exit=CLOSE`
for any short leg. Stock uses `CLOSE`; long options may use either supported
exit.

An `OptionLeg` contains only immutable `contract_id`, `quote_id`, opening action
(`BUY_TO_OPEN` or `SELL_TO_OPEN`), and positive integer package ratio. Contract
terms and observations are resolved from `MarketSnapshot`; symbols are never
reconstructed from display text. Requested option-package quantity is capped at
10,000, requested stock shares at 1,000,000, option legs at 8, and each wire
ratio at 100. Before permutation generation, the operational engine enforces
`MAX_OPERATIONAL_UNIT_EVENTS = 4`; larger resolved packages receive a typed
fail-closed complexity result without expansion.

Schema validation and domain validation are intentionally distinct. Wrong JSON
types, missing required fields, zero/negative requested quantity, closing-action
enum values, and dangling IDs are input-schema errors (CLI exit `64`) and do not
produce a `TradeValidationResult`. Syntactically representable but prohibited
facts—including `cash_funded=false`, nonstandard contract terms,
`atomic_package=false`, `legging_allowed=true`, a missing short-leg close rule,
and unsafe leg shapes—reach the deterministic rules and produce stable domain
results.

Term enums intentionally represent common syntactically valid unsupported facts
needed for domain evidence: `ExerciseStyle` includes `AMERICAN`/`EUROPEAN`,
`SettlementStyle` includes `PHYSICAL`/`CASH`, `Currency` includes
`USD`/`CAD`/`EUR`/`GBP`, and underlying/deliverable kinds include standard share
and non-share alternatives. Only the version-1 combination described below can
pass. `OptionAction` is the exception: it contains opening actions only because
closing actions are malformed for a new candidate contract.

### Market records

`MarketSnapshot` contains its UUID, one `UnderlyingQuote` with a normalized
`sector_id`, a tuple of unique `OptionContract` records, and a tuple of unique
`OptionQuote` records. Validation rejects dangling IDs, duplicate IDs, a quote
linked to the wrong contract, an absent sector, or a candidate symbol that
differs from the market records.

`UnderlyingQuote` contains `quote_id`, symbol, sector ID, bid, ask, saved
midpoint, and `observed_at`, with the same integrity and freshness rules as
option quotes. Stock candidates are buy-limit orders: the limit is a maximum
executable price and may intentionally rest below the ask. Option net limits are
likewise conditional package limits. The engine reports distance from the
natural saved package market but never assumes a fill or replaces the submitted
limit with a mark. Per underlying share:

```text
natural_package_net_cash =
  sum(STO ratio × bid) - sum(BTO ratio × ask)

submitted_package_net_cash = +credit_limit or -debit_limit
limit_distance_from_natural =
  submitted_package_net_cash - natural_package_net_cash
```

Version 1 option contracts must be:

- standard and unadjusted;
- American exercise style and physically settled;
- USD-denominated U.S. stock or ETF options;
- exactly 100 shares of the same underlying with no cash or other-security
  deliverable; and
- identical across all legs for underlying, expiration, exercise style,
  settlement, currency, multiplier, and deliverable.

An option quote contains bid, ask, saved midpoint, volume, open interest, and
`observed_at`. Prices use decimal strings; volume and open interest are
nonnegative integers. The saved midpoint must equal `(bid + ask) / 2`, and
crossed markets are not usable. The wire model represents these syntactically
valid bad observations so the global quote-integrity rule can return
`INCOMPLETE`; it does not silently normalize them.

### Strict decimal and canonical JSON behavior

All models inherit:

```python
class V1Model(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, strict=True)
```

`DecimalString` accepts JSON/Python strings matching
`^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?$`. It rejects JSON numbers, booleans,
exponents, NaN, and infinities. It normalizes trailing zeroes and negative zero
and serializes in plain notation. Domain-specific aliases apply positive or
nonnegative constraints after parsing. Version 1 accepts at most 16 integer
digits and 8 fractional digits per input value; larger magnitudes or scales are
schema errors.

Every calculation uses a local decimal context with precision 50 and
`ROUND_HALF_EVEN`; no code mutates the process-global context. At output and
capacity-comparison boundaries, nonnegative losses, cash demands, and notionals
round outward toward positive infinity to cents; nonnegative profits round
toward zero to cents. Breakeven division is a deterministic approximation
quantized to 8 fractional digits and carries that precision in the result. The
implementation never calls a rounded breakeven "exact," and rounded roots never
participate in bounded-loss or sizing decisions. Addition and multiplication in
risk/capacity paths trap unexpected `Inexact` or `Rounded` signals; only the
isolated breakeven-display division permits and then explicitly quantizes them.

Canonical JSON is UTF-8 with sorted keys, no whitespace, Unicode left unescaped,
and nonfinite values prohibited. Aware datetimes normalize to UTC with a `Z`
suffix. UUIDs serialize lowercase. `result_id` is
`uuid5(uuid.NAMESPACE_URL, "urn:ai-stock-forum:risk-v1.0.0:<request_sha256>")`,
so identical requests produce identical result bytes without a completion
timestamp.

### Entry and transaction costs

`PackageLimit` has kind `NET_DEBIT` or `NET_CREDIT` and a positive USD amount per
underlying share. It applies once to the complete package.

`PackageLimit` is the user's executable worst acceptable net debit or minimum
acceptable net credit. Entry slippage is therefore not added outside that limit.
If an upstream source has only a reference mark, it must construct a conservative
limit before calling Phase 1.

`CostAssumptions` contains these nonnegative USD decimal strings:

- `fixed_open_order_fee` and `fixed_planned_exit_order_fee`;
- `open_fee_per_contract` and `planned_exit_fee_per_contract`;
- `planned_exit_slippage_reserve_per_underlying_share`;
- `exercise_fee_per_contract` and `assignment_fee_per_contract`; and
- stock `open_fee_per_share`, `planned_exit_fee_per_share`, and
  `planned_exit_slippage_reserve_per_share`.

For options quantity `q`, where `I(q) = 1` when `q > 0` and `0` otherwise:

```text
opening_cost(q) =
  I(q) × fixed_open_order_fee
  + q × sum(leg ratios) × open_fee_per_contract

planned_close_cost_reserve(q) =
  I(q) × fixed_planned_exit_order_fee
  + q × sum(leg ratios) × planned_exit_fee_per_contract
  + q × multiplier × sum(leg ratios)
      × planned_exit_slippage_reserve_per_underlying_share

maximum_settlement_fee_reserve(q) =
  q × sum(BTO leg ratios) × exercise_fee_per_contract
  + q × sum(STO leg ratios) × assignment_fee_per_contract
```

The engine reports two mutually exclusive paths. `expiration_payoff` subtracts
opening cost and the maximum settlement-fee reserve, but no planned-close cost.
`managed_close_loss_envelope` subtracts opening and planned-close reserves, but
no settlement fee. `conservative_maximum_loss` is the larger loss across those
paths and is the loss used for sizing. Operational states report their actual
configured exercise/assignment fees separately.

The managed-close envelope is executable from the same generic kernel; it is not
an unspecified option-pricing forecast. Define the signed structural position
value, excluding entry cash and all fees:

```text
position_value_q(S) =
  q × M × sum(s_i × r_i × intrinsic_i(S))

managed_close_P_q(S) =
  entry_cash(q) - opening_cost(q)
  + position_value_q(S)
  - planned_close_cost_reserve(q)
```

The engine evaluates `managed_close_P_q` at spot zero, every strike, and both
tail semantics exactly like expiration payoff. This produces typed managed-close
minimum/maximum profit, loss, breakpoints, and unbounded flags. It represents the
conservative no-arbitrage structural liquidation envelope plus configured close
fees/slippage; it does not claim to predict the actual pre-expiration mark. With
zero close and settlement fees, the managed and expiration curves coincide.

For long stock shares `n`, with the same indicator convention:

```text
stock_opening_cost(n) =
  I(n) × fixed_open_order_fee + n × open_fee_per_share

stock_exit_cost_reserve(n) =
  I(n) × fixed_planned_exit_order_fee
  + n × (planned_exit_fee_per_share
         + planned_exit_slippage_reserve_per_share)

stock_entry_cash(n) = n × entry_limit + stock_opening_cost(n)
F(n) = stock_opening_cost(n) + stock_exit_cost_reserve(n)
```

For `q = 0` or `n = 0`, every cost and capacity demand is zero. Zero is an
internal sizing sentinel only; requested quantities remain schema-positive.

### Generic expiration payoff proof

For one options package, with multiplier `M`, positive leg ratio `r_i`, signed
side `s_i` (`+1` for `BUY_TO_OPEN`, `-1` for `SELL_TO_OPEN`), and underlying
price `S >= 0`:

```text
call_intrinsic(S, K) = max(S - K, 0)
put_intrinsic(S, K)  = max(K - S, 0)

entry_cash(q) =
  -q × M × net_debit, or
  +q × M × net_credit

expiration_P_q(S) = entry_cash(q) - opening_cost(q)
                    - maximum_settlement_fee_reserve(q)
         + q × M × sum(s_i × r_i × intrinsic_i(S))
```

The engine evaluates `S = 0`, every distinct strike, and the upper tail. Above
the highest strike:

```text
upper_tail_slope = q × M × sum(call-leg s_i × r_i)
```

- A negative slope means unbounded loss and is rejected.
- A zero slope means the upper tail is constant at the last strike.
- A positive slope means maximum profit is explicitly `UNBOUNDED`; minimum
  payoff is still at a finite breakpoint.

Before applying fees, the engine evaluates the raw premium-plus-intrinsic payoff.
Its worst payoff must be strictly negative and it must have a strictly positive
achievable payoff. This cost-independent economic-price check rejects credit at
or above wing width, debit at or above maximum structure value, and apparent
arbitrage even when fixed fees would manufacture a small reported loss. It also
guarantees maximum-loss demand is monotonic in quantity.

`expiration_payoff_maximum_loss = max(0, -minimum_expiration_payoff)`.
`conservative_maximum_loss` is the greater of that value and the managed-close
loss envelope described above. A zero raw loss or no positive raw reward is a
hard rejection rather than an unlimited sizing denominator.

Raw sanity is necessary but not sufficient. `POSITIVE_ACHIEVABLE_REWARD` uses
the fee-adjusted maximum profit for the candidate's `intended_exit` path. After
the required conservative profit rounding, it must be at least `$0.01`.
Zero/negative or positive-sub-cent profit that displays as `$0.00` fails with
`NO_POSITIVE_REWARD_AFTER_COSTS`. The alternate lifecycle path remains visible
as stress evidence but cannot rescue an unprofitable intended exit.

Breakevens are deterministic roots of each affine segment over `S >= 0`, rounded
only for output under the precision rule above. The result has deduplicated
`breakeven_points` plus `zero_payoff_intervals` for flat zero segments; the
solver never divides by a zero slope. `PayoffSummary` also exposes
the risk/reward ratio with basis
`intended_exit_maximum_profit / conservative_maximum_loss`. It is a decimal
string for finite positive maximum profit and loss, explicitly
`UNBOUNDED_PROFIT` when upside is unbounded, and explicitly
`UNDEFINED_ZERO_LOSS` for the apparent-arbitrage case that the rule layer
rejects. Fee-adjusted nonpositive intended-exit reward uses
`NO_POSITIVE_REWARD_AFTER_COSTS`. The basis fields remain visible so the ratio
cannot hide its cost-path assumptions.

For long stock with `n` shares, entry limit `E`, and round-trip stock cost
`F(n)`:

```text
P_n(S) = n × (S - E) - F(n)
maximum_loss = n × E + F(n)
maximum_profit = UNBOUNDED
breakeven = E + F(n) / n
```

The full purchase debit is the stock's risk and cash demand. A proposed stop is
not an input to Phase 1 sizing.

### Allowed structures

The matcher derives one of these structures from resolved legs and then requires
`declared_strategy` to match:

1. cash-funded long stock;
2. long call or long put, one `BUY_TO_OPEN` with ratio `1`, `NET_DEBIT`;
3. bull call debit, bear put debit, bull put credit, or bear call credit vertical,
   exactly one long and one short, both ratio `1`;
4. symmetric long call or long put butterfly with ratios `+1/-2/+1`, three
   strictly increasing equidistant strikes, `NET_DEBIT`;
5. symmetric short-credit iron butterfly with long put wing, short put and call
   at the same body strike, and long call wing, ratios `1/1/1/1`, where both
   wing widths are equal; and
6. short-credit iron condor with strikes
   `long put < short put < short call < long call`, ratios `1/1/1/1`.

Iron-condor wing widths may differ; the larger side loss governs. Equal strikes
are rejected within one option type; the iron butterfly's put/call body is the
intentional cross-type same-strike exception. Broken-wing single-type
butterflies, unequal-wing iron butterflies, reverse iron structures, short
single-type butterflies, calendars, diagonals, noncanonical ratios, duplicate
contracts, and every other shape are rejected in version 1. Ratios are never
reduced by a greatest common divisor: `2/2` is rejected rather than treated as a
`1/1` package.

`StructureAssessment` separates `allowlisted`, `payoff_terms_supported`, and
`operational_terms_supported`. Standard, same-expiration/common-deliverable
signed legs may enter the linear payoff kernel even when their shape or label is
rejected, which allows evidence such as a naked call's negative upper tail.
Adjusted/non-share deliverables, mixed expirations, or other terms without one
valid Phase-1 payoff domain leave payoff absent/`NOT_APPLICABLE`; the engine does
not fabricate a curve. Operational terms are supported only for a fully
allowlisted standard template within the four-event cap. Every options request
still invokes each gate and records why a calculation is present or not.

### Operational scenario semantics

For one package, expand each leg ratio into unit option events. At each
`LifecycleBoundary` (`EARLY_ASSIGNMENT`, `EX_DIVIDEND`, `EXPIRATION_CUTOFF`, and
`OVERNIGHT_AFTER_EXPIRATION`), enumerate every subset and every event ordering.
Store each prefix state with its boundary and unconditional stress assumption so
simultaneous, sequential, partial, contrary, and exercise-by-exception outcomes
contribute to the maxima. Moneyness never removes a state.

The generator is invoked for every resolved options candidate, but it computes
physical states only after the structural assessment establishes standard
100-share deliverables and an allowlisted shape. Adjusted/nonstandard contracts,
noncanonical ratios, or an over-cap event count return a typed candidate-local
`NOT_APPLICABLE` operational result without expansion; the structural `FAIL`
keeps the overall result `REJECT`. They never receive fabricated 100-share
evidence and never turn into a global `INCOMPLETE`. A supported template whose
required operational context is unavailable returns global `UNKNOWN`/
`INCOMPLETE`.

Each unit event has these physical effects:

```text
long call exercise:     shares +100, cash -strike × 100
short call assignment:  shares -100, cash +strike × 100
long put exercise:      shares -100, cash +strike × 100
short put assignment:   shares +100, cash -strike × 100
```

Each event identity is `(contract_id, unit_index)`, making partial ratio-two
assignment deterministic. For a prefix with signed share deltas and signed
strike cash flows:

```text
net_shares = sum(signed share deltas)
gross_shares_moved = sum(abs(signed share deltas))
gross_purchase_cash = sum(abs(negative strike cash flows))
gross_sale_proceeds = sum(positive strike cash flows)
event_fee_outflow = sum(exercise/assignment fee for triggered events)
settlement_net_cash = gross_sale_proceeds
                      - gross_purchase_cash
                      - event_fee_outflow
gross_settlement_notional = gross_purchase_cash + gross_sale_proceeds
temporary_long_shares = max(net_shares, 0)
temporary_short_shares = max(-net_shares, 0)
temporary_stock_notional = abs(net_shares) × maximum stress price
```

`settlement_net_cash` deliberately excludes entry premium and opening fees.
`OperationalContext` references the immutable underlying `quote_id` and supplies
a nonempty tuple of conservative `temporary_stock_stress_prices` whose minimum
must be at least the saved underlying ask. The tuple includes the current quote
and an upper stress price; temporary notional always uses its maximum, never an
unlinked user-supplied reference mark.

The summary takes a maximum for each capacity dimension without offsetting
purchases against sale proceeds or long shares against short shares across
states. Gross sale proceeds and gross shares are preserved as evidence; gross
purchase cash, event fees, gross settlement notional, temporary long/short
shares, and temporary stock notional also feed capacity demand.

The exhaustive state set is generated for one package only when expanded unit
events are at most four. The result labels this
`per_package_exhaustive`. Requested-quantity stress is labeled
`requested_quantity_upper_bound` and conservatively equals
`quantity × per_package_maximum`; it does not claim to enumerate every
multi-package path. A quantity-two property test must show brute-force sampled
states never exceed the scaled summary. Candidates above the four-event cap get
a typed `OPERATIONAL_SCENARIO_COMPLEXITY` result without factorial expansion.

### Market and event gates

`OperationalContext` is a discriminated union. `COMPLETE` supplies a context
UUID, immutable provenance IDs, underlying quote ID, temporary-stock stress
prices, planned holding-window end, earnings-in-window, ex-dividend-in-window,
corporate-action pending, low-extrinsic short contract IDs, and pin-risk contract
IDs. In a complete context, `false` and an empty ID tuple mean the fact was
checked and absent. `INCOMPLETE` supplies a context UUID and nonempty reason
codes but none of those facts. It yields global `UNKNOWN` operational/event rules,
maximum quantity zero, and overall `INCOMPLETE`; missing acquisition is never
conflated with a confirmed negative fact. `RiskPolicy` controls rejection
thresholds; it cannot remove scenario states.

The rule layer calculates quote age from request `as_of`, bid/ask spread as a
percentage of midpoint, days to expiration, and short-leg close buffer. It
enforces maximum quote age/spread, minimum volume/open interest, minimum/maximum
DTE, holding-window compatibility, and configured hard earnings, dividend,
corporate-action, low-extrinsic, and pin gates.

Option expiration is an aware UTC `expiration_at`, normalized upstream from the
exchange cutoff. DTE gates compare exact elapsed seconds, inclusively:

```text
minimum_days × 86,400 <= expiration_at - as_of
                            <= maximum_days × 86,400

expiration_at - planned_exit_at
    >= max(candidate.close_before_expiration_days,
           policy.minimum_short_close_buffer_days) × 86,400
```

No date truncation, local-midnight rule, or trading-calendar inference occurs in
Phase 1. Display DTE may be an 8-decimal day value, but gate comparisons use the
integer-second duration. Quote age similarly uses exact nonnegative elapsed
seconds and passes at the configured maximum boundary.

`RiskPolicy` has a UUID and immutable version plus exactly these settings:

```text
per_trade_loss_cap_usd
per_trade_loss_percentage_of_equity   # decimal in (0, 1], 0.02 means 2%
maximum_aggregate_open_defined_risk_usd
maximum_long_stock_allocation_usd
maximum_long_stock_allocation_percentage_of_equity  # decimal in (0, 1]
absolute_max_option_packages          # positive integer, maximum 10,000
absolute_max_stock_shares             # positive integer, maximum 1,000,000
permitted_strategies                  # nonempty lexicographically sorted unique tuple
maximum_quote_age_seconds
maximum_bid_ask_spread_percentage     # decimal in [0, 1], 0.10 means 10%
minimum_contract_volume
minimum_contract_open_interest
minimum_days_to_expiration
maximum_days_to_expiration
minimum_short_close_buffer_days
reject_earnings_in_holding_window
```

Ex-dividend short-call exposure, pending corporate actions, low-extrinsic short
legs, and pin risk are unconditional version-1 failures and therefore have no
disable flags. A complete context must use matching holding/evaluation timestamps
across candidate, account payoff profile, and context.

### Capacity and quantity semantics

`CapacityVector` has these nonnegative coordinates. USD fields are decimal
strings; the two share fields are nonnegative integers:

```text
expiration_loss_usd
entry_cash_usd
broker_buying_power_usd
gross_purchase_cash_usd
cumulative_cash_required_usd
temporary_long_shares
temporary_short_shares
temporary_stock_notional_usd
gross_option_settlement_notional_usd
long_stock_allocation_usd
symbol_concentration_usd
sector_concentration_usd
```

`AccountRiskSnapshot` contains account equity, the total allowed vector, and
separate `CapacityCommitment` records for existing positions, open orders, and
active approval reservations. Each commitment is a discriminated union:
`COMPLETE` requires a full vector; `INCOMPLETE` requires reason codes and has no
vector. This represents unknown data without nullable financial coordinates.

The account record also contains `candidate_symbol` and `candidate_sector_id`
that must match the candidate and market snapshot. Portfolio-wide dimensions
are aggregate limits/commitments; symbol and sector coordinates are explicitly
the limits/commitments projected to those matching candidate keys.

`candidate_buying_power_floor` contains a per-package USD value, immutable
provenance record ID, and `SYNTHETIC_FIXTURE` method in Phase 1. Demand at
quantity `q` uses `q × per_package_usd`; later adapters may produce the same
typed record from a reviewed read-only source.

`existing_portfolio_payoff_profile` is another discriminated union. `COMPLETE`
contains a piecewise-linear function for relevant existing positions at the
candidate payoff-evaluation time: option expiration for an option package and
planned holding-window end for stock. It starts at spot zero, has strictly
increasing payoff points, and declares its upper-tail slope. An empty account uses the
explicit zero profile. `INCOMPLETE` contains reason codes only. Any incomplete
commitment/profile or unsupported-exposure code yields `INCOMPLETE` and maximum
quantity zero. A negative complete-profile upper-tail slope is also unsupported
in version 1; the engine will not bless a new recommendation while current
normalized exposure already has unbounded upside-tail loss.

The complete positions commitment and payoff profile carry the same immutable
`positions_snapshot_id`, `observed_at`, and payoff-evaluation timestamp. The
profile's conservatively rounded maximum loss must be less than or equal to
`positions.vector.expiration_loss_usd`. A provenance/time mismatch or a profile
loss larger than committed expiration loss makes account normalization
`INCOMPLETE`; the two representations cannot expose contradictory free capacity.

```text
remaining[j] = total_limit[j]
               - positions.vector[j]
               - open_orders.vector[j]
               - active_reservations.vector[j]
```

For every quantity `q`:

```text
candidate_loss(q) <= min(
  min(account total expiration-loss limit,
      policy.maximum_aggregate_open_defined_risk_usd)
    - committed expiration loss,
  policy.per_trade_loss_cap_usd,
  account_equity × policy.per_trade_loss_percentage_of_equity
)
```

Existing commitments consume only the aggregate coordinate, not the per-trade
cap. A fully known overcommit produces maximum quantity zero and `REJECT`; an
incomplete commitment produces `INCOMPLETE`.

For long stock, allocation demand is its full purchase cash. It must also fit:

```text
min(account total long-stock-allocation limit,
    policy.maximum_long_stock_allocation_usd,
    account_equity
      × policy.maximum_long_stock_allocation_percentage_of_equity)
- committed long-stock allocation
```

Option candidates consume zero in the long-stock-allocation coordinate. The
account vector provides broker/account hard limits; the immutable risk policy
provides user limits, and the engine always applies the stricter derived cap.

For options quantity `q`, cumulative cash demand is the non-netted sum of entry
cash outflow, maximum gross settlement purchase cash, and maximum event-fee
outflow. Net entry credit and gross sale proceeds never reduce it. For stock it
is the full purchase cash including opening fees. When sizing a stock candidate
at trial quantity `q`, it consumes `temporary_long_shares = q`, zero temporary
short shares, and temporary stock notional equal to `q` times the larger of its
executable limit and maximum quote-linked stress price; its option-settlement
coordinates are zero and marked not applicable in rule evidence. Requested
shares are used only for the final pass check and reservation draft.

Symbol and sector concentration demand are both the maximum of conservative
loss, entry cash, temporary stock notional, and gross option settlement
notional. Stock buying-power demand is its full entry cash. Options buying-power
demand is the larger of conservative maximum loss and
`q × account.candidate_buying_power_floor.per_package_usd`; Phase 1 never calls this
broker-exact margin.

The engine combines the candidate payoff with the normalized existing profile
at the union of both breakpoint sets and adds their upper-tail slopes. It reports
candidate-only and post-trade portfolio payoff separately. Existing-position
capacity and existing payoff are therefore both visible: one cannot substitute
for the other.

Because order costs can contain fixed components, sizing uses monotonic integer
binary search over `0..absolute_cap_for(candidate)`, selecting the option-package
or stock-share policy cap, not a single division. The maximum
permitted quantity is the largest `q` whose demand is componentwise within the
remaining vector. The requested quantity passes only when it is no greater than
that maximum and every non-capacity rule passes. The output reservation draft is
always for `requested_quantity`, never the maximum permitted quantity. If the
absolute quantity cap alone binds, it is reported as its own binding reason even
though `max + 1` may fit every financial coordinate.

### Stable result and CLI contract

The result has:

```python
class TradeValidationResult(V1Model):
    schema_version: Literal["1.0"]
    result_id: UUID
    request_id: UUID
    request_sha256: str
    calculation_input_sha256: str
    engine_version: Literal["risk-v1.0.0"]
    status: ValidationStatus  # PASS | REJECT | INCOMPLETE
    resolved_structure: ResolvedStructure | None
    payoff: PayoffAssessment | None  # candidate and post-trade portfolio
    operational_exposure: OperationalExposure | None
    market_assessment: MarketAssessment | None
    sizing: SizingReport | None
    reservation_draft: CapacityReservationDraft | None
    rules: tuple[RuleResult, ...]
```

Rule results appear in a fixed enum order. Each has a stable code, `PASS`,
`FAIL`, `UNKNOWN`, or `NOT_APPLICABLE`, a concise evidence message, observed
values, thresholds, the IDs of the fixture records used, and `scope` equal to
`GLOBAL_INTEGRITY` or `CANDIDATE`. Stock results mark option-only rules
`NOT_APPLICABLE` rather than omitting or falsely passing them. Unknown required
evidence produces `INCOMPLETE`.

The ordered `RuleCode` values are:

```text
INPUT_RESOLVED
STANDARD_CONTRACTS
COMMON_PACKAGE_TERMS
ATOMIC_SELF_CONTAINED_PACKAGE
DECLARED_STRUCTURE_MATCH
STRATEGY_PERMITTED_BY_POLICY
ECONOMIC_PRICE_SANITY
FINITE_EXPIRATION_LOSS
POSITIVE_ACHIEVABLE_REWARD
OPERATIONAL_SCENARIOS_COMPLETE
QUOTE_INTEGRITY
QUOTE_FRESHNESS
LIQUIDITY
HOLDING_WINDOW
EARNINGS_POLICY
EX_DIVIDEND_RISK
CORPORATE_ACTION_RISK
LOW_EXTRINSIC_ASSIGNMENT_RISK
PIN_RISK
ACCOUNT_NORMALIZATION_COMPLETE
OPERATIONAL_SCENARIO_COMPLEXITY
EXPIRATION_LOSS_CAPACITY
ENTRY_CASH_CAPACITY
BUYING_POWER_CAPACITY
GROSS_PURCHASE_CASH_CAPACITY
CUMULATIVE_CASH_CAPACITY
TEMPORARY_SHARES_CAPACITY
TEMPORARY_STOCK_NOTIONAL_CAPACITY
GROSS_SETTLEMENT_NOTIONAL_CAPACITY
AGGREGATE_DEFINED_RISK_CAPACITY
LONG_STOCK_ALLOCATION_CAPACITY
SYMBOL_CONCENTRATION_CAPACITY
SECTOR_CONCENTRATION_CAPACITY
REQUESTED_QUANTITY_ALLOWED
```

Status precedence is deterministic. Any global integrity failure or unknown
(`INPUT_RESOLVED`, quote integrity/freshness, account normalization, or
operational completeness) produces `INCOMPLETE` even when a candidate-local
rule also fails. Otherwise any candidate-local `FAIL` produces `REJECT`, then
any remaining `UNKNOWN` produces `INCOMPLETE`; `NOT_APPLICABLE` is neutral; all
other results produce `PASS`. Only `PASS` includes a reservation draft.

Public calculation interfaces are:

```python
def canonical_json_bytes(value: BaseModel | JsonValue) -> bytes: ...
def canonical_sha256(value: BaseModel | JsonValue) -> str: ...
def resolve_candidate(request: TradeValidationRequest) -> ResolvedCandidate: ...
def validate_structure(candidate: ResolvedCandidate) -> StructureAssessment: ...
def expiration_profit(candidate: ResolvedCandidate, costs: CostAssumptions,
                      quantity: int, spot: Decimal) -> Decimal: ...
def analyze_expiration_payoff(candidate: ResolvedCandidate,
                              costs: CostAssumptions,
                              quantity: int) -> PayoffSummary: ...
def combine_with_existing_portfolio(
    candidate_payoff: PayoffSummary,
    existing: ExistingPortfolioPayoffProfile,
) -> PayoffSummary: ...
def generate_operational_scenarios(candidate: ResolvedOptionCandidate,
                                   structure: StructureAssessment,
                                   context: OperationalContext
                                   ) -> OperationalScenarioSet: ...
def summarize_operational_exposure(
    scenario_set: OperationalScenarioSet,
) -> OperationalExposure | None: ...
def assess_market_and_events(request: TradeValidationRequest,
                             candidate: ResolvedCandidate
                             ) -> MarketAssessment: ...
def subtract_capacity(account: AccountRiskSnapshot) -> CapacityVector: ...
def demand_for_quantity(candidate: ResolvedCandidate,
                        costs: CostAssumptions,
                        operational: OperationalExposure | None,
                        account: AccountRiskSnapshot,
                        quantity: int) -> CapacityVector: ...
def max_permitted_quantity(request: TradeValidationRequest,
                           candidate: ResolvedCandidate,
                           operational: OperationalExposure | None
                           ) -> SizingReport: ...
def validate_trade(request: TradeValidationRequest) -> TradeValidationResult: ...
def main(argv: Sequence[str] | None = None) -> int: ...
```

CLI behavior:

```text
asf-risk validate INPUT.json [--pretty]
asf-risk schema --output schemas/v1
```

- Default stdout is canonical JSON with one trailing newline.
- `--pretty` changes presentation only and is excluded from all hashes.
- Diagnostics go to stderr; successful domain results do not write stderr.
- Exit `0`: `PASS`.
- Exit `2`: deterministic `REJECT`.
- Exit `3`: deterministic `INCOMPLETE`.
- Exit `64`: CLI usage, I/O, or input-schema error.
- Exit `70`: unexpected internal error with no secret or fixture dump.

---

## Implementation Tasks

### Task 1: Bootstrap the locked Python package

**Files:**

- Create: `.python-version`
- Create: `pyproject.toml`
- Create: `uv.lock`
- Create: `src/ai_stock_forum/__init__.py`
- Create: `src/ai_stock_forum/risk/__init__.py`
- Create: `src/ai_stock_forum/risk/engine_version.py`
- Create: `tests/unit/test_package.py`

- [ ] Add `.python-version` with `3.13`, matching the available local Python,
  while setting `requires-python = ">=3.12"` in `pyproject.toml`.
- [ ] Configure Hatchling (`hatchling>=1.27,<2`) with the `src` layout and
  console script `asf-risk = "ai_stock_forum.cli:main"`.
- [ ] Declare runtime dependency `pydantic>=2.13.4,<3` and a uv `dev` dependency
  group containing `pytest>=9.1.1,<10`, `hypothesis>=6.160,<7`,
  `pytest-cov>=7.1,<8`, `ruff>=0.15.22,<1`, and `mypy>=2.3,<3`.
- [ ] Configure Ruff for Python 3.12, 88-character formatting, import sorting,
  bugbear, pyupgrade, annotations, and pytest-style rules. Configure mypy with
  `strict = true`, `warn_unreachable = true`, and package checking under `src`.
- [ ] Create empty `src/ai_stock_forum/__init__.py` and
  `src/ai_stock_forum/risk/__init__.py` package markers so Hatchling can install
  the editable project; do not add version constants yet.
- [ ] Add `tests/unit/test_package.py` asserting
  `ai_stock_forum.__version__ == "0.1.0"` and
  `ENGINE_VERSION == "risk-v1.0.0"` before creating `engine_version.py`.
- [ ] Run `uv lock && uv sync`; dependency resolution may require approved
  network access. Check in the generated `uv.lock`.
- [ ] Run `uv run pytest tests/unit/test_package.py -q` and confirm failure with
  `ModuleNotFoundError: ai_stock_forum.risk.engine_version`.
- [ ] Add `__version__` and the immutable engine-version constant.
- [ ] Run `uv run pytest tests/unit/test_package.py -q` and confirm it passes.
- [ ] Run `uv run ruff check .`, `uv run ruff format --check .`, and
  `uv run mypy src`.
- [ ] Commit with `chore: bootstrap deterministic risk package`.

### Task 2: Implement strict decimal strings and canonical serialization

**Files:**

- Create: `src/ai_stock_forum/contracts/__init__.py`
- Create: `src/ai_stock_forum/contracts/base.py`
- Create: `src/ai_stock_forum/contracts/decimal_string.py`
- Create: `src/ai_stock_forum/serialization/__init__.py`
- Create: `src/ai_stock_forum/serialization/canonical.py`
- Create: `tests/unit/contracts/test_decimal_string.py`
- Create: `tests/unit/serialization/test_canonical.py`

- [ ] Write parameterized failing tests proving `"0"`, `"0.10"`,
  `"123.4500"`, and `"-0.00"` parse and serialize as `"0"`, `"0.1"`,
  `"123.45"`, and `"0"`.
- [ ] Add failing tests rejecting Python/JSON integers, floats, booleans,
  exponent notation, leading plus signs, leading zeroes, NaN, and infinities.
- [ ] Add failing tests for domain aliases: positive values reject zero and
  negatives; nonnegative values accept zero and reject negatives.
- [ ] Add boundary tests for the 16-integer-digit and 8-fractional-digit limits,
  including rejection one digit beyond either bound.
- [ ] Add a test proving unexpected `Inexact`/`Rounded` signals trap in financial
  addition/multiplication while the isolated breakeven quantizer has explicit
  deterministic behavior.
- [ ] Run
  `uv run pytest tests/unit/contracts/test_decimal_string.py -q` and confirm the
  import failure.
- [ ] Implement `V1Model`, the decimal grammar/parser, plain-string serializer,
  `DecimalString`, `PositiveDecimalString`, and `NonNegativeDecimalString` using
  a local precision-50, half-even context plus explicit outward money-demand and
  inward profit quantizers.
- [ ] Rerun the decimal test and confirm it passes.
- [ ] Write failing canonical tests proving nested key sorting, compact UTF-8,
  Unicode preservation, UTC `Z` normalization, lowercase UUIDs, tuple-to-array
  conversion, and byte equality for numerically equivalent decimal strings.
- [ ] Add a test that canonical serialization does not mutate its input and
  rejects nonfinite or unsupported Python values.
- [ ] Run `uv run pytest tests/unit/serialization/test_canonical.py -q` and
  confirm the import failure.
- [ ] Implement `JsonValue`, `canonical_json_bytes`, and `canonical_sha256` with
  one recursive normalization boundary and standard-library `json.dumps`.
- [ ] Run both new test modules, then the full unit suite.
- [ ] Run Ruff, format check, and mypy.
- [ ] Commit with `feat: add strict decimal and canonical JSON foundation`.

### Task 3: Define complete version-1 input and output contracts

**Files:**

- Create: `src/ai_stock_forum/contracts/v1/__init__.py`
- Create: `src/ai_stock_forum/contracts/v1/enums.py`
- Create: `src/ai_stock_forum/contracts/v1/market.py`
- Create: `src/ai_stock_forum/contracts/v1/trade.py`
- Create: `src/ai_stock_forum/contracts/v1/policy.py`
- Create: `src/ai_stock_forum/contracts/v1/result.py`
- Create: `src/ai_stock_forum/contracts/v1/validation.py`
- Create: `tests/factories.py`
- Create: `tests/conftest.py`
- Create: `tests/unit/contracts/test_v1_models.py`

- [ ] Define the enums named in this plan with explicit uppercase values:
  `AssetType`, `UnderlyingAssetKind`, `OptionType`, `OptionAction`,
  `DeliverableKind`, `ExerciseStyle`, `SettlementStyle`, `Currency`, `LimitKind`,
  `DeclaredStrategy`, `ResolvedStructure`, `ValidationStatus`, `RuleStatus`,
  `RuleScope`, `BoundKind`, `RiskRewardReason`, `IntendedExit`,
  `LifecycleBoundary`, `OperationalContextStatus`,
  `CapacityCommitmentStatus`, `PayoffProfileStatus`, and `RuleCode`.
- [ ] In `tests/factories.py`, create deterministic constructors for a UTC
  `as_of`, underlying quote, standard call/put contracts, option quotes, zero
  costs, permissive policy, unused account capacity, and each candidate type.
  Every UUID must be a fixed literal, not random.
- [ ] Write a failing happy-path model test for one long call
  `TradeValidationRequest` and its JSON round trip.
- [ ] Add failing tests for naive timestamps, unknown fields, mutation,
  non-string money, nonpositive or over-limit quantities/ratios, more than eight
  option legs,
  duplicate/dangling contract or quote IDs, a mismatched quote/contract link,
  absent `sector_id`, invalid candidate union fields, and a closing action.
- [ ] Add model-acceptance tests for syntactically valid unsafe domain facts:
  `cash_funded=false`, adjusted/non-100 contracts,
  `atomic_package=false`, `legging_allowed=true`, and a missing short-leg close
  rule. These must survive parsing so later rules can reject them.
- [ ] Add schema tests that an option candidate has exactly one package limit
  object and uses only the two opening-action enum values.
- [ ] Run `uv run pytest tests/unit/contracts/test_v1_models.py -q` and confirm
  the missing-contract import failure.
- [ ] Implement the input models exactly as defined in this plan, including
  model-level referential-integrity validation in `TradeValidationRequest`.
- [ ] Add complete result models for finite/unbounded values, payoff points and
  zero intervals, operational scenarios/exposure, market observations, capacity
  vectors, sizing, reservation draft, rule results, and
  `TradeValidationResult`. Optional calculation sections must be explicitly
  nullable only because an earlier gate may fail.
- [ ] Add input validation for an explicit zero existing-portfolio payoff
  profile, a complete nonzero piecewise-linear profile, per-package conservative
  buying-power floor, nonnegative committed vectors, and unsupported-exposure
  codes. Profile points must begin at spot zero and increase strictly. Complete
  position commitment/profile provenance and evaluation timestamps must match.
- [ ] Add valid complete and incomplete operational-context union tests. Complete
  false/empty facts mean confirmed absence; incomplete context requires reason
  codes and rejects event/stress fields so unknown cannot masquerade as false.
- [ ] Require `permitted_strategies` to be a nonempty, lexicographically sorted,
  duplicate-free tuple. Add two separate process invocations proving request
  hashing cannot depend on set/hash iteration order.
- [ ] Rerun the model tests and confirm all pass without coercion warnings.
- [ ] Add a serialization test proving all decimal output fields remain JSON
  strings and all enum values are uppercase.
- [ ] Run the full unit suite, Ruff, format check, and mypy.
- [ ] Commit with `feat: add v1 trade validation contracts`.

### Task 4: Resolve candidates and enforce the long-stock/long-option allowlist

**Files:**

- Create: `src/ai_stock_forum/risk/violations.py`
- Create: `src/ai_stock_forum/risk/structure.py`
- Create: `tests/unit/risk/test_structure_long_options.py`

- [ ] Create a fixed `RuleCode` order and a `Violation` constructor that
  requires stable code, evidence message, observed values, thresholds, and
  fixture record IDs. Do not use free-form exception messages as rule identity.
- [ ] Write failing resolution tests for stock, one long call, and one long put.
  Assert the resolved option leg contains the exact immutable contract and quote
  selected by the candidate references.
- [ ] Write failing acceptance tests for cash-funded long stock and debit-paid
  single long call/put packages.
- [ ] Write table-driven failing rejections for a naked short, wrong declared
  strategy, credit single long option, duplicate leg contract, adjusted or
  nonstandard contract, nonphysical settlement, non-American style, non-USD
  currency, non-100 multiplier/deliverable, mixed underlying, long-option ratio
  other than one, and `cash_funded=false`. User-policy strategy permission is a
  validator rule in Task 11. Dangling market IDs remain schema errors covered in
  Task 3.
- [ ] Run
  `uv run pytest tests/unit/risk/test_structure_long_options.py -q` and confirm
  the missing implementation failure.
- [ ] Implement `resolve_candidate` as a pure lookup/normalization step and
  `validate_structure` for stock and long single options only. It must return a
  typed assessment rather than raising for domain rejection.
- [ ] Test the assessment's calculation eligibility: a mislabeled otherwise
  standard package keeps generic payoff eligibility; adjusted deliverables and
  mixed expirations do not; only an allowlisted standard template receives
  operational eligibility.
- [ ] Rerun the focused tests and confirm stable code ordering independent of
  input leg ordering.
- [ ] Run the full unit suite, Ruff, format check, and mypy.
- [ ] Commit with `feat: validate stock and long-option structures`.

### Task 5: Build the generic expiration-payoff kernel

**Files:**

- Create: `src/ai_stock_forum/risk/payoff.py`
- Create: `tests/unit/risk/test_payoff_stock.py`
- Create: `tests/unit/risk/test_payoff_options.py`

- [ ] Write failing stock tests: 20 shares at `$50` with zero cost has `$1,000`
  maximum loss, unbounded profit, and `$50` breakeven; a `$10` fixed opening fee
  changes maximum loss to `$1,010` and breakeven to `$50.5`.
- [ ] Run `uv run pytest tests/unit/risk/test_payoff_stock.py -q` and confirm the
  missing payoff implementation.
- [ ] Implement stock `expiration_profit` and `analyze_expiration_payoff` using
  only `Decimal`, then rerun the stock tests.
- [ ] Write failing long-option golden tests with multiplier 100 and zero cost:
  a `$100` call for `$5` has `$500` loss, unbounded profit, and `$105`
  breakeven; a `$100` put for `$4` has `$400` loss, `$9,600` profit, and `$96`
  breakeven. Assert expiration and managed-close curves coincide under zero
  close/settlement costs.
- [ ] Add failing tests that a `$10` fixed opening fee increases debit maximum
  loss by exactly `$10`, the `0.10 + 0.20` case remains exactly `"0.3"`, and an
  impossible long-put debit above its strike is reported with no positive
  achievable payoff.
- [ ] Add path-isolation oracles: a `$3` planned-close fee shifts only the
  managed-close curve, a `$4` settlement-fee reserve shifts only the expiration
  curve, and conservative maximum loss selects the larger resulting loss.
- [ ] Add intended-exit reward tests where raw profit is positive but configured
  fees make adjusted profit negative, zero, or a positive amount below one cent.
  Each payoff summary carries `NO_POSITIVE_REWARD_AFTER_COSTS`; exactly `$0.01`
  after conservative rounding carries a positive reward. Task 11 maps this typed
  payoff fact to the `POSITIVE_ACHIEVABLE_REWARD` rule.
- [ ] Add failing breakpoint tests for payoff at zero, exactly at a strike, and
  above a strike; assert continuity and exact values.
- [ ] Run `uv run pytest tests/unit/risk/test_payoff_options.py -q` and confirm
  the missing option implementation.
- [ ] Implement the generic signed-leg intrinsic sum, separated
  opening/planned-close/settlement cost functions, unique sorted breakpoints,
  tail-slope analysis, finite/unbounded bound model, worst-price set,
  deterministic 8-decimal breakeven solver, flat-zero interval reporting, and
  explicit finite/unbounded/undefined risk-reward representation.
- [ ] Add failing post-trade payoff tests that combine a candidate with the zero
  profile, then with a nonzero existing piecewise-linear profile having different
  breakpoints. Assert union-breakpoint interpolation, tail-slope addition, and
  candidate-only values remaining unchanged.
- [ ] Implement `combine_with_existing_portfolio` and rerun the payoff tests.
- [ ] Rerun both payoff modules. Confirm no strategy-specific closed-form formula
  is used in production.
- [ ] Run the full unit suite, Ruff, format check, and mypy.
- [ ] Commit with `feat: calculate deterministic expiration payoff`.

### Task 6: Add all four defined-risk vertical structures

**Files:**

- Create: `tests/unit/risk/test_structure_verticals.py`
- Extend: `src/ai_stock_forum/risk/structure.py`
- Extend: `tests/unit/risk/test_payoff_options.py`
- Create: `tests/property/test_payoff_properties.py`

- [ ] Write table-driven failing acceptance tests for exactly these 1:1 shapes:
  lower long call/higher short call debit; higher long put/lower short put debit;
  higher short put/lower long put credit; and lower short call/higher long call
  credit.
- [ ] Write individual failing rejection tests for unequal ratios, duplicate or
  equal strikes, mixed call/put types, mixed expiration, mismatched deliverable,
  wrong protective-strike ordering, debit/credit direction mismatch, declared
  strategy mismatch, missing protective leg, a noncanonical `2/2` ratio,
  legging, and a missing close-before-expiration rule. Closing actions are schema
  errors, not domain fixtures.
- [ ] Run
  `uv run pytest tests/unit/risk/test_structure_verticals.py -q` and confirm the
  vertical cases are rejected by the current matcher.
- [ ] Extend the matcher with four explicit vertical templates and require the
  package metadata and actual leg shape to agree.
- [ ] Rerun the structure tests and confirm all rejection codes are stable.
- [ ] Add these failing golden payoff cases with zero costs:

  | Structure | Limit | Maximum loss | Maximum profit | Breakeven |
  |---|---:|---:|---:|---:|
  | 100/110 bull call debit | `$4` debit | `$400` | `$600` | `$104` |
  | 110/100 bear put debit | `$3` debit | `$300` | `$700` | `$107` |
  | 90/100 bull put credit | `$3` credit | `$700` | `$300` | `$97` |
  | 100/110 bear call credit | `$2` credit | `$800` | `$200` | `$102` |

- [ ] Add a `$10` fixed-opening-fee case proving credit maximum profit falls and
  expiration maximum loss rises by exactly `$10`; test close and settlement
  reserves in their own mutually exclusive paths.
- [ ] Add generic-kernel tests showing an uncovered short call and a `-2/+1`
  call ratio have negative upper-tail slope and explicit unbounded loss, even
  before structural rejection.
- [ ] Add raw economic-price regressions for credit exactly equal to width,
  credit greater than width with a fixed fee, and debit equal to width. Assert
  a typed cost-independent payoff failure and monotonic conservative loss
  demand; Task 11 maps the typed fact to `ECONOMIC_PRICE_SANITY` rejection.
- [ ] Add focused Hypothesis properties for valid vertical closed-form parity,
  leg-order invariance, continuity at both strikes, and generation of uncovered
  signed-call portfolios with a negative upper tail. Run these foundation
  properties before complex structures are added.
- [ ] Run the payoff and structure suites, then the full unit suite, Ruff,
  format check, and mypy.
- [ ] Commit with `feat: validate and value defined-risk verticals`.

### Task 7: Enumerate mandatory exercise and assignment states

**Files:**

- Create: `src/ai_stock_forum/risk/scenarios.py`
- Create: `tests/unit/risk/test_scenarios.py`
- Create: `tests/property/test_scenario_properties.py`

- [ ] Write failing unit-event tests for the exact share and strike-cash signs of
  long-call exercise, short-call assignment, long-put exercise, and short-put
  assignment.
- [ ] Write a failing long-option test proving no-event and exercise states are
  generated even for an out-of-the-money fixture; a long put exercise must show
  temporary short stock and sale proceeds.
- [ ] Write a failing two-leg vertical test expecting five ordered prefix
  scenarios per lifecycle boundary (`1 + P(2,1) + P(2,2)`) and all four
  terminal event subsets at each boundary.
- [ ] Add failing tests for short-only, long-only, both-leg, simultaneous-equivalent,
  sequential short-first, sequential long-first, and contrary-exercise states.
- [ ] Run `uv run pytest tests/unit/risk/test_scenarios.py -q` and confirm the
  missing scenario implementation.
- [ ] Implement generic unit-event expansion, all ordered subsets for every
  lifecycle boundary in stable lexicographic order, and prefix-state
  accumulation. There must be no
  `enable_scenarios`, moneyness filter, or other disable flag in any signature.
- [ ] Implement per-state net shares, gross shares, gross purchase cash, gross
  sale proceeds, settlement net cash, gross settlement notional, temporary stock
  notional, option-event fees, and short-stock flag.
- [ ] Add a closed-form 100/110 call-vertical oracle: long-only exercise has 100
  long shares and `$10,000` purchase cash; short-only assignment has 100 short
  shares and `$11,000` proceeds; both have zero net shares, 200 gross shares,
  `$10,000` purchases, `$11,000` proceeds, and `$21,000` gross settlement
  notional before configured event fees. Verify temporary notional uses the
  largest quote-linked stress price.
- [ ] Implement summary maxima without offsetting purchases with sale proceeds
  or long shares with short shares.
- [ ] Add a hostile ratio-100 fixture and assert the generator returns the typed
  candidate-local `NOT_APPLICABLE` four-event-cap result before allocating
  permutations; enforce a deterministic runtime/memory-safe path for
  structurally rejected candidates.
- [ ] Add focused properties that every unit-event subset appears, summaries
  dominate their states, and event order is deterministic. Run them before
  adding four-leg structures.
- [ ] Add a regression test where two offsetting assignments end at zero net
  shares but still retain 200 gross shares and the full gross purchase/sale
  settlement metrics.
- [ ] Rerun the focused tests, then unit suite, Ruff, format check, and mypy.
- [ ] Commit with `feat: enumerate assignment and exercise stress`.

### Task 8: Implement multi-dimensional capacity and reservation math

**Files:**

- Create: `src/ai_stock_forum/risk/capacity.py`
- Create: `tests/unit/risk/test_capacity.py`
- Create: `tests/property/test_capacity_properties.py`

- [ ] Write failing component-subtraction tests for unused capacity and for
  independent existing-position, open-order, and active-reservation consumption.
- [ ] Add failing tests that a known overcommit returns zero capacity and a
  candidate-local capacity `FAIL`, while an `INCOMPLETE` typed position, order,
  or reservation commitment returns a global capacity `UNKNOWN`; neither may
  silently clamp and pass. Task 11 maps those rule scopes to overall status.
- [ ] Add reconciliation tests where the existing payoff profile's rounded
  maximum loss equals, is below, and exceeds the positions expiration-loss
  commitment. Equality/below are consistent; excess loss, snapshot-ID mismatch,
  or timestamp mismatch returns global `UNKNOWN` and zero quantity for Task 11
  to map to `INCOMPLETE`.
- [ ] Run `uv run pytest tests/unit/risk/test_capacity.py -q` and confirm the
  missing implementation.
- [ ] Implement componentwise `subtract_capacity` and typed capacity-domain
  errors; never use generic dictionary keys outside the model boundary.
- [ ] Write failing `demand_for_quantity` tests for stock, long option, debit
  vertical, and credit vertical. Assert concentration demand uses the documented
  maximum, the per-package conservative buying-power floor binds when larger
  than expiration loss, cumulative cash adds opening outflow plus purchase cash
  plus event fees, sale proceeds/entry credit do not offset it, and temporary
  stock notional uses the maximum stress price.
- [ ] For stock, call demand at two different trial quantities and prove shares,
  entry/cumulative cash, buying power, temporary notional, concentration, and
  loss scale from the trial `q`, not the candidate's requested share count.
- [ ] Implement candidate demand for an arbitrary positive quantity and a
  componentwise `build_reservation_draft`.
- [ ] Write failing sizing tests for all binding cases:

  - `$1,500` expiration-loss capacity and `$700` loss per spread permits two;
  - `$15,000` gross-purchase-cash capacity and `$10,000` demand per package
    permits one even when loss permits two;
  - existing/reserved purchase-cash use leaving `$9,000` permits zero;
  - a `$500` long-call opening outflow plus `$10,000` exercise purchase requires
    `$10,500` cumulative cash and fails a `$10,000` cumulative limit;
  - a 150-share temporary limit and 100 shares per package permits one;
  - an exact boundary passes and one cent less reduces quantity;
  - symbol and sector concentration each bind independently; and
  - fixed-plus-variable fees return the exact largest integer rather than a
    simple floor based on one-package loss.

- [ ] Add a 2%-of-equity oracle: on `$50,000` equity the percentage cap is
  exactly `$1,000`; verify it, the dollar cap, and remaining aggregate loss are
  compared as three independent minima.
- [ ] Add an aggregate-defined-risk oracle where account capacity is generous
  but existing/open-order/reserved expiration loss exhausts the user's
  `maximum_aggregate_open_defined_risk_usd`.
- [ ] Add long-stock allocation tests where the policy dollar cap, policy
  equity-percentage cap, account hard limit, and committed stock allocation each
  bind independently. Option candidates must consume zero in this coordinate.

- [ ] Add failing tests that zero maximum loss, an absent asset-specific absolute
  quantity cap,
  overcommitted input, unsupported existing exposure, or unknown reservation
  state never creates an unlimited quantity.
- [ ] Implement monotonic integer binary search over
  `0..policy.absolute_cap_for(candidate)` and return the binding dimensions and
  their headroom.
- [ ] Add reservation-conservation tests proving
  `remaining_before - reservation == remaining_after` componentwise. Test the
  pure draft helper at a capacity-bound maximum and prove one additional unit
  fails; test the absolute-quantity-cap case separately as an explicit binding
  reason even when one additional unit fits all financial coordinates.
- [ ] Add focused monotonic-capacity and reservation-conservation properties and
  run them before integrating complex structures.
- [ ] Run the focused suite, full unit suite, Ruff, format check, and mypy.
- [ ] Commit with `feat: add capacity sizing and reservation math`.

### Task 9: Add butterflies, iron butterflies, and iron condors

**Files:**

- Create: `tests/unit/risk/test_structure_complex.py`
- Create: `tests/unit/risk/test_payoff_complex.py`
- Extend: `src/ai_stock_forum/risk/structure.py`
- Extend: `tests/unit/risk/test_scenarios.py`

- [ ] Write failing accepted-shape tests for symmetric long call and long put
  butterflies, symmetric short-credit iron butterflies, symmetric-wing iron
  condors, and unequal-wing iron condors.
- [ ] Write one failing rejection test per malformed shape: wrong butterfly
  ratio, nonequidistant butterfly strikes, a short single-type butterfly,
  broken-wing butterfly, reverse iron butterfly, reverse iron condor, unordered
  condor strikes, overlapping put/call bodies, duplicate contract, mixed
  expiration, mixed deliverable, unequal iron-butterfly wing widths, missing
  wing, extra leg, debit iron structure, and declared-strategy mismatch.
- [ ] Run
  `uv run pytest tests/unit/risk/test_structure_complex.py -q` and confirm the
  complex structures are not yet recognized.
- [ ] Add explicit complex-template matchers after the existing stock, long, and
  vertical matchers. Match normalized leg tuples; do not infer structure from
  the declared label.
- [ ] Rerun the complex structure tests.
- [ ] Add failing golden payoff tests:

  | Structure | Limit | Expected result |
  |---|---:|---|
  | 90/100/110 long call butterfly | `$2` debit | loss `$200`, profit `$800`, BEs `$92/$108` |
  | 90/100/110 long put butterfly | `$2` debit | loss `$200`, profit `$800`, BEs `$92/$108` |
  | 90p/100p + 100c/110c iron butterfly | `$4` credit | loss `$600`, profit `$400`, BEs `$96/$104` |
  | 90/95 put + 105/110 call iron condor | `$2` credit | loss `$300`, profit `$200`, BEs `$93/$107` |
  | 85/95 put + 105/110 call iron condor | `$2` credit | profit `$200`, lower loss `$800`, upper loss `$300`, BEs `$93/$107` |

- [ ] Add cost tests showing a `$10` fixed opening fee shifts both lifecycle
  payoff paths down by exactly `$10`; separately test planned-close-only and
  settlement-only fees. Add rejection tests for raw zero loss/apparent arbitrage
  and no positive raw achievable payoff.
- [ ] Run `uv run pytest tests/unit/risk/test_payoff_complex.py -q`, implement no
  new payoff formula, and confirm the existing generic kernel satisfies every
  case.
- [ ] Add scenario tests for a ratio-two butterfly body: one of the two short
  contracts assigned, both assigned, and each assignment order must exist.
- [ ] Assert a four-unit-event package emits 65 ordered prefix scenarios per
  lifecycle boundary (`sum(P(4,k), k=0..4)`) and 16 distinct terminal subset
  signatures at each boundary for the iron condor. Confirm all long/short
  call/put event signs appear.
- [ ] Add a regression test showing an iron butterfly can end with zero net
  shares while retaining gross values. The two 100-strike body assignments have
  200 gross shares, `$10,000` purchases, and `$10,000` proceeds. The all-four-
  event state has 400 gross shares, `$21,000` purchases, `$19,000` proceeds,
  `$40,000` gross settlement notional, and `-$2,000` settlement net cash before
  event fees.
- [ ] Run all structure, payoff, and scenario tests; then the full unit suite,
  Ruff, format check, and mypy.
- [ ] Commit with `feat: support defined-risk complex option packages`.

### Task 10: Enforce quote, liquidity, holding-window, and event gates

**Files:**

- Create: `src/ai_stock_forum/risk/market_rules.py`
- Create: `tests/unit/risk/test_market_rules.py`

- [ ] Write failing tests for exact quote age at the policy boundary, one second
  stale, a future observation, crossed bid/ask, zero midpoint, saved midpoint
  mismatch, exact maximum spread, and spread one decimal quantum too wide.
- [ ] Run the integrity, age, spread, and conditional-limit cases for both an
  `UnderlyingQuote`/long-stock candidate and option legs. Verify the reported
  natural package market is derived from saved leg bids/asks and the submitted
  package limit remains the payoff input.
- [ ] Assert stale, future, crossed, zero-midpoint, and midpoint-mismatch evidence
  produces global `INCOMPLETE`; a valid but too-wide spread is a candidate-local
  liquidity `REJECT`.
- [ ] Add failing tests for volume and open interest at and below their minimums.
  Every candidate leg must pass individually; package averages are prohibited.
- [ ] Add failing tests for minimum/maximum DTE, planned holding end after
  expiration, and a short package whose planned exit violates the configured
  close-before-expiration buffer. Test exact inclusive second boundaries and one
  second on either failing side; do not use date truncation. Add two cases where
  the candidate buffer is stricter than policy and where policy is stricter than
  the candidate, proving the maximum governs.
- [ ] Add failing provenance tests for a mismatched operational underlying quote
  ID, an empty stress-price set, and a stress price below saved ask. Each is a
  global integrity failure; a valid context uses its maximum stress price.
- [ ] Add an incomplete operational-context request and assert operational and
  event assessments are global `UNKNOWN` even when all known quote fields are
  valid. Overall status and CLI mapping are added after validator/CLI exist.
- [ ] Add one failing test for each deterministic hard event flag: configured
  earnings avoidance, ex-dividend exposure, corporate action, low short-leg
  extrinsic value, and pin risk.
- [ ] Add tests proving a disabled earnings-avoidance policy may allow earnings,
  but no policy toggle can suppress corporate-action inconsistency or operational
  scenario generation.
- [ ] Run `uv run pytest tests/unit/risk/test_market_rules.py -q` and confirm the
  missing implementation.
- [ ] Implement `assess_market_and_events` with request `as_of` as its only time
  source. Return typed observations and stable rule results; do not perform I/O.
- [ ] Rerun the focused tests and inspect failures for exact boundary semantics.
- [ ] Run the full unit suite, Ruff, format check, and mypy.
- [ ] Commit with `feat: enforce market and lifecycle risk gates`.

### Task 11: Compose the deterministic validator

**Files:**

- Create: `src/ai_stock_forum/risk/validator.py`
- Create: `tests/unit/risk/test_validator.py`

- [ ] Write a failing long-call happy-path test asserting exact request hash,
  normalized calculation-input hash, UUIDv5 result ID, engine version, resolved
  structure, candidate/post-trade payoff, operational exposure, market
  assessment, requested/max quantity, reservation draft, fixed rule order, and
  `PASS` status.
- [ ] Run `uv run pytest tests/unit/risk/test_validator.py -q` and confirm the
  missing validator implementation.
- [ ] Implement the pure pipeline in this order: canonical request hash; candidate
  resolution; structural assessment; generic payoff only when payoff terms are
  supported; unconditional operational-gate invocation with enumeration only
  when operational terms are supported; market/event assessment; capacity
  sizing; stable rules; status; deterministic result identity.
- [ ] Add a spy-based regression test proving scenario generation is called for
  every resolved options candidate, including a structurally rejected uncovered
  short. Unsupported contract terms or a package over the expanded-event cap
  return typed candidate-local `NOT_APPLICABLE` operational evidence without
  enumeration and preserve the structural `REJECT`. Dangling contract/quote IDs
  are rejected earlier by the request schema and are not a dead validator
  branch.
- [ ] Add failing rejection tests for unbounded payoff, low liquidity, event
  block, fee-erased intended-exit reward, strategy disabled by policy, and
  requested quantity above capacity.
  Each result must retain the calculation evidence available before rejection.
- [ ] Add failing incomplete tests for a stale/future quote and invalid quote
  integrity. Assert these mandatory-evidence failures cannot be consumed as a
  completed candidate rejection.
- [ ] Add failing incomplete tests for unsupported positions, unknown open-order
  exposure, unknown reservation reconciliation, and an incomplete existing
  payoff profile. Assert maximum quantity zero and no reservation draft. A JSON
  object that simply omits a required quote field is a schema error, not a
  domain-level incomplete assessment.
- [ ] Add the Task 10 incomplete operational context to the composed validator;
  assert quantity zero, no reservation draft, and overall `INCOMPLETE`.
- [ ] Add a precedence test combining unsupported account exposure with an
  obviously invalid candidate. The overall status remains `INCOMPLETE`, maximum
  quantity is zero, and candidate-local failures remain visible in `rules`.
- [ ] Add a structure-first safety test: changing only `declared_strategy`
  cannot change calculated payoff, but it does add a structural rule failure and
  prevents `PASS`.
- [ ] Add a risk-gate precedence test: user-requested quantity and permissive
  labels cannot override one failing hard rule.
- [ ] Add a contract test proving the validator's emitted reservation draft uses
  requested quantity only, while `maximum_permitted_quantity` remains advisory.
- [ ] Add a stock-result test proving every option-only rule remains in fixed
  order as `NOT_APPLICABLE` and does not force rejection or incompleteness.
- [ ] Add a repeated-run test proving identical input produces identical result
  bytes, IDs, hashes, and rule order. Add a separate leg-permutation test proving
  calculated metrics, normalized calculation-input hash, and rule order remain
  equal while exact request hash and result ID may differ. Canonicalize resolved
  legs by immutable contract ID for the calculation hash and retain the exact
  normalized request hash separately.
- [ ] Rerun the validator tests, then all unit tests, Ruff, format check, and
  mypy.
- [ ] Commit with `feat: compose deterministic trade validator`.

### Task 12: Export schemas and ship the fixture CLI

**Files:**

- Create: `src/ai_stock_forum/cli.py`
- Create: `schemas/v1/trade-validation-request.schema.json`
- Create: `schemas/v1/trade-validation-result.schema.json`
- Create: `examples/fixtures/pass/*.json`
- Create: `examples/fixtures/reject/*.json`
- Create: `examples/fixtures/incomplete/*.json`
- Create: `tests/golden/*.json`
- Create: `tests/unit/contracts/test_schema_exports.py`
- Create: `tests/integration/test_cli.py`
- Create: `tests/integration/test_golden_results.py`

- [ ] Write a failing schema-export test that generates both top-level Pydantic
  schemas in memory, canonicalizes them, and compares them byte-for-byte with the
  two checked-in files.
- [ ] Implement a private schema-export function used by the CLI; invoke it once
  to create both schema files, then rerun the drift test.
- [ ] Hand-author the synthetic fixtures listed in the repository shape. Use
  fixed UUIDs and timestamps, one symbol such as `XYZ`, no real account values,
  and comments only in adjacent README prose because JSON fixtures remain strict.
- [ ] Validate each fixture directly with `TradeValidationRequest.model_validate_json`
  before generating expected results.
- [ ] Generate the listed golden result files through `validate_trade`, inspect
  every financial value against the closed-form table, then check them in. Do
  not provide a test flag that silently rewrites goldens.
- [ ] Write a parameterized golden regression test mapping every example fixture
  to its expected result file and comparing fresh canonical result bytes exactly.
  The test must never regenerate or update its oracle.
- [ ] Write failing CLI tests for stdin-independent file input, canonical stdout,
  `--pretty`, schema output to an explicit directory, stderr separation, and
  exact exit codes `0`, `2`, `3`, `64`, and `70`.
- [ ] Cover exit `3` with both stale evidence and an otherwise valid request
  whose operational context is explicitly `INCOMPLETE`.
- [ ] Add tests for nonexistent files, malformed JSON, schema-invalid JSON, and
  an injected internal exception. Schema-invalid cases include zero requested
  quantity, a closing action, and a dangling quote ID and must exit `64`. Error
  output must identify the category and JSON path without echoing the whole
  fixture.
- [ ] Run `uv run pytest tests/integration/test_cli.py -q` and confirm the
  missing CLI behavior.
- [ ] Implement `argparse` subcommands and the smallest explicit exception
  boundary needed to satisfy the contract. The CLI does not read environment
  configuration.
- [ ] Run every pass, reject, and incomplete example once from the shell and
  confirm its status, hash, stdout/stderr behavior, and exit code.
- [ ] Run the schema, golden, and CLI suites; then full unit tests, Ruff, format
  check, and mypy.
- [ ] Commit with `feat: ship fixture-backed risk validator CLI`.

### Task 13: Add property tests and adversarial boundary coverage

**Files:**

- Extend: `tests/property/test_payoff_properties.py`
- Extend: `tests/property/test_scenario_properties.py`
- Extend: `tests/property/test_capacity_properties.py`
- Extend: `tests/unit/risk/test_payoff_options.py`
- Extend: `tests/unit/risk/test_payoff_complex.py`

- [ ] Write Hypothesis strategies that generate valid decimal strikes, widths,
  premiums, fees, quantities, and policy limits without converting through
  floats.
- [ ] Add payoff properties: leg permutation invariance; continuity at every
  strike; generic output equals closed-form long/vertical/butterfly/iron
  oracles; loss never becomes less conservative when cost rises; and multiplying
  quantity scales variable payoff while applying fixed fees exactly once.
- [ ] Generate arbitrary signed call-leg portfolios within the event cap and
  prove every negative upper-tail slope is classified as unbounded loss and
  cannot pass, independent of its strategy label.
- [ ] Add adversarial breakeven cases: a root exactly at a strike, no root, one
  root on the unbounded upper ray, duplicate roots from neighboring segments,
  and a flat zero-payoff interval. Assert no division-by-zero path.
- [ ] Add scenario properties: all unit event subsets appear; summary values are
  at least every state value; ordering is deterministic; adding an event cannot
  reduce gross shares/notional for that state; and sampled two-package brute
  force never exceeds twice the per-package conservative summary.
- [ ] Add capacity properties: increasing any committed component never raises
  maximum quantity; increasing a limit never lowers it; a capacity-bound
  maximum-quantity draft fits; one additional unit fails at a reported financial
  binding dimension; an absolute-cap-only case reports the nonfinancial binding
  reason; and capacity subtraction/reservation is exact.
- [ ] Run each property module separately with its failure database enabled and
  fix the implementation, not the generated example, for every counterexample.
- [ ] Rerun with
  `uv run pytest tests/property -q --hypothesis-show-statistics` and confirm no
  flaky deadline dependence. Use `deadline=None` only at the test profile level,
  not to hide algorithmic slowness.
- [ ] Run the complete test and static-analysis gate.
- [ ] Commit with `test: harden risk engine with property coverage`.

### Task 14: Document, review, and verify the phase

**Files:**

- Modify: `README.md`
- Modify: `architecture.md`
- Modify: `docs/superpowers/plans/2026-08-09-ai-stock-forum-roadmap.md`
- Modify: this plan by checking completed task boxes during execution

- [ ] Add a concise README quick start covering `uv sync`, schema export, one
  passing validation, one rejected validation, exit codes, and the guarantee
  that inputs are fixtures with no network or broker access.
- [ ] Add a Phase 1 implementation-status link from `architecture.md` without
  changing the approved architecture.
- [ ] Mark Phase 1 complete in the roadmap only after every exit criterion is
  demonstrated.
- [ ] Search production code for forbidden dependencies and capabilities:

  ```text
  rg -n "requests|httpx|aiohttp|socket|fastapi|sqlalchemy|subprocess|os\.environ|datetime\.now|utcnow|hermes|mcp|broker" src
  ```

  Review every match; expected `broker` matches are typed conservative-capacity
  field names, not clients or network calls. Other expected matches are
  documentation strings or none. A network/process import or clock/environment
  read fails the phase.
- [ ] Search for placeholder or weakened behavior:

  ```text
  rg -n "TODO|TBD|FIXME|pass$|NotImplemented|mock return|type: ignore" src tests schemas examples README.md architecture.md
  ```

  Resolve every production match and justify any test-only match in the commit
  body.
- [ ] Run schema and lockfile drift checks:

  ```text
  uv lock --check
  risk_schema_check_dir="$(mktemp -d)"
  uv run asf-risk schema --output "$risk_schema_check_dir"
  diff -ru schemas/v1 "$risk_schema_check_dir"
  ```

- [ ] Run the final quality gate from a clean environment:

  ```text
  uv sync --locked
  uv run pytest -q
  uv run pytest tests/property -q --hypothesis-show-statistics
  uv run pytest --cov=ai_stock_forum --cov-branch --cov-report=term-missing --cov-fail-under=95
  uv run ruff check .
  uv run ruff format --check .
  uv run mypy src tests
  git diff --check
  git status --short
  ```

- [ ] Use `superpowers:requesting-code-review` for an independent review against
  the design specification, this plan, Decimal/canonicalization invariants,
  generic boundedness proof, gross operational capacity, and fail-closed sizing.
- [ ] Address every confirmed high- or medium-severity issue with a new failing
  regression test before changing production code.
- [ ] Repeat the entire final quality gate and inspect the exact output before
  claiming completion.
- [ ] Commit documentation and review fixes with
  `docs: complete deterministic risk core phase`.

---

## Explicitly Deferred from Phase 1

- Hermes installation, profile containers, personas, prompts, debate, and
  ChatGPT subscription admission.
- Web research, filings/news/market acquisition, option-chain acquisition, and
  GEX MCP calls.
- Live quotes, account data, position/open-order normalization, brokerage
  credentials, and broker-specific buying-power parity.
- SQLite, migrations, audit events, persistent/atomic capacity reservations,
  FastAPI, SSE, and React.
- Human approval records and approval-time refresh.
- Greeks, implied volatility, probability estimates, ranking, optimization,
  calendars, diagonals, broken-wing butterflies, ratios, adjusted contracts,
  nonstandard deliverables, and multi-expiration structures.
- Hybrid memory, outcome evaluation, and historical replay.

These are later-phase boundaries, not missing error paths. Phase 1 contracts
must reject their unsupported forms explicitly.

## Reference Material

- Approved design: [AI Stock Forum design specification](../specs/2026-08-08-ai-stock-forum-design.md)
- Architecture: [architecture.md](../../../architecture.md)
- uv locking and syncing: <https://docs.astral.sh/uv/concepts/projects/sync/>
- Pydantic package metadata: <https://pypi.org/project/pydantic/>
- pytest package metadata: <https://pypi.org/project/pytest/>
- Hypothesis package metadata: <https://pypi.org/project/hypothesis/>
