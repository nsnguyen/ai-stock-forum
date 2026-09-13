use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    agents::{AgentProfileVersion, AgentProfileVersionRef},
    domain::{
        Digest, DomainError, EpisodicSummaryId, EventId, MemoryNamespaceId, ObjectVersion,
        canonical_json_bytes, sha256,
    },
};

use super::entry::{canonicalize_memory_multiline, canonicalize_memory_single_line};
use super::{
    MemoryEntryDraft, PLAINTEXT_VALIDATION_VERSION_V1, PlaintextField, validate_plaintext,
};

const EPISODIC_EVENT_TYPE_MAX_BYTES: usize = 128;
const MAX_EPISODIC_SOURCES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EpisodicSourceRef {
    sequence: u64,
    event_id: EventId,
    event_type: String,
    event_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EpisodicSourceRefWire {
    sequence: u64,
    event_id: EventId,
    event_type: String,
    event_digest: Digest,
}

impl EpisodicSourceRef {
    pub fn new(
        sequence: u64,
        event_id: EventId,
        event_type: String,
        event_digest: Digest,
    ) -> Result<Self, DomainError> {
        let event_type = canonicalize_memory_single_line(
            "episodic_event_type",
            &event_type,
            1,
            EPISODIC_EVENT_TYPE_MAX_BYTES,
        )?;
        Ok(Self {
            sequence,
            event_id,
            event_type,
            event_digest,
        })
    }

    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn event_id(&self) -> EventId {
        self.event_id
    }
    pub fn event_type(&self) -> &str {
        &self.event_type
    }
    pub fn event_digest(&self) -> &Digest {
        &self.event_digest
    }
}

impl<'de> Deserialize<'de> for EpisodicSourceRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EpisodicSourceRefWire::deserialize(deserializer)?;
        let source = Self::new(
            wire.sequence,
            wire.event_id,
            wire.event_type.clone(),
            wire.event_digest,
        )
        .map_err(serde::de::Error::custom)?;
        if source.event_type != wire.event_type {
            return Err(serde::de::Error::custom("noncanonical episodic source"));
        }
        Ok(source)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EpisodicSummaryRef {
    summary_id: EpisodicSummaryId,
    version: ObjectVersion,
    namespace_id: MemoryNamespaceId,
    profile: AgentProfileVersionRef,
    creation_event_sequence: u64,
    creation_event_id: EventId,
    source_set_digest: Digest,
    content_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EpisodicSummaryRefWire {
    summary_id: EpisodicSummaryId,
    version: ObjectVersion,
    namespace_id: MemoryNamespaceId,
    profile: AgentProfileVersionRef,
    creation_event_sequence: u64,
    creation_event_id: EventId,
    source_set_digest: Digest,
    content_digest: Digest,
}

impl EpisodicSummaryRef {
    #[allow(clippy::too_many_arguments)]
    fn new(
        summary_id: EpisodicSummaryId,
        version: ObjectVersion,
        namespace_id: MemoryNamespaceId,
        profile: AgentProfileVersionRef,
        creation_event_sequence: u64,
        creation_event_id: EventId,
        source_set_digest: Digest,
        content_digest: Digest,
    ) -> Result<Self, DomainError> {
        if version.get() != 1 {
            return Err(DomainError::InvalidEpisodicSummary);
        }
        Ok(Self {
            summary_id,
            version,
            namespace_id,
            profile,
            creation_event_sequence,
            creation_event_id,
            source_set_digest,
            content_digest,
        })
    }

    pub fn summary_id(&self) -> EpisodicSummaryId {
        self.summary_id
    }
    pub fn version(&self) -> ObjectVersion {
        self.version
    }
    pub fn namespace_id(&self) -> MemoryNamespaceId {
        self.namespace_id
    }
    pub fn profile(&self) -> &AgentProfileVersionRef {
        &self.profile
    }
    pub fn creation_event_sequence(&self) -> u64 {
        self.creation_event_sequence
    }
    pub fn creation_event_id(&self) -> EventId {
        self.creation_event_id
    }
    pub fn source_set_digest(&self) -> &Digest {
        &self.source_set_digest
    }
    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }
}

impl<'de> Deserialize<'de> for EpisodicSummaryRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EpisodicSummaryRefWire::deserialize(deserializer)?;
        Self::new(
            wire.summary_id,
            wire.version,
            wire.namespace_id,
            wire.profile,
            wire.creation_event_sequence,
            wire.creation_event_id,
            wire.source_set_digest,
            wire.content_digest,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EpisodicQualification {
    #[serde(rename = "Summary — verify sources")]
    SummaryVerifySources,
}

impl EpisodicQualification {
    pub fn label(self) -> &'static str {
        "Summary — verify sources"
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EpisodicSummary {
    summary_id: EpisodicSummaryId,
    version: ObjectVersion,
    namespace_id: MemoryNamespaceId,
    profile: AgentProfileVersionRef,
    label: String,
    body: String,
    purpose_tags: Vec<String>,
    sources: Vec<EpisodicSourceRef>,
    plaintext_validation_version: u16,
    created_at_ms: i64,
    creation_event_sequence: u64,
    creation_event_id: EventId,
    source_set_digest: Digest,
    content_digest: Digest,
    record_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EpisodicSummaryWire {
    summary_id: EpisodicSummaryId,
    version: ObjectVersion,
    namespace_id: MemoryNamespaceId,
    profile: AgentProfileVersionRef,
    label: String,
    body: String,
    purpose_tags: Vec<String>,
    sources: Vec<EpisodicSourceRef>,
    plaintext_validation_version: u16,
    created_at_ms: i64,
    creation_event_sequence: u64,
    creation_event_id: EventId,
    source_set_digest: Digest,
    content_digest: Digest,
    record_digest: Digest,
}

impl EpisodicSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        summary_id: EpisodicSummaryId,
        profile: &AgentProfileVersion,
        label: String,
        body: String,
        purpose_tags: Vec<String>,
        sources: Vec<EpisodicSourceRef>,
        created_at_ms: i64,
        creation_event_sequence: u64,
        creation_event_id: EventId,
    ) -> Result<Self, DomainError> {
        let label = canonicalize_memory_single_line("episodic_label", &label, 1, 128)?;
        let body = canonicalize_memory_multiline("episodic_body", &body, 1, 8_192)?;
        validate_plaintext(
            PLAINTEXT_VALIDATION_VERSION_V1,
            PlaintextField::EpisodicLabel,
            &label,
        )?;
        validate_plaintext(
            PLAINTEXT_VALIDATION_VERSION_V1,
            PlaintextField::EpisodicBody,
            &body,
        )?;
        let purpose_tags = canonical_purpose_tags(purpose_tags)?;
        let source_set_digest = source_set_digest(&sources)?;
        let mut summary = Self {
            summary_id,
            version: ObjectVersion::new(1)?,
            namespace_id: profile.memory_namespace_id(),
            profile: profile.reference(),
            label,
            body,
            purpose_tags,
            sources,
            plaintext_validation_version: PLAINTEXT_VALIDATION_VERSION_V1,
            created_at_ms,
            creation_event_sequence,
            creation_event_id,
            source_set_digest,
            content_digest: sha256(&[]),
            record_digest: sha256(&[]),
        };
        summary.validate_structure()?;
        summary.content_digest = summary.compute_content_digest()?;
        summary.record_digest = summary.compute_record_digest()?;
        Ok(summary)
    }

    pub fn reference(&self) -> EpisodicSummaryRef {
        EpisodicSummaryRef {
            summary_id: self.summary_id,
            version: self.version,
            namespace_id: self.namespace_id,
            profile: self.profile.clone(),
            creation_event_sequence: self.creation_event_sequence,
            creation_event_id: self.creation_event_id,
            source_set_digest: self.source_set_digest.clone(),
            content_digest: self.content_digest.clone(),
        }
    }
    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn body(&self) -> &str {
        &self.body
    }
    pub fn purpose_tags(&self) -> &[String] {
        &self.purpose_tags
    }
    pub fn sources(&self) -> &[EpisodicSourceRef] {
        &self.sources
    }
    pub fn plaintext_validation_version(&self) -> u16 {
        self.plaintext_validation_version
    }
    pub fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }
    pub fn creation_event_sequence(&self) -> u64 {
        self.creation_event_sequence
    }
    pub fn creation_event_id(&self) -> EventId {
        self.creation_event_id
    }
    pub fn source_set_digest(&self) -> &Digest {
        &self.source_set_digest
    }
    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }
    pub fn record_digest(&self) -> &Digest {
        &self.record_digest
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        summary_id: EpisodicSummaryId,
        version: ObjectVersion,
        namespace_id: MemoryNamespaceId,
        profile: AgentProfileVersionRef,
        label: String,
        body: String,
        purpose_tags: Vec<String>,
        sources: Vec<EpisodicSourceRef>,
        plaintext_validation_version: u16,
        created_at_ms: i64,
        creation_event_sequence: u64,
        creation_event_id: EventId,
        source_set_digest: Digest,
        content_digest: Digest,
        record_digest: Digest,
    ) -> Result<Self, DomainError> {
        let summary = Self {
            summary_id,
            version,
            namespace_id,
            profile,
            label,
            body,
            purpose_tags,
            sources,
            plaintext_validation_version,
            created_at_ms,
            creation_event_sequence,
            creation_event_id,
            source_set_digest,
            content_digest,
            record_digest,
        };
        summary.validate_structure()?;
        if summary.compute_source_set_digest()? != summary.source_set_digest
            || summary.compute_content_digest()? != summary.content_digest
            || summary.compute_record_digest()? != summary.record_digest
        {
            return Err(DomainError::InvalidEpisodicSummary);
        }
        Ok(summary)
    }

    fn validate_structure(&self) -> Result<(), DomainError> {
        if self.version.get() != 1
            || self.plaintext_validation_version != PLAINTEXT_VALIDATION_VERSION_V1
        {
            return Err(DomainError::InvalidEpisodicSummary);
        }
        let label = canonicalize_memory_single_line("episodic_label", &self.label, 1, 128)?;
        let body = canonicalize_memory_multiline("episodic_body", &self.body, 1, 8_192)?;
        validate_plaintext(
            self.plaintext_validation_version,
            PlaintextField::EpisodicLabel,
            &label,
        )?;
        validate_plaintext(
            self.plaintext_validation_version,
            PlaintextField::EpisodicBody,
            &body,
        )?;
        if label != self.label
            || body != self.body
            || canonical_purpose_tags(self.purpose_tags.clone())? != self.purpose_tags
        {
            return Err(DomainError::InvalidEpisodicSummary);
        }
        validate_sources(
            &self.sources,
            self.creation_event_sequence,
            self.creation_event_id,
        )
    }

    fn compute_source_set_digest(&self) -> Result<Digest, DomainError> {
        source_set_digest(&self.sources)
    }
    fn compute_content_digest(&self) -> Result<Digest, DomainError> {
        episodic_content_digest(
            &self.label,
            &self.body,
            &self.purpose_tags,
            self.plaintext_validation_version,
            self.created_at_ms,
            &self.source_set_digest,
        )
    }
    fn compute_record_digest(&self) -> Result<Digest, DomainError> {
        Ok(sha256(&canonical_json_bytes(
            &EpisodicRecordDigestMaterial {
                summary_id: self.summary_id,
                version: self.version,
                namespace_id: self.namespace_id,
                profile: &self.profile,
                label: &self.label,
                body: &self.body,
                purpose_tags: &self.purpose_tags,
                sources: &self.sources,
                plaintext_validation_version: self.plaintext_validation_version,
                created_at_ms: self.created_at_ms,
                creation_event_sequence: self.creation_event_sequence,
                creation_event_id: self.creation_event_id,
                source_set_digest: &self.source_set_digest,
                content_digest: &self.content_digest,
            },
        )?))
    }
}

