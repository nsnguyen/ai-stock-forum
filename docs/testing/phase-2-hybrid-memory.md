# Phase 2 Hybrid Memory acceptance guide

Run this guide only against disposable local state. The application does not
encrypt Hybrid Memory at rest. Application-managed records are local plaintext;
owner-only permissions are access control, not encryption. Entry and history
rows, proposals and rationales, summaries and source links, mutation events,
request/outcome receipts, SQLite WAL/journal sidecars, deliberate detail views,
and copied backups can retain plaintext. Use synthetic text only; never enter
credentials, production data, or private material. The bounded credential
deny-list is best-effort and cannot prove that text contains no secret. Delete
adds a tombstone and is not secure erasure.

## Automated gate

From the feature worktree, run:

```sh
cargo build --release --locked
cargo test --test hybrid_memory_acceptance
cargo test --test tui_navigation_contract
```

The automated acceptance fixture is the authoritative proof for namespace
isolation, deterministic bounded retrieval, restart/replay, and parity between
the TUI and fallback host. It is also the approved internal producer harness:
`support::seed_manual_memory_acceptance` creates the five pending proposals,
and `record_test_episodic_summary_at` records the source-linked summary. Phase 2
has no production `/memory propose`, summary-write, or snapshot presentation
route.

## Disposable state

Never repoint a normal user's home directory and never point this guide at the
normal application-state directory. Build first so Cargo and rustup continue to
use their normal configuration.

On Linux, create a task-specific parent while the final application-state
directory is absent. Its parent exists before the seeder atomically claims the
final directory:

```sh
memory_smoke_root="$(mktemp -d)"
mkdir -p "$memory_smoke_root/xdg-data"
export XDG_DATA_HOME="$memory_smoke_root/xdg-data"
memory_acceptance_state="$memory_smoke_root/xdg-data/ai-stock-forum"
test -d "$(dirname "$memory_acceptance_state")"
test ! -e "$memory_acceptance_state"
printf '%s\n' "$memory_acceptance_state"
```

Keep this shell and exact environment for every launch and restart. Every
process must discover and use the identical discovered state path.

On macOS, the `directories` v6 derives its normal path from `HOME`. Use a
disposable local macOS user account and record its exact
`~/Library/Application Support/ai-stock-forum` path. That disposable-account
requirement is a safety policy, not a limitation of `BaseDirs`. Do not override
the signed-in normal user's `HOME` for this procedure. Without access to a
disposable account, record the macOS manual gate as unperformed/blocked and
leave Phase 2 pending.

Seed before any live launch and exactly once for that target. On macOS, set
`memory_acceptance_state` to the disposable user's exact discovered path. The
ignored `seed_manual_acceptance_state` test refuses a mismatched or pre-existing
target and calls `support::seed_manual_memory_acceptance`:

```sh
AI_STOCK_FORUM_MEMORY_ACCEPTANCE_STATE_DIR="$memory_acceptance_state" \
  cargo test --test hybrid_memory_acceptance seed_manual_acceptance_state \
  -- --ignored --exact --nocapture
```

Record only these seven printed labels and their synthetic identifiers:

```text
profile_id=
tui_approval_proposal_id=
tui_rejection_proposal_id=
fallback_approval_proposal_id=
fallback_rejection_proposal_id=
restart_pending_proposal_id=
summary_id=
```

If seeding fails after atomically claiming the directory, do not retry that
directory. Exit the process, inspect or discard the exact target, create a
fresh target with an existing parent and absent final directory, and seed that
fresh target once. Never seed after a live application launch.

## Direct memory and immutable history

1. Launch `target/release/ai-stock-forum` with the disposable environment.
   Press bare `a`, select the printed synthetic agent, open Detail, select
   Memory with Left/Right, and press Enter.
