# Task 7 report — Hybrid Memory repository

Status: implemented and verified.

Base: `c723bad690f580620a3ea45e06f11aea4d847c0a`

Commit SHA: `e6484de8a46a1eeb0b8794ee7f323e8a98174d5a` (amended below only to record this final SHA).

Files changed:

- `src/persistence/memory_repository.rs`
- `src/persistence/mod.rs`
- `src/persistence/database.rs`
- `src/memory/mod.rs`
- `tests/memory_persistence_contract.rs`
- `tests/memory_integrity_contract.rs`

RED evidence: `cargo test --test memory_persistence_contract --test memory_integrity_contract` failed before implementation with `E0432`, unresolved import `ai_stock_forum::persistence::MemoryRepository` in both contract files.

Verification outcomes:

- Targeted Task 7 contracts: pass (5 persistence, 1 integrity).
- Required repository regressions: pass (`memory_*`, `agent_profile_persistence_contract`, and `skill_persistence_contract`).
- Full `cargo test`: pass.
- `cargo fmt --check`, `cargo clippy -- -D warnings`, and `git diff --check`: pass.

SQL-before-materialization evidence: current entries, proposal lists, episodic summaries, and entry history issue ordered SQL with `LIMIT min(requested, 100)` and separate checked counts.  The contract inserts 101 current entries and asserts `total_count=101`, `returned_count=100`, `omitted_count=1`, and exact first/last SQL ordering.

Streaming evidence: `build_snapshot` iterates SQLite row cursors directly through two deterministic passes per tagged group (matching tags, then untagged fallback), validates each immutable record before calling `MemorySnapshotBuilder::consider_*`, and does not collect candidate records. Tag matches use `EXISTS(json_each(...))`, so each row is selected once.

Design notes:

- Immutable entry, proposal, resolution, and episodic-summary rows are reconstructed from canonical record JSON, then compared against every mirrored SQLite column and validated against predecessor/profile/approval/source context.
- Approval actors use the strict human/system/agent kind-ID matrix. Pending proposal capacity is checked before inserting the approval, so overflow writes no orphan approval row.
- This task validates repository-row and relational context only. It does not claim event-payload authentication; typed payload correlation remains for Tasks 8 and 12.

Snippet interpretation: the brief repeated `source_count` and `created_at_ms` in the `EpisodicSummaryListRecord` prose/snippet. Each is defined exactly once; the duplicate was treated as an obvious duplicated line.

Deviations/concerns: no encryption or navigation changes were made. Episodic summary insertion remains crate-visible (`pub(crate)`) exactly as specified for its later service wiring; its row and source validation is implemented in the repository.
