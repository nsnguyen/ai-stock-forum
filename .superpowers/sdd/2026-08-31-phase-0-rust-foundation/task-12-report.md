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

## Fix round 1

Implementation commit: `6f42574e67c599d579a3b3aa92256cbceb45bb4a`

### RED, GREEN, and full-suite evidence

Focused RED command: `cargo test --test fallback_fix_round_contract --locked`

After the approved download of the locked `wait-timeout` dev dependency, compilation failed with `E0432` because `BufferedLineSource`, `LineSource`, `LineSourceCancellation`, and `LineSourceEvent` did not exist. This was the expected missing-contract RED failure for the reviewed source-ownership design.

Focused GREEN command: `cargo test --test fallback_contract --test fallback_fix_round_contract --locked`

Result: 28 passed, 0 failed. The original 17 Task 12 tests and all 11 fix-round regressions passed together.

Full-suite command: `cargo test --locked`

Result: 193 passed, 0 failed. The output contained no compiler warnings.

### Exact reader design

The reader now treats physical-line framing and logical-line metadata as one incremental operation. `LineAccumulator` holds a SHA-256 state, an exact checked byte count, one pending-CR bit, and at most `MAX_INPUT_BYTES + 1` retained bytes. A CR is delayed until the next physical byte is known: it is discarded only when that byte is the actual LF delimiter, and otherwise it is committed to the retained prefix, exact count, and digest. LF is never included. A pending CR at EOF is committed. The finalized `RawLine` therefore carries the exact normalized full byte length and full-line digest even when only 4097 bytes are retained.

`FallbackRunner` checks `RawLine::was_oversized` before calling `parse_line`. Oversized input bypasses UTF-8 parsing entirely and becomes `ApplicationCommand::RejectInput` with category `Oversized`, no safe token, and the authoritative full length and digest. Tests independently fix the 32 KiB digest to `2d864c0b789a43214eee8524d3182075125e5ca2cd527f3582ec87ffd94076bc` and cover the adversarial `4096 * x + CR + X + LF` boundary.

### Exact line-source and host design

`LineSource` owns blocking input behavior and supplies a separate `LineSourceCancellation` handle. `FallbackHost` obtains that handle before moving the source into its named input thread. Every body result, including quit, SIGINT, EOF, read error, write error, runtime failure, or caught host panic, triggers cancellation and a synchronous thread join before the host calls `finish_and_join` and returns. A failed source-thread spawn synchronously finishes the application runtime with `ApplicationError`. No host-created thread survives `run`.

The Unix production source creates a nonblocking Unix socket-pair cancellation event and polls both `STDIN_FILENO` and the cancellation descriptor. It reads only after stdin readiness, incrementally feeds the bounded line accumulator, wakes immediately when cancellation is signaled, and owns no application or database work. Injected host tests use explicit cancellable sources; bounded buffered sources remain available for finite cursor/error fixtures. On non-Unix targets, stdio initialization returns the typed safe `LineSourceUnavailable` error before bootstrap.

The interrupt select arm is disabled by replacing a disconnected receiver with `crossbeam_channel::never`. This happens inside the current prompt wait, so disconnection cannot produce repeated prompts or a livelock.

### Exact composition and startup cleanup design

`main` now calls `StdioResources::initialize` before `AppPaths::discover` or `ApplicationService::bootstrap`. Ctrl-C installation and Unix cancellation-resource creation therefore complete before any durable process session is opened.

`ApplicationRuntime::spawn_application` creates the service worker synchronously and wraps service ownership in `ServiceWorker`. Worker initialization failure explicitly attempts `service.finish(ApplicationError)`. Invalid capacity, thread-spawn failure, or another failure that drops the unstarted wrapper invokes its panic-protected `Drop` cleanup synchronously before the startup call returns. Regression tests verify an `application_error` terminal row and a warning-free next launch after an injected thread-spawn failure.

### Exact command-panic cleanup design

When `execute_user` panics, `execute_request` catches the panic and separately wraps `executor.finish(ApplicationError)` in a second `catch_unwind`. Only after that best-effort finish does it publish `WorkerPanicked`, notify the pending outcome, and resume the original panic for worker join classification. `ServiceWorker` records successful finish so its `Drop` fallback cannot duplicate the terminal write. A real `ApplicationService` test with a panicking policy verifies the persisted `application_error` reason and a warning-free next launch.

### Rendering and binary lifecycle evidence

Audit lines now render sequence, timestamp, actor, bounded control-escaped event kind, correlation ID, and bounded control-escaped typed summary. The contract covers all eleven `ApplicationEvent` summary mappings and proves that event JSON, payload names, digests, database paths, rejected full input, and a credential-like secret are absent.

Only recoverable backpressure is rendered inside `FallbackRunner`. Terminal runtime errors propagate without stdout output, allowing `main` to emit one typed safe stderr line.

The Unix SIGINT binary test starts the real executable with stdin held open, waits for the durable open-session condition, sends `SIGINT`, and uses `wait-timeout` for a bounded process wait. It verifies successful exit, exactly one safe interrupt line, an `interrupted` persisted end reason, empty stderr, and no previous-session warning on the next launch. Binary quit and EOF tests now verify persisted `user_quit` and `input_closed` reasons respectively.

