# Phase 0 Rust Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the complete Phase 0 Rust foundation: a fallback command executable over a typed application boundary, append-only SQLite events, rebuildable projections, deterministic recovery, safe setup/policy skeletons, and offline verification.

**Architecture:** One Rust modular monolith accepts parsed commands through bounded in-process ports. The application service applies deny-wins policy, commits normalized events and projections atomically to SQLite, then returns typed view models for text rendering. The event stream is authoritative; startup verifies its sequence and digest chain before rebuilding or trusting projections.

**Tech Stack:** Rust 1.98.0, Rust 2024 edition, Cargo lockfile, `rusqlite` with bundled SQLite, `serde`/`serde_json`, `thiserror`, `sha2`, `uuid`, `directories`, `crossbeam-channel`, `ctrlc`, and `tempfile` for tests.

**Spec:** [docs/superpowers/specs/2026-08-31-phase-0-rust-foundation-design.md](../specs/2026-08-31-phase-0-rust-foundation-design.md)

## Global Constraints

- Pin Rust `1.98.0` in both `rust-toolchain.toml` and `Cargo.toml`.
- Use Rust edition `2024`; commit `Cargo.lock`.
- SQLite is the sole authoritative durable store; do not preserve or extend the rejected JSONL implementation.
- Do not delete any local JSONL prototype data.
- The fallback adapter may parse and render but may not open SQLite, call repositories, or evaluate policy.
- The application event stream is append-only and protected against update/delete.
- Commit events and projection mutations in one SQLite transaction.
- Verify contiguous sequence numbers, supported event versions, payload decoding, and the full digest chain at startup.
- Raw rejected input must never be persisted; retain only a safe token, byte length, and SHA-256 digest.
- Input is capped at `4096` bytes per line.
- `/audit tail` defaults to `20`; explicit limits must be in `1..=100`.
- Default command and outcome queue capacity is `32`; capacity remains injectable.
- Unix application directories use mode `0700`; the database uses mode `0600`.
- `/setup status` may append its audit event but may not create setup, configuration, readiness, or approval state.
- Setup and approval records are inert typed skeletons in Phase 0.
- Every expected failure returns a typed error; user input and corrupt/incompatible databases must not panic.
- Keep all tests deterministic with injected clocks, IDs, paths, and channels.
- Default tests require no network, provider, subscription, terminal renderer, Python, Node, or browser.
- Preserve unrelated files and existing ignore rules.
- Start every behavior with a focused failing test and commit after its focused suite passes.

---

## Locked File Map

### Project and documentation

- `rust-toolchain.toml`: exact compiler, formatter, and linter policy.
- `Cargo.toml`: package metadata and direct dependencies only.
- `Cargo.lock`: exact resolved dependency graph.
- `.gitignore`: existing rules plus Rust/local-state output.
- `README.md`: canonical Rust status, commands, paths, and quality gates.
- `migrations/0001_phase0.sql`: complete Phase 0 SQLite schema.
- Four historical documents listed in Task 13: superseded warnings only.

### Runtime source

- `src/main.rs`: discover paths, bootstrap, run fallback host, map safe errors to exit status.
- `src/lib.rs`: module exports.
- `src/domain/{mod.rs,id.rs,clock.rs,digest.rs,object.rs,error.rs}`: stable primitives and errors.
- `src/app/{mod.rs,command.rs,event.rs,outcome.rs,service.rs}`: typed application contract and handlers.
- `src/runtime/mod.rs`: bounded worker/client lifecycle.
- `src/ui/mod.rs`: presentation namespace.
- `src/ui/command/{mod.rs,parser.rs,reader.rs,renderer.rs,runner.rs}`: fallback adapter.
- `src/ui/tui/mod.rs`: Phase 1 boundary documentation only.
- `src/config/{mod.rs,paths.rs}`: platform state paths and owner-only permissions.
- `src/persistence/{mod.rs,database.rs,migrations.rs,event_repository.rs,projection_repository.rs}`: SQLite boundary.
- `src/audit/mod.rs`: redacted audit view mapping.
- `src/recovery/{mod.rs,reducer.rs,coordinator.rs}`: replay, rebuild, and session recovery.
- `src/setup/{mod.rs,models.rs}`: inert setup/readiness types.
- `src/policy/{mod.rs,capability.rs,approval.rs}`: deny-wins policy and approval skeleton.
- `src/{agents,rooms,providers,runtimes,skills,memory,mcp,jobs}/mod.rs`: documented future boundaries.
- `src/domains/{mod.rs,finance/mod.rs}`: documented domain-pack boundary.

### Tests

- `tests/support/mod.rs`: temporary database, fake clock, and deterministic ID helpers.
- `tests/topology_contract.rs`: crate/module topology.
- `tests/domain_contract.rs`: IDs, clocks, canonical digests, and object references.
- `tests/command_contract.rs`: parser and command types.
- `tests/policy_contract.rs`: deny-wins and approval validation.
- `tests/config_contract.rs`: path injection and permissions.
- `tests/migration_contract.rs`: schema, migrations, compatibility, integrity.
- `tests/event_repository_contract.rs`: append-only events, chain verification, audit mapping.
- `tests/projection_contract.rs`: reducers and persisted projection parity.
- `tests/recovery_contract.rs`: bootstrap, restart, interruption, and rebuild.
- `tests/application_contract.rs`: commands through policy/transaction/projection/view.
- `tests/runtime_contract.rs`: bounded command/outcome ports.
- `tests/fallback_contract.rs`: byte reader, text renderer, scripted sessions, shutdown.
- `tests/documentation_contract.rs`: README and superseded warning inventory.
- `tests/phase0_acceptance.rs`: roadmap exit-gate scenario.

---

### Task 1: Pin the Rust toolchain and establish the modular crate

**Files:**
- Create: `rust-toolchain.toml`
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/lib.rs`
- Create: module files from the locked map
- Modify: `.gitignore`
- Test: `tests/topology_contract.rs`

**Interfaces:**
- Consumes: the approved design specification.
- Produces: a Rust 1.98.0 crate exporting every Phase 0 and future boundary module.

- [ ] **Step 1: Install the approved toolchain**

Download the official installer without piping it into a shell, install
`rustup`, then install the exact toolchain and components:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/ai-stock-forum-rustup-init.sh
sh /tmp/ai-stock-forum-rustup-init.sh -y --profile minimal --default-toolchain none
/Users/nguyen-mini/.cargo/bin/rustup toolchain install 1.98.0 --profile minimal --component rustfmt --component clippy
/Users/nguyen-mini/.cargo/bin/rustup default 1.98.0
/Users/nguyen-mini/.cargo/bin/rustc --version
/Users/nguyen-mini/.cargo/bin/cargo --version
```

Expected: `rustc 1.98.0` and the Cargo release bundled with Rust 1.98.0.

- [ ] **Step 2: Create the package and toolchain manifests**

```toml
# rust-toolchain.toml
[toolchain]
channel = "1.98.0"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

```toml
# Cargo.toml
[package]
name = "ai-stock-forum"
version = "0.1.0"
edition = "2024"
rust-version = "1.98.0"
publish = false
description = "Local terminal agent platform"
license = "UNLICENSED"

[dependencies]

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Write the failing topology test**

```rust
// tests/topology_contract.rs
use ai_stock_forum::{
    agents, app, audit, config, domains, jobs, mcp, memory, persistence, policy,
    providers, recovery, rooms, runtime, runtimes, setup, skills, ui,
};

#[test]
fn phase_zero_exports_the_approved_module_boundaries() {
    let names = [
        agents::MODULE_NAME,
        app::MODULE_NAME,
        audit::MODULE_NAME,
        config::MODULE_NAME,
        domains::MODULE_NAME,
        jobs::MODULE_NAME,
        mcp::MODULE_NAME,
        memory::MODULE_NAME,
        persistence::MODULE_NAME,
        policy::MODULE_NAME,
        providers::MODULE_NAME,
        recovery::MODULE_NAME,
        rooms::MODULE_NAME,
        runtime::MODULE_NAME,
        runtimes::MODULE_NAME,
        setup::MODULE_NAME,
        skills::MODULE_NAME,
        ui::MODULE_NAME,
    ];
    assert_eq!(names.len(), 18);
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test --test topology_contract --locked`

Expected: compilation fails because `src/lib.rs` and the named modules do not yet exist.

- [ ] **Step 5: Add the minimal module topology**

