# Task 14 Acceptance Report

## Result

Phase 0 Task 14 acceptance is complete on branch
`codex/phase-0-rust-foundation`. The tested implementation head is
`cb93641ff94e4956a875304c2ed193e4a1687e46`, based on
`2f432ad925850a41ecebbdfd8386008a7db84c70`.

The report commit necessarily follows the tested implementation commit. Its
exact hash and the final branch head are returned in the task handoff because a
Git commit cannot embed its own hash in its contents.

## TDD acceptance evidence

The prescribed `tests/phase0_acceptance.rs` scenario was absent. It was added
before its support implementation.

| Stage | Exact command | Exit | Salient result |
| --- | --- | ---: | --- |
| RED | `cargo test --test phase0_acceptance --locked` | 101 | `E0425`: `support::persistent_fixture` not found. |
| GREEN | `cargo test --test phase0_acceptance --locked` | 0 | 1 passed, 0 failed. |

The smallest implementation was a test-only persistent fixture that reuses
temporary `AppPaths`, a deterministic clock and ID generator, the real
`ApplicationService`/`ApplicationRuntime`, `EventRepository::verify`, direct
event reduction, and persisted projection loading. No production interface was
added for acceptance testing.

## Genuine gate failures and root causes

### Formatting

`cargo fmt --all --check` initially exited 1 with approximately 5,998 diff
lines across the clean base tree. Rust `1.98.0` and `rustfmt 1.9.0-stable` are
pinned and no `rustfmt.toml` override exists. Root cause: inherited source and
tests were not normalized for the pinned formatter.

Correction and verification:

| Exact command | Exit | Salient result |
| --- | ---: | --- |
| `cargo fmt --all` | 0 | Pinned formatter applied. |
| `cargo fmt --all --check` | 0 | No diff. |

After the Clippy rewrites, one `parser.rs` closure-layout diff was exposed.
`cargo fmt --all` exited 0 and the immediate `cargo fmt --all --check` exited 0.

### Clippy

The first `cargo clippy --workspace --all-targets --all-features -- -D warnings`
exited 101 with six inherited library errors: two `collapsible_if`, one
`derivable_impls`, one `map_flatten`, and two `io_other_error` diagnostics.
The smallest compiler-suggested, behavior-preserving rewrites were applied.

The next exact Clippy run exited 101 after progressing to integration targets,
where it exposed four more inherited diagnostics: three `io_other_error` and
one `manual_is_multiple_of`. Those test-only expressions were minimally
rewritten. The next exact Clippy run exited 0 with no warnings.

No new suppression was added. These were non-behavioral gate failures, so
the exact formatting and Clippy commands are the focused regression gates; no
gratuitous behavior test was added.

## Final acceptance and quality matrix

All commands below were rerun on the acceptance-review source tree represented
by implementation commit `cb93641ff94e4956a875304c2ed193e4a1687e46`.

| Gate | Exact command | Exit | Salient output |
| --- | --- | ---: | --- |
| Phase 0 acceptance | `cargo test --test phase0_acceptance --locked` | 0 | 1 passed, 0 failed. |
| Recovery contract | `cargo test --test recovery_contract --locked` | 0 | 22 passed, 0 failed. |
| Fallback contract | `cargo test --test fallback_contract --locked` | 0 | 18 passed, 0 failed. |
| Formatting | `cargo fmt --all --check` | 0 | No diff. |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | Finished with no warnings. |
| Full locked all-target suite | `cargo test --workspace --all-targets --locked` | 0 | 212 passed, 0 failed across 20 target runners. |
| Locked build | `cargo build --workspace --locked` | 0 | Dev build finished successfully. |
| Offline full suite | `cargo test --workspace --all-targets --locked --offline` | 0 | 212 passed, 0 failed across 20 target runners; Cargo was constrained from registry/index/package network access, while test runtime networking was not sandboxed or independently observed. |
| Explicit binary smoke | `cargo test --test fallback_contract binary_smoke_quit_and_eof_exit_successfully --locked` | 0 | 1 passed, 0 failed; real binary quit and EOF subprocess paths succeeded. |

The full fallback suite additionally passed real-binary startup-failure
redaction and previous-session warning scenarios.

