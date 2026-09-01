# SDD ledger — plan: docs/superpowers/plans/2026-08-31-phase-0-rust-foundation.md

Branch base: db511e2
Spec: docs/superpowers/specs/2026-08-31-phase-0-rust-foundation-design.md
Worktree: /private/tmp/ai-stock-forum-phase-0-rust-foundation

## Preflight task self-consistency scan

| Task | Files/tests against implementation text | Finding |
|---|---|---|
| 1 | Toolchain, manifests, module tree, topology compile contract | Internally consistent; Rust installation is an external prerequisite the controller must perform. |
| 2 | Domain files and domain contract | Internally consistent; ID newtypes must derive Ord for Task 8's BTreeMap. Plan already says Eq + Ord + Hash. |
| 3 | Commands, outcomes, byte parser, grammar tests | Internally consistent after plan self-review removed raw input and interim event types. |
| 4 | Capability and approval files/tests | Internally consistent; rejected commands deliberately use HelpRead for safe guidance/audit. |
| 5 | Paths and permission tests | Internally consistent; production discovery and injected test roots stay separate. |
| 6 | Migration SQL, database startup, compatibility tests | Internally consistent; schema_migrations is runner-bootstrap state rather than part of migration 0001. |
| 7 | Event types, repository, audit, test support | Internally consistent; Task 7 is the first creator of tests/support. |
| 8 | Setup models, reducer, projection repository/tests | Internally consistent; SetupStatus must implement Default as NotStarted. |
| 9 | Recovery coordinator/hooks and recovery tests | Internally consistent; projection rebuild precedes installation/session bootstrap. |
| 10 | Application service and transactional command tests | Internally consistent; RequestShutdown records intent and finish records actual termination. |
| 11 | Bounded runtime and concurrency tests | Conflict: with a blocked worker and capacity one, the second command fills the queue; only the third observes backpressure. |
| 12 | Reader, renderer, runner, binary/tests | Internally consistent; runner returns a reason and owning host persists it exactly once. |
| 13 | README, four legacy warnings, documentation contract | Rubric tension: test inspects human/agent instruction text, normally a change detector. Roadmap explicitly makes the four warnings an exit-gate safety invariant. |
| 14 | Acceptance test and quality gates | Internally consistent; it consumes all earlier public behavior without adding production APIs. |

## Preflight shared-file and interface scan