`src/lib.rs` exports every module from the test. Each boundary-only `mod.rs`
defines only its exact marker and module documentation:

```rust
//! Future agent-profile boundary. Behavior begins in Phase 2.
pub const MODULE_NAME: &str = "agents";
```

Use the corresponding lowercase module name in every marker. `src/main.rs`
contains only `fn main() {}` until Task 12.

- [ ] **Step 6: Preserve ignore rules and add Phase 0 output**

Keep every existing `.gitignore` entry and add only:

```gitignore
/target/
/.ai-stock-forum/
```

- [ ] **Step 7: Run the focused test and create the lockfile**

Run: `cargo test --test topology_contract --locked`

If `Cargo.lock` does not yet exist, first run `cargo generate-lockfile`, then rerun the locked test.

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add rust-toolchain.toml Cargo.toml Cargo.lock .gitignore src tests/topology_contract.rs
git commit -m "chore: establish phase 0 rust workspace"
```

---

### Task 2: Add deterministic domain primitives

**Files:**
- Create: `src/domain/id.rs`
- Create: `src/domain/clock.rs`
- Create: `src/domain/digest.rs`
- Create: `src/domain/object.rs`
- Create: `src/domain/error.rs`
- Modify: `src/domain/mod.rs`
- Modify: `Cargo.toml`
- Test: `tests/domain_contract.rs`

**Interfaces:**
- Consumes: Rust crate from Task 1.
- Produces: `Clock`, `IdGenerator`, typed ID newtypes, `Sha256Digest`, `canonical_json_bytes`, `ObjectRef`, `ObjectVersion`, and `DomainError`.

- [ ] **Step 1: Add only the domain dependencies**

Run:

```bash
cargo add serde --features derive
cargo add serde_json
cargo add sha2
cargo add hex
cargo add uuid --features v4,serde
cargo add thiserror
```

- [ ] **Step 2: Write failing primitive tests**

```rust
// tests/domain_contract.rs
use ai_stock_forum::domain::{
    canonical_json_bytes, sha256, CorrelationId, EventId, ObjectVersion,
    Sha256Digest,
};
use serde_json::json;
use uuid::Uuid;

#[test]
fn canonical_json_sorts_nested_object_keys() {
    let left = json!({"z": {"b": 2, "a": 1}, "a": true});
    let right = json!({"a": true, "z": {"a": 1, "b": 2}});
    assert_eq!(canonical_json_bytes(&left).unwrap(), canonical_json_bytes(&right).unwrap());
}

#[test]
fn typed_ids_do_not_interchange() {
    let raw = Uuid::from_u128(7);
    let event = EventId::from_uuid(raw);
    let correlation = CorrelationId::from_uuid(raw);
    assert_eq!(event.to_string(), correlation.to_string());
}

#[test]
fn object_versions_reject_zero() {
    assert!(ObjectVersion::new(0).is_err());
    assert_eq!(ObjectVersion::new(1).unwrap().get(), 1);
}

#[test]
fn digest_requires_lowercase_sha256_hex() {
    let digest = sha256(b"phase-zero");
    assert_eq!(digest.as_str().len(), 64);
    assert!(Sha256Digest::parse(&digest.to_string().to_uppercase()).is_err());
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --test domain_contract --locked`

Expected: compilation fails on missing domain exports.

- [ ] **Step 4: Implement typed IDs, clocks, and objects**

Use a private macro to define UUID-backed IDs for:

```rust
InstallationId, SessionId, CommandId, EventId, CorrelationId, CausationId,
ApprovalId, SetupDraftId, ConfigurationVersionId
```

Each type is `Copy + Clone + Eq + Ord + Hash` and provides `from_uuid`,
`as_uuid`, `Display`, `FromStr`, `Serialize`, and `Deserialize`. Define:

```rust
pub trait Clock: Send + Sync {
    fn now_millis(&self) -> i64;
}

pub trait IdGenerator: Send + Sync {
    fn next_uuid(&self) -> uuid::Uuid;
}

pub struct SystemClock;
pub struct UuidGenerator;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    Human,
    System,
}
```

`ObjectRef` contains `kind: String`, `id: String`, `version: ObjectVersion`, and
`digest: Sha256Digest`. Constructors reject empty kind/ID values.

- [ ] **Step 5: Implement canonical JSON and digests**

Recursively sort every JSON object key into a `BTreeMap`, preserve array order,
reject non-finite numbers through `serde_json`, serialize without whitespace,
and hash with SHA-256:

```rust
pub fn canonical_json_bytes<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, DomainError>;
pub fn sha256(bytes: &[u8]) -> Sha256Digest;
```

- [ ] **Step 6: Run focused tests**

Run: `cargo test --test domain_contract --locked`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/domain tests/domain_contract.rs
git commit -m "feat: add deterministic domain primitives"
```

---

### Task 3: Define typed commands and the safe byte parser

**Files:**
- Create: `src/app/command.rs`
- Create: `src/app/outcome.rs`
- Modify: `src/app/mod.rs`
- Create: `src/ui/command/parser.rs`
- Modify: `src/ui/command/mod.rs`
- Test: `tests/command_contract.rs`

**Interfaces:**
- Consumes: `CorrelationId`, `CommandId`, `Actor`, and digest helpers from Task 2.
- Produces: `ApplicationCommand`, `CommandEnvelope`, `AuditLimit`, `InputRejection`, `ParsedLine`, `parse_line`, and typed view models.

- [ ] **Step 1: Write the failing command grammar matrix**

```rust
// tests/command_contract.rs
use ai_stock_forum::app::{ApplicationCommand, InputRejectionCategory};
use ai_stock_forum::ui::command::{parse_line, ParsedLine};

fn command(bytes: &[u8]) -> ApplicationCommand {
    match parse_line(bytes) {
        ParsedLine::Command(command) => command,
        ParsedLine::Ignored => panic!("expected command"),
    }
}

#[test]
fn parses_the_complete_phase_zero_grammar() {
    assert_eq!(command(b" /help \n"), ApplicationCommand::ShowHelp);
    assert_eq!(command(b"/status"), ApplicationCommand::ShowStatus);
    assert_eq!(command(b"/setup status"), ApplicationCommand::ShowSetupStatus);
    assert_eq!(command(b"/audit tail"), ApplicationCommand::audit_tail(20).unwrap());
    assert_eq!(command(b"/audit tail 100"), ApplicationCommand::audit_tail(100).unwrap());
    assert_eq!(command(b"/quit"), ApplicationCommand::RequestShutdown);
}

#[test]
fn rejects_bad_audit_limits_without_defaulting() {
    for input in [b"/audit tail 0".as_slice(), b"/audit tail 101", b"/audit tail nope"] {
        let ApplicationCommand::RejectInput(rejection) = command(input) else {
            panic!("expected rejection");
        };
        assert_eq!(rejection.category, InputRejectionCategory::Malformed);
        assert!(!serde_json::to_string(&rejection).unwrap().contains("raw_input"));
    }
}

#[test]
fn never_carries_unknown_raw_input() {
    let ApplicationCommand::RejectInput(rejection) = command(b"/secret hunter2") else {
        panic!("expected rejection");
    };
    assert_eq!(rejection.safe_token.as_deref(), Some("/secret"));
    assert_eq!(rejection.byte_length, 15);
    let encoded = serde_json::to_string(&rejection).unwrap();
    assert!(!encoded.contains("hunter2"));
    assert!(!encoded.contains("raw_input"));
}

#[test]
fn rejects_invalid_utf8_and_oversized_input() {
    let invalid = command(&[0xff, 0xfe]);
    assert!(matches!(invalid, ApplicationCommand::RejectInput(ref value)
        if value.category == InputRejectionCategory::InvalidEncoding));

    let oversized = command(&vec![b'x'; 4097]);
    assert!(matches!(oversized, ApplicationCommand::RejectInput(ref value)
        if value.category == InputRejectionCategory::Oversized));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test command_contract --locked`

Expected: compilation fails because the command and parser contracts do not exist.

- [ ] **Step 3: Implement the typed command model**

Define:

```rust
pub const MAX_INPUT_BYTES: usize = 4096;
pub const DEFAULT_AUDIT_LIMIT: u16 = 20;
pub const MAX_AUDIT_LIMIT: u16 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationCommand {
    ShowHelp,
    ShowStatus,
    ShowSetupStatus,
    ShowAuditTail { limit: AuditLimit },
    RejectInput(InputRejection),
    RequestShutdown,
}

pub struct CommandEnvelope {
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub actor: Actor,
    pub command: ApplicationCommand,
}
```

