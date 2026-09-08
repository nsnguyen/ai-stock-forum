use std::{cmp::Reverse, collections::BTreeSet};

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    agents::{AgentProfileVersion, AgentProfileVersionRef},
    domain::{Digest, DomainError, MemoryNamespaceId, canonical_json_bytes, sha256},
};

use super::{
    EpisodicQualification, EpisodicSourceRef, EpisodicSummary, EpisodicSummaryRef,
    MemoryEntryDraft, MemoryEntryRef, MemoryEntryState, MemoryEntryVersion,
};

pub const MAX_MEMORY_RETRIEVAL_ENTRIES: u32 = 32;
pub const MAX_MEMORY_RETRIEVAL_SUMMARIES: u32 = 8;
pub const MAX_MEMORY_RETRIEVAL_BYTES: u64 = 32_768;
pub const MAX_MEMORY_RETRIEVAL_SOURCES: u64 = 128;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum MemoryPurposeScope {
    General,
    Tagged(Vec<String>),
}

impl MemoryPurposeScope {
    pub fn tagged(tags: Vec<String>) -> Result<Self, DomainError> {
        if tags.is_empty() || tags.len() > 8 {
            return Err(DomainError::InvalidMemoryRetrievalScope);
        }
        let canonical = canonical_purpose_tags(tags)?;
        if canonical.len() > 8 {
            return Err(DomainError::InvalidMemoryRetrievalScope);
        }
        Ok(Self::Tagged(canonical))
    }

    fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::General => Ok(()),
            Self::Tagged(tags) => {
                let canonical = Self::tagged(tags.clone())?;
                if &canonical != self {
                    Err(DomainError::InvalidMemoryRetrievalScope)
                } else {
                    Ok(())
                }
            }
        }
    }
}

#[derive(Deserialize)]
enum MemoryPurposeScopeWire {
    General,
    Tagged(Vec<String>),
}

