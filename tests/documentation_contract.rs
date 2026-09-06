use std::{fs, path::Path};

const README: &str = include_str!("../README.md");
const PHASES: &str = include_str!("../phases.md");
const DESIGN: &str =
    include_str!("../docs/superpowers/specs/2026-08-31-phase-0-rust-foundation-design.md");
const LEGACY: [&str; 4] = [
    include_str!("../docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md"),
    include_str!("../docs/superpowers/plans/2026-08-09-ai-stock-forum-roadmap.md"),
    include_str!("../docs/superpowers/plans/2026-08-09-phase-1-deterministic-risk-core.md"),
    include_str!("../docs/superpowers/plans/2026-08-15-phase-0a-workspace-contract-foundation.md"),
];
const EXPECTED_COMMAND_ROWS: [&str; 6] = [
    "| `/help` | Outputs `Available commands:` followed by `/help`, `/status`, `/setup status`, `/audit tail [limit: 1-100]`, and `/quit`; commits `HelpViewed`. | Continues. |",
    "| `/status` | Outputs exactly `Installation: ready` and `Session: active`; commits `StatusViewed`. | Continues. |",
    "| `/audit tail` | Outputs `Audit tail (limit 20):` plus the selected entries or `No audit entries.`; commits `AuditTailViewed(limit=20)`. | Continues. |",
    "| `/audit tail N` | Outputs `Audit tail (limit N):` plus the selected entries or `No audit entries.` for `N` from 1 through 100; commits `AuditTailViewed(limit=N)`. | Continues. |",
    "| `/setup status` | Outputs exactly `Setup: not started` and `Guided setup is not implemented in Phase 0.` on a fresh installation; commits `SetupStatusViewed`. | Continues. |",
    "| `/quit` | Outputs exactly `Shutting down.`; commits `ShutdownRequested` and ends the session with `UserQuit`. | Ends normally. |",
];
const EXPECTED_PRIVACY_BLOCK: &str = "Privacy warning: users must not enter secrets; Phase 0 has no supported secret, credential, or profile workflow.\n\nOn rejection, a bounded escaped first token, category, exact byte count, and SHA-256 digest may be persisted. Audit rendering may show the category, bounded safe token, and byte count; the digest and rejected full line are not rendered.";

fn read_repository_document(relative_path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("required documentation {} is unavailable: {error}", path.display()))
}

#[test]
fn readme_is_the_canonical_phase_zero_rust_guide() {
    for required in [
        "[Architecture](architecture.md)",
        "[Delivery phases](phases.md)",
        "[Approved design specification](docs/superpowers/specs/2026-08-31-phase-0-rust-foundation-design.md)",
        "[Phase 0 implementation plan](docs/superpowers/plans/2026-08-31-phase-0-rust-foundation.md)",
        "Rust `1.98.0`",
        "database schema version `1`",
        "event schema version `1`",
        "`0700`",
        "`0600`",
        "`~/Library/Application Support/ai-stock-forum/`",
        "`$XDG_DATA_HOME/ai-stock-forum/`",
        "`~/.local/share/ai-stock-forum/`",
        "`%APPDATA%\\ai-stock-forum\\`",
        "`%APPDATA%\\ai-stock-forum\\ai-stock-forum.sqlite3`",
        "`%APPDATA%\\ai-stock-forum\\phase0-bootstrap.lock`",
        "`phase0-bootstrap.lock`",
        "Windows runtime verification has not been performed",
        "events remain authoritative for audit and projections",
        "receipts are durable command-idempotency evidence",
        "## Explicit non-goals",
        "cargo fmt --all --check",
        "cargo clippy --workspace --all-targets --all-features -- -D warnings",
        "cargo test --workspace --all-targets --locked",
        "cargo build --workspace --locked",
    ] {
        assert!(README.contains(required), "README is missing: {required}");
    }

    for row in EXPECTED_COMMAND_ROWS {
        assert!(
            README.contains(row),
            "README is missing exact CLI row: {row}"
        );
    }

    assert!(README.contains(EXPECTED_PRIVACY_BLOCK));

    assert!(!README.contains("docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md"));
    assert!(!README.contains("Phase-0 foundation prototype"));
    assert!(!README.contains("uv run"));
    assert!(!README.contains("npm run"));
    assert!(!README.contains("make verify"));
    assert!(!README.contains("podman"));
    assert!(!README.contains("/Users/nguyen-mini"));
    assert!(!README.contains("/private/tmp"));
    assert!(!README.contains("%LOCALAPPDATA%"));
    assert!(!README.contains("digest may be persisted and shown in audit"));
    assert!(!README.contains("digest is shown in audit"));
}

#[test]
fn canonical_design_records_the_prerelease_receipt_ruling() {
    for required in [
        "`command_receipts`",
        "`command_event_refs`",
        "immutable command receipts",
        "ordered command-event references",
        "before release",
        "Events remain authoritative for audit and projections",
        "receipts are durable command-idempotency evidence",
        "pre-receipt unreleased development databases",
        "recreation",
        "checksum mismatch",
    ] {
        assert!(
            DESIGN.contains(required),
            "canonical design is missing: {required}"
        );
    }
    assert!(!DESIGN.contains("/Users/nguyen-mini"));
    assert!(!DESIGN.contains("/private/tmp"));
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

#[test]
fn declarative_skills_guide_preserves_the_inert_versioned_workflow_contract() {
    let guide = read_repository_document("docs/testing/declarative-skills.md");

    for required in [
        "inert, bounded context",
        "cannot execute",
        "shell",
        "filesystem",
        "Git",
        "MCP",
        "provider",
        "network",
        "Evidence Review",
        "Filing Analysis",
        "Catalyst Mapping",
        "Risk Checklist",
        "immutable version",
        "exact version",
        "does not auto-upgrade",
        "historical version",
        "explicit upgrade",
        "unassign",
        "Press `s`",
        "`Up` and `Down`",
        "`Enter`",
        "`Esc`",
        "You never need `:next` or `:create`",
        "Bare `q` is inert",
        "`/quit` exits",
        "stages a review",
        "does not mutate directly",
        "cancel",
        "stale",
        "rejected",
        "restart",
        "compact terminal",
        "Inference and chat begin in Phase 3",
    ] {
        assert!(guide.contains(required), "Skills guide is missing: {required}");
    }

    for command in [
        "/skill list",
        "/skills",
        "/skill add",
        "/skill show <name-or-id> [version]",
        "/skill assign <skill> <agent> [version]",
        "/skill unassign <skill> <agent>",
        "cargo test --test documentation_contract --test topology_contract",
        "cargo build --release --locked",
    ] {
        assert!(guide.contains(command), "Skills guide is missing command: {command}");
    }
}

#[test]
fn readme_points_to_the_detailed_declarative_skills_guide() {
    for required in [
        "Phase 2 Declarative Skills Milestone 2",
        "[Declarative Skills testing and workflow guide](docs/testing/declarative-skills.md)",
        "Inference and chat remain deferred to Phase 3",
    ] {
        assert!(README.contains(required), "README is missing: {required}");
    }
}

#[test]
fn roadmap_marks_only_declarative_skills_complete() {
    for required in [
        "[x] **Milestone 2: Declarative skills.**",
        "[ ] **Milestone 3: Hybrid memory.**",
        "Phase 2 as a whole remains in progress",
        "Phase 3 remains pending",
    ] {
        assert!(PHASES.contains(required), "roadmap is missing: {required}");
    }

    assert!(!PHASES.contains(
        "[ ] **Milestone 2: Declarative skills.** Versioned skill manifests, assignment,\n  retrieval, and presentation remain deferred."
    ));
}