## Initial repository and source safety scans (superseded)

The Task 14 brief contains no additional literal scan commands, so the
following acceptance-owner scans implement the safety checks requested in the
task handoff. For raw `rg`, exit 1 means no match and is a clean result.

| Concern | Exact command | Exit | Result/classification |
| --- | --- | ---: | --- |
| Credential signatures | `rg -n -I '(-----BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY-----|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9_]{20,}|sk-(live|prod)-[A-Za-z0-9]{16,})' . --hidden --glob '!.git/**' --glob '!target/**'` | 1 | No matches. |
| Absolute local paths | `rg -n -I '(/Users/|/home/|/private/tmp/|[A-Za-z]:\\\\Users\\\\)' . --hidden --glob '!.git/**' --glob '!target/**'` | 0 | 7 deliberate/non-production matches: linked-worktree `.git` metadata (1), toolchain install commands in the approved plan (4), and documentation tests asserting local paths are absent (2). No production source match. |
| Debug/TODO stubs | `rg -n -I '\b(dbg!|todo!|unimplemented!)|TODO|FIXME|HACK|XXX' src` | 1 | No production source matches. |
| Mock/fake/stub adapters | `rg -n -i -I '\b(mock|fake|stub)(ed|s|ing)?\b' src` | 1 | No production source matches. |
| External-action source | `rg -n -I '\b(TcpStream|TcpListener|UdpSocket|std::process::Command|Command::new|reqwest|hyper|ureq|curl|oauth|broker)\b' src` | 0 | One `Command::new` match in the `#[cfg(test)]` panic-boundary subprocess test; no production external action. |
| External-action dependencies | `rg -n '^name = "(reqwest|hyper|ureq|curl|tonic|axum|actix-web|tokio|alpaca|ibapi)"$' Cargo.lock` | 1 | No matches. |
| Credential terms in source | `rg -n -i -I '\b(secret|credential|password|passwd|api[_ -]?key|access[_ -]?token|private[_ -]?key)\b' src` | 0 | 3 matches, all in the `#[cfg(test)]` panic-redaction test in `src/panic_boundary.rs`; no production credential path. |
| Credential terms in tests | `rg -n -i -I '\b(secret|credential|password|passwd|api[_ -]?key|access[_ -]?token|private[_ -]?key)\b' tests` | 0 | 38 deliberate fixture/assertion matches covering redaction, non-persistence, parser safety, and documentation privacy. They are fake test strings, not credentials. |

This initial scan set was incomplete because it omitted common credential,
path, and networking forms and because `rg --hidden` still respected ignored
paths. It is retained as historical evidence only and is superseded by the
acceptance-review scan section below.

## Review

The acceptance owner reviewed the whitespace-insensitive substantive changes
against the approved design specification and Task 14 plan. `git diff --check`
exited 0. No actionable review findings remained.

The plan's `superpowers:requesting-code-review` workflow normally dispatches a
reviewer subagent. The task handoff explicitly prohibited subagents, so no
independent reviewer was dispatched; the acceptance owner performed the final
review directly.

## Changed files

Implementation commit: `174ebefcaff23e187dcff5c2f1789348318e82e0`
(`test: prove phase 0 exit gate`), 49 files changed, 2,467 insertions, 860
deletions.

Substantive acceptance/lint files:

- `tests/phase0_acceptance.rs` (new prescribed acceptance scenario)
- `tests/support/mod.rs` (persistent acceptance fixture)
- `src/persistence/projection_repository.rs`
- `src/setup/models.rs`
- `src/ui/command/parser.rs`
- `src/ui/command/runner.rs`
- `tests/fallback_contract.rs`
- `tests/fallback_fix_round_contract.rs`

Formatter-only files normalized by pinned `rustfmt 1.9.0-stable`:

- `src/app/command.rs`
- `src/app/event.rs`
- `src/app/mod.rs`
- `src/app/outcome.rs`
- `src/app/service.rs`
- `src/audit/mod.rs`
- `src/config/paths.rs`
- `src/config/process_guard.rs`
- `src/domain/digest.rs`
- `src/domain/id.rs`
- `src/domain/mod.rs`
- `src/main.rs`
- `src/panic_boundary.rs`
- `src/persistence/command_receipt_repository.rs`
- `src/persistence/database.rs`
- `src/persistence/event_repository.rs`
- `src/persistence/migrations.rs`
- `src/persistence/mod.rs`
- `src/policy/mod.rs`
- `src/recovery/coordinator.rs`
- `src/recovery/mod.rs`
- `src/recovery/reducer.rs`
- `src/runtime/mod.rs`
- `src/ui/command/mod.rs`
- `src/ui/command/reader.rs`
- `src/ui/command/renderer.rs`
- `src/ui/command/windows.rs`
- `tests/application_contract.rs`
- `tests/command_contract.rs`
- `tests/documentation_contract.rs`
- `tests/domain_contract.rs`
- `tests/event_repository_contract.rs`
- `tests/event_repository_hardening_contract.rs`
- `tests/fallback_fix_round_2_contract.rs`
- `tests/migration_contract.rs`
- `tests/policy_contract.rs`
- `tests/projection_contract.rs`
- `tests/recovery_contract.rs`
- `tests/runtime_contract.rs`
- `tests/topology_contract.rs`
- `tests/windows_source_static_contract.rs`

`Cargo.toml` and `Cargo.lock` were unchanged.

## Residual risks and unverified status

- The brief does not require a Windows target or cross-compile command, so none
  was attempted and no target was installed. Windows runtime behavior remains
  unverified; Windows-specific static contract coverage passed inside the full
  all-target suite on macOS.
- No independent reviewer subagent was used because the task explicitly
  prohibited subagents.
- The formatter correction is intentionally broad (41 formatter-only files)
  because the required pinned formatter rejected the complete inherited slice.
  Formatting, warnings-denied Clippy, locked tests/build, and offline tests all
  passed after normalization.
- Source scans found no intended production networking APIs and no launches of
  Python, Node, browsers, daemons, shells, or network clients. Test binaries
  were not network-sandboxed or independently observed, so runtime network
  non-use is not claimed.

## Acceptance-review fix wave

Acceptance review identified four Important test/evidence defects and one Minor
report defect. The focused code/test correction is commit
`cb93641ff94e4956a875304c2ed193e4a1687e46`
(`test: strengthen phase 0 acceptance evidence`).

### Strict TDD evidence

| Stage | Exact command | Exit | Salient result |
| --- | --- | ---: | --- |
| RED | `cargo test --test phase0_acceptance --locked` | 101 | Eight compile errors identified the wished-for persisted-event, table-absence, projection-removal, event-count, and independent projection-oracle APIs. |
| Harness correction | `cargo test --test phase0_acceptance --locked` | 101 | One test-only compile error showed that `ImmediateTransaction::transaction()` is crate-private; the fixture switched to a direct SQLite transaction on the injected test path. |
| GREEN | `cargo test --test phase0_acceptance --locked` | 0 | 1 passed, 0 failed. |

The strengthened scenario now:

- asserts the complete literal first-run rendering for `/help`, `/status`,
  `/setup status`, an unknown command, and `/quit`;
- captures the exact ordered first-run event IDs, kinds, payloads, and
  independently derived audit summaries;
- deletes only recoverable projection rows, then restarts through the real
  `ApplicationService::bootstrap` path;
- requires exactly one `ProjectionRebuilt` event through the first-run
  sequence, preserved installation identity, and the original event records as
  an unchanged ordered prefix;
- runs `/audit tail 100` without `/status` and proves each first-run sequence,
  kind, and hand-derived summary remains rendered in order;
- checks projection metadata, installation projection, and every session
  projection directly against authoritative SQLite event rows and payloads,
  without calling the production reducer for the expected result; and
- requires zero rows in `setup_drafts`,
  `installation_configuration_versions`,
  `active_installation_configuration`, `setup_step_outcomes`,
  `capability_readiness`, and `approval_records`, while also requiring that
  `credentials` and `broker_accounts` tables do not exist.

### Final post-review gate matrix