| Producer -> consumer | Shared file/interface | Finding |
|---|---|---|
| 1 -> 2 | Cargo manifests, domain module | Task 2 adds only its direct dependencies and concrete domain files. |
| 1 -> 3 | app/ui module roots | Task 3 replaces boundary-only files without changing module names. |
| 1 -> 4 | policy module root | Task 4 replaces the boundary-only file with exact policy modules. |
| 1 -> 5 | config module root | Task 5 preserves the root export while adding paths. |
| 1 -> 6 | persistence module root | Task 6 preserves the root export while adding SQLite. |
| 2 -> 3 | Actor, command/correlation IDs, SHA-256 | Names and ownership agree. |
| 2 -> 4 | ApprovalId, ObjectRef, timestamps | Approval skeleton consumes exact Task 2 primitives. |
| 2 -> 5 | StartupError | Task 5 extends/uses typed safe startup errors. |
| 2 -> 6 | Digest and persistence/startup errors | Migration checksum and startup mapping agree. |
| 3 -> 4 | ApplicationCommand::required_capability | Task 4 adds one method without changing variants. |
| 3 -> 7 | AuditLimit, InputRejection, views | Task 7 event variants consume exact Task 3 types. |
| 3 -> 10 | ApplicationCommand and CommandView | Task 10 composes the final CommandOutcome after EventEnvelope exists. |
| 3 -> 12 | parse_line and ParsedLine | Fallback runner consumes the total byte parser. |
| 4 -> 6 | approval_records schema | SQL fields cover the typed approval identity/status contract. |
| 4 -> 10 | policy evaluator and safe grants | Service grants only five Phase 0 capabilities and future families stay denied. |
| 5 -> 6 | AppPaths and file modes | Database opens only after state-directory validation. |
| 5 -> 10 | AppPaths bootstrap input | Service delegates all storage discovery to config. |
| 6 -> 7 | Database and event_stream | Repository uses caller transactions over migration 0001. |
| 6 -> 8 | projection/setup tables | Projection repository mutates rebuildable tables only. |
| 6 -> 9 | Database startup | Recovery begins only after migration/integrity checks. |
| 7 -> 8 | EventEnvelope and ApplicationEvent | Reducer consumes the final typed event contract. |
| 7 -> 9 | EventRepository verify/load/append | Recovery ordering agrees with repository API. |
| 7 -> 10 | events, audit tail, CommandOutcome | Service appends before rendering/audit reads. |
| 8 -> 9 | ProjectionState and repository | Recovery detects stale metadata and rebuilds via Task 8. |
| 8 -> 10 | projected installation/setup/session state | Service views derive from committed projection state. |
| 9 -> 10 | BootstrapState and finish_session | Service owns the active bootstrapped session lifecycle. |
| 10 -> 11 | ApplicationService as CommandExecutor | Runtime serializes one mutable service instance. |
| 11 -> 12 | RuntimeClient and finish_and_join | Runner submits; owning host finishes and joins once. |
| 12 -> 14 | FallbackRunner and rendered output | Acceptance uses the same headless fallback path. |
| 13 -> 14 | README/legacy exit-gate state | Acceptance gates run after documentation contract. |

## Preflight rulings

Ruling: The controller installs Rust 1.98.0 before Task 1 dispatch — toolchain installation writes outside the worktree and is an immediate shared prerequisite, not delegated production work — if wrong, Task 1 may need to repeat only environment setup.

Ruling: Task 11's backpressure test submits a blocked first command, a queued second command, then expects the third command to return Backpressure — bounded-channel capacity measures queued work, not the command already executing — if wrong, the concurrency test could become timing-sensitive and require redesign.

Ruling: Task 13 retains a narrow contract checking the README and exactly four legacy warning headers — architecture.md/phases.md make warning coverage a safety exit gate and no runnable document consumer exists in Phase 0 — if wrong, this creates maintenance coupling to warning wording and should be replaced by a later documentation-policy harness.

## Task execution

