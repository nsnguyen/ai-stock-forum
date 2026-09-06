use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

use crate::domain::DomainError;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SkillField {
    DisplayName,
    Description,
    UseWhen,
    Tag,
    Instructions,
    ResourceName,
    ResourceBody,
}

impl SkillField {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DisplayName => "display_name",
            Self::Description => "description",
            Self::UseWhen => "use_when",
            Self::Tag => "tag",
            Self::Instructions => "instructions",
            Self::ResourceName => "resource_name",
            Self::ResourceBody => "resource_body",
        }
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct NormalizedSkillName(String);

impl NormalizedSkillName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) fn canonicalize_visible_text(
    field: SkillField,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<String, DomainError> {
    let value = normalize_line_endings(value);
    reject_unsafe(field, &value)?;
    let canonical = fold_whitespace(&value);
    validate_length(field, &canonical, max_bytes, allow_empty)?;
    Ok(canonical)
}

pub(crate) fn canonicalize_multiline_text(
    field: SkillField,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<String, DomainError> {
    let canonical = normalize_line_endings(value);
    reject_unsafe(field, &canonical)?;
    validate_length(field, &canonical, max_bytes, allow_empty)?;
    Ok(canonical)
}

pub(crate) fn normalize_skill_name(value: &str) -> Result<NormalizedSkillName, DomainError> {
    Ok(NormalizedSkillName(normalize_key(
        SkillField::DisplayName,
        value,
        64,
    )?))
}

pub(crate) fn normalize_tag_key(value: &str) -> Result<String, DomainError> {
    normalize_key(SkillField::Tag, value, 32)
}

pub(crate) fn normalize_resource_name_key(value: &str) -> Result<String, DomainError> {
    normalize_key(SkillField::ResourceName, value, 64)
}

fn normalize_key(field: SkillField, value: &str, max_bytes: usize) -> Result<String, DomainError> {
    let value = canonicalize_visible_text(field, value, max_bytes, false)?;
    Ok(fold_whitespace(
        &value
            .nfkc()
            .collect::<String>()
            .case_fold()
            .collect::<String>(),
    ))
}

fn normalize_line_endings(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn validate_length(
    field: SkillField,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), DomainError> {
    if (!allow_empty && value.is_empty()) || value.len() > max_bytes {
        Err(DomainError::InvalidSkillField {
            field: field.as_str(),
        })
    } else {
        Ok(())
    }
}

fn reject_unsafe(field: SkillField, value: &str) -> Result<(), DomainError> {
    if value.chars().any(is_unsafe_terminal_character) {
        Err(DomainError::UnsafeSkillText {
            field: field.as_str(),
        })
    } else {
        Ok(())
    }
}

fn fold_whitespace(value: &str) -> String {
    let mut folded = String::new();
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = !folded.is_empty();
        } else {
            if pending_space {
                folded.push(' ');
                pending_space = false;
            }
            folded.push(character);
        }
    }
    folded
}

fn is_unsafe_terminal_character(character: char) -> bool {
    matches!(character, '\0'..='\u{0009}' | '\u{000B}'..='\u{001F}' | '\u{007F}'..='\u{009F}')
        || matches!(character, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}