| Gate | Exact command | Exit | Exact outcome |
| --- | --- | ---: | --- |
| Acceptance | `cargo test --test phase0_acceptance --locked` | 0 | 1 passed, 0 failed. |
| Recovery | `cargo test --test recovery_contract --locked` | 0 | 22 passed, 0 failed. |
| Fallback | `cargo test --test fallback_contract --locked` | 0 | 18 passed, 0 failed. |
| Formatting | `cargo fmt --all --check` | 0 | No diff. |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | No emitted warnings or errors. |
| Full locked all-target suite | `cargo test --workspace --all-targets --locked` | 0 | 212 passed, 0 failed across 20 target runners. |
| Locked build | `cargo build --workspace --locked` | 0 | Build completed. |
| Offline full suite | `cargo test --workspace --all-targets --locked --offline` | 0 | 212 passed, 0 failed across 20 target runners. |
| Binary smoke | `cargo test --test fallback_contract binary_smoke_quit_and_eof_exit_successfully --locked` | 0 | 1 passed, 0 failed; 17 filtered out. |
| Patch hygiene | `git diff --check` | 0 | No whitespace errors. |

### Phase 0 exit-condition traceability

Every exact test below ran as part of
`cargo test --workspace --all-targets --locked`; focused commands show how to
rerun the criterion directly.

| # | Phase 0 exit condition | Exact test/command evidence |
| ---: | --- | --- |
| 1 | Canonical README and superseded legacy documents | `cargo test --test documentation_contract --locked` (3/3), including `readme_is_the_canonical_phase_zero_rust_guide` and `every_legacy_document_starts_with_a_superseded_warning`. |
| 2 | Fresh install plus complete migration inventory | `cargo test --test migration_contract fresh_database_has_the_complete_phase_zero_schema --locked`; `cargo test --test migration_contract migration_records_and_complete_schema_are_exact --locked`; full `migration_contract` result 17/17. The inventory covers event/receipt/reference tables and triggers, projections, all setup/readiness/approval tables, indexes, migration records, pragmas, constraints, and immutability. |
| 3 | Restart preserves installation identity and event history | `cargo test --test phase0_acceptance --locked` (1/1) now proves the original event-ID prefix and audit summaries survive a restart without relying on `/status` output. |
| 4 | Incompatible/corrupt startup paths fail without recreation | `cargo test --test migration_contract corrupt_database_is_rejected_without_recreation --locked`; `cargo test --test migration_contract foreign_application_database_is_rejected_without_recreation --locked`; `cargo test --test migration_contract newer_schema_is_rejected --locked`; `cargo test --test migration_contract migration_checksum_mismatch_is_rejected_as_invalid_migration_state --locked`; `cargo test --test recovery_contract corrupt_event_stream_refuses_bootstrap_without_appending_recovery_events --locked`; `cargo test --test recovery_contract malformed_event_refuses_bootstrap_without_mutation --locked`. |
| 5 | Typed presentation to application to repository flow | `cargo test --test application_contract every_command_uses_its_exact_capability_event_and_typed_view --locked`; `cargo test --test fallback_contract scripted_fallback_session_renders_required_commands_and_quits --locked`; strengthened acceptance 1/1. |
| 6 | Events rebuild identical projections | Strengthened acceptance forces missing projection recovery and independently checks rebuilt rows; `cargo test --test recovery_contract missing_projections_rebuild_from_the_verified_event_stream --locked`; `cargo test --test projection_contract rebuild_writes_the_same_projection_without_changing_event_authority --locked`. |
| 7 | `/setup status` applies no configuration or external setup state | Strengthened acceptance checks six exact zero-row tables plus absent credential/broker tables; `cargo test --test application_contract setup_status_does_not_invent_configuration --locked`. |
| 8 | Unknown/malformed input does not panic or expose raw input | Strengthened acceptance covers an unknown command and persisted redacted summary; `cargo test --test command_contract --locked` (10/10); `cargo test --test fallback_contract --locked` (18/18). |
| 9 | Formatting, lint, tests, and build pass | `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test --workspace --all-targets --locked`; `cargo build --workspace --locked`, all exit 0. |
| 10 | Offline Rust-only execution with no default external dependency | `cargo test --workspace --all-targets --locked --offline` (212/212); offline 78-package inventory command below; filename-bearing source/API and lockfile scans below. |

### Improved filename-bearing safety evidence