2. Create key `thesis` with value `synthetic value v1` and a synthetic purpose
   tag. Confirm the plaintext warning appears in the editor, review, and
   confirmation. Press Esc once at review and verify no entry exists. Repeat,
   then confirm the exact displayed `set <review-digest>` action.
3. Open deliberate detail, edit the value to `synthetic value v2`, review, and
   confirm. Open History and verify versions 2 and 1 are newest-first. Deliberate
   version-1 detail must still display its original plaintext.
4. Delete the entry, review the exact entry identity and canonical diff, and
   cancel once with Esc. Repeat and confirm `delete <review-digest>`. Verify the
   current list omits the key while History retains the set, overwrite, and
   tombstone versions. Immutable versions, events, and receipts accumulate
   monotonically and repeated edits consume additional local capacity; SQLite
   may reuse pages, so this does not claim monotonically growing physical file
   size.
5. Exit the TUI with `/quit`. Launch the same release binary in fallback mode
   against the identical discovered state path:

   ```text
   target/release/ai-stock-forum --command-mode
   /memory set <agent> fallback_thesis
   synthetic fallback value
   analysis
   set <displayed-review-digest>
   /memory list <agent>
   /memory get <agent> fallback_thesis
   /memory history <agent> fallback_thesis
   /quit
   ```

   Replace placeholders only with the printed selector and exact digest shown
   by that review. Verify list output omits values; deliberate detail/history
   reveal only the requested record; and fallback and TUI show the same typed
   identity, version, state, and digest. The automated acceptance test is the
   exact cross-host command/event/view parity proof.

## Proposals and episodic summaries

1. Use the five proposal IDs and summary ID printed by the one-time seeder. Do
   not run the seeder again.
2. Restart the TUI against the identical path. Under Agents → Memory, open
   Proposals and inspect exact proposer, namespace-owner, namespace, proposal,
   expected-state, approval, and digest fields. Select
   `tui_approval_proposal_id`, cancel its review once, then approve the exact
   displayed action. Verify one accepted entry version and one terminal
   approval. Select `tui_rejection_proposal_id`; Reject must change no entry.
3. Exit through `/quit`, launch `target/release/ai-stock-forum --command-mode`,
   and use only the fallback-labelled proposal IDs:

   ```text
   /memory proposal <fallback_approval_proposal_id>
   /memory approve <fallback_approval_proposal_id>
   yes
   approve <displayed-review-digest>
   /memory proposal <fallback_rejection_proposal_id>
   /memory reject <fallback_rejection_proposal_id>
   approve <displayed-review-digest>
   reject <displayed-review-digest>
   /memory proposals <agent> all
   /quit
   ```

   The `yes` and opposite-action attempts must not commit. Verify the exact
   actions commit once. Do not resolve `restart_pending_proposal_id`; retain the
   fifth pending proposal through restart.
4. Relaunch the TUI, open Episodic Summaries, select the printed `summary_id`,
   and inspect its deliberate detail. It is read-only, labelled
   `Summary — verify sources`, and shows bounded exact source references.
   Generic Help, Status, Audit, navigation, and safe error output must contain
   no entry value, proposal candidate, rationale, summary body, tag, or source
   prose.

An Agent proposal is durable, but it cannot become an accepted entry version
without distinct Human approval. Fallback/TUI parity covers direct Human
mutations and proposal resolutions; proposal creation is test-harness-only.

## Isolation, restart, recovery, and capacity

1. Create two synthetic profiles and reuse key `thesis` with different values.
   Verify list, detail, history, proposal, and episodic views never cross either
   profile direction.
2. Exercise the deterministic internal retrieval boundary:

   ```sh
   cargo test --test hybrid_memory_acceptance \
     bounded_snapshot_is_deterministic_and_never_crosses_namespaces -- --exact
   ```

   No production command presents a snapshot.
