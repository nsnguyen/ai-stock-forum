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

fn heading_level(line: &str) -> Option<usize> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    (level > 0 && line.as_bytes().get(level) == Some(&b' ')).then_some(level)
}

fn markdown_section<'a>(document: &'a str, heading: &str) -> &'a str {
    let marker = format!("{heading}\n");
    let start = document
        .find(&marker)
        .unwrap_or_else(|| panic!("missing Markdown heading: {heading}"))
        + marker.len();
    let level = heading_level(heading).expect("test heading must be valid Markdown");
    let remainder = &document[start..];
    let mut consumed = 0;
    for line in remainder.split_inclusive('\n') {
        let candidate = line.trim_end_matches(['\r', '\n']);
        if heading_level(candidate).is_some_and(|candidate_level| candidate_level <= level) {
            return &remainder[..consumed];
        }
        consumed += line.len();
    }
    remainder
}

fn markdown_table_row<'a>(section: &'a str, first_cell: &str) -> Option<Vec<&'a str>> {
    section.lines().find_map(|line| {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
            return None;
        }
        let cells = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect::<Vec<_>>();
        (cells.first() == Some(&first_cell)).then_some(cells)
    })
}

fn validate_pane_keys(
    pane_section: &str,
    pane: &str,
    required: &[&str],
    forbidden: &[&str],
) -> Result<(), String> {
    let row = markdown_table_row(pane_section, pane)
        .ok_or_else(|| format!("missing pane-control row: {pane}"))?
        .join(" | ");
    for key in required {
        if !row.contains(key) {
            return Err(format!("{pane} does not document {key}"));
        }
    }
    for key in forbidden {
        if row.contains(key) {
            return Err(format!("{pane} incorrectly documents {key}"));
        }
    }
    Ok(())
}

fn fenced_commands(section: &str) -> Vec<&str> {
    let mut in_text_fence = false;
    let mut commands = Vec::new();
    for line in section.lines() {
        match line.trim() {
            "```text" => in_text_fence = true,
            "```" if in_text_fence => break,
            candidate if in_text_fence && candidate.starts_with('/') => commands.push(candidate),
            _ => {}
        }
    }
    commands
}

fn validate_skill_slash_commands(section: &str) -> Result<(), String> {
    const EXPECTED: [&str; 6] = [
        "/skill list",
        "/skills",
        "/skill add",
        "/skill show <name-or-id> [version]",
        "/skill assign <skill> <agent> [version]",
        "/skill unassign <skill> <agent>",
    ];
    let actual = fenced_commands(section);
    (actual == EXPECTED)
        .then_some(())
        .ok_or_else(|| format!("unsupported or missing Skills slash command: {actual:?}"))
}

fn validate_control_guidance(document: &str) -> Result<(), String> {
    for line in document.lines() {
        let normalized = line.to_ascii_lowercase();
        for contradiction in [
            "press `q` to quit",
            "bare `q` exits",
            "`q` exits",
            "`q` quits",
            "`q` to quit",
        ] {
            if normalized.contains(contradiction) {
                return Err(format!("contradictory quit guidance: {line}"));
            }
        }

        if normalized.contains("`:next`") || normalized.contains("`:create`") {
            let requires_control = normalized.contains("must use")
                || normalized.contains("required")
                || normalized.contains("require ")
                || normalized.contains("need to use");
            let explicitly_optional = normalized.contains("never need")
                || normalized.contains("not required")
                || normalized.contains("does not require");
            if requires_control && !explicitly_optional {
                return Err(format!("colon control is incorrectly required: {line}"));
            }
        }
    }
    Ok(())
}