Raw `rg` exit 1 means no match. Regex scans are indicators with documented
scope and exceptions; they do not prove the absence of every possible secret,
path, process launch, or network operation.

| Evidence | Exact command | Exit | Scoped result |
| --- | --- | ---: | --- |
| Common credential forms | `rg -n --with-filename --no-ignore --hidden --glob '!.git' --glob '!.git/**' --glob '!target/**' -e '-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----' -e '\b(?:AKIA|ASIA)[0-9A-Z]{16}\b' -e '\bgithub_pat_[A-Za-z0-9_]{20,}\b' -e '\bgh[pousr]_[A-Za-z0-9_]{20,}\b' -e '\bsk-proj-[A-Za-z0-9_-]{16,}\b' -e '\bsk-(?:live|test)_[A-Za-z0-9]{16,}\b' -e '\bxox[baprs]-[A-Za-z0-9-]{16,}\b' -e '\bAIza[0-9A-Za-z_-]{30,}\b' .` | 1 | 0 matches across non-`.git`, non-`target` files, including ignored evidence directories. Covers PEM private keys, AWS AKIA/ASIA, GitHub classic/fine-grained tokens, `sk-proj-`, Stripe-style, Slack, and Google API-key forms. |
| Local absolute path forms | `rg -n --with-filename --no-ignore --hidden --glob '!.git' --glob '!.git/**' --glob '!target/**' -e '/Users/' -e '/home/' -e '/tmp/' -e '/private/tmp/' -e '/var/folders/' -e '[A-Za-z]:[\\/]+Users[\\/]' -e '\\\\[A-Za-z0-9._-]+\\[A-Za-z0-9.$_-]+' .` | 0 | 36 filename-bearing matches: 6 approved plan install commands, 2 negative documentation assertions, and 28 historical `.superpowers` briefs/reports/diffs/progress records. No `src`, migration, manifest, or lockfile match; no `/var/folders`, drive-user, or UNC match. These are documented fixtures/history, not production path leakage. |
| Present compiled-target inventory | `for scan_path in build.rs examples benches; do if test -e "$scan_path"; then printf 'present: %s\n' "$scan_path"; else printf 'absent: %s\n' "$scan_path"; fi; done` | 0 | `build.rs`, `examples`, and `benches` were absent. Therefore `src` and `tests` are the complete present compiled project-target roots. |
| Networking/process APIs in all present compiled targets | `rg -n --with-filename -e '\bstd::net\b' -e '\bToSocketAddrs\b' -e '\b(TcpStream|TcpListener|UdpSocket)\b' -e '\bstd::process\b' -e '\bCommand::new\s*\(' -e '\b(libc::(socket|connect|bind|listen|accept|getaddrinfo|send|recv)|socket2::|reqwest::|hyper::|ureq::|curl::)' src tests` | 0 | 6 filename-bearing matches: 4 process launches and 2 child-probe exits, classified below. There were 0 networking API matches. |
| Forbidden literal process-launch arguments | `rg -n --with-filename -i -U '(?:std::process::)?Command::new\s*\(\s*"(?:python3?|node|npm|npx|deno|bun|bash|sh|zsh|fish|cmd(?:\.exe)?|powershell|pwsh|osascript|open|xdg-open|google-chrome|chrome|chromium|firefox|safari|curl|wget|ssh|nc|netcat|socat|telnet|docker|podman|systemctl|launchctl|daemon)"\s*\)' src tests` | 1 | 0 literal launch-argument matches. Dynamic launch arguments were separately classified from their source expressions below. |
| Forbidden executable-name source indicators | `rg -n --with-filename -i '\b(python3?|node|npm|npx|deno|bun|bash|zsh|fish|powershell|pwsh|osascript|xdg-open|google-chrome|chrome|chromium|firefox|safari|curl|wget|ssh|netcat|socat|telnet|docker|podman|systemctl|launchctl|daemon)\b' src tests` | 0 | 2 matches, both in `tests/documentation_contract.rs`: negative README assertions rejecting `npm run` and `podman`. Neither is a launch site. |
| Debug/TODO/mock markers | `rg -n --with-filename -i -e '\b(dbg!|todo!|unimplemented!)' -e '\b(TODO|FIXME|HACK|XXX)\b' -e '\b(mock|fake|stub)(ed|s|ing)?\b' src` | 1 | 0 production source matches. |
| Known networking/runtime dependencies | `rg -n --with-filename '^name = "(reqwest|hyper|hyper-util|ureq|curl|isahc|surf|tonic|axum|actix-web|rocket|warp|tokio|async-std|smol|socket2|mio|rustls|native-tls|openssl|oauth2|aws-config|aws-sdk-[^"]+)"$' Cargo.lock` | 1 | 0 matches. This named-family scan is supplemented by the complete Cargo inventory, not treated as exhaustive alone. |
| Existing allowances | `rg -n --with-filename '#!?\[allow' src tests` | 0 | 3 existing lines: test-only shared-support `dead_code`, plus two function-local `clippy::too_many_arguments` constructor allowances. |
| New allowances across committed Task 14 implementation | `git diff -U0 2f432ad925850a41ecebbdfd8386008a7db84c70..cb93641ff94e4956a875304c2ed193e4a1687e46 -- src tests \| rg -n '^\+.*#.*\[allow'` | 1 | 0 added allowance lines in the explicit base-to-implementation range. Exit 1 is ripgrep's no-match result. |

