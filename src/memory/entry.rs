use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

use crate::{
    domain::DomainError,
    memory::normalization::{PLAINTEXT_VALIDATION_VERSION_V1, PlaintextField, validate_plaintext},
};

pub const DISPLAY_KEY_MAX_BYTES: usize = 128;
pub const VALUE_MAX_BYTES: usize = 4_096;
pub const MAX_PURPOSE_TAGS: usize = 8;
pub const PURPOSE_TAG_MAX_BYTES: usize = 32;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct NormalizedMemoryKey(String);

impl NormalizedMemoryKey {
    pub fn new(display_key: &str) -> Result<Self, DomainError> {
        normalize_memory_key(display_key)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for NormalizedMemoryKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let normalized = normalize_memory_key(&value).map_err(serde::de::Error::custom)?;
        if normalized.as_str() != value {
            return Err(serde::de::Error::custom("invalid normalized memory key"));
        }
        Ok(normalized)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryEntryDraft {
    display_key: String,
    value: String,
    purpose_tags: Vec<String>,
}

impl MemoryEntryDraft {
    pub fn new(
        display_key: String,
        value: String,
        purpose_tags: Vec<String>,
    ) -> Result<Self, DomainError> {
        let display_key =
            canonicalize_memory_single_line("display_key", &display_key, 1, DISPLAY_KEY_MAX_BYTES)?;
        normalize_memory_key(&display_key)?;
        let value = canonicalize_memory_multiline("value", &value, 1, VALUE_MAX_BYTES)?;
        validate_plaintext(
            PLAINTEXT_VALIDATION_VERSION_V1,
            PlaintextField::MemoryValue,
            &value,
        )?;
        if purpose_tags.len() > MAX_PURPOSE_TAGS {
            return Err(DomainError::InvalidMemoryField {
                field: "purpose_tags",
            });
        }

        let mut tag_keys = BTreeSet::new();
        let mut canonical_tags = Vec::with_capacity(purpose_tags.len());
        for tag in purpose_tags {
            let tag =
                canonicalize_memory_single_line("purpose_tag", &tag, 1, PURPOSE_TAG_MAX_BYTES)?;
            let key = normalized_comparison_key("purpose_tag", &tag)?;
            if !tag_keys.insert(key.clone()) {
                return Err(DomainError::InvalidMemoryField {
                    field: "purpose_tags",
                });
            }
            canonical_tags.push((key, tag));
        }
        canonical_tags.sort_by(|left, right| left.0.cmp(&right.0));

        Ok(Self {
            display_key,
            value,
            purpose_tags: canonical_tags.into_iter().map(|(_, tag)| tag).collect(),
        })
    }

    pub fn display_key(&self) -> &str {
        &self.display_key
    }

    pub fn normalized_key(&self) -> NormalizedMemoryKey {
        // Construction and deserialization both validate the key.
        normalize_memory_key(&self.display_key).expect("validated memory draft key")
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn purpose_tags(&self) -> &[String] {
        &self.purpose_tags
    }
}

impl<'de> Deserialize<'de> for MemoryEntryDraft {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawMemoryEntryDraft {
            display_key: String,
            value: String,
            purpose_tags: Vec<String>,
        }

        let raw = RawMemoryEntryDraft::deserialize(deserializer)?;
        let canonical = Self::new(
            raw.display_key.clone(),
            raw.value.clone(),
            raw.purpose_tags.clone(),
        )
        .map_err(serde::de::Error::custom)?;
        if canonical.display_key != raw.display_key
            || canonical.value != raw.value
            || canonical.purpose_tags != raw.purpose_tags
        {
            return Err(serde::de::Error::custom("noncanonical memory draft"));
        }
        Ok(canonical)
    }
}

pub fn normalize_memory_key(display_key: &str) -> Result<NormalizedMemoryKey, DomainError> {
    let display_key =
        canonicalize_memory_single_line("display_key", display_key, 1, DISPLAY_KEY_MAX_BYTES)?;
    let normalized = normalized_comparison_key("display_key", &display_key)?;
    if normalized == "general" || is_reserved_credential_key(&normalized) {
        return Err(DomainError::InvalidMemoryField {
            field: "display_key",
        });
    }
    Ok(NormalizedMemoryKey(normalized))
}

pub(crate) fn canonicalize_memory_single_line(
    field: &'static str,
    value: &str,
    min_bytes: usize,
    max_bytes: usize,
) -> Result<String, DomainError> {
    let normalized = normalize_line_endings(value);
    if normalized.contains('\n') || normalized.chars().any(is_unsafe_memory_character) {
        return Err(DomainError::UnsafeMemoryText { field });
    }
    let canonical = fold_whitespace(&normalized);
    validate_length(field, &canonical, min_bytes, max_bytes)?;
    Ok(canonical)
}

pub(crate) fn canonicalize_memory_multiline(
    field: &'static str,
    value: &str,
    min_bytes: usize,
    max_bytes: usize,
) -> Result<String, DomainError> {
    let canonical = normalize_line_endings(value);
    if canonical.chars().any(is_unsafe_memory_character) {
        return Err(DomainError::UnsafeMemoryText { field });
    }
    validate_length(field, &canonical, min_bytes, max_bytes)?;
    Ok(canonical)
}

fn normalized_comparison_key(field: &'static str, value: &str) -> Result<String, DomainError> {
    let normalized = fold_whitespace(
        &value
            .nfkc()
            .collect::<String>()
            .case_fold()
            .collect::<String>(),
    );
    if normalized.is_empty() {
        return Err(DomainError::InvalidMemoryField { field });
    }
    Ok(normalized)
}

fn is_reserved_credential_key(normalized_key: &str) -> bool {
    let comparison = normalized_key
        .split([' ', '-', '_', '.', ':', '/', '\\'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    matches!(
        comparison.as_str(),
        "password"
            | "passwords"
            | "passphrase"
            | "passphrases"
            | "api key"
            | "api keys"
            | "access token"
            | "access tokens"
            | "refresh token"
            | "refresh tokens"
            | "session token"
            | "session tokens"
            | "private key"
            | "private keys"
            | "secret"
            | "secrets"
            | "credential"
            | "credentials"
    )
}

fn normalize_line_endings(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn validate_length(
    field: &'static str,
    value: &str,
    min_bytes: usize,
    max_bytes: usize,
) -> Result<(), DomainError> {
    if value.len() < min_bytes || value.len() > max_bytes {
        Err(DomainError::InvalidMemoryField { field })
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

fn is_unsafe_memory_character(character: char) -> bool {
    matches!(character, '\0' | '\t' | '\u{0001}'..='\u{0008}' | '\u{000B}'..='\u{001F}' | '\u{007F}'..='\u{009F}')
        || matches!(character, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}
