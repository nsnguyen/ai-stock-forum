use crate::app::{
    ApplicationCommand, DEFAULT_AUDIT_LIMIT, InputRejection, InputRejectionCategory,
    MAX_INPUT_BYTES, SafeToken,
};
use crate::{
    agents::{ProfileTemplateId, builtin_profile_templates},
    domain::AgentProfileId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentWorkflowCommand {
    SelectCreateTemplate,
    Create { template_id: ProfileTemplateId },
    Edit { profile_id: AgentProfileId },
}

#[expect(
    clippy::large_enum_variant,
    reason = "fallback parsing returns owned typed commands without a second allocation contract"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackParsedLine {
    Command(ApplicationCommand),
    AgentWorkflow(AgentWorkflowCommand),
    Ignored,
}

#[expect(
    clippy::large_enum_variant,
    reason = "command parsing returns owned typed commands without a second allocation contract"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedLine {
    Command(ApplicationCommand),
    Ignored,
}

pub fn parse_line(input: &[u8]) -> ParsedLine {
    if input.len() <= MAX_INPUT_BYTES
        && std::str::from_utf8(input)
            .ok()
            .and_then(|line| line.split_whitespace().next())
            == Some("agent")
    {
        let line = std::str::from_utf8(input).expect("agent token requires valid UTF-8");
        return ParsedLine::Command(reject(
            InputRejectionCategory::Unknown,
            safe_token(line),
            input,
        ));
    }
    match parse_fallback_line(input) {
        FallbackParsedLine::Command(command) => ParsedLine::Command(command),
        FallbackParsedLine::Ignored => ParsedLine::Ignored,
        FallbackParsedLine::AgentWorkflow(_) => unreachable!("agent input returned above"),
    }
}

pub fn parse_fallback_line(input: &[u8]) -> FallbackParsedLine {
    if input.len() > MAX_INPUT_BYTES {
        return FallbackParsedLine::Command(reject(InputRejectionCategory::Oversized, None, input));
    }

    let line = match std::str::from_utf8(input) {
        Ok(line) => line,
        Err(_) => {
            return FallbackParsedLine::Command(reject(
                InputRejectionCategory::InvalidEncoding,
                None,
                input,
            ));
        }
    };
    let line = line.trim();

    if line.is_empty() {
        return FallbackParsedLine::Ignored;
    }

    let command = match line.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["agent", "list"] => ApplicationCommand::ListAgentProfiles,
        ["agent", "show", profile_id] => match profile_id.parse::<AgentProfileId>() {
            Ok(profile_id) => ApplicationCommand::ShowAgentProfile { profile_id },
            Err(_) => reject(InputRejectionCategory::Malformed, safe_token(line), input),
        },
        ["agent", "history", profile_id] => match profile_id.parse::<AgentProfileId>() {
            Ok(profile_id) => ApplicationCommand::ShowAgentProfileHistory { profile_id },
            Err(_) => reject(InputRejectionCategory::Malformed, safe_token(line), input),
        },
        ["agent", "create"] => {
            return FallbackParsedLine::AgentWorkflow(AgentWorkflowCommand::SelectCreateTemplate);
        }
        ["agent", "create", template_id] => {
            let template_id = builtin_profile_templates()
                .iter()
                .find(|template| template.id.as_str() == *template_id)
                .map(|template| template.id.clone());
            return match template_id {
                Some(template_id) => {
                    FallbackParsedLine::AgentWorkflow(AgentWorkflowCommand::Create { template_id })
                }
                None => FallbackParsedLine::Command(reject(
                    InputRejectionCategory::Malformed,
                    safe_token(line),
                    input,
                )),
            };
        }
        ["agent", "edit", profile_id] => {
            return match profile_id.parse::<AgentProfileId>() {
                Ok(profile_id) => {
                    FallbackParsedLine::AgentWorkflow(AgentWorkflowCommand::Edit { profile_id })
                }
                Err(_) => FallbackParsedLine::Command(reject(
                    InputRejectionCategory::Malformed,
                    safe_token(line),
                    input,
                )),
            };
        }
        ["/help"] => ApplicationCommand::ShowHelp,
        ["/status"] => ApplicationCommand::ShowStatus,
        ["/setup", "status"] => ApplicationCommand::ShowSetupStatus,
        ["/audit", "tail"] => ApplicationCommand::audit_tail(DEFAULT_AUDIT_LIMIT)
            .expect("default audit limit is within the supported range"),
        ["/audit", "tail", limit] => match limit.parse::<u16>() {
            Ok(limit) => ApplicationCommand::audit_tail(limit).unwrap_or_else(|_| {
                reject(InputRejectionCategory::Malformed, safe_token(line), input)
            }),
            Err(_) => reject(InputRejectionCategory::Malformed, safe_token(line), input),
        },
        ["/quit"] => ApplicationCommand::RequestShutdown,
        [
            "agent" | "/help" | "/status" | "/setup" | "/audit" | "/quit",
            ..,
        ] => reject(InputRejectionCategory::Malformed, safe_token(line), input),
        _ => reject(InputRejectionCategory::Unknown, safe_token(line), input),
    };

    FallbackParsedLine::Command(command)
}

fn reject(
    category: InputRejectionCategory,
    safe_token: Option<SafeToken>,
    input: &[u8],
) -> ApplicationCommand {
    ApplicationCommand::RejectInput(InputRejection::from_input(category, safe_token, input))
}

fn safe_token(line: &str) -> Option<SafeToken> {
    line.split_whitespace().next().and_then(|token| {
        let mut escaped_token = String::new();
        let mut output_scalar_count = 0;

        for character in token.chars() {
            let escaped_fragment = character.escape_default().to_string();
            let fragment_scalar_count = escaped_fragment.chars().count();
            if output_scalar_count + fragment_scalar_count > 64 {
                break;
            }

            escaped_token.push_str(&escaped_fragment);
            output_scalar_count += fragment_scalar_count;
        }

        SafeToken::new(escaped_token).ok()
    })
}
