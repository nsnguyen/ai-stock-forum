# Task 12 Implementation Report

## Commit

Implementation commit: `cce5ee22168409d92d749a6203dfc3d9d792309f`

## Changed files

- `Cargo.toml`
- `Cargo.lock`
- `src/app/service.rs`
- `src/main.rs`
- `src/ui/command/mod.rs`
- `src/ui/command/reader.rs`
- `src/ui/command/renderer.rs`
- `src/ui/command/runner.rs`
- `tests/fallback_contract.rs`
- `tests/support/mod.rs`

README and legacy-warning documentation were not changed.

## TDD evidence

RED command: `cargo test --test fallback_contract --locked`

The first sandboxed invocation could not download the newly locked `ctrlc` transitive crates. After the approved network retry, compilation failed with `E0432` because `BoundedLineReader`, `FallbackHost`, `FallbackRunner`, `TextRenderer`, and `UiError` did not exist. This was the expected feature-missing RED failure.

Focused GREEN command: `cargo test --test fallback_contract --locked`

Result: 17 passed, 0 failed. The focused contract covers bounded oversized input, oversized invalid UTF-8, CRLF, EOF without newline, reader errors, exhaustive safe rendering, rejected-input and digest redaction, backpressure, worker panic/disconnect behavior, input/write/panic shutdown, quit/EOF/interrupt shutdown, startup redaction, one-time previous-session warning, and binary quit/EOF/startup smoke cases.

Full-suite command: `cargo test --locked`

Result: 182 passed, 0 failed. This includes the 17 Task 12 tests and all 165 tests present at the clean base.

## Reader and renderer design

`BoundedLineReader` retains at most `MAX_INPUT_BYTES + 1` bytes. It scans buffered input for LF, consumes and discards the remainder of an oversized physical line without allocating for that remainder, strips LF and an optional preceding CR, and returns partial EOF lines. Size classification therefore occurs before `parse_line` attempts UTF-8 decoding.

`TextRenderer` exhaustively matches all six `CommandView` variants, all setup states, input rejection categories, shutdown dispositions and reasons, runtime errors, application errors, startup errors, and UI host errors. It omits command IDs, correlation IDs, installation/session/setup IDs, digests, event JSON, database details, policy decisions, and raw rejected input. Audit output renders bounded escaped event kinds and stable structural metadata, not summaries or payloads.

## Host and shutdown design

`FallbackRunner` processes one complete bounded line at a time. It parses bytes, uses `RuntimeClient::try_submit`, waits for that command's own pending result before reading the next command, renders the typed outcome, and treats backpressure as a safe recoverable response. Terminal runtime failures are rendered safely and returned as typed `UiError` values.

`FallbackHost` owns `ApplicationRuntime`. A capacity-one input channel connects a dedicated blocking reader thread to a `select_biased!` loop whose first branch is the bounded interrupt channel. The host wraps its body in `catch_unwind`, maps EOF, quit, interrupt, input failure, write failure, worker failure, and panic to deterministic shutdown reasons, then calls `finish_and_join` exactly once before returning. Accepted application and database work remains on the application runtime worker, and the runtime worker is joined on every host return path.

The composition root discovers `AppPaths`, bootstraps `ApplicationService` with production clock and IDs, preserves the bootstrap-owned process guard by moving the service into a capacity-32 runtime, emits the transient previous-session warning once inside the host boundary, installs one process-global Ctrl-C handler, and returns `ExitCode::SUCCESS` or `ExitCode::FAILURE` with one safe typed error line.

## Concerns

The dedicated stdin reader may remain blocked in an operating-system read after explicit quit or Ctrl-C. Rust does not provide a portable cancellation mechanism for that read. The thread owns no application service or database work and is bounded to one queued line; process exit releases it after the host has deterministically joined the application runtime worker.

The `ctrlc` dependency adds platform-specific transitive crates to the lockfile. No GUI/TUI, network, broker, credential, or legacy-warning behavior was added.