`AuditLimit::new` accepts only `1..=100`. `InputRejection` derives serialization
and contains only category, `Option<String>` safe token capped at 64 Unicode
scalar values, byte length, and input digest. It has no raw-input field.

- [ ] **Step 4: Implement total byte parsing**

`parse_line(&[u8]) -> ParsedLine` checks byte length before UTF-8 decoding,
ignores blank input, recognizes only the approved grammar, and converts every
failure into `ApplicationCommand::RejectInput`. Escape control characters in
safe tokens with `escape_default`; hash the original bytes before discarding
them.

- [ ] **Step 5: Add typed outcomes and view models**

Define `HelpView`, `StatusView`, `SetupStatusView`, `AuditTailView`,
`InputRejectedView`, `ShutdownView`, `CommandView`, and
`ShutdownDisposition`. Task 10 composes these views with Task 7's final
`EventEnvelope` in `CommandOutcome`; Task 3 does not declare an interim event
type.

- [ ] **Step 6: Run focused tests**

Run: `cargo test --test command_contract --locked`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/app src/ui/command tests/command_contract.rs
git commit -m "feat: add typed phase 0 command grammar"
```

---

### Task 4: Add deny-wins policy and approval skeletons

**Files:**
- Create: `src/policy/capability.rs`
- Create: `src/policy/approval.rs`
- Modify: `src/policy/mod.rs`
- Modify: `src/app/command.rs`
- Test: `tests/policy_contract.rs`

**Interfaces:**
- Consumes: `ApplicationCommand`, typed IDs, `ObjectRef`, and clock values.
- Produces: `Capability`, `PolicyRule`, `PolicyDecision`, `evaluate`, `ApprovalRecord`, and `ApplicationCommand::required_capability`.

- [ ] **Step 1: Write failing policy tests**

```rust
// tests/policy_contract.rs
use ai_stock_forum::app::ApplicationCommand;
use ai_stock_forum::policy::{
    evaluate, ApprovalAction, ApprovalRecord, ApprovalStatus, Capability, Effect,
    PolicyDecision, PolicyRule,
};
use ai_stock_forum::ui::command::{parse_line, ParsedLine};

#[test]
fn explicit_denial_wins_over_a_grant() {
    let rules = [
        PolicyRule::new(Effect::Grant, Capability::AuditRead),
        PolicyRule::new(Effect::Deny, Capability::AuditRead),
    ];
    assert_eq!(evaluate(Capability::AuditRead, &rules), PolicyDecision::Denied);
}

#[test]
fn missing_rule_denies_by_default() {
    assert_eq!(evaluate(Capability::GitPush, &[]), PolicyDecision::DeniedByDefault);
}

#[test]
fn commands_map_to_exact_safe_capabilities() {
    assert_eq!(ApplicationCommand::ShowHelp.required_capability(), Capability::HelpRead);
    assert_eq!(ApplicationCommand::RequestShutdown.required_capability(), Capability::Shutdown);
    let ParsedLine::Command(rejected) = parse_line(b"/not-supported") else {
        panic!("expected rejected command");
    };
    assert_eq!(rejected.required_capability(), Capability::HelpRead);
}

#[test]
fn approval_requires_an_exact_object_and_pending_status() {
    assert!(ApprovalRecord::builder(ApprovalAction::GitPush).build().is_err());
    assert_eq!(ApprovalStatus::Pending.is_terminal(), false);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test policy_contract --locked`

Expected: compilation fails on missing policy and approval types.

- [ ] **Step 3: Implement capability evaluation**

Define these exact capabilities:

```rust
HelpRead, StatusRead, SetupStatusRead, AuditRead, Shutdown,
DiscussionRun, McpUse, EngineeringJobRun, GitMerge, GitPush,
FinanceRecommendation
```

`evaluate` scans matching rules and returns `Denied` if any denial exists,
otherwise `Granted` if any grant exists, otherwise `DeniedByDefault`.

- [ ] **Step 4: Implement immutable approval records**

`ApprovalRecord` includes approval ID, action, exact `ObjectRef`, actor, pending
status, creation timestamp, optional expiry, and optional resolution. Builder
validation rejects missing object identity, non-pending creation status, and an
expiry not later than creation. No accept/reject command is added.

- [ ] **Step 5: Run focused tests**

Run: `cargo test --test policy_contract --locked`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/policy src/app/command.rs tests/policy_contract.rs
git commit -m "feat: establish deny-wins policy contracts"
```

---

### Task 5: Discover and secure application state paths

**Files:**
- Create: `src/config/paths.rs`
- Modify: `src/config/mod.rs`
- Modify: `Cargo.toml`
- Test: `tests/config_contract.rs`

**Interfaces:**
- Consumes: typed startup errors from Task 2.
- Produces: `AppPaths::discover`, `AppPaths::for_test`, `AppPaths::ensure`, and `database_path`.

- [ ] **Step 1: Add the platform-directory dependency**

Run: `cargo add directories`

- [ ] **Step 2: Write failing path and permission tests**

```rust
// tests/config_contract.rs
use ai_stock_forum::config::AppPaths;

#[test]
fn injected_paths_use_the_phase_zero_database_name() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    assert_eq!(paths.database_path(), temp.path().join("ai-stock-forum.sqlite3"));
}

#[cfg(unix)]
#[test]
fn ensure_makes_the_state_directory_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
    let paths = AppPaths::for_test(&state);
    paths.ensure().unwrap();
    assert_eq!(std::fs::metadata(&state).unwrap().permissions().mode() & 0o777, 0o700);
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --test config_contract --locked`

Expected: compilation fails because `AppPaths` does not exist.

- [ ] **Step 4: Implement discovery and injected paths**

`AppPaths::discover` uses `directories::BaseDirs::data_dir()` and appends
`ai-stock-forum`. `for_test` accepts an explicit root. `ensure` creates the
directory, confirms it is a directory, and applies Unix mode `0700`.

Implement this exact public surface:

```rust
impl AppPaths {
    pub fn discover() -> Result<Self, StartupError>;
    pub fn for_test(root: impl AsRef<std::path::Path>) -> Self;
    pub fn state_dir(&self) -> &std::path::Path;
    pub fn database_path(&self) -> std::path::PathBuf;
    pub fn ensure(&self) -> Result<(), StartupError>;
}
```

Never read or log credentials or environment values. Return
`StartupError::StateDirectoryUnavailable` or
`StartupError::StatePermissions` with a safe path description.

- [ ] **Step 5: Run focused tests**

Run: `cargo test --test config_contract --locked`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/config tests/config_contract.rs
git commit -m "feat: secure local application paths"
```

---

### Task 6: Create ordered SQLite migrations and startup checks

**Files:**
- Create: `migrations/0001_phase0.sql`
- Create: `src/persistence/database.rs`
- Create: `src/persistence/migrations.rs`
- Modify: `src/persistence/mod.rs`
- Modify: `Cargo.toml`
- Test: `tests/migration_contract.rs`

**Interfaces:**
- Consumes: `AppPaths`, `Sha256Digest`, and typed startup errors.
- Produces: `Database::open`, `Database::schema_version`, `Database::connection`, and migration constant `LATEST_SCHEMA_VERSION = 1`.

- [ ] **Step 1: Add bundled SQLite**

Run: `cargo add rusqlite --features bundled`

- [ ] **Step 2: Write failing migration tests**

```rust
// tests/migration_contract.rs
use ai_stock_forum::config::AppPaths;
use ai_stock_forum::persistence::{Database, LATEST_SCHEMA_VERSION};

#[test]
fn fresh_database_has_the_complete_phase_zero_schema() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    let database = Database::open(&paths).unwrap();
    assert_eq!(database.schema_version(), LATEST_SCHEMA_VERSION);
    for table in [
        "event_stream", "installation_projection", "process_session_projection",
        "projection_metadata", "setup_drafts", "installation_configuration_versions",
        "active_installation_configuration", "setup_step_outcomes",
        "capability_readiness", "approval_records",
    ] {
        assert!(database.has_table(table).unwrap(), "missing {table}");
    }
}

#[test]
fn reopening_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    drop(Database::open(&paths).unwrap());
    assert_eq!(Database::open(&paths).unwrap().applied_migrations().unwrap().len(), 1);
}

