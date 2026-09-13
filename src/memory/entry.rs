use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

use crate::{
    domain::{
        Actor, Digest, DomainError, EventId, MemoryEntryId, MemoryEntryVersionId,
        MemoryNamespaceId, ObjectVersion, canonical_json_bytes, sha256,
    },
    memory::normalization::{PLAINTEXT_VALIDATION_VERSION_V1, PlaintextField, validate_plaintext},
};

use super::MemoryProposalRef;

pub const DISPLAY_KEY_MAX_BYTES: usize = 96;
pub const VALUE_MAX_BYTES: usize = 4_096;
pub const MAX_PURPOSE_TAGS: usize = 8;
pub const PURPOSE_TAG_MAX_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryEntryState {
    Present,
    Deleted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryEntryRef {
    namespace_id: MemoryNamespaceId,
    entry_id: MemoryEntryId,
    entry_version_id: MemoryEntryVersionId,
    version: ObjectVersion,
    normalized_key: NormalizedMemoryKey,
    state: MemoryEntryState,
    content_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryEntryRefWire {
    namespace_id: MemoryNamespaceId,
    entry_id: MemoryEntryId,
    entry_version_id: MemoryEntryVersionId,
    version: ObjectVersion,
    normalized_key: NormalizedMemoryKey,
    state: MemoryEntryState,
    content_digest: Digest,
}

impl MemoryEntryRef {
    fn new(
        namespace_id: MemoryNamespaceId,
        entry_id: MemoryEntryId,
        entry_version_id: MemoryEntryVersionId,
        version: ObjectVersion,
        normalized_key: NormalizedMemoryKey,
        state: MemoryEntryState,
        content_digest: Digest,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            namespace_id,
            entry_id,
            entry_version_id,
            version,
            normalized_key,
            state,
            content_digest,
        })
    }

    pub fn namespace_id(&self) -> MemoryNamespaceId {
        self.namespace_id
    }

    pub fn entry_id(&self) -> MemoryEntryId {
        self.entry_id
    }

    pub fn entry_version_id(&self) -> MemoryEntryVersionId {
        self.entry_version_id
    }

    pub fn version(&self) -> ObjectVersion {
        self.version
    }

    pub fn normalized_key(&self) -> &NormalizedMemoryKey {
        &self.normalized_key
    }

    pub fn state(&self) -> MemoryEntryState {
        self.state
    }

    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }
}

