use ai_stock_forum::{
    app::{AgentProfileSelector, ApplicationCommand},
    ui::command::{
        AgentWorkflowCommand, FallbackParsedLine, ParsedLine, parse_fallback_line, parse_line,
    },
};

#[test]
fn canonical_slash_agent_commands_accept_quoted_name_selectors() {
    assert!(matches!(
        parse_line(b"/agent list"),
        ParsedLine::Command(ApplicationCommand::ListAgentProfiles)
    ));

    let ParsedLine::Command(ApplicationCommand::ShowAgentProfile { selector }) =
        parse_line("/agent show \"Ma\u{00df}e Desk\"".as_bytes())
    else {
        panic!("canonical show command was not parsed");
    };
    assert_eq!(selector.display_name(), Some("Ma\u{00df}e Desk"));

    let ParsedLine::Command(ApplicationCommand::ShowAgentProfileHistory { selector }) =
        parse_line(b"/agent history \"Bull Researcher\"")
    else {
        panic!("canonical history command was not parsed");
    };
    assert!(matches!(selector, AgentProfileSelector::Name(_)));
}

#[test]
fn fallback_keeps_documented_bare_aliases() {
    assert!(matches!(
        parse_fallback_line(b"agent list"),
        FallbackParsedLine::Command(ApplicationCommand::ListAgentProfiles)
    ));
    assert!(matches!(
        parse_fallback_line(b"agent show \"Bull Researcher\""),
        FallbackParsedLine::Command(ApplicationCommand::ShowAgentProfile {
            selector: AgentProfileSelector::Name(_)
        })
    ));
}

#[test]
fn short_and_full_template_aliases_resolve_to_canonical_template_ids() {
    for (alias, expected) in [
        ("bull", "builtin.bull"),
        ("bear", "builtin.bear"),
        ("chief", "builtin.chief"),
        ("engineering", "builtin.engineering"),
        ("custom", "builtin.custom"),
        ("builtin.bull", "builtin.bull"),
    ] {
        let command = format!("/agent create {alias}");
        let ParsedLine::AgentWorkflow(AgentWorkflowCommand::Create { template_id }) =
            parse_line(command.as_bytes())
        else {
            panic!("template alias {alias} was not parsed");
        };
        assert_eq!(template_id.as_str(), expected);
    }
}

#[test]
fn history_command_can_select_an_exact_version() {
    let ParsedLine::Command(ApplicationCommand::ShowAgentProfileVersion { selector, version }) =
        parse_line(b"/agent history \"Bull Researcher\" 2")
    else {
        panic!("exact historical version command was not parsed");
    };
    assert_eq!(selector.display_name(), Some("Bull Researcher"));
    assert_eq!(version.get(), 2);
}

#[test]
fn edit_workflow_carries_a_name_or_id_selector() {
    let ParsedLine::AgentWorkflow(AgentWorkflowCommand::Edit { selector }) =
        parse_line(b"/agent edit \"Bull Researcher\"")
    else {
        panic!("name-selected edit was not parsed");
    };
    assert_eq!(selector.display_name(), Some("Bull Researcher"));
}