3. Through the normal profile creator, create a distinct profile while
   deliberately reproducing allowed non-memory fields from the original. Do
   not use or claim a duplicate-profile command. Verify the distinct profile
   receives a fresh empty namespace. Activate a new immutable version of the
   original profile and verify that it retains the original namespace and
   memory. The automated acceptance assertion remains the authoritative proof
   that copied profile data receives a fresh namespace.
4. Leave an uncommitted Memory draft or review and exit with `/quit`. Restart
   against the identical state path. Verify tombstone history, two accepted and
   two rejected proposal resolutions, `restart_pending_proposal_id` still
   pending with its exact approval identity, and the source-linked summary are
   unchanged. Drafts and process-local review tokens must not survive restart.
   The automated acceptance test separately proves that command-looking text
   survives restart; that text is not part of the manual seed. Replay the
   original snapshot command receipt only through the automated acceptance
   test.
5. Do not hand-edit a live database. Run the isolated recovery contract, which
   corrupts only its own test copy and fails without printing memory prose:

   ```sh
   cargo test --test memory_recovery_contract
   ```

6. Run the isolated atomicity contract:

   ```sh
   cargo test --test memory_atomicity_contract
   ```

   Its `SqliteCapacity` seam must prove that raw event, entry, proposal,
   approval, projection, audit, and receipt snapshots are byte-identical before
   and after failure, then prove the same exact review can retry. Every mutation
   uses one `BEGIN IMMEDIATE` transaction, so capacity/full-disk failure leaves
   no partial durable state.

## Layout, shortcuts, shutdown, and cleanup

Exercise these exact outer terminal sizes:

- `60x18`: one-pane Memory layout;
- `100x24`: the navigation rail consumes 20 columns, leaving an 80-column
  Memory workspace and two panes; and
- `140x30`: the navigation rail consumes 20 columns, leaving a 120-column
  Memory workspace and three panes.

At every size, verify stable agent identity, selected action, required plaintext
warning, Enter, and Esc remain visible in the active panel. Verify the global
destination rail contains exactly these ordered labels and no duplicate or
seventh destination:

```text
1 Overview
2 Setup
3 Audit
4 Help
a Agents
s Skills
```

Memory remains nested under Agents; bare `m` and bare `7` are not destinations.
Modified shortcuts are inert. From every non-text Memory state, each of the six
bare global keys navigates to its destination and bare `a` returns to the exact
retained Memory state. While a Memory editor owns text, type `1234as`; typed
shortcut characters remain editor text and do not navigate. Bare `q` is inert;
`/quit` requests normal shutdown. `?` remains a Help alias, not a destination
label.

Cancel one registered review with Esc, then separately exercise `/quit`,
EOF/interrupt, and the approved host-failure seam. Verify exactly one review
cancellation, no unintended mutation, terminal restoration, and no plaintext
in the safe error line. Exit every process before cleanup.

After all restart and evidence checks, inspect the recorded target. Move only
the exact disposable directory to Trash with the desktop file manager. Keep it
when more evidence or restart inspection is needed. Never target a normal user
state directory or a broad parent. Do not use a recursive deletion command for
this cleanup.

Do not report Linux, Windows, macOS, terminal-resize, recovery, corruption, or
capacity evidence unless that exact action ran. Automated tests require no
credential, network, provider, model, or live terminal dependency.

## Evidence record

Write an ignored record under the existing task SDD directory. Include:

- the exact commit tested;
- host and disposable account/environment;
- the exact disposable-state arrangement and identical discovered path;
- seeder labels and IDs only, never seeded prose;
- exact terminal dimensions;
- automated, focused, full-suite, and release gate summaries;
- each manual outcome, including cancellation, direct mutation, proposal
  resolution, episodic detail, isolation, restart, recovery, capacity, layout,
  shutdown, and restoration;
- cleanup/retention state; and
- every check not performed, recorded without inferring success.

Keep product documentation date-independent. Do not mark Milestone 3 or the
Phase 2 exit gate complete until every required automated, review, release, and
manual check has actually passed.
