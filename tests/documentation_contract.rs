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
    "| `/help` | Outputs `Available commands:` followed by the complete supported Phase 0, Skills, and Hybrid Memory grammar; explicitly states that internal Memory producers are unavailable; commits `HelpViewed`. | Continues. |",
    "| `/status` | Outputs exactly `Installation: ready` and `Session: active`; commits `StatusViewed`. | Continues. |",
    "| `/audit tail` | Outputs `Audit tail (limit 20):` plus the selected entries or `No audit entries.`; commits `AuditTailViewed(limit=20)`. | Continues. |",
    "| `/audit tail N` | Outputs `Audit tail (limit N):` plus the selected entries or `No audit entries.` for `N` from 1 through 100; commits `AuditTailViewed(limit=N)`. | Continues. |",
    "| `/setup status` | Outputs exactly `Setup: not started` and `Guided setup is not implemented in Phase 0.` on a fresh installation; commits `SetupStatusViewed`. | Continues. |",
    "| `/quit` | Outputs exactly `Shutting down.`; commits `ShutdownRequested` and ends the session with `UserQuit`. | Ends normally. |",
];
const EXPECTED_MEMORY_COMMANDS: [&str; 11] = [
    "/memory list <agent>",
    "/memory get <agent> <key>",
    "/memory history <agent> <key> [positive-version]",
    "/memory set <agent> <key>",
    "/memory delete <agent> <key>",
    "/memory proposals <agent> [pending|all]",
    "/memory proposal <proposal-id>",
    "/memory approve <proposal-id>",
    "/memory reject <proposal-id>",
    "/memory episodes <agent>",
    "/memory episode <summary-id>",
];
const EXPECTED_NAVIGATION_DESTINATIONS: [&str; 9] = [
    "1 Home",
    "2 Chat",
    "3 Agents",
    "4 Skills",
    "5 Connections",
    "6 Activity",
    "7 Setup",
    "8 Audit",
    "9 Help",
];
const EXPECTED_MEMORY_ROUTE_NAMES: [&str; 11] = [
    "list",
    "get",
    "history",
    "set",
    "delete",
    "proposals",
    "proposal",
    "approve",
    "reject",
    "episodes",
    "episode",
];
const EXPECTED_MANUAL_SEED_LABELS: [&str; 7] = [
    "profile_id=",
    "tui_approval_proposal_id=",
    "tui_rejection_proposal_id=",
    "fallback_approval_proposal_id=",
    "fallback_rejection_proposal_id=",
    "restart_pending_proposal_id=",
    "summary_id=",
];
const EXPECTED_PRIVACY_BLOCK: &str = "Privacy warning: users must not enter secrets; Phase 0 has no supported secret, credential, or profile workflow.\n\nOn rejection, a bounded escaped first token, category, exact byte count, and SHA-256 digest may be persisted. Audit rendering may show the category, bounded safe token, and byte count; the digest and rejected full line are not rendered.";

fn read_repository_document(relative_path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path);
    fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "required documentation {} is unavailable: {error}",
            path.display()
        )
    })
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

fn fenced_text_lines(section: &str) -> Vec<&str> {
    let mut in_text_fence = false;
    let mut lines = Vec::new();
    for line in section.lines() {
        match line.trim() {
            "```text" => in_text_fence = true,
            "```" if in_text_fence => break,
            candidate if in_text_fence && !candidate.is_empty() => lines.push(candidate),
            _ => {}
        }
    }
    lines
}

fn markdown_table_commands(section: &str, prefix: &str) -> Vec<String> {
    section
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
                return None;
            }
            let code_start = trimmed.find('`')? + 1;
            let code_end = code_start + trimmed[code_start..].find('`')?;
            let command = trimmed[code_start..code_end].replace("\\|", "|");
            command.starts_with(prefix).then_some(command)
        })
        .collect()
}

