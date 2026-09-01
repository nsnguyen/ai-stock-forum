const README: &str = include_str!("../README.md");
const DESIGN: &str =
    include_str!("../docs/superpowers/specs/2026-08-31-phase-0-rust-foundation-design.md");
const LEGACY: [&str; 4] = [
    include_str!("../docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md"),
    include_str!("../docs/superpowers/plans/2026-08-09-ai-stock-forum-roadmap.md"),
    include_str!("../docs/superpowers/plans/2026-08-09-phase-1-deterministic-risk-core.md"),
    include_str!("../docs/superpowers/plans/2026-08-15-phase-0a-workspace-contract-foundation.md"),
];

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
        "`%LOCALAPPDATA%\\ai-stock-forum\\`",
        "`phase0-bootstrap.lock`",
        "Windows runtime verification has not been performed",
        "events remain authoritative for audit and projections",
        "receipts are durable command-idempotency evidence",
        "there is no supported secret, credential, or profile workflow",
        "Do not enter secrets.",
        "Rejected full lines are not stored verbatim",
        "a bounded escaped first token, exact byte count, and SHA-256 digest may be persisted and shown in audit",
        "## Explicit non-goals",
        "cargo fmt --all --check",
        "cargo clippy --workspace --all-targets --all-features -- -D warnings",
        "cargo test --workspace --all-targets --locked",
        "cargo build --workspace --locked",
    ] {
        assert!(README.contains(required), "README is missing: {required}");
    }

    for command in [
        "| `/help` |",
        "| `/status` |",
        "| `/audit tail` |",
        "| `/audit tail N` |",
        "| `/setup status` |",
        "| `/quit` |",
    ] {
        assert!(README.contains(command), "README is missing CLI form: {command}");
    }

    assert!(!README.contains("docs/superpowers/specs/2026-08-08-ai-stock-forum-design.md"));
    assert!(!README.contains("Phase-0 foundation prototype"));
    assert!(!README.contains("uv run"));
    assert!(!README.contains("npm run"));
    assert!(!README.contains("make verify"));
    assert!(!README.contains("podman"));
    assert!(!README.contains("/Users/nguyen-mini"));
    assert!(!README.contains("/private/tmp"));
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
        assert!(DESIGN.contains(required), "canonical design is missing: {required}");
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
