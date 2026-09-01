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
