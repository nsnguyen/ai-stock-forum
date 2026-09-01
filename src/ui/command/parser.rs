use crate::app::{
    ApplicationCommand, InputRejection, InputRejectionCategory, SafeToken, DEFAULT_AUDIT_LIMIT,
    MAX_INPUT_BYTES,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedLine {
    Command(ApplicationCommand),
    Ignored,
}

pub fn parse_line(input: &[u8]) -> ParsedLine {
    if input.len() > MAX_INPUT_BYTES {
        return ParsedLine::Command(reject(
            InputRejectionCategory::Oversized,
            None,
            input,
        ));
    }

    let line = match std::str::from_utf8(input) {
        Ok(line) => line,
        Err(_) => {
            return ParsedLine::Command(reject(
                InputRejectionCategory::InvalidEncoding,
                None,
                input,
            ));
        }
    };
    let line = line.trim();

    if line.is_empty() {
        return ParsedLine::Ignored;
    }

    let command = match line.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["/help"] => ApplicationCommand::ShowHelp,
        ["/status"] => ApplicationCommand::ShowStatus,
        ["/setup", "status"] => ApplicationCommand::ShowSetupStatus,
        ["/audit", "tail"] => ApplicationCommand::audit_tail(DEFAULT_AUDIT_LIMIT)
            .expect("default audit limit is within the supported range"),
        ["/audit", "tail", limit] => match limit.parse::<u16>() {
            Ok(limit) => ApplicationCommand::audit_tail(limit)
                .unwrap_or_else(|_| reject(InputRejectionCategory::Malformed, safe_token(line), input)),
            Err(_) => reject(InputRejectionCategory::Malformed, safe_token(line), input),
        },
        ["/quit"] => ApplicationCommand::RequestShutdown,
        ["/help" | "/status" | "/setup" | "/audit" | "/quit", ..] => {
            reject(InputRejectionCategory::Malformed, safe_token(line), input)
        }
        _ => reject(InputRejectionCategory::Unknown, safe_token(line), input),
    };

    ParsedLine::Command(command)
}

fn reject(
    category: InputRejectionCategory,
    safe_token: Option<SafeToken>,
    input: &[u8],
) -> ApplicationCommand {
    ApplicationCommand::RejectInput(InputRejection::from_input(category, safe_token, input))
}

fn safe_token(line: &str) -> Option<SafeToken> {
    line.split_whitespace().next().map(|token| {
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
    .flatten()
}
