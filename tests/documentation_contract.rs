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

fn logical_markdown_units(document: &str) -> Vec<String> {
    fn flush(units: &mut Vec<String>, current: &mut String) {
        if !current.is_empty() {
            units.push(std::mem::take(current));
        }
    }

    let mut units = Vec::new();
    let mut current = String::new();
    let mut in_fence = false;
    for line in document.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            flush(&mut units, &mut current);
            in_fence = !in_fence;
        } else if trimmed.is_empty() {
            flush(&mut units, &mut current);
        } else if in_fence || trimmed.starts_with('|') || trimmed.starts_with('#') {
            flush(&mut units, &mut current);
            units.push(trimmed.to_owned());
        } else {
            let starts_list_item = trimmed.starts_with("- ")
                || trimmed
                    .split_once(". ")
                    .is_some_and(|(prefix, _)| prefix.bytes().all(|byte| byte.is_ascii_digit()));
            if starts_list_item {
                flush(&mut units, &mut current);
            } else if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(trimmed);
        }
    }
    flush(&mut units, &mut current);
    units
}

fn normalized_guidance_tokens(unit: &str) -> Vec<String> {
    let mut normalized = String::with_capacity(unit.len());
    for character in unit.chars() {
        if matches!(character, '`' | '\'' | '’') {
            continue;
        }
        for lower in character.to_lowercase() {
            if lower.is_alphanumeric() || matches!(lower, '/' | ':') {
                normalized.push(lower);
            } else {
                normalized.push(' ');
            }
        }
    }
    let mut tokens = Vec::new();
    for token in normalized.split_whitespace() {
        if token.starts_with(':') {
            tokens.push(token.to_owned());
        } else {
            tokens.extend(
                token
                    .split(':')
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    tokens
}

fn token_window_is_negated(tokens: &[String], left: usize, right: usize) -> bool {
    let start = left.saturating_sub(4);
    let end = right.saturating_add(6).min(tokens.len().saturating_sub(1));
    let window = &tokens[start..=end];
    window.iter().any(|token| {
        matches!(
            token.as_str(),
            "not"
                | "never"
                | "neither"
                | "no"
                | "inert"
                | "ignored"
                | "unsupported"
                | "cannot"
                | "cant"
                | "wont"
                | "doesnt"
                | "isnt"
                | "optional"
        )
    }) || window.windows(2).any(|pair| pair[0] == "no" && pair[1] == "longer")
}

fn unit_maps_bare_q_to_exit(tokens: &[String]) -> bool {
    const EXIT_WORDS: [&str; 6] = ["quit", "quits", "exit", "exits", "close", "closes"];
    for (q_index, _) in tokens.iter().enumerate().filter(|(_, token)| token.as_str() == "q") {
        for (verb_index, _) in tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| EXIT_WORDS.contains(&token.as_str()))
        {
            let distance = q_index.abs_diff(verb_index);
            if distance > 5 {
                continue;
            }
            let left = q_index.min(verb_index);
            let right = q_index.max(verb_index);
            let verb_belongs_to_slash_quit = verb_index
                .checked_sub(1)
                .and_then(|index| tokens.get(index))
                .is_some_and(|token| token == "/quit")
                || tokens[left..=right].iter().any(|token| token == "/quit");
            if !verb_belongs_to_slash_quit && !token_window_is_negated(tokens, left, right) {
                return true;
            }
        }
    }
    false
}

fn unit_requires_colon_skill_control(tokens: &[String]) -> bool {
    const OUTCOMES: [&str; 15] = [
        "continue", "continues", "continuing", "advance", "advances", "advanced", "finish",
        "finishes", "finished", "create", "creates", "created", "creating", "complete",
        "completed",
    ];
    const REQUIREMENTS: [&str; 11] = [
        "must", "required", "requires", "mandatory", "need", "needs", "use", "enter", "type",
        "press", "run",
    ];
    for (control_index, _) in tokens.iter().enumerate().filter(|(_, token)| {
        matches!(token.as_str(), ":next" | ":create")
    }) {
        for (outcome_index, _) in tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| OUTCOMES.contains(&token.as_str()))
        {
            let left = control_index.min(outcome_index);
            let right = control_index.max(outcome_index);
            let start = left.saturating_sub(4);
            let end = right.saturating_add(4).min(tokens.len().saturating_sub(1));
            let window = &tokens[start..=end];
            let requires = window
                .iter()
                .any(|token| REQUIREMENTS.contains(&token.as_str()))
                || window.windows(2).any(|pair| pair[0] == "have" && pair[1] == "to");
            if requires && !token_window_is_negated(tokens, left, right) {
                return true;
            }
        }
    }
    false
}

fn validate_control_guidance(document: &str) -> Result<(), String> {
    for unit in logical_markdown_units(document) {
        let tokens = normalized_guidance_tokens(&unit);
        if unit_maps_bare_q_to_exit(&tokens) {
            return Err(format!("bare q is mapped to shutdown: {unit}"));
        }
        if unit_requires_colon_skill_control(&tokens) {
            return Err(format!("colon control is required by Skills guidance: {unit}"));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct RoadmapStatusUnit {
    phase: Option<u8>,
    in_status_section: bool,
    text: String,
}

fn roadmap_status_units(document: &str) -> Vec<RoadmapStatusUnit> {
    fn flush(
        units: &mut Vec<RoadmapStatusUnit>,
        current: &mut String,
        phase: Option<u8>,
        in_status_section: bool,
    ) {
        if !current.is_empty() {
            let text = std::mem::take(current);
            units.extend(
                text.split(['.', '!', '?'])
                    .map(str::trim)
                    .filter(|sentence| !sentence.is_empty())
                    .map(|sentence| RoadmapStatusUnit {
                        phase,
                        in_status_section,
                        text: sentence.to_owned(),
                    }),
            );
        }
    }

    let mut units = Vec::new();
    let mut current = String::new();
    let mut phase = None;
    let mut in_status_section = false;
    for line in document.lines() {
        let trimmed = line.trim();
        if let Some(level) = heading_level(trimmed) {
            flush(&mut units, &mut current, phase, in_status_section);
            let tokens = normalized_guidance_tokens(trimmed);
            if level == 2 {
                phase = tokens
                    .windows(2)
                    .find(|pair| pair[0] == "phase")
                    .and_then(|pair| pair[1].parse::<u8>().ok());
                in_status_section = false;
            } else if level == 3 {
                in_status_section = tokens.iter().any(|token| token == "status");
            }
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut units, &mut current, phase, in_status_section);
            continue;
        }

        let starts_list_item = trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("+ ");
        if starts_list_item {
            flush(&mut units, &mut current, phase, in_status_section);
        } else if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(trimmed);
    }
    flush(&mut units, &mut current, phase, in_status_section);
    units
}

fn checkbox_status(text: &str) -> Option<bool> {
    let candidate = text
        .trim_start()
        .trim_start_matches(['-', '*', '+'])
        .trim_start();
    let marker = candidate.strip_prefix('[')?.split_once(']')?.0.trim();
    match marker.to_ascii_lowercase().as_str() {
        "x" => Some(true),
        "" => Some(false),
        _ => None,
    }
}

fn token_sequence_position(tokens: &[String], first: &str, second: &str) -> Option<usize> {
    tokens
        .windows(2)
        .position(|pair| pair[0] == first && pair[1] == second)
}

fn phase_three_completion_claim(unit: &RoadmapStatusUnit, tokens: &[String]) -> bool {
    const COMPLETE: [&str; 4] = ["complete", "completed", "shipped", "done"];
    let checkbox_complete = checkbox_status(&unit.text) == Some(true);
    if unit.phase == Some(3) && checkbox_complete {
        return true;
    }

    let explicit_phase = token_sequence_position(tokens, "phase", "3");
    for (completion_index, _) in tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| COMPLETE.contains(&token.as_str()))
    {
        if let Some(phase_index) = explicit_phase {
            let left = phase_index.min(completion_index);
            let right = phase_index.max(completion_index);
            if phase_index.abs_diff(completion_index) <= 5
                && !token_window_is_negated(tokens, left, right)
            {
                return true;
            }
        }

        let starts_status_claim = tokens.first().is_some_and(|token| token == "status")
            || tokens
                .windows(2)
                .next()
                .is_some_and(|pair| pair[0] == "this" && pair[1] == "phase");
        if unit.phase == Some(3)
            && (unit.in_status_section || starts_status_claim)
            && !token_window_is_negated(tokens, completion_index, completion_index)
        {
            return true;
        }
    }
    false
}

fn validate_phase_status_contract(document: &str) -> Result<(), String> {
    const COMPLETE: [&str; 4] = ["complete", "completed", "shipped", "done"];
    const PENDING: [&str; 4] = ["pending", "incomplete", "planned", "deferred"];
    let mut milestone_two_completions = 0;

    for unit in roadmap_status_units(document) {
        let tokens = normalized_guidance_tokens(&unit.text);
        if phase_three_completion_claim(&unit, &tokens) {
            return Err(format!("Phase 3 is documented as complete: {}", unit.text));
        }

        let Some(milestone_index) = token_sequence_position(&tokens, "milestone", "2") else {
            continue;
        };
        let checkbox = checkbox_status(&unit.text);
        let nearby_complete = tokens.iter().enumerate().any(|(index, token)| {
            COMPLETE.contains(&token.as_str())
                && milestone_index.abs_diff(index) <= 6
                && !token_window_is_negated(
                    &tokens,
                    milestone_index.min(index),
                    milestone_index.max(index),
                )
        });
        let negated_nearby_complete = tokens.iter().enumerate().any(|(index, token)| {
            COMPLETE.contains(&token.as_str())
                && milestone_index.abs_diff(index) <= 6
                && token_window_is_negated(
                    &tokens,
                    milestone_index.min(index),
                    milestone_index.max(index),
                )
        });
        let nearby_pending = tokens.iter().enumerate().any(|(index, token)| {
            PENDING.contains(&token.as_str()) && milestone_index.abs_diff(index) <= 6
        });
        let completed = checkbox == Some(true) || nearby_complete;
        let pending = checkbox == Some(false)
            || nearby_pending
            || negated_nearby_complete
            || tokens
                .windows(2)
                .enumerate()
                .any(|(index, pair)| {
                    pair[0] == "not"
                        && pair[1] == "started"
                        && milestone_index.abs_diff(index) <= 6
                });
        let status_statement = checkbox.is_some()
            || completed
            || pending
            || unit.in_status_section
            || tokens.iter().any(|token| token == "status");
        if !status_statement {
            continue;
        }
        if pending {
            return Err(format!("Milestone 2 has a pending status claim: {}", unit.text));
        }
        if completed {
            milestone_two_completions += 1;
        }
    }

    if milestone_two_completions != 1 {
        return Err(format!(
            "expected exactly one Milestone 2 completion status, found {milestone_two_completions}"
        ));
    }
    Ok(())
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

    for contradictory in [
        "Press `q` to exit.",
        "Q exits the app!",
        "| `q` | Close the application. |",
        "Use q to quit now.",
        "To close the cockpit, press q.",
        "You MUST use `:next` to continue.",
        "| `:create` | Required to finish creating the skill. |",
        "Enter :next to advance the Skills editor.",
        "To create the skill, type `:create`.",
    ] {
        assert!(
            validate_control_guidance(contradictory).is_err(),
            "contradiction was accepted: {contradictory}"
        );
    }

    for legitimate in [
        "Bare `q` is inert; `/quit` exits.",
        "`q` does not exit or close the app.",
        "`/quit` exits through normal shutdown.",
        "You never need `:next` or `:create` for this workflow.",
        "The historical `:next` control is optional and not required.",
        "Historical notes may say `q` once quit; that behavior is not supported.",
    ] {
        assert!(
            validate_control_guidance(legitimate).is_ok(),
            "legitimate guidance was rejected: {legitimate}"
        );
    }

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
    validate_phase_status_contract(PHASES).unwrap();
}

#[test]
fn roadmap_status_parser_detects_a_completed_phase_three_entry() {
    let incorrect = "## Phase 2 — Skills\n### Milestone status\n- [x] Milestone 2: Declarative skills.\n## Phase 3 — Chat\n### Milestone status\n- [x] Inference\n";
    assert!(validate_phase_status_contract(incorrect).is_err());
}

#[test]
fn roadmap_status_contract_rejects_conflicting_duplicate_phase_two_sections() {
    let conflicting = "# Roadmap\n## Phase 2 — Skills\n### Milestone status\n- [x] Milestone 2: Declarative skills. Complete.\n## Phase 2 — Duplicate\nStatus: Milestone 2 remains pending.\n## Phase 3 — Chat\nStatus: deferred.\n";
    assert!(validate_phase_status_contract(conflicting).is_err());

    let duplicate_complete = "## Phase 2 — Skills\n### Status\nMilestone 2 is complete.\n## Phase 2 — Duplicate\n### Status\nMilestone 2 shipped.\n## Phase 3 — Chat\nNot started.\n";
    assert!(validate_phase_status_contract(duplicate_complete).is_err());
}

#[test]
fn roadmap_status_contract_rejects_phase_three_completion_in_any_status_form() {
    for completed in [
        "## Phase 2 — Skills\n### Milestone status\n- [x] Milestone 2: Declarative skills.\n## Phase 3 — Chat\nPhase 3 is complete.\n",
        "## Phase 2 — Skills\n### Milestone status\n- [x] Milestone 2: Declarative skills.\n## Phase 3 — Chat\n### Delivery\nStatus: SHIPPED!\n",
        "## Phase 2 — Skills\n### Milestone status\n- [x] Milestone 2: Declarative skills.\n## Phase 3 — Chat\n- [x] Chat delivered\n",
        "## Phase 2 — Skills\n### Milestone status\n- [x] Milestone 2: Declarative skills.\n## Phase 3 — Chat\nPlanned.\n## Phase 3 — Duplicate\nThis phase is done.\n",
    ] {
        assert!(
            validate_phase_status_contract(completed).is_err(),
            "Phase 3 completion was accepted: {completed}"
        );
    }
}

#[test]
fn roadmap_status_contract_allows_valid_phase_three_deferral() {
    for valid in [
        "## Phase 2 — Skills\n### Milestone status\n- [x] Milestone 2: Declarative skills.\n- [ ] Milestone 3: Hybrid memory.\nPhase 2 remains in progress. Phase 3 remains pending.\n## Phase 3 — Chat\nStatus: planned.\n",
        "## Phase 2 — Skills\n### Status\nMilestone 2 is complete.\n## Phase 3 — Chat\nNot started; inference is deferred.\n",
        "## Phase 2 — Skills\n### Status\nMilestone 2 shipped.\n## Phase 3 — Chat\nPhase 3 completion is deferred.\n",
    ] {
        let result = validate_phase_status_contract(valid);
        assert!(
            result.is_ok(),
            "valid deferred roadmap was rejected: {valid}\n{result:?}"
        );
    }
}