impl<'de> Deserialize<'de> for MemoryEntryRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemoryEntryRefWire::deserialize(deserializer)?;
        Self::new(
            wire.namespace_id,
            wire.entry_id,
            wire.entry_version_id,
            wire.version,
            wire.normalized_key,
            wire.state,
            wire.content_digest,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryEntryVersion {
    namespace_id: MemoryNamespaceId,
    entry_id: MemoryEntryId,
    entry_version_id: MemoryEntryVersionId,
    version: ObjectVersion,
    predecessor_version_id: Option<MemoryEntryVersionId>,
    display_key: String,
    normalized_key: NormalizedMemoryKey,
    state: MemoryEntryState,
    value: Option<String>,
    purpose_tags: Vec<String>,
    created_by: Actor,
    created_at_ms: i64,
    accepted_proposal: Option<MemoryProposalRef>,
    plaintext_validation_version: u16,
    creation_event_id: EventId,
    content_digest: Digest,
    record_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryEntryVersionWire {
    namespace_id: MemoryNamespaceId,
    entry_id: MemoryEntryId,
    entry_version_id: MemoryEntryVersionId,
    version: ObjectVersion,
    predecessor_version_id: Option<MemoryEntryVersionId>,
    display_key: String,
    normalized_key: NormalizedMemoryKey,
    state: MemoryEntryState,
    value: Option<String>,
    purpose_tags: Vec<String>,
    created_by: Actor,
    created_at_ms: i64,
    accepted_proposal: Option<MemoryProposalRef>,
    plaintext_validation_version: u16,
    creation_event_id: EventId,
    content_digest: Digest,
    record_digest: Digest,
}

impl MemoryEntryVersion {
    #[allow(clippy::too_many_arguments)]
    pub fn create_present(
        namespace_id: MemoryNamespaceId,
        entry_id: MemoryEntryId,
        entry_version_id: MemoryEntryVersionId,
        draft: MemoryEntryDraft,
        created_by: Actor,
        created_at_ms: i64,
        accepted_proposal: Option<MemoryProposalRef>,
        creation_event_id: EventId,
    ) -> Result<Self, DomainError> {
        let normalized_key = draft.normalized_key();
        Self::build(
            namespace_id,
            entry_id,
            entry_version_id,
            ObjectVersion::new(1)?,
            None,
            draft.display_key,
            normalized_key,
            MemoryEntryState::Present,
            Some(draft.value),
            draft.purpose_tags,
            created_by,
            created_at_ms,
            accepted_proposal,
            PLAINTEXT_VALIDATION_VERSION_V1,
            creation_event_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn next_present(
        &self,
        entry_version_id: MemoryEntryVersionId,
        draft: MemoryEntryDraft,
        created_by: Actor,
        created_at_ms: i64,
        accepted_proposal: Option<MemoryProposalRef>,
        creation_event_id: EventId,
    ) -> Result<Self, DomainError> {
        let normalized_key = draft.normalized_key();
        if normalized_key != self.normalized_key {
            return Err(DomainError::InvalidMemoryEntry);
        }
        Self::build(
            self.namespace_id,
            self.entry_id,
            entry_version_id,
            next_object_version(self.version)?,
            Some(self.entry_version_id),
            draft.display_key,
            normalized_key,
            MemoryEntryState::Present,
            Some(draft.value),
            draft.purpose_tags,
            created_by,
            created_at_ms,
            accepted_proposal,
            PLAINTEXT_VALIDATION_VERSION_V1,
            creation_event_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn next_deleted(
        &self,
        entry_version_id: MemoryEntryVersionId,
        created_by: Actor,
        created_at_ms: i64,
        accepted_proposal: Option<MemoryProposalRef>,
        creation_event_id: EventId,
    ) -> Result<Self, DomainError> {
        Self::build(
            self.namespace_id,
            self.entry_id,
            entry_version_id,
            next_object_version(self.version)?,
            Some(self.entry_version_id),
            self.display_key.clone(),
            self.normalized_key.clone(),
            MemoryEntryState::Deleted,
            None,
            Vec::new(),
            created_by,
            created_at_ms,
            accepted_proposal,
            PLAINTEXT_VALIDATION_VERSION_V1,
            creation_event_id,
        )
    }

    pub fn reference(&self) -> MemoryEntryRef {
        MemoryEntryRef {
            namespace_id: self.namespace_id,
            entry_id: self.entry_id,
            entry_version_id: self.entry_version_id,
            version: self.version,
            normalized_key: self.normalized_key.clone(),
            state: self.state,
            content_digest: self.content_digest.clone(),
        }
    }

    pub fn display_key(&self) -> &str {
        &self.display_key
    }

    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    pub fn purpose_tags(&self) -> &[String] {
        &self.purpose_tags
    }

    pub fn predecessor_version_id(&self) -> Option<MemoryEntryVersionId> {
        self.predecessor_version_id
    }

    pub fn created_by(&self) -> &Actor {
        &self.created_by
    }

    pub fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }

    pub fn accepted_proposal(&self) -> Option<&MemoryProposalRef> {
        self.accepted_proposal.as_ref()
    }

    pub fn plaintext_validation_version(&self) -> u16 {
        self.plaintext_validation_version
    }

    pub fn creation_event_id(&self) -> EventId {
        self.creation_event_id
    }

    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }

    pub fn record_digest(&self) -> &Digest {
        &self.record_digest
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        namespace_id: MemoryNamespaceId,
        entry_id: MemoryEntryId,
        entry_version_id: MemoryEntryVersionId,
        version: ObjectVersion,
        predecessor_version_id: Option<MemoryEntryVersionId>,
        display_key: String,
        normalized_key: NormalizedMemoryKey,
        state: MemoryEntryState,
        value: Option<String>,
        purpose_tags: Vec<String>,
        created_by: Actor,
        created_at_ms: i64,
        accepted_proposal: Option<MemoryProposalRef>,
        plaintext_validation_version: u16,
        creation_event_id: EventId,
        content_digest: Digest,
        record_digest: Digest,
    ) -> Result<Self, DomainError> {
        let entry = Self {
            namespace_id,
            entry_id,
            entry_version_id,
            version,
            predecessor_version_id,
            display_key,
            normalized_key,
            state,
            value,
            purpose_tags,
            created_by,
            created_at_ms,
            accepted_proposal,
            plaintext_validation_version,
            creation_event_id,
            content_digest,
            record_digest,
        };
        entry.validate_structure()?;
        if entry.compute_content_digest()? != entry.content_digest
            || entry.compute_record_digest()? != entry.record_digest
        {
            return Err(DomainError::InvalidMemoryEntry);
        }
        Ok(entry)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        namespace_id: MemoryNamespaceId,
        entry_id: MemoryEntryId,
        entry_version_id: MemoryEntryVersionId,
        version: ObjectVersion,
        predecessor_version_id: Option<MemoryEntryVersionId>,
        display_key: String,
        normalized_key: NormalizedMemoryKey,
        state: MemoryEntryState,
        value: Option<String>,
        purpose_tags: Vec<String>,
        created_by: Actor,
        created_at_ms: i64,
        accepted_proposal: Option<MemoryProposalRef>,
        plaintext_validation_version: u16,
        creation_event_id: EventId,
    ) -> Result<Self, DomainError> {
        let mut entry = Self {
            namespace_id,
            entry_id,
            entry_version_id,
            version,
            predecessor_version_id,
            display_key,
            normalized_key,
            state,
            value,
            purpose_tags,
            created_by,
            created_at_ms,
            accepted_proposal,
            plaintext_validation_version,
            creation_event_id,
            content_digest: sha256(&[]),
            record_digest: sha256(&[]),
        };
        entry.validate_structure()?;
        entry.content_digest = entry.compute_content_digest()?;
        entry.record_digest = entry.compute_record_digest()?;
        Ok(entry)
    }

    fn validate_structure(&self) -> Result<(), DomainError> {
        let has_predecessor = self.predecessor_version_id.is_some();
        if (self.version.get() == 1) == has_predecessor
            || self.predecessor_version_id == Some(self.entry_version_id)
            || self.created_by != Actor::Human
            || self.plaintext_validation_version != PLAINTEXT_VALIDATION_VERSION_V1
        {
            return Err(DomainError::InvalidMemoryEntry);
        }

        match (self.state, self.value.as_deref()) {
            (MemoryEntryState::Present, Some(value)) => {
                let canonical = MemoryEntryDraft::new(
                    self.display_key.clone(),
                    value.to_owned(),
                    self.purpose_tags.clone(),
                )?;
                if canonical.display_key != self.display_key
                    || canonical.normalized_key() != self.normalized_key
                    || canonical.value != value
                    || canonical.purpose_tags != self.purpose_tags
                {
                    return Err(DomainError::InvalidMemoryEntry);
                }
            }
            (MemoryEntryState::Deleted, None) if self.purpose_tags.is_empty() => {
                let display_key = canonicalize_memory_single_line(
                    "display_key",
                    &self.display_key,
                    1,
                    DISPLAY_KEY_MAX_BYTES,
                )?;
                if display_key != self.display_key
                    || NormalizedMemoryKey::new(&display_key)? != self.normalized_key
                {
                    return Err(DomainError::InvalidMemoryEntry);
                }
            }
            _ => return Err(DomainError::InvalidMemoryEntry),
        }
        Ok(())
    }

    fn compute_content_digest(&self) -> Result<Digest, DomainError> {
        Ok(sha256(&canonical_json_bytes(
            &MemoryEntryContentDigestMaterial {
                display_key: &self.display_key,
                normalized_key: &self.normalized_key,
                state: self.state,
                value: self.value.as_deref(),
                purpose_tags: &self.purpose_tags,
                plaintext_validation_version: self.plaintext_validation_version,
            },
        )?))
    }

    fn compute_record_digest(&self) -> Result<Digest, DomainError> {
        Ok(sha256(&canonical_json_bytes(
            &MemoryEntryRecordDigestMaterial {
                namespace_id: self.namespace_id,
                entry_id: self.entry_id,
                entry_version_id: self.entry_version_id,
                version: self.version,
                predecessor_version_id: self.predecessor_version_id,
                display_key: &self.display_key,
                normalized_key: &self.normalized_key,
                state: self.state,
                value: self.value.as_deref(),
                purpose_tags: &self.purpose_tags,
                created_by: &self.created_by,
                created_at_ms: self.created_at_ms,
                accepted_proposal: self.accepted_proposal.as_ref(),
                plaintext_validation_version: self.plaintext_validation_version,
                creation_event_id: self.creation_event_id,
                content_digest: &self.content_digest,
            },
        )?))
    }
}

impl<'de> Deserialize<'de> for MemoryEntryVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemoryEntryVersionWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.namespace_id,
            wire.entry_id,
            wire.entry_version_id,
            wire.version,
            wire.predecessor_version_id,
            wire.display_key,
            wire.normalized_key,
            wire.state,
            wire.value,
            wire.purpose_tags,
            wire.created_by,
            wire.created_at_ms,
            wire.accepted_proposal,
            wire.plaintext_validation_version,
            wire.creation_event_id,
            wire.content_digest,
            wire.record_digest,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize)]
struct MemoryEntryContentDigestMaterial<'a> {
    display_key: &'a str,
    normalized_key: &'a NormalizedMemoryKey,
    state: MemoryEntryState,
    value: Option<&'a str>,
    purpose_tags: &'a [String],
    plaintext_validation_version: u16,
}

#[derive(Serialize)]
struct MemoryEntryRecordDigestMaterial<'a> {
    namespace_id: MemoryNamespaceId,
    entry_id: MemoryEntryId,
    entry_version_id: MemoryEntryVersionId,
    version: ObjectVersion,
    predecessor_version_id: Option<MemoryEntryVersionId>,
    display_key: &'a str,
    normalized_key: &'a NormalizedMemoryKey,
    state: MemoryEntryState,
    value: Option<&'a str>,
    purpose_tags: &'a [String],
    created_by: &'a Actor,
    created_at_ms: i64,
    accepted_proposal: Option<&'a MemoryProposalRef>,
    plaintext_validation_version: u16,
    creation_event_id: EventId,
    content_digest: &'a Digest,
}

fn next_object_version(current: ObjectVersion) -> Result<ObjectVersion, DomainError> {
    current
        .get()
        .checked_add(1)
        .ok_or(DomainError::InvalidObjectVersion)
        .and_then(ObjectVersion::new)
}

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
            if key == "general" || !tag_keys.insert(key.clone()) {
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
    if is_reserved_credential_key(&normalized) {
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