Process API classification:

- `src/panic_boundary.rs:66` launches `std::env::current_exe()` with an exact
  Rust test name and `--nocapture`; it re-executes the current test harness for
  panic-redaction validation. Its child branch calls `std::process::exit(0)` at
  line 63 after writing the expected safe line.
- `tests/fallback_contract.rs:482` likewise launches
  `std::env::current_exe()` with an exact Rust test name and `--nocapture` for
  panic-redaction validation. Its child branch calls `std::process::exit(0)` at
  line 479.
- `tests/fallback_contract.rs:618` and
  `tests/fallback_fix_round_contract.rs:569` launch only
  `env!("CARGO_BIN_EXE_ai-stock-forum")`, Cargo's path to this project's built
  Rust binary, with isolated `HOME` and `XDG_DATA_HOME` test environments.
- None of the four launch arguments names or constructs Python, Node, a
  browser, daemon, shell, network client, or another external executable.
  This is source-level evidence over every currently present compiled target,
  not an OS sandbox or a proof against executable names synthesized at runtime.

Allowance scope and rationale:

- `tests/support/mod.rs:1` has the existing module-level `dead_code` allowance.
  The same support module is compiled independently into many integration-test
  crates, each of which intentionally consumes only a subset. It affects test
  support only, not production code.
- `src/setup/models.rs:63` and `src/setup/models.rs:137` have existing
  function-local `clippy::too_many_arguments` allowances on exact schema-model
  constructors whose required fields mirror persisted contracts. These are
  already narrowly scoped to two functions.
- No new suppression was added. Clippy with `-D warnings` emits no unsuppressed
  warning, but this report does not claim that the three existing allowances
  are absent or that suppressed lint classes were independently zero.

### Offline complete dependency inventory

`cargo tree --workspace --all-features --target all --locked --offline` exited
0 and printed the complete target-inclusive dependency tree. The normalized
inventory command
`cargo tree --workspace --all-features --target all --locked --offline --prefix none | awk '$2 ~ /^v/ {print $1, $2}' | sort -u`
also exited 0 and produced 78 distinct package/version entries:

```text
ai-stock-forum v0.1.0, bitflags v2.13.1, block-buffer v0.12.1,
block2 v0.6.2, bumpalo v3.20.3, cc v1.4.4, cfg-if v1.0.4,
cfg_aliases v0.2.2, const-oid v0.10.2, cpufeatures v0.3.1,
crossbeam-channel v0.5.16, crossbeam-utils v0.8.22,
crypto-common v0.2.2, ctrlc v3.5.2, digest v0.11.3,
directories v6.0.0, dirs-sys v0.5.0, dispatch2 v0.3.1,
errno v0.3.14, fallible-iterator v0.3.0,
fallible-streaming-iterator v0.1.9, fastrand v2.5.0,
find-msvc-tools v0.1.11, foldhash v0.2.0, getrandom v0.2.17,
getrandom v0.4.3, hashbrown v0.16.1, hashbrown v0.17.1,
hashlink v0.12.1, hex v0.4.3, hybrid-array v0.4.14, itoa v1.0.18,
js-sys v0.3.104, libc v0.2.189, libredox v0.1.21,
libsqlite3-sys v0.38.2, linux-raw-sys v0.12.1, memchr v2.8.3,
nix v0.31.3, objc2 v0.6.4, objc2-encode v4.1.0,
once_cell v1.21.4, option-ext v0.2.0, pkg-config v0.3.34,
proc-macro2 v1.0.107, quote v1.0.47, r-efi v6.0.0,
redox_users v0.5.2, rsqlite-vfs v0.1.1, rusqlite v0.40.2,
rustix v1.1.4, rustversion v1.0.23, serde v1.0.229,
serde_core v1.0.229, serde_derive v1.0.229, serde_json v1.0.151,
sha2 v0.11.0, shlex v2.0.1, smallvec v1.15.2,
sqlite-wasm-rs v0.5.5, syn v2.0.119, syn v3.0.4,
tempfile v3.27.0, thiserror v2.0.20, thiserror-impl v2.0.20,
typenum v1.20.1, unicode-ident v1.0.24, uuid v1.26.0,
vcpkg v0.2.15, wait-timeout v0.2.1,
wasi v0.11.1+wasi-snapshot-preview1, wasm-bindgen v0.2.127,
wasm-bindgen-macro v0.2.127, wasm-bindgen-macro-support v0.2.127,
wasm-bindgen-shared v0.2.127, windows-link v0.2.1,
windows-sys v0.61.2, zmij v1.0.23
```

Cargo `--offline` constrains Cargo's registry/index/package acquisition. It does
not sandbox test binaries or prevent runtime socket calls. Runtime-network
confidence therefore comes from the tested Phase 0 behavior, filename-bearing
source/API scan, manifest/lock inventory, and architectural scope together;
none is represented as a formal proof against every dynamically constructed or
FFI-mediated external action.

### Follow-up changed files and residual limits

- `tests/phase0_acceptance.rs`: exact first rendering, persisted ID/order and
  independent audit-summary assertions, forced projection recovery, expanded
  forbidden-state checks.
- `tests/support/mod.rs`: raw persisted-event snapshots, table absence/count
  helpers, recoverable projection deletion, and independent SQL
  projection-to-event parity oracle.
- Code/test commit stats: 2 files changed, 339 insertions, 25 deletions.
- Windows cross-compilation was not added or attempted because Task 14 still
  specifies no Windows target command. Existing Windows static/unit coverage is
  included in the 212-test all-target suite; runtime Windows behavior remains
  unverified.
- The final report commit follows tested implementation head
  `cb93641ff94e4956a875304c2ed193e4a1687e46`; its exact hash is returned in the
  handoff because a commit cannot embed its own identifier.

## Whole-branch approval status persistence review fix

The final whole-branch approval status correction is implementation commit
`39ed43941e0e3cea0e1d25b264666472a89e3e0d` (`fix: align approval status persistence contract`).
The 212-test matrices above remain exact historical evidence for
`cb93641ff94e4956a875304c2ed193e4a1687e46`; the current branch aggregate is
superseded by the evidence below.

### Strict TDD evidence

| Stage | Exact command | Exit | Exact outcome |
| --- | --- | ---: | --- |
| RED | `cargo test --test policy_contract --test migration_contract --locked` | 101 | 13 `E0599` diagnostics proved that the canonical `Accepted` and `Cancelled` variants were absent. |
| GREEN | `cargo test --test policy_contract --test migration_contract --locked` | 0 | 37 passed, 0 failed: migration 19/19 and policy 18/18. |

### Current verification

| Gate | Exact command | Exit | Exact outcome |
| --- | --- | ---: | --- |
| Formatting | `cargo fmt --all --check` | 0 | No diff after applying the pinned formatter. |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | Finished with no warnings. |
| Full locked all-target suite | `cargo test --workspace --all-targets --locked` | 0 | 215 passed, 0 failed across 20 target runners. |

No offline suite was rerun for this review fix, and this section makes no
runtime-network non-use claim. The earlier offline results and their explicit
limitations remain historical evidence for the earlier tested implementation.