fn checked_roadmap_statuses(document: &str) -> Vec<(u8, &str)> {
    let mut phase = None;
    let mut in_milestone_status = false;
    let mut checked = Vec::new();
    for line in document.lines() {
        if let Some(rest) = line.strip_prefix("## Phase ") {
            phase = rest
                .split_whitespace()
                .next()
                .and_then(|number| number.parse::<u8>().ok());
            in_milestone_status = false;
        } else if line.starts_with("### ") {
            in_milestone_status = line == "### Milestone status";
        } else if in_milestone_status
            && line.starts_with("- [x] ")
            && let Some(phase) = phase
        {
            checked.push((phase, line));
        }
    }
    checked
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
fn declarative_skills_sections_preserve_capability_and_version_boundaries() {
    let guide = read_repository_document("docs/testing/declarative-skills.md");
    let introduction = guide.split("\n## ").next().expect("guide introduction");
    for required in [
        "inert, bounded context", "cannot execute", "shell", "filesystem", "Git", "MCP",
        "provider", "network", "Inference and chat begin in Phase 3",
    ] {
        assert!(introduction.contains(required), "guide introduction is missing: {required}");
    }

    let version_model = markdown_section(&guide, "## Library and version model");
    for required in [
        "Evidence Review", "Filing Analysis", "Catalyst Mapping", "Risk Checklist",
        "immutable version", "exact version", "does not auto-upgrade", "historical version",
        "explicit upgrade", "Unassign",
    ] {
        assert!(version_model.contains(required), "version model is missing: {required}");
    }
}

#[test]
fn keyboard_guide_matches_the_shipped_pane_specific_controller_contract() {
    let guide = read_repository_document("docs/testing/declarative-skills.md");
    let keyboard = markdown_section(&guide, "## Keyboard-first workflow");
    let panes = markdown_section(&guide, "### Pane controls");

    assert!(keyboard.contains("Press `s`"));
    assert!(keyboard.contains("You never need `:next` or `:create`"));
    validate_pane_keys(panes, "Library", &["Up/Down", "skill rows"], &["Left/Right"]).unwrap();
    validate_pane_keys(panes, "Create source", &["Up/Down", "starting point"], &["Left/Right"]).unwrap();
    validate_pane_keys(panes, "Detail actions", &["Left/Right", "action"], &["Up/Down"]).unwrap();
    validate_pane_keys(panes, "History", &["Up/Down", "version rows"], &["Left/Right"]).unwrap();
    validate_pane_keys(panes, "Agent picker", &["Up/Down", "agent rows"], &["Left/Right"]).unwrap();
    validate_pane_keys(
        panes,
        "Agent assigned skills",
        &["Up/Down", "pinned skill rows", "Left/Right", "actions"],
        &[],
    )
    .unwrap();
}

#[test]
fn review_and_confirmation_are_documented_as_distinct_controller_states() {
    let guide = read_repository_document("docs/testing/declarative-skills.md");
    let panes = markdown_section(&guide, "### Pane controls");
    validate_pane_keys(
        panes,
        "Review",
        &["Enter", "validates", "opens Confirmation"],
        &["commits"],
    )
    .unwrap();
    validate_pane_keys(
        panes,
        "Confirmation",
        &["Enter", "commits"],
        &["validates"],
    )
    .unwrap();
}

#[test]
fn pane_key_validator_rejects_up_down_for_action_selection() {
    let incorrect = "| Pane | Selection | Enter | Esc |\n| --- | --- | --- | --- |\n| Detail actions | Up/Down selects action | Opens | Back |\n";
    assert!(
        validate_pane_keys(
            incorrect,
            "Detail actions",
            &["Left/Right", "action"],
            &["Up/Down"],
        )
        .is_err()
    );
}

#[test]
fn optional_slash_section_contains_only_the_supported_skill_commands() {
    let guide = read_repository_document("docs/testing/declarative-skills.md");
    let slash = markdown_section(&guide, "## Optional slash fallbacks");
    validate_skill_slash_commands(slash).unwrap();
    for required in ["stages a review", "does not mutate directly", "Bare `q` is inert", "`/quit` exits"] {
        assert!(slash.contains(required), "slash section is missing: {required}");
    }
}

#[test]
fn slash_and_control_validators_reject_unsupported_or_contradictory_guidance() {
    let unsupported = "```text\n/skill list\n/skills\n/skill add\n/skill show <name-or-id> [version]\n/skill assign <skill> <agent> [version]\n/skill unassign <skill> <agent>\n/skill delete everything\n```";
    assert!(validate_skill_slash_commands(unsupported).is_err());
    assert!(validate_control_guidance("Press `q` to quit.").is_err());
    assert!(validate_control_guidance("You must use `:next` to continue.").is_err());

    let guide = read_repository_document("docs/testing/declarative-skills.md");
    validate_control_guidance(&guide).unwrap();
}

#[test]
fn workflow_recovery_compact_and_local_test_sections_remain_complete() {
    let guide = read_repository_document("docs/testing/declarative-skills.md");
    let recovery = markdown_section(&guide, "## Confirmation, cancellation, and recovery");
    for required in ["cancel", "rejected", "stale", "restart", "review"] {
        assert!(recovery.contains(required), "recovery section is missing: {required}");
    }

    let compact = markdown_section(&guide, "## Compact terminal expectations");
    for required in ["compact terminal", "60x18", "Enter", "Esc", "bare `q`"] {
        assert!(compact.contains(required), "compact section is missing: {required}");
    }

    let commands = markdown_section(&guide, "## Exact local commands");
    for required in [
        "cargo test --test documentation_contract --test topology_contract",
        "cargo build --release --locked",
        "XDG_DATA_HOME",
    ] {
        assert!(commands.contains(required), "local commands section is missing: {required}");
    }

    let checklist = markdown_section(&guide, "## Manual acceptance checklist");
    for required in [
        "Create a custom skill", "Create version 2", "Explicitly upgrade", "History",
        "Unassign", "stale review", "Restart",
    ] {
        assert!(checklist.contains(required), "manual checklist is missing: {required}");
    }
}

#[test]
fn readme_points_to_the_detailed_declarative_skills_guide() {
    let sources = markdown_section(README, "## Sources of truth");
    assert!(sources.contains(
        "[Declarative Skills testing and workflow guide](docs/testing/declarative-skills.md)"
    ));
    let milestone = markdown_section(README, "## Phase 2 Declarative Skills Milestone 2");
    for required in ["Inference and chat remain deferred to Phase 3"] {
        assert!(milestone.contains(required), "README milestone is missing: {required}");
    }
}

#[test]
fn roadmap_marks_only_declarative_skills_complete() {
    let phase_two = markdown_section(PHASES, "## Phase 2 — Agent profiles, skills, and hybrid memory");
    let milestone_status = markdown_section(PHASES, "### Milestone status");
    for required in [
        "[x] **Milestone 2: Declarative skills.**",
        "[ ] **Milestone 3: Hybrid memory.**",
        "Phase 2 as a whole remains in progress",
        "Phase 3 remains pending",
    ] {
        assert!(phase_two.contains(required), "Phase 2 roadmap is missing: {required}");
    }
    assert_eq!(milestone_status.matches("- [x] ").count(), 2);
    assert_eq!(milestone_status.matches("- [ ] ").count(), 1);
    assert!(checked_roadmap_statuses(PHASES).iter().all(|(phase, _)| *phase <= 2));
    assert!(checked_roadmap_statuses(PHASES).iter().all(|(phase, _)| *phase != 3));
}

#[test]
fn roadmap_status_parser_detects_a_completed_phase_three_entry() {
    let incorrect = "## Phase 2 — Skills\n### Milestone status\n- [x] Skills\n## Phase 3 — Chat\n### Milestone status\n- [x] Inference\n";
    assert!(checked_roadmap_statuses(incorrect).iter().any(|(phase, _)| *phase == 3));
}