Task 1: dispatched implementer 01a05bb5-4f1d-7573-8019-7a06038e3ec4 at base db511e2.
Task 1: implementer commit b883630; RED unresolved crate, GREEN topology_contract 1/1; reviewer 01a05bb8-f82d-71c3-9deb-eab923de85ba dispatched.
Task 1: reviewer warning resolved — controller observed rustc 1.98.0 installer output and read complete RED/GREEN evidence in task-1-report.md.
Task 1: complete (commits db511e2..b883630, review clean).
Task 2: dispatched implementer 01a05bba-0ee8-7c61-9008-0786a0437fb5 at base b883630.
Task 2: implementer commit 67d3f5b; RED missing domain export, GREEN focused 4/4 and full suite 5/5; reviewer 01a05bbe-8383-7972-83af-76b5e1d70d49 dispatched.
Task 2: fix round 1/5 (1 addressed, 0 open — validating ObjectVersion/ObjectRef deserialization; commits 67d3f5b..07942d1).
Task 2: complete (commits b883630..07942d1, review clean).
Task 3: dispatched implementer 01a05bc4-09f6-7b12-8a7a-6cee67729b1e at base 07942d1.
Task 3: implementer commit d63b951; RED missing command/parser API, GREEN focused 4/4 and full suite 14/14; reviewer 01a05bc8-3ff9-7fc0-aa8c-daf1c36750b9 dispatched.
- Task 3 review: Important safe-token truncation defect confirmed; fix round 1 assigned to original implementer 01a05bc4-09f6-7b12-8a7a-6cee67729b1e with focused boundary regressions.
- Task 3 complete: fix round 1 commit 3f47e8c; focused 8/8, full suite 18/18; scoped re-review 01a05bcd-47f3-72c1-8c2b-1634b370cbca approved with no remaining issues.
- Task 4 dispatched: implementer 01a05bce-3d41-7962-b73a-2e3d7e71b963; base 3f47e8c; strict TDD and independent review required.
- Task 4 review: two Important findings confirmed (approval invariant bypass; insufficient behavioral coverage); fix round 1 assigned to original implementer 01a05bce-3d41-7962-b73a-2e3d7e71b963.
- Task 4 complete: fix round 1 commit e0bd37b; focused 17/17, full suite 35/35; scoped re-review 01a05bd9-02a1-7c92-8d6a-1a1ebb67dfd8 approved with no remaining issues.
- Task 5 dispatched: implementer 01a05bda-8a3e-7e62-85ba-a11e08755656; base e0bd37b; strict TDD and independent review required.
- Task 5 review: Important descriptor-safety finding confirmed (TOCTOU/path-based chmod plus intermediate symlink traversal); fix round 1 assigned to original implementer 01a05bda-8a3e-7e62-85ba-a11e08755656.
- Task 5 complete: fix round 1 implementation 2a9074e and report 54483d1; focused 11/11, full suite 46/46; scoped re-review 01a05beb-979f-7e12-8406-973f40a4091b approved with no remaining issues.
- Task 6 dispatched: implementer 01a05bed-3b41-7ec2-aff3-69366a314f36; base 54483d1; strict TDD and independent review required.
- Task 6 review: four Important findings confirmed (descriptor boundary lost at SQLite open, unverified pragmas, migration-state bypass, insufficient contract coverage); fix round 1 assigned to original implementer 01a05bed-3b41-7ec2-aff3-69366a314f36.
- Ruling 4 (Task 6 SQLite path boundary): a sound macOS descriptor-backed SQLite open requires a custom VFS for main/WAL/SHM and locking, outside the approved Phase 0 plan and rusqlite surface. Phase 0 retains Task 5 descriptor-safe creation/validation, private 0700 state storage, 0600 DB, and SQLITE_OPEN_NOFOLLOW for the terminal component; it explicitly does not claim resistance to malicious same-UID intermediate-path replacement during SQLite open. Cost if wrong: a same-UID process could race an intermediate path and redirect SQLite/WAL operations.
- Task 6 fix round 1 resumed after ruling 4: implementer will complete pragma verification, migration-state validation, comprehensive contract tests, and the strongest supported SQLite no-follow boundary.
- Task 6 scoped re-review: pragma, migration-state, rollback/checksum/mode, and terminal NOFOLLOW findings resolved; one Important exact-schema coverage gap remains. Fix round 2 assigned to original implementer 01a05bed-3b41-7ec2-aff3-69366a314f36.
- Task 6 scoped re-review round 2: remaining Important test-oracle gaps are hidden autoindexes/extra UNIQUE constraints, missing accepted enum boundaries, and punctuation-sensitive SQL normalization. Final fix round 3 assigned to original implementer 01a05bed-3b41-7ec2-aff3-69366a314f36.
- Ruling 5 (Task 6 SDD convergence): after the third fix-round re-review found one narrow test-only SQL normalizer defect, continue with one surgical round-3 review correction instead of blocking the full Phase 0 rollout. Scope is limited to preserving quoted literal case plus one regression, followed by scoped re-review. Cost if wrong: this relaxes the nominal three-round SDD cap and could hide broader non-convergence; no production behavior is permitted in the exception.
- Task 6 round-3 review correction assigned to original implementer 01a05bed-3b41-7ec2-aff3-69366a314f36 for quoted-literal case preservation only.
- Ruling 6 (Task 6 residual Minor): accept the final scoped reviewer Minor that the escaped-quote regression compares identical literals, because no Critical/Important findings remain and the implementation was independently confirmed to preserve escaped quoted tokens correctly. Cost if wrong: a future regression specifically in doubled-quote tokenization could evade this test oracle.
- Task 6 complete: final correction implementation 734436f and report 2221f0b; focused 16/16, full suite 65/65; no Critical/Important findings remain, with ruling 6 recording one accepted Minor.
- Task 7 dispatched: implementer 01a05c16-a6d2-73e1-a807-7f213d40d366; base 2221f0b; strict TDD and independent review required.
- Task 7 review: seven Important authoritative-stream findings confirmed (validation, SafeToken privacy, write serialization, idempotency scope, typed errors, Unknown category, independent durability fixtures); fix round 1 assigned to original implementer 01a05c16-a6d2-73e1-a807-7f213d40d366. Event-ID versus command/batch semantics must follow the exact Task 7 brief without overclaiming.
- Task 7 scoped re-review: all hardening fixes approved except one Important parser-path gap (unknown command names still emitted Malformed). Fix round 2 assigned to original implementer 01a05c16-a6d2-73e1-a807-7f213d40d366.
- Task 7 complete: fix round 1 cacb8a2, fix round 2 ebba92f; focused contracts green and full suite 82/82; scoped re-review 01a05c39-2b6e-7d32-bd9c-8815e1469e11 approved with no findings. Task 7 explicitly provides single-event/event-ID replay; command/batch idempotency remains assigned to Task 10 by the plan.
- Task 8 dispatched: implementer 01a05c3a-d5d5-78e1-81f3-6a2cb13c6e6d; base ebba92f; strict TDD and independent review required.
- Task 8 review: six Important projection/reducer findings confirmed (stale/fabricated overwrite, rebuild transaction gap, overlapping sessions, historical startup flag, unreachable serde states, insufficient proof) plus sequence overflow Minor; fix round 1 assigned to original implementer 01a05c3a-d5d5-78e1-81f3-6a2cb13c6e6d.
- Task 8 scoped re-review: four Important issues remain (non-transactional load snapshot, missing transient startup interruption effect, incomplete reachable-state validation, unverified stale snapshot before newer store) plus focused coverage gaps; fix round 2 assigned to original implementer 01a05c3a-d5d5-78e1-81f3-6a2cb13c6e6d.
- Task 8 scoped re-review round 2: production fixes accepted; remaining Important gaps are ineffective lower-bound regression, untested authoritative-prefix rejection, and no load-specific snapshot concurrency test, plus one already-ended-case Minor. Final fix round 3 assigned to original implementer 01a05c3a-d5d5-78e1-81f3-6a2cb13c6e6d.
- Ruling 7 (Task 8 SDD convergence): after final fix-round re-review found three branch-selection defects in test fixtures but confirmed production behavior and load concurrency, continue with one surgical test-only round-3 review correction. Cost if wrong: this again relaxes the nominal three-round cap and could mask broader non-convergence; production changes are forbidden.
- Task 8 round-3 review correction assigned to original implementer 01a05c3a-d5d5-78e1-81f3-6a2cb13c6e6d for branch-specific fixtures only.
- Task 8 complete: fix round 1 c99bc0c, fix round 2 77d4d0c, fix round 3 ce697cd, surgical correction 1ead647; focused 20/20 and full suite 102/102; final scoped re-review 01a05c62-bcb6-73d1-ae65-3a7145b9d41d approved with no findings.
- Task 9 dispatched: implementer 01a05c65-1d0c-76e3-9576-f511e5d46a67; base 1ead647; strict TDD and independent review required.
- Task 9 review: four Important lifecycle findings confirmed (missing process-lifetime single-instance guard, empty-stream corrupt projection not rebuildable, caller-memory shutdown idempotency, insufficient recovery/concurrency/dependency proof); fix round 1 assigned to original implementer 01a05c65-1d0c-76e3-9576-f511e5d46a67.
- Task 9 scoped re-review: core behavior approved; remaining Important gaps are movable/public guard lifetime, sequential-not-simultaneous startup test, missing secure lock-file regressions, and incomplete deterministic dependency call counts, plus corruption-case clarity. Fix round 2 assigned to original implementer 01a05c65-1d0c-76e3-9576-f511e5d46a67.
- Task 9 scoped re-review round 2: ownership, simultaneous startup, no-follow structure, call counts, and independent corruption paths approved; one Important combined-test early-return skips socket/mode assertions. Final test-only fix round 3 assigned to original implementer 01a05c65-1d0c-76e3-9576-f511e5d46a67.
- Task 9 complete: fix round 1 9f989fd, fix round 2 a6f0f03, fix round 3 4dbb076; focused 22/22 and full suite 124/124; final scoped re-review 01a05c89-331f-7c83-be17-d90f256efebf approved with no findings.
- Task 10 dispatched: implementer 01a05c8a-9d25-70e1-979e-b75a23389082; base 4dbb076; strict TDD and independent review required.
- Ruling 8 (Task 10 durable command idempotency): extend the prerelease Phase 0 schema with a unique command receipt and ordered command-to-event references because event causation alone cannot make zero-event deny/approval outcomes idempotent. Keep the migration/schema contract exact and document pre-fix development DB checksum incompatibility. Cost if wrong: schema scope diverges from the original version-1 table list and previously created unreleased development databases require recreation.
- Task 10 review: Critical zero-event idempotency hole plus post-commit fallibility, missing durable command invariant, concurrency/failure coverage, and false stale-projection test confirmed; fix round 1 assigned to original implementer 01a05c8a-9d25-70e1-979e-b75a23389082.
- Task 10 scoped re-review: durable receipt architecture approved; remaining Important gaps are unbound peer lifecycle, absent zero-event concurrency/failure/conflict matrix, untested receipt JSON/privacy validators, and incomplete exact CHECK oracle. Fix round 2 assigned to original implementer 01a05c8a-9d25-70e1-979e-b75a23389082.
- Task 10 scoped re-review round 2: remaining Important issues are finish-vs-ID allocation race, embedded-NUL fingerprint acceptance, and full-service worker API exposing owner-only panic/finish methods. Final fix round 3 assigned to original implementer 01a05c8a-9d25-70e1-979e-b75a23389082.
- Task 10 complete: fix round 1 bae433f/908574c, fix round 2 d2fe8e2/327a2a1, fix round 3 654cf5b/10140ea; focused 27 application + 17 migration and full suite 152/152; final scoped re-review 01a05cc6-0591-7721-8856-e1559ac688ee approved with no findings.
- Task 11 dispatched: implementer 01a05cc9-0876-7343-8b88-6273df62caad; base 10140ea; strict TDD and independent review required; preflight backpressure correction applied (blocked first, queued second, rejected third).
- Task 11 implementation checkpoint: focused GREEN compile found terminal_error matching MutexGuard<RuntimeState> as RuntimeState; root cause identified and original implementer resumed for smallest dereference fix and continuation. No implementation commit existed at checkpoint.
- Task 11 review: Critical lock-held blocking-send deadlock plus owner-drop leak, premature terminal-before-join, lost terminal errors, constructor mismatch, unbounded/race-insufficient tests, and WorkerStartup mapping confirmed; fix round 1 assigned to original implementer 01a05cc9-0876-7343-8b88-6273df62caad.
- Task 11 scoped re-review: redesign approved except blocking submit regression, split failure-publication window, and race-insufficient submit/shutdown plus waiter tests. Fix round 2 assigned to original implementer 01a05cc9-0876-7343-8b88-6273df62caad.
- Task 11 complete: implementation f062aa3, fix round 1 ffa1b31, fix round 2 43406c5; focused 13/13 and full suite 165/165; scoped re-review 01a05cf1-64dd-7381-a5ca-a0b74ca9ebcc approved with no findings.
- Task 12 dispatched: implementer 01a05cf8-0c73-7ba2-82a8-a208d74e87db; base 43406c5; strict TDD and independent review required.
- Task 12 review: seven Important findings confirmed (oversized-prefix bypass/full metadata, stdin thread leak, interrupt-channel livelock, post-bootstrap finish gaps, panic cleanup, incomplete audit fields, missing real SIGINT/binary persistence tests) plus duplicate terminal rendering Minor; fix round 1 assigned to original implementer 01a05cf8-0c73-7ba2-82a8-a208d74e87db.
- Task 12 scoped re-review: first hardening approved except raw panic-hook leakage, uncancellable BufferedLineSource, ignored Unix POLLNVAL, non-Unix executable failure, and missing quit/EOF restart warning assertion. Fix round 2 assigned to original implementer 01a05cf8-0c73-7ba2-82a8-a208d74e87db.
- Task 12 scoped re-review round 2: Unix cancellation, panic redaction, POLLNVAL, relaunch, and prior fixes approved; remaining Important issues are Windows active ReadFile cancellation, broken-pipe EOF mapping, and nonportable binary state isolation. Final fix round 3 assigned to original implementer 01a05cf8-0c73-7ba2-82a8-a208d74e87db.
- Ruling 9 (Task 12 SDD convergence): after final fix-round re-review found one Windows-only cancel-before-ReadFile-pending race, continue with one surgical Windows state-machine correction instead of blocking Phase 0. Cost if wrong: this relaxes the nominal three-round cap and Windows behavior remains statically rather than runtime verified on this host; no unrelated production changes are permitted.
- Task 12 round-3 review correction assigned to original implementer 01a05cf8-0c73-7ba2-82a8-a208d74e87db for cancellation handshake/retry only.
- Task 12 complete: initial cce5ee2/71f06b5, fix round 1 6f42574/44d5fa6, fix round 2 53a5ae1/4de66f5, fix round 3 eb8161c/4219424, surgical correction 91d08b3/1a2d9ef; full suite 208/208; final scoped re-review 01a05d50-ff65-72c1-a3cd-9e81f5cf5b98 approved with no findings. Residual: Windows target compile/runtime remains unverified on this host.
- Task 13 dispatched: implementer 01a05d56-0dc8-75e3-b68a-21af92684986; base 1a2d9ef; strict docs-contract TDD and independent review required; source-text legacy warning ruling retained.
- Task 13 review: five Important documentation gaps confirmed (superseded approved-spec link, missing receipt/state-path contract, privacy overclaim, incomplete six-command behavior, weak README assertions) plus report inventory Minor; fix round 1 assigned to original implementer 01a05d56-0dc8-75e3-b68a-21af92684986.
- Task 13 scoped re-review: remaining Important gaps are Windows %APPDATA% path accuracy, persisted-vs-rendered digest wording, incomplete /setup status and /quit table rows, and presence-only assertions. Fix round 2 assigned to original implementer 01a05d56-0dc8-75e3-b68a-21af92684986.
- Task 13 complete: initial eca7622/9acd4cb, fix round 1 270fa4f/d89346a, fix round 2 a85c49f/2f432ad; docs contract 3/3 and full suite green; scoped re-review 01a05d6d-9ddf-75b1-8a78-07f8866dd122 approved with no findings.
- Task 14 dispatched: acceptance owner 01a05d6f-f1f7-77d0-a13c-5c9a1a2da115; base 2f432ad; all plan/README gates, systematic failure handling, exact evidence, and independent review required.
- Ruling 10 (Task 14 SDD convergence): after a third scoped review reconciled the safety scans but found two runtime-network evidence overclaims, permit one surgical report-only correction instead of blocking Phase 0. Cost if wrong: this relaxes the nominal review-round cap; test runtime networking was not sandboxed or independently observed, so residual runtime network non-observation remains disclosed. No production or test change is permitted by this exception.
