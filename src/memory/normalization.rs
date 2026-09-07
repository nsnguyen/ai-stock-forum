use crate::domain::DomainError;

pub const PLAINTEXT_VALIDATION_VERSION_V1: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaintextField {
    MemoryValue,
    ProposalRationale,
    EpisodicLabel,
    EpisodicBody,
}

impl PlaintextField {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::MemoryValue => "value",
            Self::ProposalRationale => "proposal_rationale",
            Self::EpisodicLabel => "episodic_label",
            Self::EpisodicBody => "episodic_body",
        }
    }

    const fn maximum_token_characters(self) -> usize {
        match self {
            Self::MemoryValue => 4_096,
            Self::ProposalRationale => 512,
            Self::EpisodicLabel => 128,
            Self::EpisodicBody => 8_192,
        }
    }
}

pub struct CredentialPatternSetV1;

impl CredentialPatternSetV1 {
    pub fn validate(field: PlaintextField, value: &str) -> Result<(), DomainError> {
        let bytes = value.as_bytes();
        if has_private_key_marker(bytes)
            || has_bearer_credential(bytes, field.maximum_token_characters())
            || has_prefixed_token(bytes, field.maximum_token_characters())
        {
            Err(DomainError::UnsafeMemoryText {
                field: field.as_str(),
            })
        } else {
            Ok(())
        }
    }
}

pub fn validate_plaintext(
    validation_version: u16,
    field: PlaintextField,
    value: &str,
) -> Result<(), DomainError> {
    match validation_version {
        PLAINTEXT_VALIDATION_VERSION_V1 => CredentialPatternSetV1::validate(field, value),
        _ => Err(DomainError::UnknownPlaintextValidationVersion),
    }
}

fn has_private_key_marker(bytes: &[u8]) -> bool {
    let prefix = b"-----BEGIN ";
    let suffix = b"PRIVATE KEY-----";
    if bytes.len() < prefix.len() {
        return false;
    }
    for start in 0..=bytes.len() - prefix.len() {
        if !ascii_eq_ignore_case(&bytes[start..start + prefix.len()], prefix) {
            continue;
        }
        let mut cursor = start + prefix.len();
        for kind in [
            b"RSA ".as_slice(),
            b"DSA ".as_slice(),
            b"EC ".as_slice(),
            b"OPENSSH ".as_slice(),
        ] {
            if bytes
                .get(cursor..cursor + kind.len())
                .is_some_and(|part| ascii_eq_ignore_case(part, kind))
            {
                cursor += kind.len();
                break;
            }
        }
        if bytes
            .get(cursor..cursor + suffix.len())
            .is_some_and(|part| ascii_eq_ignore_case(part, suffix))
        {
            return true;
        }
    }
    false
}

fn has_bearer_credential(bytes: &[u8], maximum_token_characters: usize) -> bool {
    let marker = b"authorization";
    if bytes.len() < marker.len() {
        return false;
    }
    for start in 0..=bytes.len() - marker.len() {
        if !ascii_eq_ignore_case(&bytes[start..start + marker.len()], marker) {
            continue;
        }
        let mut cursor = start + marker.len();
        cursor += usize::from(bytes.get(cursor) == Some(&b' '));
        if bytes.get(cursor) != Some(&b':') {
            continue;
        }
        cursor += 1;
        cursor += usize::from(bytes.get(cursor) == Some(&b' '));
        let bearer = b"bearer";
        if !bytes
            .get(cursor..cursor + bearer.len())
            .is_some_and(|part| ascii_eq_ignore_case(part, bearer))
        {
            continue;
        }
        cursor += bearer.len();
        let spaces = bytes[cursor..]
            .iter()
            .take_while(|&&byte| byte == b' ')
            .count();
        if spaces == 0 {
            continue;
        }
        cursor += spaces;
        let token_length = bytes[cursor..]
            .iter()
            .take_while(|&&byte| is_bearer_token_byte(byte))
            .count();
        if (16..=maximum_token_characters).contains(&token_length) {
            return true;
        }
    }
    false
}

fn has_prefixed_token(bytes: &[u8], maximum_token_characters: usize) -> bool {
    let prefixes = [b"sk-ant-".as_slice(), b"sk-".as_slice(), b"xai-".as_slice()];
    for start in 0..bytes.len() {
        if start > 0 && is_token_byte(bytes[start - 1]) {
            continue;
        }
        let Some(prefix) = prefixes.into_iter().find(|prefix| {
            bytes
                .get(start..start + prefix.len())
                .is_some_and(|part| ascii_eq_ignore_case(part, prefix))
        }) else {
            continue;
        };
        let token_start = start + prefix.len();
        let token_length = bytes[token_start..]
            .iter()
            .take_while(|&&byte| is_token_byte(byte))
            .count();
        if (20..=maximum_token_characters).contains(&token_length) {
            return true;
        }
    }
    false
}

fn ascii_eq_ignore_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(&left, &right)| left.eq_ignore_ascii_case(&right))
}

fn is_bearer_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'+' | b'/' | b'=' | b'-')
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}
