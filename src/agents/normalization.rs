use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

use crate::domain::DomainError;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ProfileField {
    DisplayName,
    Description,
    PrimarySpecialty,
    SpecialtyTag,
    Personality,
    Instructions,
    ModelProvider,
    ModelName,
    SkillRefs,
    McpRefs,
}

impl ProfileField {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DisplayName => "display_name",
            Self::Description => "description",
            Self::PrimarySpecialty => "primary_specialty",
            Self::SpecialtyTag => "specialty_tag",
            Self::Personality => "personality",
            Self::Instructions => "instructions",
            Self::ModelProvider => "model_provider",
            Self::ModelName => "model_name",
            Self::SkillRefs => "skill_refs",
            Self::McpRefs => "mcp_refs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NormalizedProfileName(String);

impl NormalizedProfileName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn validate_visible_text(
    field: ProfileField,
    value: &str,
    max_bytes: usize,
) -> Result<(), DomainError> {
    if value.is_empty() || value.chars().all(char::is_whitespace) || value.len() > max_bytes {
        return Err(DomainError::InvalidProfileField {
            field: field.as_str(),
        });
    }

    if value.chars().any(is_unsafe_terminal_character) {
        return Err(DomainError::UnsafeProfileText {
            field: field.as_str(),
        });
    }

    Ok(())
}

pub fn normalize_profile_name_key(value: &str) -> Result<NormalizedProfileName, DomainError> {
    validate_visible_text(ProfileField::DisplayName, value, 64)?;
    let normalized = fold_whitespace(&value.nfkc().collect::<String>().case_fold().collect::<String>());

    if normalized.is_empty() {
        return Err(DomainError::InvalidProfileField {
            field: ProfileField::DisplayName.as_str(),
        });
    }

    Ok(NormalizedProfileName(normalized))
}

pub fn normalize_tag_key(value: &str) -> Result<String, DomainError> {
    validate_visible_text(ProfileField::SpecialtyTag, value, 48)?;
    let normalized = fold_whitespace(&value.nfkc().collect::<String>().case_fold().collect::<String>());

    if normalized.is_empty() {
        return Err(DomainError::InvalidProfileField {
            field: ProfileField::SpecialtyTag.as_str(),
        });
    }

    Ok(normalized)
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
    matches!(character, '\0'..='\u{0008}' | '\u{000A}'..='\u{001F}' | '\u{007F}'..='\u{009F}')
        || matches!(character, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}