## Fix round 2

### Implementation commit

- `53a5ae1db43663c22c5b0d4a1f05b0104d259184` (`fix(task-12): harden panic and input boundaries`)

### Changed files

- `src/lib.rs`
- `src/panic_boundary.rs`
- `src/runtime/mod.rs`
- `src/ui/command/mod.rs`
- `src/ui/command/runner.rs`
- `tests/fallback_contract.rs`
- `tests/fallback_fix_round_contract.rs`
- `tests/fallback_fix_round_2_contract.rs`

### RED evidence

- `cargo test --locked --test fallback_fix_round_2_contract`
  - Failed because `CancellableLineSource` and `UnixLineSource` did not exist and the old host contract still exposed `LineSource`.
- `cargo test --locked --lib panic_boundary::tests::caught_sensitive_panic_subprocess_emits_only_one_safe_line`
  - Failed because `catch_sensitive_unwind` did not exist.

### Focused GREEN evidence

- `cargo test --locked --lib panic_boundary::tests::caught_sensitive_panic_subprocess_emits_only_one_safe_line`
  - 1 passed.
- `cargo test --locked --test fallback_fix_round_2_contract`
  - 3 passed: blocked Unix read cancellation/join, `POLLNVAL` typed termination, and pipe HUP EOF without spin.
- `cargo test --locked --test fallback_contract caught_host_writer_panic_subprocess_redacts_payload_and_emits_one_safe_line`
  - 1 passed; child stderr was exactly `Command host stopped unexpectedly.\n` and excluded the secret panic payload.
- `cargo test --locked --test fallback_fix_round_contract binary_quit_and_eof_persist_exact_shutdown_reasons`
  - 1 passed; both persisted terminal reasons and both clean relaunches were verified.

### Full-suite evidence

- `cargo test --locked`
  - 198 passed, 0 failed across unit, integration, binary, and documentation targets.
- `git diff --check`
  - Passed before the implementation commit.

### Panic-hook and catch-boundary design

- `panic_boundary` installs one process-wide hook with `Once`, retains the prior hook, and delegates to it for every panic outside a sensitive application catch boundary.
- A thread-local depth counter and RAII guard suppress hook diagnostics only while `catch_sensitive_unwind` is active. Nested boundaries remain safe and guard drop restores normal delegation even during unwinding.
- Runtime worker initialization, command execution, normal finish, panic cleanup finish, and `ServiceWorker` drop cleanup use the sensitive boundary. A command panic performs a separately protected best-effort `ApplicationError` finish, publishes `WorkerPanicked`, and then resumes the already-caught unwind for deterministic join behavior.
- The host wraps its runner/writer loop and source-thread body in sensitive boundaries. A caught host panic becomes `UiError::Panicked`; the composition root remains the sole renderer of the stable safe stderr line.
- The subprocess regression injects `credential=host-writer-secret-payload` into a real host writer panic and proves no hook diagnostic or payload escapes.

### Source ownership and platform design

- The host accepts only an owned `CancellableLineSource`. The former generic `BufferedLineSource<BufRead>` adapter was removed; bounded `BufferedLineReader` tests remain pure and host tests now use finite scripted or channel-woken cancellable sources.
- The host creates one owned source thread, retains its cancellation handle, signals cancellation on every host exit path, and joins the source thread before finishing the runtime or returning. A source panic is hook-redacted, preserved as a failed join, and mapped to `ReaderThread`.
- Unix `UnixLineSource` incrementally feeds the existing bounded `LineAccumulator`, polls the input fd plus a cancellation socket pair, and treats cancellation readiness as `Cancelled`. Input `POLLNVAL` and `POLLERR` are typed I/O errors; `POLLHUP` drains pending bytes and then yields EOF. Invalid-fd, closed-pipe, and blocked-cancellation tests all use two-second bounded handshakes.
- Windows uses a manual-reset kernel event and `WaitForMultipleObjects` over stdin and the cancellation event. `SetEvent` wakes an in-progress wait, `ReadFile` feeds the same bounded accumulator, and the event handle is closed by its shared cancellation owner. All Windows declarations and code are isolated behind `cfg(windows)` and require no target-only dependency.
- Other non-Unix/non-Windows targets retain the explicit safe `LineSourceUnavailable` result. Quit, EOF, startup-failure, previous-warning, and clean-relaunch binary coverage remains outside Unix-only gating; only descriptor/SIGINT mechanics are Unix-gated.

### Lifecycle and persistence evidence

- Interrupt resources and the platform line source are still initialized before `AppPaths`, database bootstrap, and durable session creation.
- Quit and EOF binary tests now relaunch against the same state and prove no transient previous-session warning, in addition to asserting `user_quit` and `input_closed`. The existing bounded Unix SIGINT test continues to assert one safe interrupt line, persisted `interrupted`, and a warning-free next launch.
- Full-line byte count/digest authority, oversized-before-parse behavior, audit rendering, recoverable-only backpressure rendering, runtime ordering, and synchronous startup/panic cleanup tests remain GREEN.

