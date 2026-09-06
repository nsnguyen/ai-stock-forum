use crate::app::{
    AgentProfileSelector, ApplicationCommand, DEFAULT_AUDIT_LIMIT, InputRejection,
    InputRejectionCategory, MAX_INPUT_BYTES, SafeToken, SkillSelector,
};
use crate::{
    agents::{ProfileTemplateId, builtin_profile_templates},
    domain::ObjectVersion,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentWorkflowCommand {
    SelectCreateTemplate,
    Create { template_id: ProfileTemplateId },
    Edit { selector: AgentProfileSelector },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillWorkflowCommand {
    Add,
    Assign {
        skill: SkillSelector,
        agent: AgentProfileSelector,
        version: Option<ObjectVersion>,
    },
    Unassign {
        skill: SkillSelector,
        agent: AgentProfileSelector,
    },
}

#[expect(
    clippy::large_enum_variant,
    reason = "fallback parsing returns owned typed commands without a second allocation contract"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackParsedLine {
    Command(ApplicationCommand),
    AgentWorkflow(AgentWorkflowCommand),
    SkillWorkflow(SkillWorkflowCommand),
    Ignored,
}

#[expect(
    clippy::large_enum_variant,
    reason = "command parsing returns owned typed commands without a second allocation contract"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedLine {
    Command(ApplicationCommand),
    AgentWorkflow(AgentWorkflowCommand),
    SkillWorkflow(SkillWorkflowCommand),
    Ignored,
}

pub fn parse_line(input: &[u8]) -> ParsedLine {
    match parse_fallback_line(input) {
        FallbackParsedLine::Command(command) => ParsedLine::Command(command),
        FallbackParsedLine::AgentWorkflow(command) => ParsedLine::AgentWorkflow(command),
        FallbackParsedLine::SkillWorkflow(command) => ParsedLine::SkillWorkflow(command),
        FallbackParsedLine::Ignored => ParsedLine::Ignored,
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

    let Some(tokens) = tokenize(line) else {
        return FallbackParsedLine::Command(reject(
            InputRejectionCategory::Malformed,
            safe_token(line),
            input,
        ));
    };
    let tokens = tokens.iter().map(String::as_str).collect::<Vec<_>>();

    let command = match tokens.as_slice() {
        ["agent" | "/agent", "list"] => ApplicationCommand::ListAgentProfiles,
        ["agent" | "/agent", "show", selector] => {
            match AgentProfileSelector::from_input(selector) {
                Ok(selector) => ApplicationCommand::ShowAgentProfile { selector },
                Err(_) => reject(InputRejectionCategory::Malformed, safe_token(line), input),
            }
        }
        ["agent" | "/agent", "history", selector] => {
            match AgentProfileSelector::from_input(selector) {
                Ok(selector) => ApplicationCommand::ShowAgentProfileHistory { selector },
                Err(_) => reject(InputRejectionCategory::Malformed, safe_token(line), input),
            }
        }
        ["agent" | "/agent", "history", selector, version] => {
            let parsed = AgentProfileSelector::from_input(selector).and_then(|selector| {
                version
                    .parse::<u64>()
                    .ok()
                    .ok_or(crate::domain::DomainError::InvalidObjectVersion)
                    .and_then(ObjectVersion::new)
                    .map(|version| ApplicationCommand::ShowAgentProfileVersion {
                        selector,
                        version,
                    })
            });
            parsed.unwrap_or_else(|_| {
                reject(InputRejectionCategory::Malformed, safe_token(line), input)
            })
        }
        ["agent" | "/agent", "create"] => {
            return FallbackParsedLine::AgentWorkflow(AgentWorkflowCommand::SelectCreateTemplate);
        }
        ["agent" | "/agent", "create", template_id] => {
            let canonical_id = match *template_id {
                "bull" | "bear" | "chief" | "engineering" | "custom" => {
                    format!("builtin.{template_id}")
                }
                value => value.to_owned(),
            };
            let template_id = builtin_profile_templates()
                .iter()
                .find(|template| template.id.as_str() == canonical_id)
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
        ["agent" | "/agent", "edit", selector] => {
            return match AgentProfileSelector::from_input(selector) {
                Ok(selector) => {
                    FallbackParsedLine::AgentWorkflow(AgentWorkflowCommand::Edit { selector })
                }
                Err(_) => FallbackParsedLine::Command(reject(
                    InputRejectionCategory::Malformed,
                    safe_token(line),
                    input,
                )),
            };
        }
        ["/skill", "list"] | ["/skills"] => ApplicationCommand::ListSkills,
        ["/skill", "show", selector] => match SkillSelector::from_input(selector) {
            Ok(selector) => ApplicationCommand::ShowSkill { selector },
            Err(_) => reject(InputRejectionCategory::Malformed, safe_token(line), input),
        },
        ["/skill", "show", selector, version] => {
            let parsed = SkillSelector::from_input(selector).and_then(|selector| {
                positive_version(version).map(|version| ApplicationCommand::ShowSkillVersion {
                    selector,
                    version,
                })
            });
            parsed.unwrap_or_else(|_| {
                reject(InputRejectionCategory::Malformed, safe_token(line), input)
            })
        }
        ["/skill", "add"] => {
            return FallbackParsedLine::SkillWorkflow(SkillWorkflowCommand::Add);
        }
        ["/skill", "assign", skill, agent] => {
            let parsed = SkillSelector::from_input(skill).and_then(|skill| {
                AgentProfileSelector::from_input(agent).map(|agent| SkillWorkflowCommand::Assign {
                    skill,
                    agent,
                    version: None,
                })
            });
            return parsed.map_or_else(
                |_| {
                    FallbackParsedLine::Command(reject(
                        InputRejectionCategory::Malformed,
                        safe_token(line),
                        input,
                    ))
                },
                FallbackParsedLine::SkillWorkflow,
            );
        }
        ["/skill", "assign", skill, agent, version] => {
            let parsed = SkillSelector::from_input(skill).and_then(|skill| {
                AgentProfileSelector::from_input(agent).and_then(|agent| {
                    positive_version(version).map(|version| SkillWorkflowCommand::Assign {
                        skill,
                        agent,
                        version: Some(version),
                    })
                })
            });
            return parsed.map_or_else(
                |_| {
                    FallbackParsedLine::Command(reject(
                        InputRejectionCategory::Malformed,
                        safe_token(line),
                        input,
                    ))
                },
                FallbackParsedLine::SkillWorkflow,
            );
        }
        ["/skill", "unassign", skill, agent] => {
            let parsed = SkillSelector::from_input(skill).and_then(|skill| {
                AgentProfileSelector::from_input(agent)
                    .map(|agent| SkillWorkflowCommand::Unassign { skill, agent })
            });
            return parsed.map_or_else(
                |_| {
                    FallbackParsedLine::Command(reject(
                        InputRejectionCategory::Malformed,
                        safe_token(line),
                        input,
                    ))
                },
                FallbackParsedLine::SkillWorkflow,
            );
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
            "agent" | "/agent" | "/skill" | "/skills" | "/help" | "/status" | "/setup" | "/audit" | "/quit",
            ..,
        ] => reject(InputRejectionCategory::Malformed, safe_token(line), input),
        _ => reject(InputRejectionCategory::Unknown, safe_token(line), input),
    };

    FallbackParsedLine::Command(command)
}

fn positive_version(value: &str) -> Result<ObjectVersion, crate::domain::DomainError> {
    value
        .parse::<u64>()
        .ok()
        .ok_or(crate::domain::DomainError::InvalidObjectVersion)
        .and_then(ObjectVersion::new)
}

fn tokenize(line: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut token_started = false;

    for character in line.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                token_started = true;
            }
            character if character.is_whitespace() && !quoted => {
                if token_started {
                    tokens.push(std::mem::take(&mut token));
                    token_started = false;
                }
            }
            character => {
                token.push(character);
                token_started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if token_started {
        tokens.push(token);
    }
    Some(tokens)
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
