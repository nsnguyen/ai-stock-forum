# Task 14 Acceptance Report

## Result

Phase 0 Task 14 acceptance is complete on branch
`codex/phase-0-rust-foundation`. The tested implementation head is
`174ebefcaff23e187dcff5c2f1789348318e82e0`, based on
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

No lint was allowed or suppressed. These were non-behavioral gate failures, so
the exact formatting and Clippy commands are the focused regression gates; no
gratuitous behavior test was added.

## Final acceptance and quality matrix

All commands below were run on the same post-fix source tree represented by
implementation commit `174ebefcaff23e187dcff5c2f1789348318e82e0`.

| Gate | Exact command | Exit | Salient output |
| --- | --- | ---: | --- |
| Phase 0 acceptance | `cargo test --test phase0_acceptance --locked` | 0 | 1 passed, 0 failed. |
| Recovery contract | `cargo test --test recovery_contract --locked` | 0 | 22 passed, 0 failed. |
| Fallback contract | `cargo test --test fallback_contract --locked` | 0 | 18 passed, 0 failed. |
| Formatting | `cargo fmt --all --check` | 0 | No diff. |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | Finished with no warnings. |
| Full locked all-target suite | `cargo test --workspace --all-targets --locked` | 0 | 212 passed, 0 failed across 20 target runners. |
| Locked build | `cargo build --workspace --locked` | 0 | Dev build finished successfully. |
| Offline full suite | `cargo test --workspace --all-targets --locked --offline` | 0 | 212 passed, 0 failed across 20 target runners without network access. |
| Explicit binary smoke | `cargo test --test fallback_contract binary_smoke_quit_and_eof_exit_successfully --locked` | 0 | 1 passed, 0 failed; real binary quit and EOF subprocess paths succeeded. |

The full fallback suite additionally passed real-binary startup-failure
redaction and previous-session warning scenarios.

## Repository and source safety scans

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

No raw credential signature, production absolute local path, debug dump,
production TODO stub, mock adapter, network/broker action, or external-action
dependency violates the Phase 0 boundary.

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
- No network, provider, broker, paid service, Python, Node, browser, daemon,
  external account, or external-action dependency was exercised or required.