### Concerns and limitations

- The installed Rust toolchain does not include `x86_64-pc-windows-gnu`, so a Windows cross-target compile could not be executed (`WINDOWS_TARGET_NOT_INSTALLED`). The Windows implementation is target-isolated, dependency-free kernel32 FFI; Unix and portable behavior are covered by the full suite.
- No GUI/TUI, network, broker, credential, README, or legacy-warning behavior was added.

## Fix round 3 (final)

### Implementation commit

- `eb8161c2883b32f0709061950dcd89877acc8760` (`fix(task-12): cancel Windows stdin reads safely`)

### Changed files

- `Cargo.toml`
- `Cargo.lock`
- `src/ui/command/mod.rs`
- `src/ui/command/runner.rs`
- `src/ui/command/windows.rs`
- `tests/fallback_contract.rs`
- `tests/windows_source_static_contract.rs`

### RED evidence

- `cargo test --locked --test windows_source_static_contract`
  - 4 failed as expected: no `OpenThread`/`CancelSynchronousIo` thread-HANDLE path, no Windows read-error classifier, no target-scoped `windows-sys` declaration, and POSIX state binary tests lacked Unix gating.

### Focused GREEN evidence

- `cargo test --locked --lib ui::command::windows::tests`
  - 3 passed: cancellation-sensitive `ERROR_OPERATION_ABORTED`, EOF classification for `ERROR_BROKEN_PIPE` and `ERROR_HANDLE_EOF`, and genuine-error preservation.
- `cargo test --locked --test windows_source_static_contract`
  - 4 passed: synchronous-read cancellation API/HANDLE ownership, explicit error dispositions, target-scoped cfg/features, and Unix-only HOME/XDG binary tests.

### Full-suite evidence

- `cargo test --locked`
  - 205 passed, 0 failed across unit, integration, binary, static-contract, and documentation targets.
- `git diff --check`
  - Passed before the implementation commit.

### Windows synchronous-read cancellation design

- `WindowsCancellation` owns the manual-reset cancellation event, an atomic cancellation flag, and a mutex-protected optional real source-thread HANDLE.
- On its first `next_line`, the source opens its current thread with `OpenThread(THREAD_TERMINATE, false, GetCurrentThreadId())`. The mutex serializes registration with cancellation.
- Cancellation first stores the flag, signals the event to wake `WaitForMultipleObjects`, then invokes `CancelSynchronousIo` when the reader HANDLE is registered. This wakes a console line-mode `ReadFile` that became blocking after the input HANDLE was signaled by partial input.
- The register/cancel ordering is race-safe: cancel-before-register leaves the flag set and registration immediately invokes `CancelSynchronousIo`; register-before-cancel publishes the HANDLE before cancellation acquires the mutex and invokes it.
- `ERROR_OPERATION_ABORTED` maps to `LineSourceEvent::Cancelled` only when cancellation was requested. An unrelated operation-aborted result remains a genuine typed read error.
- The source thread drops its `Arc` before join completes, while the host retains the cancellation `Arc` through cancellation and join. The final `WindowsCancellation::drop` therefore runs after join and closes the optional reader thread HANDLE and event HANDLE exactly once. The borrowed standard-input HANDLE is not closed.
- Target-scoped `windows-sys 0.61.2` features declare the Foundation, FileSystem, Console, IO, and Threading API surface only for `cfg(windows)`.

### Windows EOF/error mapping

- A zero-byte successful `ReadFile`, `ERROR_BROKEN_PIPE`, and `ERROR_HANDLE_EOF` all finish the bounded `LineAccumulator`, preserving any final partial logical line and then producing EOF/input-closed.
- `ERROR_OPERATION_ABORTED` during requested cancellation produces clean source cancellation.
- Every other `ReadFile` failure preserves the original `io::Error`, which the host maps to the existing typed input-read failure.
- The shared `LineAccumulator` remains authoritative for bounded prefix retention, full-line byte count/digest, CR/LF handling, and EOF partial-line behavior.

### Binary test gating

- Binary smoke, startup-redaction, previous-session-warning, quit/EOF persistence, and SIGINT persistence tests that isolate state through HOME/XDG are Unix-gated.
- No Windows binary test can touch a real Known Folder state directory. Windows coverage is provided by host-independent executable error-mapping tests plus target-specific static API/cfg/dependency contracts until an actual Windows target check can be added later.

### Concerns and limitations

- Per the round-3 instruction, no Windows cross-compile was attempted and none is claimed. A Windows target compile/runtime check remains appropriate for Task 14 when a Windows toolchain is available.
- Microsoft documents that `CancelSynchronousIo` requires a thread HANDLE with `THREAD_TERMINATE` and reports cancelled operations with `ERROR_OPERATION_ABORTED`; the implementation follows that contract.
- No parser, renderer, runtime, composition-root, database, README, legacy-warning, GUI/TUI, network, broker, or credential behavior changed.