fn inline_code_spans(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

fn assert_contains_all(document: &str, document_name: &str, required: &[&str]) {
    let normalized_document = document.split_whitespace().collect::<Vec<_>>().join(" ");
    for fragment in required {
        let normalized_fragment = fragment.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            normalized_document.contains(&normalized_fragment),
            "{document_name} is missing required Hybrid Memory guidance: {fragment}"
        );
    }
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
        if current.is_empty() {
            return;
        }

        let logical_unit = std::mem::take(current);
        let mut sentence_start = 0;
        let mut characters = logical_unit.char_indices().peekable();
        while let Some((punctuation_start, character)) = characters.next() {
            if !matches!(character, '.' | '!' | '?') {
                continue;
            }

            let mut punctuation_end = punctuation_start + character.len_utf8();
            let mut punctuation_count = 1;
            let mut only_periods = character == '.';
            while let Some(&(index, next)) = characters.peek() {
                if !matches!(next, '.' | '!' | '?') {
                    break;
                }
                characters.next();
                punctuation_end = index + next.len_utf8();
                punctuation_count += 1;
                only_periods &= next == '.';
            }

            let next_character = logical_unit[punctuation_end..]
                .chars()
                .find(|candidate| !candidate.is_whitespace());
            let continuing_ellipsis = only_periods
                && punctuation_count > 1
                && next_character.is_some_and(char::is_lowercase);
            if continuing_ellipsis {
                continue;
            }

            let sentence = logical_unit[sentence_start..punctuation_start].trim();
            if !sentence.is_empty() {
                units.push(sentence.to_owned());
            }
            sentence_start = punctuation_end;
        }

        let sentence = logical_unit[sentence_start..].trim();
        if !sentence.is_empty() {
            units.push(sentence.to_owned());
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
    }) || window
        .windows(2)
        .any(|pair| pair[0] == "no" && pair[1] == "longer")
}

fn token_slice_contains_phrase(
    tokens: &[String],
    start: usize,
    end: usize,
    phrase: &[&str],
) -> bool {
    !phrase.is_empty()
        && start < end
        && end <= tokens.len()
        && tokens[start..end]
            .windows(phrase.len())
            .any(|window| window.iter().map(String::as_str).eq(phrase.iter().copied()))
}

fn q_exit_relation_is_negated(tokens: &[String], q_index: usize, verb_index: usize) -> bool {
    const EXPLICIT_NEGATIONS: [&str; 9] = [
        "not", "never", "neither", "no", "cannot", "cant", "wont", "doesnt", "isnt",
    ];

    let left = q_index.min(verb_index);
    let right = q_index.max(verb_index);
    let before = &tokens[left.saturating_sub(3)..left];
    let local_end = (right + 7).min(tokens.len());
    let relation_and_effect = &tokens[left..local_end];

    before
        .iter()
        .any(|token| EXPLICIT_NEGATIONS.contains(&token.as_str()))
        || relation_and_effect.iter().any(|token| {
            EXPLICIT_NEGATIONS.contains(&token.as_str())
                || matches!(token.as_str(), "inert" | "ignored" | "unsupported")
        })
        || token_slice_contains_phrase(tokens, left, local_end, &["does", "nothing"])
        || token_slice_contains_phrase(tokens, left, local_end, &["has", "no", "effect"])
        || token_slice_contains_phrase(tokens, left, local_end, &["have", "no", "effect"])
}

fn control_relation_is_negated(tokens: &[String], start: usize, end: usize) -> bool {
    const EXPLICIT_NEGATIONS: [&str; 9] = [
        "not", "never", "neither", "no", "cannot", "cant", "wont", "doesnt", "isnt",
    ];

    let local_start = start.saturating_sub(2);
    let local_end = (end + 7).min(tokens.len());
    let local = &tokens[local_start..local_end];

    local.iter().any(|token| {
        EXPLICIT_NEGATIONS.contains(&token.as_str())
            || matches!(token.as_str(), "optional" | "unsupported")
    }) || token_slice_contains_phrase(tokens, local_start, local_end, &["does", "nothing"])
        || token_slice_contains_phrase(tokens, local_start, local_end, &["has", "no", "effect"])
        || token_slice_contains_phrase(tokens, local_start, local_end, &["have", "no", "effect"])
}

fn unit_maps_bare_q_to_exit(tokens: &[String]) -> bool {
    const EXIT_WORDS: [&str; 6] = ["quit", "quits", "exit", "exits", "close", "closes"];
    for (q_index, _) in tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.as_str() == "q")
    {
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
            if !verb_belongs_to_slash_quit
                && !q_exit_relation_is_negated(tokens, q_index, verb_index)
            {
                return true;
            }
        }
    }
    false
}