#[test]
fn newer_schema_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temp.path());
    drop(Database::open(&paths).unwrap());
    let raw = rusqlite::Connection::open(paths.database_path()).unwrap();
    raw.pragma_update(None, "user_version", 99).unwrap();
    drop(raw);
    assert!(matches!(Database::open(&paths), Err(error) if error.code() == "database_schema_newer"));
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --test migration_contract --locked`

Expected: compilation fails because `Database` and migration APIs are missing.

- [ ] **Step 4: Add the complete initial migration**

`0001_phase0.sql` creates all tables from the spec. Use strict constraints:

```sql
CREATE TABLE event_stream (
    sequence INTEGER PRIMARY KEY,
    event_id TEXT NOT NULL UNIQUE,
    event_schema_version INTEGER NOT NULL CHECK (event_schema_version > 0),
    event_type TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    occurred_at_ms INTEGER NOT NULL,
    correlation_id TEXT NOT NULL,
    causation_id TEXT,
    object_kind TEXT,
    object_id TEXT,
    object_version INTEGER CHECK (object_version IS NULL OR object_version > 0),
    object_digest TEXT,
    previous_event_digest TEXT,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    event_digest TEXT NOT NULL UNIQUE
) STRICT;

CREATE TRIGGER event_stream_no_update
BEFORE UPDATE ON event_stream BEGIN
    SELECT RAISE(ABORT, 'event_stream is append-only');
END;

CREATE TRIGGER event_stream_no_delete
BEFORE DELETE ON event_stream BEGIN
    SELECT RAISE(ABORT, 'event_stream is append-only');
END;

CREATE INDEX event_stream_correlation_idx ON event_stream(correlation_id, sequence);
CREATE INDEX event_stream_type_idx ON event_stream(event_type, sequence);

CREATE TABLE installation_projection (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    installation_id TEXT NOT NULL UNIQUE,
    created_event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    created_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE process_session_projection (
    session_id TEXT PRIMARY KEY,
    started_event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    started_at_ms INTEGER NOT NULL,
    ended_event_id TEXT REFERENCES event_stream(event_id),
    ended_at_ms INTEGER,
    end_reason TEXT,
    CHECK ((ended_event_id IS NULL AND ended_at_ms IS NULL AND end_reason IS NULL)
        OR (ended_event_id IS NOT NULL AND ended_at_ms IS NOT NULL AND end_reason IS NOT NULL))
) STRICT;

CREATE TABLE projection_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    last_event_sequence INTEGER NOT NULL CHECK (last_event_sequence >= 0),
    last_event_digest TEXT,
    projection_digest TEXT NOT NULL
) STRICT;

CREATE TABLE setup_drafts (
    draft_id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    state TEXT NOT NULL CHECK (state IN ('drafting', 'reviewed', 'applied', 'superseded')),
    path TEXT NOT NULL CHECK (path IN ('quick_start', 'customize')),
    current_review_digest TEXT,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX setup_drafts_state_idx ON setup_drafts(state, updated_at_ms);

CREATE TABLE installation_configuration_versions (
    configuration_id TEXT PRIMARY KEY,
    version INTEGER NOT NULL CHECK (version > 0),
    source_draft_id TEXT NOT NULL REFERENCES setup_drafts(draft_id),
    review_digest TEXT NOT NULL,
    object_digest TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    created_event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    created_at_ms INTEGER NOT NULL,
    UNIQUE (source_draft_id, review_digest),
    UNIQUE (version)
) STRICT;

CREATE TRIGGER installation_configuration_versions_no_update
BEFORE UPDATE ON installation_configuration_versions BEGIN
    SELECT RAISE(ABORT, 'installation configuration versions are immutable');
END;

CREATE TRIGGER installation_configuration_versions_no_delete
BEFORE DELETE ON installation_configuration_versions BEGIN
    SELECT RAISE(ABORT, 'installation configuration versions are immutable');
END;

CREATE TABLE active_installation_configuration (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    configuration_id TEXT NOT NULL REFERENCES installation_configuration_versions(configuration_id),
    activated_event_id TEXT NOT NULL REFERENCES event_stream(event_id),
    activated_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE setup_step_outcomes (
    draft_id TEXT NOT NULL REFERENCES setup_drafts(draft_id),
    step_key TEXT NOT NULL,
    attempt INTEGER NOT NULL CHECK (attempt > 0),
    status TEXT NOT NULL CHECK (status IN ('passed', 'failed', 'skipped')),
    safe_code TEXT,
    occurred_at_ms INTEGER NOT NULL,
    PRIMARY KEY (draft_id, step_key, attempt)
) STRICT;

CREATE TABLE capability_readiness (
    configuration_id TEXT NOT NULL REFERENCES installation_configuration_versions(configuration_id),
    capability TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ready', 'unavailable')),
    reason_code TEXT,
    checked_at_ms INTEGER NOT NULL,
    projection_digest TEXT NOT NULL,
    PRIMARY KEY (configuration_id, capability)
) STRICT;

CREATE TABLE approval_records (
    approval_id TEXT PRIMARY KEY,
    action_kind TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    object_version INTEGER NOT NULL CHECK (object_version > 0),
    object_digest TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'accepted', 'rejected', 'expired', 'cancelled')),
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER,
    resolved_at_ms INTEGER,
    resolution_kind TEXT,
    resolution_event_id TEXT REFERENCES event_stream(event_id),
    CHECK (expires_at_ms IS NULL OR expires_at_ms > created_at_ms),
    CHECK ((status = 'pending' AND resolved_at_ms IS NULL AND resolution_kind IS NULL)
        OR (status <> 'pending' AND resolved_at_ms IS NOT NULL AND resolution_kind IS NOT NULL))
) STRICT;

CREATE INDEX approval_records_status_idx ON approval_records(status, created_at_ms);
```

`schema_migrations` is bootstrapped by the migration runner before this SQL
executes.

- [ ] **Step 5: Implement migration startup**

Use SQLite application ID `0x4149_4653` (`AIFS`) and schema version `1`.
Before writing, reject a nonzero foreign application ID or user version above
one. Store SHA-256 of the embedded SQL in `schema_migrations`; compare it on
every reopen. Apply the migration and update application/user versions in one
immediate transaction. Enable foreign keys and run `PRAGMA quick_check(1)`.

Create/correct the Unix database mode to `0600` after opening. Expose only the
small query helpers named in the interface block.

```rust
pub struct Database {
    connection: rusqlite::Connection,
    schema_version: u32,
}

impl Database {
    pub fn open(paths: &AppPaths) -> Result<Self, StartupError>;
    pub fn schema_version(&self) -> u32;
    pub fn connection(&self) -> &rusqlite::Connection;
    pub fn connection_mut(&mut self) -> &mut rusqlite::Connection;
    pub fn has_table(&self, name: &str) -> Result<bool, PersistenceError>;
    pub fn applied_migrations(&self) -> Result<Vec<AppliedMigration>, PersistenceError>;
}
```

- [ ] **Step 6: Add corrupt and foreign-database cases**

Extend the same test file to write non-SQLite bytes and to create a database
with a foreign application ID. Assert stable error codes `database_corrupt` and
`database_application_mismatch` and confirm neither file is recreated.

- [ ] **Step 7: Run focused tests**

Run: `cargo test --test migration_contract --locked`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock migrations src/persistence tests/migration_contract.rs
git commit -m "feat: add phase 0 sqlite migrations"
```

---

### Task 7: Implement normalized append-only events and audit views

**Files:**
- Create: `src/app/event.rs`
- Modify: `src/app/mod.rs`
- Create: `src/persistence/event_repository.rs`
- Modify: `src/persistence/mod.rs`
- Modify: `src/app/outcome.rs`
- Create: `src/audit/mod.rs`
- Create: `tests/support/mod.rs`
- Test: `tests/event_repository_contract.rs`

**Interfaces:**
- Consumes: `Database`, canonical digest helpers, typed IDs, actors, and object references.
- Produces: `ApplicationEvent`, `PendingEvent`, `EventEnvelope`, `EventRepository::{append,load_all,tail,verify}`, and `AuditEntry::from_event`.

- [ ] **Step 1: Write failing event and append-only tests**