impl<'de> Deserialize<'de> for EpisodicSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EpisodicSummaryWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.summary_id,
            wire.version,
            wire.namespace_id,
            wire.profile,
            wire.label,
            wire.body,
            wire.purpose_tags,
            wire.sources,
            wire.plaintext_validation_version,
            wire.created_at_ms,
            wire.creation_event_sequence,
            wire.creation_event_id,
            wire.source_set_digest,
            wire.content_digest,
            wire.record_digest,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn canonical_purpose_tags(tags: Vec<String>) -> Result<Vec<String>, DomainError> {
    let draft = MemoryEntryDraft::new("episodic tags".to_owned(), "valid".to_owned(), tags)?;
    Ok(draft.purpose_tags().to_vec())
}

fn validate_sources(
    sources: &[EpisodicSourceRef],
    creation_event_sequence: u64,
    creation_event_id: EventId,
) -> Result<(), DomainError> {
    if sources.is_empty() || sources.len() > MAX_EPISODIC_SOURCES {
        return Err(DomainError::InvalidEpisodicSummary);
    }
    let mut source_ids = BTreeSet::new();
    let mut previous_sequence = None;
    for source in sources {
        if previous_sequence.is_some_and(|previous| source.sequence <= previous) {
            return Err(DomainError::EpisodicSourcesNotOrdered);
        }
        if !source_ids.insert(source.event_id) {
            return Err(DomainError::EpisodicSourcesNotUnique);
        }
        if source.sequence >= creation_event_sequence || source.event_id == creation_event_id {
            return Err(DomainError::InvalidEpisodicSummary);
        }
        previous_sequence = Some(source.sequence);
    }
    Ok(())
}

