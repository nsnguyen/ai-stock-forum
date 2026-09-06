use std::any::{TypeId, type_name};

use ai_stock_forum::{agents, domain, skills};

const LIB_RS: &str = include_str!("../src/lib.rs");
const SKILLS_MOD_RS: &str = include_str!("../src/skills/mod.rs");

fn module_declarations(source: &str, public: bool) -> Vec<&str> {
    let prefix = if public { "pub mod " } else { "mod " };
    source
        .lines()
        .filter_map(|line| {
            let declaration = line.trim().strip_prefix(prefix)?.strip_suffix(';')?;
            (!declaration.is_empty()
                && declaration
                    .bytes()
                    .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric()))
            .then_some(declaration)
        })
        .collect()
}

#[test]
fn phase_two_profiles_and_declarative_skills_preserve_the_approved_module_boundaries() {
    assert_eq!(
        module_declarations(LIB_RS, true),
        [
            "agents", "app", "audit", "cli", "config", "domain", "domains", "jobs", "mcp",
            "memory", "persistence", "policy", "providers", "recovery", "rooms", "runtime",
            "runtimes", "setup", "skills", "ui",
        ],
    );
    assert_eq!(
        module_declarations(SKILLS_MOD_RS, false),
        ["builtin", "normalization", "projection", "retrieval", "review", "skill"]
    );
    assert!(module_declarations(SKILLS_MOD_RS, true).is_empty());

    assert_eq!(type_name::<skills::SkillDraft>(), "ai_stock_forum::skills::skill::SkillDraft");
    assert_eq!(
        type_name::<skills::SkillVersionRef>(),
        "ai_stock_forum::skills::skill::SkillVersionRef"
    );
    assert_ne!(TypeId::of::<skills::SkillDraft>(), TypeId::of::<agents::AgentProfileDraft>());
    assert_ne!(
        TypeId::of::<skills::SkillVersionRef>(),
        TypeId::of::<domain::AgentProfileVersionId>()
    );
}

#[test]
fn module_parser_ignores_comments_reexports_and_non_declarations() {
    let fixture = "// pub mod commented;\npub use nested::Thing;\nmod private;\npub mod real_boundary;\n";
    assert_eq!(module_declarations(fixture, true), ["real_boundary"]);
    assert_eq!(module_declarations(fixture, false), ["private"]);
}