```rust
// tests/event_repository_contract.rs
mod support;

use ai_stock_forum::app::{ApplicationEvent, PendingEvent};
use ai_stock_forum::audit::AuditEntry;
use ai_stock_forum::persistence::EventRepository;

#[test]
fn append_allocates_a_contiguous_digest_chain() {
    let mut fixture = support::database();
    let first = fixture.append(ApplicationEvent::HelpViewed);
    let second = fixture.append(ApplicationEvent::StatusViewed);
    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(second.previous_event_digest.as_ref(), Some(&first.event_digest));
    EventRepository::verify(fixture.database.connection()).unwrap();
}

#[test]
fn update_and_delete_are_rejected() {
    let mut fixture = support::database();
    fixture.append(ApplicationEvent::HelpViewed);
    assert!(fixture.database.connection().execute("DELETE FROM event_stream", []).is_err());
    assert!(fixture.database.connection().execute(
        "UPDATE event_stream SET event_type = 'forged' WHERE sequence = 1", []
    ).is_err());
}

#[test]
fn audit_entries_are_typed_and_redacted() {
    let event = support::rejected_event(b"/secret hunter2");
    let audit = AuditEntry::from_event(&event);
    assert!(!audit.summary.contains("hunter2"));
    assert_eq!(audit.kind, "command_rejected");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test event_repository_contract --locked`

Expected: compilation fails because event and repository types are missing.

- [ ] **Step 3: Define the exact event variants and envelope**