fn source_set_digest(sources: &[EpisodicSourceRef]) -> Result<Digest, DomainError> {
    Ok(sha256(&canonical_json_bytes(&sources.to_vec())?))
}

pub(crate) fn episodic_content_digest(
    label: &str,
    body: &str,
    purpose_tags: &[String],
    plaintext_validation_version: u16,
    created_at_ms: i64,
    source_set_digest: &Digest,
) -> Result<Digest, DomainError> {
    Ok(sha256(&canonical_json_bytes(
        &EpisodicContentDigestMaterial {
            label,
            body,
            purpose_tags,
            plaintext_validation_version,
            created_at_ms,
            source_set_digest,
        },
    )?))
}

#[derive(Serialize)]
struct EpisodicContentDigestMaterial<'a> {
    label: &'a str,
    body: &'a str,
    purpose_tags: &'a [String],
    plaintext_validation_version: u16,
    created_at_ms: i64,
    source_set_digest: &'a Digest,
}

#[derive(Serialize)]
struct EpisodicRecordDigestMaterial<'a> {
    summary_id: EpisodicSummaryId,
    version: ObjectVersion,
    namespace_id: MemoryNamespaceId,
    profile: &'a AgentProfileVersionRef,
    label: &'a str,
    body: &'a str,
    purpose_tags: &'a [String],
    sources: &'a [EpisodicSourceRef],
    plaintext_validation_version: u16,
    created_at_ms: i64,
    creation_event_sequence: u64,
    creation_event_id: EventId,
    source_set_digest: &'a Digest,
    content_digest: &'a Digest,
}