impl<'de> Deserialize<'de> for MemoryPurposeScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match MemoryPurposeScopeWire::deserialize(deserializer)? {
            MemoryPurposeScopeWire::General => Ok(Self::General),
            MemoryPurposeScopeWire::Tagged(tags) => {
                let scope = Self::tagged(tags.clone()).map_err(serde::de::Error::custom)?;
                if scope != Self::Tagged(tags) {
                    return Err(serde::de::Error::custom(
                        "noncanonical memory purpose scope",
                    ));
                }
                Ok(scope)
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryRetrievalScope {
    format_version: u16,
    profile: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    purpose: MemoryPurposeScope,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRetrievalScopeWire {
    format_version: u16,
    profile: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    purpose: MemoryPurposeScope,
}

impl MemoryRetrievalScope {
    pub fn new(
        profile: &AgentProfileVersion,
        purpose: MemoryPurposeScope,
    ) -> Result<Self, DomainError> {
        Self::from_parts(
            1,
            profile.reference(),
            profile.memory_namespace_id(),
            purpose,
        )
    }
    pub fn validate_against(&self, profile: &AgentProfileVersion) -> Result<(), DomainError> {
        if self.profile != profile.reference() || self.namespace_id != profile.memory_namespace_id()
        {
            return Err(DomainError::InvalidMemoryRetrievalScope);
        }
        Ok(())
    }
    pub fn profile(&self) -> &AgentProfileVersionRef {
        &self.profile
    }
    pub fn namespace_id(&self) -> MemoryNamespaceId {
        self.namespace_id
    }
    pub fn purpose(&self) -> &MemoryPurposeScope {
        &self.purpose
    }

    pub(crate) fn from_parts(
        format_version: u16,
        profile: AgentProfileVersionRef,
        namespace_id: MemoryNamespaceId,
        purpose: MemoryPurposeScope,
    ) -> Result<Self, DomainError> {
        if format_version != 1 {
            return Err(DomainError::InvalidMemoryRetrievalScope);
        }
        purpose.validate()?;
        Ok(Self {
            format_version,
            profile,
            namespace_id,
            purpose,
        })
    }
}

impl<'de> Deserialize<'de> for MemoryRetrievalScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemoryRetrievalScopeWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.format_version,
            wire.profile,
            wire.namespace_id,
            wire.purpose,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryRetrievalBudget {
    max_entries: u32,
    max_summaries: u32,
    max_bytes: u64,
    max_sources: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRetrievalBudgetWire {
    max_entries: u32,
    max_summaries: u32,
    max_bytes: u64,
    max_sources: u64,
}

impl MemoryRetrievalBudget {
    pub fn new(
        max_entries: u32,
        max_summaries: u32,
        max_bytes: u64,
        max_sources: u64,
    ) -> Result<Self, DomainError> {
        if max_entries > MAX_MEMORY_RETRIEVAL_ENTRIES
            || max_summaries > MAX_MEMORY_RETRIEVAL_SUMMARIES
            || max_bytes > MAX_MEMORY_RETRIEVAL_BYTES
            || max_sources > MAX_MEMORY_RETRIEVAL_SOURCES
        {
            return Err(DomainError::InvalidMemoryRetrievalBudget);
        }
        Ok(Self {
            max_entries,
            max_summaries,
            max_bytes,
            max_sources,
        })
    }
    pub fn max_entries(&self) -> u32 {
        self.max_entries
    }
    pub fn max_summaries(&self) -> u32 {
        self.max_summaries
    }
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    pub fn max_sources(&self) -> u64 {
        self.max_sources
    }
}

impl Default for MemoryRetrievalBudget {
    fn default() -> Self {
        Self::new(32, 8, 32_768, 128).expect("compiled retrieval defaults are valid")
    }
}

impl<'de> Deserialize<'de> for MemoryRetrievalBudget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemoryRetrievalBudgetWire::deserialize(deserializer)?;
        Self::new(
            wire.max_entries,
            wire.max_summaries,
            wire.max_bytes,
            wire.max_sources,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryRetrievalRequest {
    scope: MemoryRetrievalScope,
    budget: MemoryRetrievalBudget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRetrievalRequestWire {
    scope: MemoryRetrievalScope,
    budget: MemoryRetrievalBudget,
}

impl MemoryRetrievalRequest {
    pub fn new(
        scope: MemoryRetrievalScope,
        budget: MemoryRetrievalBudget,
    ) -> Result<Self, DomainError> {
        scope.purpose.validate()?;
        Ok(Self { scope, budget })
    }
    pub fn scope(&self) -> &MemoryRetrievalScope {
        &self.scope
    }
    pub fn budget(&self) -> &MemoryRetrievalBudget {
        &self.budget
    }
}

impl<'de> Deserialize<'de> for MemoryRetrievalRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemoryRetrievalRequestWire::deserialize(deserializer)?;
        Self::new(wire.scope, wire.budget).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryKvContextItem {
    entry: MemoryEntryRef,
    display_key: String,
    value: String,
    purpose_tags: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryKvContextItemWire {
    entry: MemoryEntryRef,
    display_key: String,
    value: String,
    purpose_tags: Vec<String>,
}

impl MemoryKvContextItem {
    pub fn from_entry(entry: &MemoryEntryVersion) -> Result<Self, DomainError> {
        if entry.reference().state() != MemoryEntryState::Present {
            return Err(DomainError::InvalidMemorySnapshot);
        }
        let value = entry.value().ok_or(DomainError::InvalidMemorySnapshot)?;
        Self::from_parts(
            entry.reference(),
            entry.display_key().to_owned(),
            value.to_owned(),
            entry.purpose_tags().to_vec(),
        )
    }
    fn from_parts(
        entry: MemoryEntryRef,
        display_key: String,
        value: String,
        purpose_tags: Vec<String>,
    ) -> Result<Self, DomainError> {
        let item = Self {
            entry,
            display_key,
            value,
            purpose_tags,
        };
        item.validate()?;
        Ok(item)
    }
    fn validate(&self) -> Result<(), DomainError> {
        if self.entry.state() != MemoryEntryState::Present {
            return Err(DomainError::InvalidMemorySnapshot);
        }
        let draft = MemoryEntryDraft::new(
            self.display_key.clone(),
            self.value.clone(),
            self.purpose_tags.clone(),
        )?;
        if draft.display_key() != self.display_key
            || draft.value() != self.value
            || draft.purpose_tags() != self.purpose_tags
            || draft.normalized_key() != *self.entry.normalized_key()
            || memory_entry_content_digest(
                &self.display_key,
                self.entry.normalized_key(),
                &self.value,
                &self.purpose_tags,
            )? != *self.entry.content_digest()
        {
            return Err(DomainError::InvalidMemorySnapshot);
        }
        Ok(())
    }
    pub fn entry(&self) -> &MemoryEntryRef {
        &self.entry
    }
    pub fn display_key(&self) -> &str {
        &self.display_key
    }
    pub fn value(&self) -> &str {
        &self.value
    }
    pub fn purpose_tags(&self) -> &[String] {
        &self.purpose_tags
    }
}

impl<'de> Deserialize<'de> for MemoryKvContextItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemoryKvContextItemWire::deserialize(deserializer)?;
        Self::from_parts(wire.entry, wire.display_key, wire.value, wire.purpose_tags)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EpisodicContextItem {
    summary: EpisodicSummaryRef,
    qualification: EpisodicQualification,
    label: String,
    body: String,
    purpose_tags: Vec<String>,
    sources: Vec<EpisodicSourceRef>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EpisodicContextItemWire {
    summary: EpisodicSummaryRef,
    qualification: EpisodicQualification,
    label: String,
    body: String,
    purpose_tags: Vec<String>,
    sources: Vec<EpisodicSourceRef>,
}

impl EpisodicContextItem {
    pub fn from_summary(summary: &EpisodicSummary) -> Result<Self, DomainError> {
        Self::from_parts(
            summary.reference(),
            EpisodicQualification::SummaryVerifySources,
            summary.label().to_owned(),
            summary.body().to_owned(),
            summary.purpose_tags().to_vec(),
            summary.sources().to_vec(),
        )
    }
    fn from_parts(
        summary: EpisodicSummaryRef,
        qualification: EpisodicQualification,
        label: String,
        body: String,
        purpose_tags: Vec<String>,
        sources: Vec<EpisodicSourceRef>,
    ) -> Result<Self, DomainError> {
        let item = Self {
            summary,
            qualification,
            label,
            body,
            purpose_tags,
            sources,
        };
        item.validate()?;
        Ok(item)
    }
    fn validate(&self) -> Result<(), DomainError> {
        if self.qualification != EpisodicQualification::SummaryVerifySources {
            return Err(DomainError::InvalidMemorySnapshot);
        }
        let label =
            super::entry::canonicalize_memory_single_line("episodic_label", &self.label, 1, 128)?;
        let body =
            super::entry::canonicalize_memory_multiline("episodic_body", &self.body, 1, 8_192)?;
        super::validate_plaintext(
            super::PLAINTEXT_VALIDATION_VERSION_V1,
            super::PlaintextField::EpisodicLabel,
            &label,
        )?;
        super::validate_plaintext(
            super::PLAINTEXT_VALIDATION_VERSION_V1,
            super::PlaintextField::EpisodicBody,
            &body,
        )?;
        if label != self.label
            || body != self.body
            || canonical_purpose_tags(self.purpose_tags.clone())? != self.purpose_tags
            || self.sources.is_empty()
            || self.sources.len() > MAX_MEMORY_RETRIEVAL_SOURCES as usize
            || sha256(&canonical_json_bytes(&self.sources)?) != *self.summary.source_set_digest()
            || super::episodic::episodic_content_digest(
                &self.label,
                &self.body,
                &self.purpose_tags,
                super::PLAINTEXT_VALIDATION_VERSION_V1,
                self.summary.source_set_digest(),
            )? != *self.summary.content_digest()
        {
            return Err(DomainError::InvalidMemorySnapshot);
        }
        let mut previous = None;
        let mut ids = BTreeSet::new();
        for source in &self.sources {
            if previous.is_some_and(|sequence| source.sequence() <= sequence)
                || !ids.insert(source.event_id())
                || source.sequence() >= self.summary.creation_event_sequence()
                || source.event_id() == self.summary.creation_event_id()
            {
                return Err(DomainError::InvalidMemorySnapshot);
            }
            previous = Some(source.sequence());
        }
        Ok(())
    }
    pub fn summary(&self) -> &EpisodicSummaryRef {
        &self.summary
    }
    pub fn qualification(&self) -> EpisodicQualification {
        self.qualification
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
}

impl<'de> Deserialize<'de> for EpisodicContextItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = EpisodicContextItemWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.summary,
            wire.qualification,
            wire.label,
            wire.body,
            wire.purpose_tags,
            wire.sources,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MemorySnapshotAccounting {
    eligible_entry_count: u64,
    accepted_entry_count: u64,
    omitted_entry_count: u64,
    eligible_summary_count: u64,
    accepted_summary_count: u64,
    omitted_summary_count: u64,
    accepted_byte_count: u64,
    omitted_byte_count: u64,
    accepted_source_count: u64,
    omitted_source_count: u64,
}

impl MemorySnapshotAccounting {
    pub fn eligible_entry_count(&self) -> u64 {
        self.eligible_entry_count
    }
    pub fn accepted_entry_count(&self) -> u64 {
        self.accepted_entry_count
    }
    pub fn omitted_entry_count(&self) -> u64 {
        self.omitted_entry_count
    }
    pub fn eligible_summary_count(&self) -> u64 {
        self.eligible_summary_count
    }
    pub fn accepted_summary_count(&self) -> u64 {
        self.accepted_summary_count
    }
    pub fn omitted_summary_count(&self) -> u64 {
        self.omitted_summary_count
    }
    pub fn accepted_byte_count(&self) -> u64 {
        self.accepted_byte_count
    }
    pub fn omitted_byte_count(&self) -> u64 {
        self.omitted_byte_count
    }
    pub fn accepted_source_count(&self) -> u64 {
        self.accepted_source_count
    }
    pub fn omitted_source_count(&self) -> u64 {
        self.omitted_source_count
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemorySnapshot {
    scope: MemoryRetrievalScope,
    budget: MemoryRetrievalBudget,
    entries: Vec<MemoryKvContextItem>,
    summaries: Vec<EpisodicContextItem>,
    accounting: MemorySnapshotAccounting,
    snapshot_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemorySnapshotWire {
    scope: MemoryRetrievalScope,
    budget: MemoryRetrievalBudget,
    entries: Vec<MemoryKvContextItem>,
    summaries: Vec<EpisodicContextItem>,
    accounting: MemorySnapshotAccounting,
    snapshot_digest: Digest,
}

impl MemorySnapshot {
    pub fn scope(&self) -> &MemoryRetrievalScope {
        &self.scope
    }
    pub fn budget(&self) -> &MemoryRetrievalBudget {
        &self.budget
    }
    pub fn entries(&self) -> &[MemoryKvContextItem] {
        &self.entries
    }
    pub fn summaries(&self) -> &[EpisodicContextItem] {
        &self.summaries
    }
    pub fn accounting(&self) -> &MemorySnapshotAccounting {
        &self.accounting
    }
    pub fn snapshot_digest(&self) -> &Digest {
        &self.snapshot_digest
    }
    pub fn metadata(&self) -> MemorySnapshotMetadata {
        MemorySnapshotMetadata {
            scope: self.scope.clone(),
            budget: self.budget.clone(),
            entry_refs: self.entries.iter().map(|item| item.entry.clone()).collect(),
            summary_refs: self
                .summaries
                .iter()
                .map(|item| item.summary.clone())
                .collect(),
            accounting: self.accounting.clone(),
            snapshot_digest: self.snapshot_digest.clone(),
        }
    }
    fn from_parts(
        scope: MemoryRetrievalScope,
        budget: MemoryRetrievalBudget,
        entries: Vec<MemoryKvContextItem>,
        summaries: Vec<EpisodicContextItem>,
        accounting: MemorySnapshotAccounting,
        snapshot_digest: Digest,
    ) -> Result<Self, DomainError> {
        let snapshot = Self {
            scope,
            budget,
            entries,
            summaries,
            accounting,
            snapshot_digest,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
    fn validate(&self) -> Result<(), DomainError> {
        validate_snapshot_shape(
            &self.scope,
            &self.budget,
            &self.entries,
            &self.summaries,
            &self.accounting,
        )?;
        if self.compute_digest()? != self.snapshot_digest {
            return Err(DomainError::InvalidMemorySnapshot);
        }
        Ok(())
    }
    fn compute_digest(&self) -> Result<Digest, DomainError> {
        snapshot_digest(
            &self.scope,
            &self.budget,
            self.entries.iter().map(|item| &item.entry).collect(),
            self.summaries.iter().map(|item| &item.summary).collect(),
            &self.accounting,
        )
    }
}

impl<'de> Deserialize<'de> for MemorySnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemorySnapshotWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.scope,
            wire.budget,
            wire.entries,
            wire.summaries,
            wire.accounting,
            wire.snapshot_digest,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemorySnapshotMetadata {
    scope: MemoryRetrievalScope,
    budget: MemoryRetrievalBudget,
    entry_refs: Vec<MemoryEntryRef>,
    summary_refs: Vec<EpisodicSummaryRef>,
    accounting: MemorySnapshotAccounting,
    snapshot_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemorySnapshotMetadataWire {
    scope: MemoryRetrievalScope,
    budget: MemoryRetrievalBudget,
    entry_refs: Vec<MemoryEntryRef>,
    summary_refs: Vec<EpisodicSummaryRef>,
    accounting: MemorySnapshotAccounting,
    snapshot_digest: Digest,
}

impl MemorySnapshotMetadata {
    pub fn scope(&self) -> &MemoryRetrievalScope {
        &self.scope
    }
    pub fn budget(&self) -> &MemoryRetrievalBudget {
        &self.budget
    }
    pub fn entry_refs(&self) -> &[MemoryEntryRef] {
        &self.entry_refs
    }
    pub fn summary_refs(&self) -> &[EpisodicSummaryRef] {
        &self.summary_refs
    }
    pub fn accounting(&self) -> &MemorySnapshotAccounting {
        &self.accounting
    }
    pub fn snapshot_digest(&self) -> &Digest {
        &self.snapshot_digest
    }
    fn validate(&self) -> Result<(), DomainError> {
        validate_metadata_shape(
            &self.scope,
            &self.budget,
            &self.entry_refs,
            &self.summary_refs,
            &self.accounting,
        )?;
        if snapshot_digest(
            &self.scope,
            &self.budget,
            self.entry_refs.iter().collect(),
            self.summary_refs.iter().collect(),
            &self.accounting,
        )? != self.snapshot_digest
        {
            return Err(DomainError::InvalidMemorySnapshot);
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for MemorySnapshotMetadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemorySnapshotMetadataWire::deserialize(deserializer)?;
        let metadata = Self {
            scope: wire.scope,
            budget: wire.budget,
            entry_refs: wire.entry_refs,
            summary_refs: wire.summary_refs,
            accounting: wire.accounting,
            snapshot_digest: wire.snapshot_digest,
        };
        metadata.validate().map_err(serde::de::Error::custom)?;
        Ok(metadata)
    }
}

pub fn select_snapshot<E, S>(
    request: &MemoryRetrievalRequest,
    entries: E,
    summaries: S,
) -> Result<MemorySnapshot, DomainError>
where
    E: IntoIterator<Item = Result<MemoryKvContextItem, DomainError>>,
    S: IntoIterator<Item = Result<EpisodicContextItem, DomainError>>,
{
    let mut entries = entries
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|item| entry_is_eligible(request.scope(), item))
        .collect::<Vec<_>>();
    let mut summaries = summaries
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|item| summary_is_eligible(request.scope(), item))
        .collect::<Vec<_>>();
    entries.sort_by_key(|item| {
        (
            purpose_rank(request.scope().purpose(), item.purpose_tags()),
            item.entry.normalized_key().clone(),
            item.entry.entry_id(),
            item.entry.entry_version_id(),
        )
    });
    summaries.sort_by_key(|item| {
        (
            purpose_rank(request.scope().purpose(), item.purpose_tags()),
            Reverse(item.summary.creation_event_sequence()),
            item.summary.creation_event_id(),
            item.summary.summary_id(),
        )
    });
    reject_duplicate_refs(&entries, &summaries)?;
    let mut builder = MemorySnapshotBuilder::new(request.clone())?;
    for item in entries {
        builder.consider_entry(item)?;
    }
    for item in summaries {
        builder.consider_summary(item)?;
    }
    builder.finish()
}

pub(crate) struct MemorySnapshotBuilder {
    request: MemoryRetrievalRequest,
    entries: Vec<MemoryKvContextItem>,
    summaries: Vec<EpisodicContextItem>,
    accounting: MemorySnapshotAccounting,
}

impl MemorySnapshotBuilder {
    pub(crate) fn new(request: MemoryRetrievalRequest) -> Result<Self, DomainError> {
        request.scope.purpose.validate()?;
        Ok(Self {
            request,
            entries: Vec::new(),
            summaries: Vec::new(),
            accounting: MemorySnapshotAccounting::default(),
        })
    }
    pub(crate) fn consider_entry(&mut self, item: MemoryKvContextItem) -> Result<(), DomainError> {
        if !entry_is_eligible(self.request.scope(), &item) {
            return Ok(());
        }
        let bytes = byte_cost(&item)?;
        let mut next = self.accounting.clone();
        increment(&mut next.eligible_entry_count, 1)?;
        let admit = self.entries.len() < self.request.budget.max_entries as usize
            && next
                .accepted_byte_count
                .checked_add(bytes)
                .ok_or(DomainError::MemoryRetrievalOverflow)?
                <= self.request.budget.max_bytes;
        if admit {
            increment(&mut next.accepted_entry_count, 1)?;
            increment(&mut next.accepted_byte_count, bytes)?;
            self.entries.push(item);
        } else {
            increment(&mut next.omitted_entry_count, 1)?;
            increment(&mut next.omitted_byte_count, bytes)?;
        }
        self.accounting = next;
        Ok(())
    }
    pub(crate) fn consider_summary(
        &mut self,
        item: EpisodicContextItem,
    ) -> Result<(), DomainError> {
        if !summary_is_eligible(self.request.scope(), &item) {
            return Ok(());
        }
        let bytes = byte_cost(&item)?;
        let sources =
            u64::try_from(item.sources.len()).map_err(|_| DomainError::MemoryRetrievalOverflow)?;
        let mut next = self.accounting.clone();
        increment(&mut next.eligible_summary_count, 1)?;
        let admit = self.summaries.len() < self.request.budget.max_summaries as usize
            && next
                .accepted_byte_count
                .checked_add(bytes)
                .ok_or(DomainError::MemoryRetrievalOverflow)?
                <= self.request.budget.max_bytes
            && next
                .accepted_source_count
                .checked_add(sources)
                .ok_or(DomainError::MemoryRetrievalOverflow)?
                <= self.request.budget.max_sources;
        if admit {
            increment(&mut next.accepted_summary_count, 1)?;
            increment(&mut next.accepted_byte_count, bytes)?;
            increment(&mut next.accepted_source_count, sources)?;
            self.summaries.push(item);
        } else {
            increment(&mut next.omitted_summary_count, 1)?;
            increment(&mut next.omitted_byte_count, bytes)?;
            increment(&mut next.omitted_source_count, sources)?;
        }
        self.accounting = next;
        Ok(())
    }
    pub(crate) fn finish(self) -> Result<MemorySnapshot, DomainError> {
        let digest = snapshot_digest(
            &self.request.scope,
            &self.request.budget,
            self.entries.iter().map(|item| &item.entry).collect(),
            self.summaries.iter().map(|item| &item.summary).collect(),
            &self.accounting,
        )?;
        MemorySnapshot::from_parts(
            self.request.scope,
            self.request.budget,
            self.entries,
            self.summaries,
            self.accounting,
            digest,
        )
    }
}

fn canonical_purpose_tags(tags: Vec<String>) -> Result<Vec<String>, DomainError> {
    let draft = MemoryEntryDraft::new("retrieval tags".to_owned(), "valid".to_owned(), tags)?;
    Ok(draft.purpose_tags().to_vec())
}
fn entry_is_eligible(scope: &MemoryRetrievalScope, item: &MemoryKvContextItem) -> bool {
    item.entry.namespace_id() == scope.namespace_id
        && item.entry.state() == MemoryEntryState::Present
        && purpose_matches(scope.purpose(), item.purpose_tags())
}
fn summary_is_eligible(scope: &MemoryRetrievalScope, item: &EpisodicContextItem) -> bool {
    item.summary.namespace_id() == scope.namespace_id
        && item.summary.profile() == scope.profile()
        && purpose_matches(scope.purpose(), item.purpose_tags())
}
fn purpose_matches(purpose: &MemoryPurposeScope, tags: &[String]) -> bool {
    match purpose {
        MemoryPurposeScope::General => true,
        MemoryPurposeScope::Tagged(wanted) => {
            tags.is_empty() || tags.iter().any(|tag| wanted.contains(tag))
        }
    }
}
fn purpose_rank(purpose: &MemoryPurposeScope, tags: &[String]) -> u8 {
    match purpose {
        MemoryPurposeScope::General => 0,
        MemoryPurposeScope::Tagged(wanted) if tags.iter().any(|tag| wanted.contains(tag)) => 0,
        MemoryPurposeScope::Tagged(_) => 1,
    }
}
fn byte_cost<T: Serialize>(item: &T) -> Result<u64, DomainError> {
    u64::try_from(canonical_json_bytes(item)?.len())
        .map_err(|_| DomainError::MemoryRetrievalOverflow)
}
fn memory_entry_content_digest(
    display_key: &str,
    normalized_key: &super::NormalizedMemoryKey,
    value: &str,
    purpose_tags: &[String],
) -> Result<Digest, DomainError> {
    Ok(sha256(&canonical_json_bytes(
        &MemoryEntryContentDigestMaterial {
            display_key,
            normalized_key,
            state: MemoryEntryState::Present,
            value: Some(value),
            purpose_tags,
            plaintext_validation_version: super::PLAINTEXT_VALIDATION_VERSION_V1,
        },
    )?))
}
fn increment(counter: &mut u64, by: u64) -> Result<(), DomainError> {
    *counter = counter
        .checked_add(by)
        .ok_or(DomainError::MemoryRetrievalOverflow)?;
    Ok(())
}
fn reject_duplicate_refs(
    entries: &[MemoryKvContextItem],
    summaries: &[EpisodicContextItem],
) -> Result<(), DomainError> {
    let mut entry_ids = BTreeSet::new();
    for item in entries {
        if !entry_ids.insert((item.entry.entry_id(), item.entry.entry_version_id())) {
            return Err(DomainError::InvalidMemorySnapshot);
        }
    }
    let mut summary_ids = BTreeSet::new();
    for item in summaries {
        if !summary_ids.insert(item.summary.summary_id()) {
            return Err(DomainError::InvalidMemorySnapshot);
        }
    }
    Ok(())
}
fn validate_snapshot_shape(
    scope: &MemoryRetrievalScope,
    budget: &MemoryRetrievalBudget,
    entries: &[MemoryKvContextItem],
    summaries: &[EpisodicContextItem],
    accounting: &MemorySnapshotAccounting,
) -> Result<(), DomainError> {
    if entries.len() > budget.max_entries as usize
        || summaries.len() > budget.max_summaries as usize
    {
        return Err(DomainError::InvalidMemorySnapshot);
    }
    if entries.iter().any(|item| !entry_is_eligible(scope, item))
        || summaries
            .iter()
            .any(|item| !summary_is_eligible(scope, item))
    {
        return Err(DomainError::InvalidMemorySnapshot);
    }
    let mut sorted_entries = entries.to_vec();
    let mut sorted_summaries = summaries.to_vec();
    sorted_entries.sort_by_key(|item| {
        (
            purpose_rank(scope.purpose(), item.purpose_tags()),
            item.entry.normalized_key().clone(),
            item.entry.entry_id(),
            item.entry.entry_version_id(),
        )
    });
    sorted_summaries.sort_by_key(|item| {
        (
            purpose_rank(scope.purpose(), item.purpose_tags()),
            Reverse(item.summary.creation_event_sequence()),
            item.summary.creation_event_id(),
            item.summary.summary_id(),
        )
    });
    if sorted_entries != entries || sorted_summaries != summaries {
        return Err(DomainError::InvalidMemorySnapshot);
    }
    reject_duplicate_refs(entries, summaries)?;
    validate_accounting(budget, entries, summaries, accounting)
}
fn validate_metadata_shape(
    scope: &MemoryRetrievalScope,
    budget: &MemoryRetrievalBudget,
    entries: &[MemoryEntryRef],
    summaries: &[EpisodicSummaryRef],
    accounting: &MemorySnapshotAccounting,
) -> Result<(), DomainError> {
    if entries.len() > budget.max_entries as usize
        || summaries.len() > budget.max_summaries as usize
    {
        return Err(DomainError::InvalidMemorySnapshot);
    }
    if entries.iter().any(|entry| {
        entry.namespace_id() != scope.namespace_id || entry.state() != MemoryEntryState::Present
    }) || summaries.iter().any(|summary| {
        summary.namespace_id() != scope.namespace_id || summary.profile() != scope.profile()
    }) {
        return Err(DomainError::InvalidMemorySnapshot);
    }
    let mut keys = BTreeSet::new();
    if entries
        .iter()
        .any(|entry| !keys.insert((entry.entry_id(), entry.entry_version_id())))
    {
        return Err(DomainError::InvalidMemorySnapshot);
    }
    let mut summary_ids = BTreeSet::new();
    if summaries
        .iter()
        .any(|summary| !summary_ids.insert(summary.summary_id()))
    {
        return Err(DomainError::InvalidMemorySnapshot);
    }
    if accounting.accepted_entry_count
        != u64::try_from(entries.len()).map_err(|_| DomainError::MemoryRetrievalOverflow)?
        || accounting.accepted_summary_count
            != u64::try_from(summaries.len()).map_err(|_| DomainError::MemoryRetrievalOverflow)?
        || accounting.eligible_entry_count
            != accounting
                .accepted_entry_count
                .checked_add(accounting.omitted_entry_count)
                .ok_or(DomainError::MemoryRetrievalOverflow)?
        || accounting.eligible_summary_count
            != accounting
                .accepted_summary_count
                .checked_add(accounting.omitted_summary_count)
                .ok_or(DomainError::MemoryRetrievalOverflow)?
    {
        return Err(DomainError::InvalidMemorySnapshot);
    }
    Ok(())
}
fn validate_accounting(
    budget: &MemoryRetrievalBudget,
    entries: &[MemoryKvContextItem],
    summaries: &[EpisodicContextItem],
    accounting: &MemorySnapshotAccounting,
) -> Result<(), DomainError> {
    let entry_count =
        u64::try_from(entries.len()).map_err(|_| DomainError::MemoryRetrievalOverflow)?;
    let summary_count =
        u64::try_from(summaries.len()).map_err(|_| DomainError::MemoryRetrievalOverflow)?;
    let accepted_bytes = entries
        .iter()
        .map(byte_cost)
        .chain(summaries.iter().map(byte_cost))
        .try_fold(0_u64, |sum, cost| {
            sum.checked_add(cost?)
                .ok_or(DomainError::MemoryRetrievalOverflow)
        })?;
    let accepted_sources = summaries.iter().try_fold(0_u64, |sum, item| {
        sum.checked_add(
            u64::try_from(item.sources.len()).map_err(|_| DomainError::MemoryRetrievalOverflow)?,
        )
        .ok_or(DomainError::MemoryRetrievalOverflow)
    })?;
    if accounting.accepted_entry_count != entry_count
        || accounting.accepted_summary_count != summary_count
        || accounting.accepted_byte_count != accepted_bytes
        || accounting.accepted_source_count != accepted_sources
        || accounting.eligible_entry_count
            != accounting
                .accepted_entry_count
                .checked_add(accounting.omitted_entry_count)
                .ok_or(DomainError::MemoryRetrievalOverflow)?
        || accounting.eligible_summary_count
            != accounting
                .accepted_summary_count
                .checked_add(accounting.omitted_summary_count)
                .ok_or(DomainError::MemoryRetrievalOverflow)?
        || accepted_bytes > budget.max_bytes
        || accepted_sources > budget.max_sources
    {
        return Err(DomainError::InvalidMemorySnapshot);
    }
    Ok(())
}
fn snapshot_digest(
    scope: &MemoryRetrievalScope,
    budget: &MemoryRetrievalBudget,
    entries: Vec<&MemoryEntryRef>,
    summaries: Vec<&EpisodicSummaryRef>,
    accounting: &MemorySnapshotAccounting,
) -> Result<Digest, DomainError> {
    Ok(sha256(&canonical_json_bytes(&SnapshotDigestMaterial {
        scope,
        budget,
        entries,
        summaries,
        accounting,
    })?))
}
#[derive(Serialize)]
struct SnapshotDigestMaterial<'a> {
    scope: &'a MemoryRetrievalScope,
    budget: &'a MemoryRetrievalBudget,
    entries: Vec<&'a MemoryEntryRef>,
    summaries: Vec<&'a EpisodicSummaryRef>,
    accounting: &'a MemorySnapshotAccounting,
}

#[derive(Serialize)]
struct MemoryEntryContentDigestMaterial<'a> {
    display_key: &'a str,
    normalized_key: &'a super::NormalizedMemoryKey,
    state: MemoryEntryState,
    value: Option<&'a str>,
    purpose_tags: &'a [String],
    plaintext_validation_version: u16,
}