Use the eleven variants from the spec and tagged snake-case serialization:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShutdownReason {
    UserQuit,
    InputClosed,
    Interrupted,
    ApplicationError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ApplicationEvent {
    InstallationInitialized { installation_id: InstallationId },
    ProcessSessionStarted { session_id: SessionId },
    PreviousSessionInterrupted { session_id: SessionId },
    HelpViewed,
    StatusViewed,
    SetupStatusViewed,
    AuditTailViewed { limit: AuditLimit },
    CommandRejected { rejection: InputRejection },
    ShutdownRequested,
    ProcessSessionEnded { session_id: SessionId, reason: ShutdownReason },
    ProjectionRebuilt { through_sequence: u64 },
}

pub const EVENT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEvent {
    pub event_id: EventId,
    pub event_schema_version: u16,
    pub actor: Actor,
    pub occurred_at_ms: i64,
    pub correlation_id: CorrelationId,
    pub causation_id: Option<CausationId>,
    pub object: Option<ObjectRef>,
    pub event: ApplicationEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub sequence: u64,
    pub event_id: EventId,
    pub event_schema_version: u16,
    pub actor: Actor,
    pub occurred_at_ms: i64,
    pub correlation_id: CorrelationId,
    pub causation_id: Option<CausationId>,
    pub object: Option<ObjectRef>,
    pub event: ApplicationEvent,
    pub previous_event_digest: Option<Sha256Digest>,
    pub event_digest: Sha256Digest,
}
```

`EventEnvelope` includes every normalized field from the spec. `PendingEvent`
contains all fields except sequence, previous digest, and event digest.

- [ ] **Step 4: Implement transactional append and verification**

`EventRepository::append(&Transaction, PendingEvent)` reads the final sequence
and digest, allocates the next sequence, serializes typed payload to canonical
JSON, hashes a versioned digest material struct, and inserts exactly one row.
`load_all` decodes every payload. `verify` rejects sequence gaps, unsupported
schema versions, malformed typed payloads, previous-digest mismatch, and current
digest mismatch with stable recovery errors.

```rust
impl EventRepository {
    pub fn append(
        transaction: &rusqlite::Transaction<'_>,
        pending: PendingEvent,
    ) -> Result<EventEnvelope, PersistenceError>;

    pub fn load_all(
        connection: &rusqlite::Connection,
    ) -> Result<Vec<EventEnvelope>, RecoveryError>;

    pub fn tail(
        connection: &rusqlite::Connection,
        limit: AuditLimit,
    ) -> Result<Vec<EventEnvelope>, PersistenceError>;

    pub fn verify(connection: &rusqlite::Connection) -> Result<(), RecoveryError>;
}
```

- [ ] **Step 5: Implement redacted audit mapping**

`AuditEntry` exposes sequence, timestamp, actor, event kind, correlation ID, and
a safe variant-specific summary. It never exposes `payload_json`. A rejection
summary includes category, safe token, and byte length but not source bytes.

- [ ] **Step 6: Add an inserted-forgery verification test**

Insert sequence two directly with a valid JSON payload and deliberately wrong
digests, then assert `EventRepository::verify` returns error code
`event_digest_mismatch` without deleting either row.

- [ ] **Step 7: Run focused tests**

Run: `cargo test --test event_repository_contract --locked`

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/app src/persistence src/audit tests/support tests/event_repository_contract.rs
git commit -m "feat: add normalized append-only event stream"
```

---

### Task 8: Add deterministic reducers and persisted projections

**Files:**
- Create: `src/setup/models.rs`
- Modify: `src/setup/mod.rs`
- Create: `src/recovery/reducer.rs`
- Modify: `src/recovery/mod.rs`
- Create: `src/persistence/projection_repository.rs`
- Modify: `src/persistence/mod.rs`
- Test: `tests/projection_contract.rs`

**Interfaces:**
- Consumes: verified `EventEnvelope` values and setup schema tables.
- Produces: `ProjectionState`, `InstallationProjection`, `SessionProjection`, `SetupStatus`, `reduce`, and `ProjectionRepository::{load,store,rebuild}`.

- [ ] **Step 1: Write failing reducer parity tests**

```rust
// tests/projection_contract.rs
mod support;

use ai_stock_forum::recovery::{reduce, ProjectionState};
use ai_stock_forum::setup::SetupStatus;

#[test]
fn direct_reduction_rebuilds_installation_and_sessions() {
    let events = support::installation_session_events();
    let mut state = ProjectionState::default();
    for event in &events {
        reduce(&mut state, event).unwrap();
    }
    assert_eq!(state.installation.as_ref().unwrap().installation_id, support::installation_id());
    assert_eq!(state.setup_status, SetupStatus::NotStarted);
    assert_eq!(state.last_sequence, events.last().unwrap().sequence);
}

#[test]
fn persisted_projection_matches_direct_reduction() {
    let fixture = support::database_with_installation_session();
    let persisted = fixture.load_projection();
    let direct = fixture.reduce_events();
    assert_eq!(persisted.digest().unwrap(), direct.digest().unwrap());
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test projection_contract --locked`

Expected: compilation fails on missing projection and setup types.

- [ ] **Step 3: Implement inert setup/readiness models**

Define typed states and records for `SetupDraft`, `InstallationConfigurationVersion`,
`SetupStepOutcome`, `CapabilityReadiness`, and:

```rust
pub enum SetupStatus {
    NotStarted,
    DraftSaved { draft_id: SetupDraftId },
    Applied { configuration_id: ConfigurationVersionId },
}
```

Expose constructors and validation only. Do not add creation/application commands.

- [ ] **Step 4: Implement the pure reducer**

`ProjectionState` contains optional installation, ordered sessions, setup status,
last sequence, last event digest, and whether an interruption was detected this
startup. `reduce` verifies the next expected sequence before applying each event.
Unknown event versions are errors; view-only events only advance metadata.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProjectionState {
    pub installation: Option<InstallationProjection>,
    pub sessions: std::collections::BTreeMap<SessionId, SessionProjection>,
    pub setup_status: SetupStatus,
    pub last_sequence: u64,
    pub last_event_digest: Option<Sha256Digest>,
    pub previous_session_interrupted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallationProjection {
    pub installation_id: InstallationId,
    pub created_event_id: EventId,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionProjection {
    pub session_id: SessionId,
    pub started_event_id: EventId,
    pub started_at_ms: i64,
    pub ended: Option<SessionEndProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEndProjection {
    pub ended_event_id: EventId,
    pub ended_at_ms: i64,
    pub reason: ShutdownReason,
}
```

- [ ] **Step 5: Persist projection changes transactionally**

`ProjectionRepository::store` updates installation, session, and projection
metadata rows inside the same caller-provided SQLite transaction as event append.
`rebuild` clears only rebuildable projection tables, reduces the complete verified
event stream, stores the result, and never modifies setup/configuration/approval
authoritative skeleton tables.

```rust
pub fn reduce(
    state: &mut ProjectionState,
    event: &EventEnvelope,
) -> Result<(), RecoveryError>;

impl ProjectionRepository {
    pub fn load(connection: &rusqlite::Connection) -> Result<ProjectionState, RecoveryError>;
    pub fn store(
        transaction: &rusqlite::Transaction<'_>,
        state: &ProjectionState,
    ) -> Result<(), PersistenceError>;
    pub fn rebuild(
        connection: &mut rusqlite::Connection,
        events: &[EventEnvelope],
    ) -> Result<ProjectionState, RecoveryError>;
}
```

- [ ] **Step 6: Run focused tests**

Run: `cargo test --test projection_contract --locked`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/setup src/recovery src/persistence tests/projection_contract.rs tests/support
git commit -m "feat: add replayable phase 0 projections"
```

---

### Task 9: Bootstrap installation identity and recover process sessions

**Files:**
- Create: `src/recovery/coordinator.rs`
- Modify: `src/recovery/mod.rs`
- Modify: `src/persistence/projection_repository.rs`
- Test: `tests/recovery_contract.rs`

**Interfaces:**
- Consumes: `Database`, `Clock`, `IdGenerator`, event repository, and projection reducer.
- Produces: `RecoveryCoordinator::bootstrap`, `RecoveryCoordinator::finish_session`, `RecoveryHook`, and `BootstrapState`.

- [ ] **Step 1: Write failing restart and recovery tests**

```rust
// tests/recovery_contract.rs
mod support;

use ai_stock_forum::app::ShutdownReason;
use ai_stock_forum::recovery::RecoveryCoordinator;

#[test]
fn restart_reuses_installation_identity() {
    let fixture = support::persistent_fixture();
    let first = fixture.bootstrap();
    fixture.finish(first.session_id, ShutdownReason::InputClosed);
    let second = fixture.bootstrap();
    assert_eq!(first.installation_id, second.installation_id);
    assert_ne!(first.session_id, second.session_id);
    assert!(!second.previous_session_interrupted);
}

#[test]
fn missing_end_event_is_reported_once() {
    let fixture = support::persistent_fixture();
    let abandoned = fixture.bootstrap();
    drop(abandoned);
    let recovered = fixture.bootstrap();
    assert!(recovered.previous_session_interrupted);
    assert_eq!(fixture.event_count("previous_session_interrupted"), 1);
}

#[test]
fn stale_projection_rebuilds_to_authoritative_state() {
    let fixture = support::persistent_fixture();
    let state = fixture.bootstrap();
    fixture.corrupt_projection_metadata_only();
    let recovered = fixture.bootstrap();
    assert_eq!(state.installation_id, recovered.installation_id);
    assert_eq!(fixture.event_count("projection_rebuilt"), 1);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test recovery_contract --locked`

Expected: compilation fails because the coordinator is missing.

- [ ] **Step 3: Implement startup ordering**

`bootstrap` performs these exact operations:

1. verify the event stream;
2. load projection metadata and rebuild if missing/stale/mismatched;
3. append and project `InstallationInitialized` only if no installation exists;
4. detect an unfinished latest session;
5. append/project `PreviousSessionInterrupted` and run registered hooks;
6. append/project `ProcessSessionStarted`; and
7. return `BootstrapState` with installation/session IDs and recovery summary.

Each numbered state mutation uses an immediate transaction containing both event
append and projection update.

Recovery extensions use this exact boundary:

```rust
pub trait RecoveryHook: Send + Sync {
    fn recover(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        interrupted_session: SessionId,
    ) -> Result<(), RecoveryError>;
}
```

Phase 0 registers one no-op hook in production and a recording hook in tests.

```rust
pub struct BootstrapState {
    pub installation_id: InstallationId,
    pub session_id: SessionId,
    pub projection: ProjectionState,
    pub previous_session_interrupted: bool,
}
```

```rust
impl RecoveryCoordinator {
    pub fn bootstrap(
        database: &mut Database,
        clock: &dyn Clock,
        ids: &dyn IdGenerator,
        hooks: &[Box<dyn RecoveryHook>],
    ) -> Result<BootstrapState, StartupError>;

    pub fn finish_session(
        database: &mut Database,
        state: &mut ProjectionState,
        session_id: SessionId,
        reason: ShutdownReason,
        clock: &dyn Clock,
        ids: &dyn IdGenerator,
    ) -> Result<Vec<EventEnvelope>, AppError>;
}
```

- [ ] **Step 4: Implement explicit clean termination**

`finish_session(session_id, reason)` appends/projects `ProcessSessionEnded` once.
A second call returns the existing end state without adding another event. Reasons
are `UserQuit`, `InputClosed`, `Interrupted`, and `ApplicationError`.

- [ ] **Step 5: Add authoritative corruption tests**

Insert a bad event after a valid stream and assert bootstrap returns
`event_digest_mismatch`, leaves the database untouched, and never appends a new
session event.

- [ ] **Step 6: Run focused tests**

Run: `cargo test --test recovery_contract --locked`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/recovery src/persistence tests/recovery_contract.rs tests/support
git commit -m "feat: recover installation and process sessions"
```

---

### Task 10: Implement the application service transaction boundary

**Files:**
- Create: `src/app/service.rs`
- Modify: `src/app/mod.rs`
- Modify: `src/app/outcome.rs`
- Modify: `src/persistence/event_repository.rs`
- Modify: `src/persistence/projection_repository.rs`
- Test: `tests/application_contract.rs`

**Interfaces:**
- Consumes: bootstrapped database/state, policy rules, clock, IDs, commands, repositories.
- Produces: `ApplicationService::bootstrap`, `ApplicationService::execute`, `ApplicationService::finish`, and complete typed view models.

- [ ] **Step 1: Write failing command-flow tests**

```rust
// tests/application_contract.rs
mod support;

use ai_stock_forum::app::{ApplicationCommand, CommandView, ShutdownDisposition};

#[test]
fn status_flows_through_event_and_projection_transaction() {
    let mut app = support::app();
    let outcome = app.execute_user(ApplicationCommand::ShowStatus).unwrap();
    let CommandView::Status(view) = outcome.view else { panic!("status view") };
    assert_eq!(view.installation_id, app.installation_id());
    assert_eq!(outcome.committed_events.len(), 1);
    assert_eq!(outcome.committed_events[0].correlation_id, outcome.correlation_id);
    assert_eq!(app.persisted_last_sequence(), outcome.committed_events[0].sequence);
}

#[test]
fn setup_status_does_not_invent_configuration() {
    let mut app = support::app();
    let outcome = app.execute_user(ApplicationCommand::ShowSetupStatus).unwrap();
    assert!(matches!(outcome.view, CommandView::SetupStatus(ref view) if view.is_not_started()));
    assert_eq!(app.count_rows("setup_drafts"), 0);
    assert_eq!(app.count_rows("installation_configuration_versions"), 0);
    assert_eq!(app.count_rows("capability_readiness"), 0);
    assert_eq!(app.count_rows("approval_records"), 0);
}

#[test]
fn audit_tail_includes_its_own_committed_event() {
    let mut app = support::app();
    app.execute_user(ApplicationCommand::ShowHelp).unwrap();
    let outcome = app.execute_user(ApplicationCommand::audit_tail(20).unwrap()).unwrap();
    let CommandView::AuditTail(view) = outcome.view else { panic!("audit view") };
    assert_eq!(view.entries.last().unwrap().kind, "audit_tail_viewed");
}

#[test]
fn quit_requests_but_does_not_fake_session_completion() {
    let mut app = support::app();
    let outcome = app.execute_user(ApplicationCommand::RequestShutdown).unwrap();
    assert_eq!(outcome.shutdown, ShutdownDisposition::Requested);
    assert_eq!(app.event_count("process_session_ended"), 0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test application_contract --locked`

Expected: compilation fails because `ApplicationService` is missing.

- [ ] **Step 3: Implement service bootstrap and safe default policy**

`ApplicationService::bootstrap(paths, clock, ids)` opens the database, runs
recovery, and installs explicit grants only for the five Phase 0 capabilities.
Future capability families remain denied by default.

```rust
impl ApplicationService {
    pub fn bootstrap(
        paths: &AppPaths,
        clock: std::sync::Arc<dyn Clock>,
        ids: std::sync::Arc<dyn IdGenerator>,
    ) -> Result<Self, StartupError>;

    pub fn execute_user(
        &mut self,
        command: ApplicationCommand,
    ) -> Result<CommandOutcome, AppError>;

    pub fn execute(
        &mut self,
        envelope: CommandEnvelope,
    ) -> Result<CommandOutcome, AppError>;

    pub fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError>;
}
```

- [ ] **Step 4: Implement one transaction path for every command**

For each command:

1. evaluate its required capability;
2. create one typed event;
3. begin an immediate transaction;
4. append the event;
5. reduce and persist the resulting projection;
6. commit;
7. build the typed view from committed state; and
8. return `CommandOutcome`.

`ShowAuditTail` appends `AuditTailViewed`, commits it, then reads the bounded tail
so the current request is included. `RejectInput` appends only redacted fields.
`RequestShutdown` appends `ShutdownRequested`; the host later calls `finish` with
the actual termination reason.

Define the final outcome now that Task 7 provides `EventEnvelope`:

```rust
pub struct CommandOutcome {
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub committed_events: Vec<EventEnvelope>,
    pub view: CommandView,
    pub shutdown: ShutdownDisposition,
}
```

- [ ] **Step 5: Add injected transaction-failure coverage**

Install a temporary SQLite trigger that aborts projection metadata update. Execute
`ShowStatus`; assert the service returns a persistence error and neither the event
nor projection sequence advances.

- [ ] **Step 6: Run focused tests**

Run: `cargo test --test application_contract --locked`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/app src/persistence tests/application_contract.rs tests/support
git commit -m "feat: add transactional application service"
```

---

### Task 11: Wrap the service in bounded typed channels

**Files:**
- Replace: `src/runtime/mod.rs`
- Modify: `Cargo.toml`
- Test: `tests/runtime_contract.rs`

**Interfaces:**
- Consumes: `ApplicationService`, `ApplicationCommand`, and `ShutdownReason`.
- Produces: `CommandExecutor`, `ApplicationRuntime::spawn`, `RuntimeClient::{submit,try_submit}`, `PendingOutcome::recv`, and `ApplicationRuntime::finish_and_join`.

- [ ] **Step 1: Add bounded channels**

Run: `cargo add crossbeam-channel`

- [ ] **Step 2: Write failing runtime tests**

```rust
// tests/runtime_contract.rs
mod support;

use ai_stock_forum::app::{ApplicationCommand, CommandView};
use ai_stock_forum::runtime::ApplicationRuntime;
use ai_stock_forum::app::ShutdownReason;

#[test]
fn commands_and_outcomes_cross_bounded_ports() {
    let service = support::app();
    let runtime = ApplicationRuntime::spawn(service, 32).unwrap();
    let outcome = runtime.client().submit(ApplicationCommand::ShowHelp).unwrap();
    assert!(matches!(outcome.view, CommandView::Help(_)));
    runtime.finish_and_join(ShutdownReason::InputClosed).unwrap();
}

#[test]
fn capacity_one_reports_backpressure_without_dropping_commands() {
    let service = support::blocking_app();
    let runtime = ApplicationRuntime::spawn(service, 1).unwrap();
    let client = runtime.client();
    let first = client.try_submit(ApplicationCommand::ShowHelp).unwrap();
    assert!(client.try_submit(ApplicationCommand::ShowStatus).is_err());
    support::release_blocking_app();
    first.recv().unwrap();
    runtime.finish_and_join(ShutdownReason::InputClosed).unwrap();
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --test runtime_contract --locked`

Expected: compilation fails because the runtime types do not exist.

- [ ] **Step 4: Implement the runtime worker**

Define a `CommandExecutor` trait with `execute_user(ApplicationCommand)` and
`finish(ShutdownReason)` methods; implement it for `ApplicationService`. This
lets the test use a barrier-backed executor without adding production timing
hooks. Use `crossbeam_channel::bounded(capacity)` for requests. Every request
contains an `ApplicationCommand` and a bounded one-result reply channel. The
worker owns the mutable executor, calls `execute_user` serially, and sends
exactly one typed result.

```rust
pub trait CommandExecutor: Send + 'static {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError>;
    fn finish(&mut self, reason: ShutdownReason) -> Result<(), AppError>;
}

impl ApplicationRuntime {
    pub fn spawn(
        executor: impl CommandExecutor,
        capacity: usize,
    ) -> Result<Self, RuntimeError>;
    pub fn client(&self) -> RuntimeClient;
    pub fn finish_and_join(self, reason: ShutdownReason) -> Result<(), RuntimeError>;
}
```

`try_submit` uses `try_send`, returns `PendingOutcome` when accepted, and returns
`RuntimeError::Backpressure` when full. `submit` blocks until accepted and until
its one result arrives. `finish_and_join(reason)` sends a dedicated
`Finish { reason }` request, waits for session persistence, closes the request
port, and joins the worker.

- [ ] **Step 5: Run focused tests**

Run: `cargo test --test runtime_contract --locked`

Expected: PASS with no hanging worker threads.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/runtime tests/runtime_contract.rs tests/support
git commit -m "feat: add bounded application runtime"
```

---

### Task 12: Build the fallback byte reader, renderer, host, and executable

**Files:**
- Create: `src/ui/command/reader.rs`
- Create: `src/ui/command/renderer.rs`
- Create: `src/ui/command/runner.rs`
- Modify: `src/ui/command/mod.rs`
- Replace: `src/main.rs`
- Modify: `Cargo.toml`
- Test: `tests/fallback_contract.rs`

**Interfaces:**
- Consumes: parser, `RuntimeClient`, typed views, config paths, and shutdown reasons.
- Produces: `BoundedLineReader`, `TextRenderer`, `FallbackRunner::run`, and the Phase 0 binary.

- [ ] **Step 1: Add handled-interrupt support**

Run: `cargo add ctrlc`

- [ ] **Step 2: Write failing reader and scripted-session tests**

```rust
// tests/fallback_contract.rs
mod support;

use std::io::Cursor;
use ai_stock_forum::ui::command::{BoundedLineReader, FallbackRunner};

#[test]
fn oversized_line_is_one_rejection_and_next_line_survives() {
    let mut bytes = vec![b'x'; 4097];
    bytes.extend_from_slice(b"\n/help\n");
    let mut reader = BoundedLineReader::new(Cursor::new(bytes));
    assert!(reader.next_line().unwrap().unwrap().was_oversized());
    assert_eq!(reader.next_line().unwrap().unwrap().bytes(), b"/help");
}

#[test]
fn scripted_fallback_session_renders_required_commands_and_quits() {
    let input = Cursor::new(b"/help\n/status\n/setup status\n/audit tail 3\n/quit\n".to_vec());
    let mut output = Vec::new();
    let runtime = support::runtime();
    let reason = FallbackRunner::new(runtime.client(), false).run(input, &mut output).unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Available commands"));
    assert!(text.contains("Installation"));
    assert!(text.contains("Guided setup is not implemented"));
    assert!(text.contains("Audit tail"));
    assert!(text.contains("Shutting down"));
    runtime.finish_and_join(reason).unwrap();
}

#[test]
fn eof_records_a_clean_input_closed_session() {
    let runtime = support::runtime();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(Vec::<u8>::new()), Vec::new())
        .unwrap();
    runtime.finish_and_join(reason).unwrap();
    assert_eq!(support::last_shutdown_reason(), "input_closed");
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test --test fallback_contract --locked`

Expected: compilation fails because reader, renderer, and runner are missing.

- [ ] **Step 4: Implement bounded line reading**

Read at most `MAX_INPUT_BYTES + 1` bytes before discarding through newline. Return
a `RawLine` with bytes and an oversized flag. Remove one trailing LF and optional
preceding CR. Never allocate based on the untrusted complete line length.

```rust
impl<R: std::io::BufRead> BoundedLineReader<R> {
    pub fn new(reader: R) -> Self;
    pub fn next_line(&mut self) -> std::io::Result<Option<RawLine>>;
}
```

- [ ] **Step 5: Implement typed rendering**

`TextRenderer` exhaustively matches `CommandView`. It escapes control characters,
prints stable safe errors, and renders audit entries rather than event JSON.
The help view lists only the five required commands and audit limit syntax.

- [ ] **Step 6: Implement fallback lifecycle**

`FallbackRunner` reads one bounded line, calls `parse_line`, submits nonblank typed
commands, renders outcomes, and stops on `ShutdownDisposition::Requested`. It
returns `ShutdownReason::InputClosed` on normal EOF or
`ShutdownReason::UserQuit` after `/quit`; it does not persist session completion.
The owning host passes that reason to `finish_and_join` exactly once.

```rust
impl FallbackRunner {
    pub fn new(client: RuntimeClient, show_prompt: bool) -> Self;
    pub fn run<R: std::io::BufRead, W: std::io::Write>(
        &self,
        reader: R,
        writer: W,
    ) -> Result<ShutdownReason, UiError>;
}
```

The production host installs `ctrlc` once and sends a bounded control message.
Use a dedicated stdin reader thread plus `crossbeam_channel::select!` so Ctrl-C
can request `Finish(Interrupted)` without waiting for another line. No background
application work survives process exit.

- [ ] **Step 7: Replace the executable entrypoint**

`main` discovers `AppPaths`, bootstraps `ApplicationService`, starts the bounded
runtime with capacity 32, runs fallback mode, finishes the session, joins the
worker, and returns `ExitCode::SUCCESS`. Typed startup/runtime errors render one
safe line to stderr and return failure. Do not print debug representations.

- [ ] **Step 8: Run focused tests**

Run: `cargo test --test fallback_contract --locked`

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs src/ui/command tests/fallback_contract.rs tests/support
git commit -m "feat: ship phase 0 fallback command host"
```

---

### Task 13: Make documentation precedence unambiguous

**Files:**
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md`
- Modify: `docs/superpowers/plans/2026-08-09-ai-stock-forum-roadmap.md`
- Modify: `docs/superpowers/plans/2026-08-09-phase-1-deterministic-risk-core.md`
- Modify: `docs/superpowers/plans/2026-08-15-phase-0a-workspace-contract-foundation.md`
- Test: `tests/documentation_contract.rs`

**Interfaces:**
- Consumes: final fallback commands and quality gates.
- Produces: a canonical README and machine-checked superseded warnings.

- [ ] **Step 1: Write the failing documentation contract**

```rust
// tests/documentation_contract.rs
const README: &str = include_str!("../README.md");
const LEGACY: [&str; 4] = [
    include_str!("../docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md"),
    include_str!("../docs/superpowers/plans/2026-08-09-ai-stock-forum-roadmap.md"),
    include_str!("../docs/superpowers/plans/2026-08-09-phase-1-deterministic-risk-core.md"),
    include_str!("../docs/superpowers/plans/2026-08-15-phase-0a-workspace-contract-foundation.md"),
];

#[test]
fn readme_points_to_the_rust_sources_of_truth() {
    assert!(README.contains("architecture.md"));
    assert!(README.contains("phases.md"));
    assert!(README.contains("cargo run"));
    assert!(README.contains("/setup status"));
    assert!(!README.contains("Phase-0 foundation prototype"));
}

#[test]
fn every_legacy_document_starts_with_a_superseded_warning() {
    for document in LEGACY {
        let first_lines = document.lines().take(8).collect::<Vec<_>>().join("\n");
        assert!(first_lines.contains("SUPERSEDED - DO NOT EXECUTE"));
        assert!(first_lines.contains("architecture.md"));
        assert!(first_lines.contains("phases.md"));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --test documentation_contract --locked`

Expected: FAIL because the README is stale and legacy files lack warnings.

- [ ] **Step 3: Rewrite the README for the canonical Rust slice**

Document current Phase 0 behavior, Rust 1.98.0, `cargo run`, required commands,
platform state location, explicit deferrals, and all four quality-gate commands.
Link `architecture.md`, `phases.md`, the approved design spec, and this plan.

- [ ] **Step 4: Add warnings without rewriting history**

Insert this block immediately after each legacy document title:

```markdown
> **SUPERSEDED - DO NOT EXECUTE**
>
> This document describes the retired Python/React/Hermes architecture. The
> canonical Rust design is [architecture.md](../../../architecture.md) and the
> active delivery roadmap is [phases.md](../../../phases.md).
```

Use correct relative links from each file. Do not otherwise edit historical text.

- [ ] **Step 5: Run focused tests**

Run: `cargo test --test documentation_contract --locked`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add README.md docs/superpowers tests/documentation_contract.rs
git commit -m "docs: supersede retired implementation plans"
```

---

### Task 14: Prove the Phase 0 exit gate and harden the complete slice

**Files:**
- Create: `tests/phase0_acceptance.rs`
- Modify only when a failing acceptance test exposes a specification gap: Phase 0 source/tests

**Interfaces:**
- Consumes: every public Phase 0 interface.
- Produces: one acceptance scenario and final quality-gate evidence.

- [ ] **Step 1: Write the failing end-to-end acceptance scenario**

```rust
// tests/phase0_acceptance.rs
mod support;

use std::io::Cursor;
use ai_stock_forum::ui::command::FallbackRunner;

#[test]
fn fresh_run_restart_and_audit_replay_satisfy_phase_zero() {
    let fixture = support::persistent_fixture();

    let first_runtime = fixture.runtime();
    let mut first_output = Vec::new();
    let first_reason = FallbackRunner::new(first_runtime.client(), false)
        .run(Cursor::new(b"/help\n/status\n/setup status\n/not-a-command\n/quit\n"), &mut first_output)
        .unwrap();
    first_runtime.finish_and_join(first_reason).unwrap();

    let installation = fixture.installation_id();
    let event_count = fixture.event_count_all();
    assert_eq!(fixture.count_rows("setup_drafts"), 0);
    assert_eq!(fixture.count_rows("installation_configuration_versions"), 0);

    let second_runtime = fixture.runtime();
    let mut second_output = Vec::new();
    let second_reason = FallbackRunner::new(second_runtime.client(), false)
        .run(Cursor::new(b"/status\n/audit tail 100\n/quit\n"), &mut second_output)
        .unwrap();
    second_runtime.finish_and_join(second_reason).unwrap();

    assert_eq!(fixture.installation_id(), installation);
    assert!(fixture.event_count_all() > event_count);
    let installation_text = installation.to_string();
    assert!(String::from_utf8(second_output).unwrap().contains(installation_text.as_str()));
    fixture.verify_event_stream().unwrap();
    fixture.assert_projection_matches_replay();
}
```

- [ ] **Step 2: Run the complete-slice acceptance test**

Run: `cargo test --test phase0_acceptance --locked`

Expected: PASS because Tasks 1-13 already implemented every asserted behavior.

- [ ] **Step 3: Run the acceptance and recovery suites**

Run:

```bash
cargo test --test phase0_acceptance --locked
cargo test --test recovery_contract --locked
cargo test --test fallback_contract --locked
```

Expected: PASS.

- [ ] **Step 4: Run the complete required gates**

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --locked
cargo build --workspace --locked
```

Expected: all commands exit zero with no lint warnings or failed tests.

- [ ] **Step 5: Prove tests are offline after dependency fetch**

Run:

```bash
cargo test --workspace --all-targets --locked --offline
```

Expected: PASS without network access.

- [ ] **Step 6: Commit the acceptance gate**

```bash
git add src tests Cargo.toml Cargo.lock
git commit -m "test: prove phase 0 exit gate"
```

- [ ] **Step 7: Request final code review**

Use `superpowers:requesting-code-review` against the design spec and this plan.
Resolve findings through `superpowers:receiving-code-review`, rerun the complete
quality gates, and create a focused follow-up commit for any accepted fixes.

---

## Task Dependency Order

```text
1 Toolchain/topology
  -> 2 Domain primitives
  -> 3 Commands/parser
  -> 4 Policy/approvals
  -> 5 Paths/permissions
  -> 6 SQLite migrations
  -> 7 Events/audit
  -> 8 Projections/setup models
  -> 9 Recovery/session lifecycle
  -> 10 Application service
  -> 11 Bounded runtime
  -> 12 Fallback host/binary
  -> 13 Documentation transition
  -> 14 Acceptance and quality gates
```

Tasks are intentionally sequential because each later public contract consumes
the exact types produced by earlier tasks. Parallel work is limited to review or
research that does not edit shared files.

## Execution Notes

- Execute in an isolated worktree created with `superpowers:using-git-worktrees`.
- The current main checkout contains rejected uncommitted prototype files; do not
  copy them into the worktree and do not delete them from the user's checkout.
- Start the worktree from the commit containing this plan and the approved spec.
- Use `superpowers:test-driven-development` for every task.
- Prefer `superpowers:subagent-driven-development` when multi-agent tools are
  available; otherwise use `superpowers:executing-plans` inline.
- Every task requires specification review first and code-quality review second
  before advancing.