fn unit_requires_colon_skill_control(tokens: &[String]) -> bool {
    const OUTCOMES: [&str; 15] = [
        "continue",
        "continues",
        "continuing",
        "advance",
        "advances",
        "advanced",
        "finish",
        "finishes",
        "finished",
        "create",
        "creates",
        "created",
        "creating",
        "complete",
        "completed",
    ];
    const REQUIREMENTS: [&str; 11] = [
        "must",
        "required",
        "requires",
        "mandatory",
        "need",
        "needs",
        "use",
        "enter",
        "type",
        "press",
        "run",
    ];
    for (control_index, _) in tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| matches!(token.as_str(), ":next" | ":create"))
    {
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
            let requirement_index = (start..=end)
                .filter(|index| REQUIREMENTS.contains(&tokens[*index].as_str()))
                .min_by_key(|index| index.abs_diff(control_index));
            let have_to_index = window
                .windows(2)
                .position(|pair| pair[0] == "have" && pair[1] == "to")
                .map(|offset| start + offset);
            let relation_requirement = requirement_index.or(have_to_index);
            if relation_requirement.is_some_and(|requirement_index| {
                let relation_start = requirement_index.min(left);
                !control_relation_is_negated(tokens, relation_start, right)
            }) {
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
            return Err(format!(
                "colon control is required by Skills guidance: {unit}"
            ));
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

        let starts_list_item =
            trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ");
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
            || tokens.windows(2).enumerate().any(|(index, pair)| {
                pair[0] == "not" && pair[1] == "started" && milestone_index.abs_diff(index) <= 6
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
            return Err(format!(
                "Milestone 2 has a pending status claim: {}",
                unit.text
            ));
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
        "inert, bounded context",
        "cannot execute",
        "shell",
        "filesystem",
        "Git",
        "MCP",
        "provider",
        "network",
        "Inference and chat begin in Phase 3",
    ] {
        assert!(
            introduction.contains(required),
            "guide introduction is missing: {required}"
        );
    }

    let version_model = markdown_section(&guide, "## Library and version model");
    for required in [
        "Evidence Review",
        "Filing Analysis",
        "Catalyst Mapping",
        "Risk Checklist",
        "immutable version",
        "exact version",
        "does not auto-upgrade",
        "historical version",
        "explicit upgrade",
        "Unassign",
    ] {
        assert!(
            version_model.contains(required),
            "version model is missing: {required}"
        );
    }
}

#[test]
fn keyboard_guide_matches_the_shipped_pane_specific_controller_contract() {
    let guide = read_repository_document("docs/testing/declarative-skills.md");
    let keyboard = markdown_section(&guide, "## Keyboard-first workflow");
    let panes = markdown_section(&guide, "### Pane controls");

    assert!(keyboard.contains("press bare `4` to open Skills"));
    assert!(keyboard.contains("Bare `1`-`9` navigate from non-text browsing panes"));
    assert!(keyboard.contains("characters remain text"));
    assert!(keyboard.contains("pending confirmation"));
    assert!(!keyboard.contains("Option+"));
    assert!(!keyboard.contains("Alt+"));
    assert!(keyboard.contains("You never need `:next` or `:create`"));
    validate_pane_keys(
        panes,
        "Library",
        &["Up/Down", "skill rows"],
        &["Left/Right"],
    )
    .unwrap();
    validate_pane_keys(
        panes,
        "Create source",
        &["Up/Down", "starting point"],
        &["Left/Right"],
    )
    .unwrap();
    validate_pane_keys(
        panes,
        "Detail actions",
        &["Left/Right", "action"],
        &["Up/Down"],
    )
    .unwrap();
    validate_pane_keys(
        panes,
        "History",
        &["Up/Down", "version rows"],
        &["Left/Right"],
    )
    .unwrap();
    validate_pane_keys(
        panes,
        "Agent picker",
        &["Up/Down", "agent rows"],
        &["Left/Right"],
    )
    .unwrap();
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
    validate_pane_keys(panes, "Confirmation", &["Enter", "commits"], &["validates"]).unwrap();
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
    for required in [
        "stages a review",
        "does not mutate directly",
        "Bare `q` is inert",
        "`/quit` exits",
    ] {
        assert!(
            slash.contains(required),
            "slash section is missing: {required}"
        );
    }
}

#[test]
fn slash_and_control_validators_reject_unsupported_or_contradictory_guidance() {
    let unsupported = "```text\n/skill list\n/skills\n/skill add\n/skill show <name-or-id> [version]\n/skill assign <skill> <agent> [version]\n/skill unassign <skill> <agent>\n/skill delete everything\n```";
    assert!(validate_skill_slash_commands(unsupported).is_err());

    for contradictory in [
        "Bare q is inert. Press q to exit.",
        "You never need :next. Press :next to continue.",
        "Bare **`q`** is INERT?!\nPress   `Q`... to EXIT!!!",
        "You NEVER need `:next`.\nPress **`:NEXT`** to continue!",
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
        "Press q to exit does nothing.",
        "Press `q`; it does nothing.",
        "You never need `:next` to continue.",
        "Use `/quit` to exit; bare `q` remains inert.",
        "Press q to close has no effect.",
        "The q key is ignored and has no effect.",
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
        assert!(
            recovery.contains(required),
            "recovery section is missing: {required}"
        );
    }

    let compact = markdown_section(&guide, "## Compact terminal expectations");
    for required in ["compact terminal", "60x18", "Enter", "Esc", "bare `q`"] {
        assert!(
            compact.contains(required),
            "compact section is missing: {required}"
        );
    }

    let commands = markdown_section(&guide, "## Exact local commands");
    for required in [
        "cargo test --test documentation_contract --test topology_contract",
        "cargo build --release --locked",
        "XDG_DATA_HOME",
    ] {
        assert!(
            commands.contains(required),
            "local commands section is missing: {required}"
        );
    }

    let checklist = markdown_section(&guide, "## Manual acceptance checklist");
    for required in [
        "Create a custom skill",
        "Create version 2",
        "Explicitly upgrade",
        "History",
        "Unassign",
        "stale review",
        "Restart",
    ] {
        assert!(
            checklist.contains(required),
            "manual checklist is missing: {required}"
        );
    }
}

#[test]
fn readme_points_to_the_detailed_declarative_skills_guide() {
    let sources = markdown_section(README, "## Sources of truth");
    assert!(sources.contains(
        "[Declarative Skills testing and workflow guide](docs/testing/declarative-skills.md)"
    ));
    let milestone = markdown_section(README, "## Phase 2 Declarative Skills Milestone 2");
    let required = "Inference and chat remain deferred to Phase 3";
    assert!(
        milestone.contains(required),
        "README milestone is missing: {required}"
    );
}

#[test]
fn readme_documents_hybrid_memory_plaintext_review_and_exact_fallback_grammar() {
    let introduction = README.split("\n## ").next().expect("README introduction");
    assert_contains_all(
        introduction,
        "README introduction",
        &[
            "Phase 2 Hybrid Memory Milestone 3",
            "local reviewed memory",
            "durable agent proposals",
            "source-qualified episodic summaries",
            "bounded deterministic retrieval",
            "without adding inference or chat",
        ],
    );

    let cockpit = markdown_section(README, "## Phase 0B Adaptive Cockpit");
    assert_contains_all(
        cockpit,
        "README Adaptive Cockpit navigation summary",
        &[
            "The current top navigation has exactly nine destinations: Home, Chat, Agents, Skills, Connections, Activity, Setup, Audit, and Help",
            "Widths from 60 through 99 show one logical pane",
        ],
    );
    assert!(!cockpit.contains("The cockpit has four native, non-transcript views"));

    let sources = markdown_section(README, "## Sources of truth");
    assert_contains_all(
        sources,
        "README Sources of truth",
        &[
            "[Phase 2 Hybrid Memory design](docs/superpowers/specs/2026-09-07-phase-2-hybrid-memory-design.md)",
            "[Phase 2 Hybrid Memory testing guide](docs/testing/phase-2-hybrid-memory.md)",
        ],
    );

    let milestone = markdown_section(README, "## Phase 2 Hybrid Memory Milestone 3");
    assert_contains_all(
        milestone,
        "README Hybrid Memory milestone",
        &[
            "Agents → Memory",
            "Direct Human edits",
            "reviewed before commit",
            "pending proposals",
            "distinct Human approval",
            "does not encrypt Hybrid Memory at rest",
            "owner-only permissions",
            "access control, not encryption",
            "Entry and history rows",
            "proposals and rationales",
            "summaries and source links",
            "mutation events",
            "request/outcome receipts",
            "SQLite WAL/journal sidecars",
            "copied backups",
            "Deliberate detail views display plaintext",
            "bounded credential deny-list",
            "best-effort",
            "cannot prove that text contains no secret",
            "Delete adds a tombstone",
            "not secure erasure",
            "Immutable versions, events, and receipts accumulate monotonically",
            "repeated edits consume additional local capacity",
            "SQLite may reuse pages",
            "BEGIN IMMEDIATE",
            "no partial",
            "Summary — verify sources",
            "bounded deterministic retrieval",
            "Startup",
            "fail closed",
            "There is no dedicated Memory destination",
            "bare `m`",
            "Modified shortcuts are inert",
            "typed shortcut characters remain editor text",
            "`?` remains a Help alias",
            "not a destination label",
        ],
    );
    let deferred = markdown_section(README, "### Recovery and deferred Hybrid Memory boundaries");
    assert_contains_all(
        deferred,
        "README Hybrid Memory deferred features",
        &[
            "remain deferred",
            "Inference/chat",
            "automatic memory extraction",
            "semantic/vector search",
            "embeddings",
            "autonomous proposal generation",
            "remote sync",
            "encryption at rest",
            "credential vaulting",
            "retention pruning",
            "secure erasure",
            "proposal creation",
            "summary mutation",
            "snapshot presentation",
        ],
    );
    assert_contains_all(
        deferred,
        "README production Memory boundary",
        &[
            "no production proposal creator",
            "summary writer",
            "snapshot presentation route",
        ],
    );
    for destination in EXPECTED_NAVIGATION_DESTINATIONS {
        assert!(
            milestone.contains(destination),
            "README Hybrid Memory milestone is missing navigation destination: {destination}"
        );
    }
    assert_eq!(
        fenced_text_lines(milestone),
        EXPECTED_NAVIGATION_DESTINATIONS
    );

    let fallback = markdown_section(README, "### Hybrid Memory fallback commands");
    assert_eq!(fenced_commands(fallback), EXPECTED_MEMORY_COMMANDS);
    assert_contains_all(
        fallback,
        "README Hybrid Memory fallback commands",
        &[
            "There is no production `/memory propose`",
            "summary-write",
            "snapshot",
        ],
    );
    assert!(!fallback.contains("proposal-uuid"));
    assert!(!fallback.contains("summary-uuid"));

    let supported = markdown_section(README, "## Supported commands");
    assert_eq!(
        markdown_table_commands(supported, "/memory "),
        EXPECTED_MEMORY_COMMANDS
    );

    let storage = markdown_section(README, "## Storage, security, and privacy");
    assert_contains_all(
        storage,
        "README storage and privacy section",
        &[
            "Hybrid Memory",
            "local plaintext",
            "not encrypted at rest",
            "Current and historical entries",
            "proposals and rationales",
            "summaries and source links",
            "mutation events",
            "receipts",
            "SQLite WAL/journal sidecars",
            "deliberate detail displays",
            "copied backups",
            "Owner-only filesystem permissions",
            "not encryption",
            "bounded credential deny-list",
            "best-effort",
            "cannot prove",
            "immutable tombstone",
            "does not securely erase earlier versions",
            "repeated edits consume capacity",
            "SQLite may reuse pages",
        ],
    );

    let non_goals = markdown_section(README, "## Explicit non-goals");
    assert_contains_all(
        non_goals,
        "README explicit non-goals",
        &[
            "automatic memory extraction",
            "semantic/vector search",
            "autonomous proposal generation",
            "production proposal/summary/snapshot producer routes",
        ],
    );
    assert!(!non_goals.contains("do not add hybrid memory"));
}

#[test]
fn architecture_documents_hybrid_memory_authority_atomicity_and_recovery() {
    let architecture = read_repository_document("architecture.md");
    let section_twelve = markdown_section(&architecture, "## 12. Skills, personality, and memory");
    assert_eq!(
        section_twelve
            .matches("### Hybrid Memory durability and recovery")
            .count(),
        1
    );
    assert_eq!(
        architecture
            .matches("### Hybrid Memory durability and recovery")
            .count(),
        1
    );
    assert!(!architecture.contains("\n## Hybrid Memory durability and recovery\n"));
    assert!(!architecture.contains("\n### Memory\n"));
    assert!(
        section_twelve
            .find("### Hybrid Memory durability and recovery")
            .expect("Hybrid Memory architecture heading")
            < section_twelve
                .find("### Room context and resume")
                .expect("Room context architecture heading")
    );

    let memory = markdown_section(&architecture, "### Hybrid Memory durability and recovery");
    assert_contains_all(
        memory,
        "architecture Hybrid Memory section",
        &[
            "stable agent memory namespace",
            "different or copied profile",
            "fresh empty namespace",
            "memory-scope-v1",
            "General",
            "Tagged",
            "immutable application event stream",
            "authoritative",
            "request/outcome receipts",
            "rebuildable pointers",
            "Generic audit",
            "local plaintext",
            "Application-managed durable records are local plaintext",
            "current entry and history rows",
            "proposals and rationales",
            "summaries and source links",
            "mutation events",
            "SQLite WAL/journal sidecars",
            "deliberate detail output",
            "Copied backups can retain plaintext wherever stored",
            "Owner-only permissions are access control, not encryption",
            "Credentials are prohibited",
            "bounded credential deny-list",
            "best-effort",
            "cannot prove",
            "tombstone",
            "not secure erasure",
            "retain prior plaintext in immutable history",
            "Immutable versions, events, and receipts accumulate monotonically",
            "repeated edits consume additional local capacity",
            "SQLite may reuse pages",
            "BEGIN IMMEDIATE",
            "Human",
            "Agent",
            "System",
            "a Human may directly set/delete and may Approve or Reject",
            "an Agent may only create a proposal for its own exact profile identity",
            "System cannot mutate or resolve memory",
            "process-local",
            "one-use",
            "pending approval",
            "without modifying an entry",
            "Approve",
            "Reject",
            "capacity/full-disk",
            "cannot leave a partial",
            "event",
            "entry",
            "proposal",
            "approval",
            "pointer",
            "projection",
            "audit record",
            "receipt",
            "canonical request fingerprint",
            "never repeats current retrieval selection",
            "deterministically ordered",
            "fixed byte/item limits",
            "source references",
            "Summary — verify sources",
            "safe startup refusal",
            "altered",
            "unexplained",
        ],
    );
    assert!(!memory.contains("Every application-managed durable copy is local plaintext"));
    assert!(!memory.contains("copied backups remain sensitive plaintext"));

    let production_boundary = markdown_section(memory, "#### Production and deferred boundary");
    assert_contains_all(
        production_boundary,
        "architecture production and deferred boundary",
        &[
            "no production proposal creator",
            "summary writer",
            "snapshot route",
            "automatic extraction",
            "semantic/vector search",
            "embeddings",
            "autonomous proposal generation",
            "inference/chat",
            "remote sync",
            "encryption at rest",
            "credential vault",
            "retention pruning",
            "secure erasure",
        ],
    );

    let command_mode = markdown_section(
        &architecture,
        "## 8. Full-screen TUI and fallback command mode",
    );
    let memory_command_row = command_mode
        .lines()
        .find(|line| line.trim_start().starts_with("| `/memory"))
        .expect("architecture command table must document /memory routes");
    let command_cell = memory_command_row
        .trim()
        .trim_start_matches('|')
        .split('|')
        .next()
        .expect("architecture /memory command cell");
    let route_names = inline_code_spans(command_cell)
        .into_iter()
        .map(|span| span.strip_prefix("/memory ").unwrap_or(span))
        .collect::<Vec<_>>();
    assert_eq!(route_names, EXPECTED_MEMORY_ROUTE_NAMES);

    let persistence = markdown_section(&architecture, "## 15. Persistence, audit, and secrets");
    assert_contains_all(
        persistence,
        "architecture persistence section",
        &[
            "accepted memory mutation events intentionally retain approved plaintext",
            "provider/runtime credentials",
            "SQLite stores only opaque references",
        ],
    );
    let persistence_units = logical_markdown_units(persistence);
    assert!(!persistence_units.iter().any(|unit| {
        unit == "Operational events are append-only and have stable IDs, actor, timestamp, correlation ID, object version/digest, and redacted payload"
    }));
    assert!(
        !persistence_units
            .iter()
            .any(|unit| unit == "SQLite stores only opaque references and safe labels")
    );
    assert!(!architecture.contains(
        "The next step after review is to write a detailed implementation plan for Phase 0."
    ));
}

#[test]
fn hybrid_memory_acceptance_guide_is_safe_exact_and_reproducible() {
    let manual = read_repository_document("docs/testing/phase-2-hybrid-memory.md");
    assert_contains_all(
        &manual,
        "Hybrid Memory acceptance guide",
        &[
            "# Phase 2 Hybrid Memory acceptance guide",
            "disposable local state",
            "does not encrypt Hybrid Memory at rest",
            "owner-only permissions",
            "access control, not encryption",
            "synthetic text only",
            "never enter credentials",
            "cargo build --release --locked",
            "cargo test --test hybrid_memory_acceptance",
            "cargo test --test tui_navigation_contract",
            "seed_manual_acceptance_state",
            "seed_manual_memory_acceptance",
            "final application-state directory is absent",
            "parent exists",
            "before any live launch",
            "exactly once",
            "identical discovered state path",
            "atomically claiming the directory",
            "do not retry that directory",
            "fresh target",
            "restart_pending_proposal_id",
            "retain",
            "Exit every process before cleanup",
            "Move only the exact disposable directory to Trash",
            "directories` v6 derives its normal path from `HOME`",
            "safety policy",
            "unperformed/blocked",
            "create a distinct profile",
            "allowed non-memory fields",
            "authoritative proof",
            "fresh empty namespace",
            "60x18",
            "100x24",
            "28-column list",
            "140x30",
            "clamped 36-column list",
            "Modified shortcuts are inert",
            "typed shortcut characters remain editor text",
            "Bare `q` is inert",
            "`/quit`",
            "Summary — verify sources",
            "memory_recovery_contract",
            "memory_atomicity_contract",
            "exact commit",
            "host and disposable account/environment",
            "seeder labels and IDs only",
            "exact terminal dimensions",
            "cleanup/retention state",
            "every check not performed",
        ],
    );
    let automated = markdown_section(&manual, "## Automated gate");
    assert_contains_all(
        automated,
        "Hybrid Memory automated gate",
        &[
            "cargo build --release --locked",
            "cargo test --test hybrid_memory_acceptance",
            "cargo test --test tui_navigation_contract",
            "support::seed_manual_memory_acceptance",
            "record_test_episodic_summary_at",
        ],
    );

    let disposable = markdown_section(&manual, "## Disposable state");
    assert_contains_all(
        disposable,
        "Hybrid Memory disposable-state procedure",
        &[
            "AI_STOCK_FORUM_MEMORY_ACCEPTANCE_STATE_DIR",
            "final application-state directory is absent",
            "parent exists",
            "before any live launch",
            "exactly once",
            "identical discovered state path",
            "atomically claiming the directory",
            "do not retry that directory",
            "fresh target",
        ],
    );
    assert_eq!(fenced_text_lines(disposable), EXPECTED_MANUAL_SEED_LABELS);
    for label in EXPECTED_MANUAL_SEED_LABELS {
        assert!(
            manual.contains(label),
            "Hybrid Memory acceptance guide is missing seeder label: {label}"
        );
    }
    for destination in EXPECTED_NAVIGATION_DESTINATIONS {
        assert!(
            manual.contains(destination),
            "Hybrid Memory acceptance guide is missing navigation destination: {destination}"
        );
    }
    let layout = markdown_section(&manual, "## Layout, shortcuts, shutdown, and cleanup");
    assert_eq!(fenced_text_lines(layout), EXPECTED_NAVIGATION_DESTINATIONS);
    assert_contains_all(
        layout,
        "Hybrid Memory layout and shortcut procedure",
        &[
            "`60x18`: one-pane Memory layout",
            "`100x24`",
            "28-column list and 72-column active workspace",
            "`140x30`",
            "clamped 36-column list and 104-column active workspace",
            "bare `m`",
            "`?` remains a Help alias",
            "type `wasd123456789`",
            "Bare `q` is inert",
            "`/quit` requests normal shutdown",
            "exactly one review cancellation",
            "terminal restoration",
            "Exit every process before cleanup",
            "Move only the exact disposable directory to Trash",
            "Do not use a recursive deletion command",
        ],
    );

    let direct = markdown_section(&manual, "## Direct memory and immutable history");
    assert_contains_all(
        direct,
        "Hybrid Memory direct mutation procedure",
        &[
            "cancel once",
            "no entry exists",
            "set <review-digest>",
            "versions 2 and 1 are newest-first",
            "version-1 detail",
            "delete <review-digest>",
            "tombstone versions",
            "list output omits values",
            "same typed identity, version, state, and digest",
        ],
    );

    let proposals = markdown_section(&manual, "## Proposals and episodic summaries");
    assert_contains_all(
        proposals,
        "Hybrid Memory proposal and episodic procedure",
        &[
            "tui_approval_proposal_id",
            "cancel its review once",
            "one accepted entry version",
            "one terminal approval",
            "tui_rejection_proposal_id",
            "Reject must change no entry",
            "yes",
            "opposite-action attempts must not commit",
            "restart_pending_proposal_id",
            "retain the fifth pending proposal through restart",
            "Summary — verify sources",
            "Generic Help, Status, Audit, navigation, and safe error output",
        ],
    );

    let recovery = markdown_section(&manual, "## Isolation, restart, recovery, and capacity");
    assert_contains_all(
        recovery,
        "Hybrid Memory isolation and recovery procedure",
        &[
            "never cross either profile direction",
            "create a distinct profile",
            "fresh empty namespace",
            "retains the original namespace and memory",
            "two accepted and two rejected proposal resolutions",
            "still pending with its exact approval identity",
            "Drafts and process-local review tokens must not survive restart",
            "The automated acceptance test separately proves that command-looking text survives restart",
            "memory_recovery_contract",
            "memory_atomicity_contract",
            "byte-identical before and after failure",
            "same exact review can retry",
            "no partial durable state",
        ],
    );
    assert!(!recovery.contains("command-looking text are unchanged"));

    let evidence = markdown_section(&manual, "## Evidence record");
    assert_contains_all(
        evidence,
        "Hybrid Memory evidence record",
        &[
            "exact commit tested",
            "host and disposable account/environment",
            "exact disposable-state arrangement",
            "seeder labels and IDs only",
            "exact terminal dimensions",
            "automated, focused, full-suite, and release gate summaries",
            "each manual outcome",
            "cleanup/retention state",
            "every check not performed",
        ],
    );
    assert!(!manual.contains("/agent duplicate"));
    assert!(!manual.contains("rm -rf"));
    assert!(!manual.contains("rm -r "));
    assert!(!manual.contains("Remove-Item -Recurse"));
}

#[test]
fn profile_foundation_guide_uses_current_navigation_and_shutdown_controls() {
    let guide = read_repository_document("docs/testing/phase-2-agent-profile-foundation.md");
    assert_contains_all(
        &guide,
        "Agent Profile Foundation testing guide",
        &[
            "keyboard-first",
            "There is no dedicated Memory destination",
            "Modified shortcuts are inert",
            "typed shortcut characters remain editor text",
            "Bare `q` is inert",
            "`/quit` requests normal shutdown",
        ],
    );
    for destination in EXPECTED_NAVIGATION_DESTINATIONS {
        assert!(
            guide.contains(destination),
            "Agent Profile Foundation testing guide is missing navigation destination: {destination}"
        );
    }
    let adaptive = markdown_section(&guide, "## Specialized isolated TUI persistence flow");
    assert_eq!(
        fenced_text_lines(adaptive),
        EXPECTED_NAVIGATION_DESTINATIONS
    );
    for obsolete_control in [":next", ":review", ":create", ":activate", ":cancel"] {
        assert!(
            !guide.contains(obsolete_control),
            "Agent Profile Foundation testing guide still documents obsolete colon control: {obsolete_control}"
        );
    }
    validate_control_guidance(&guide).unwrap();
}

#[test]
fn roadmap_marks_only_declarative_skills_complete() {
    let phase_two = markdown_section(
        PHASES,
        "## Phase 2 — Agent profiles, skills, and hybrid memory",
    );
    let milestone_status = markdown_section(PHASES, "### Milestone status");
    for required in [
        "[x] **Milestone 2: Declarative skills.**",
        "[ ] **Milestone 3: Hybrid memory.**",
        "Phase 2 as a whole remains in progress",
        "Phase 3 remains pending",
    ] {
        assert!(
            phase_two.contains(required),
            "Phase 2 roadmap is missing: {required}"
        );
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
