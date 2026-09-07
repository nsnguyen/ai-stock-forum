#![allow(dead_code)] // Crate-private reducer hooks are wired by later memory tasks.

use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::domain::{ApprovalId, DomainError, EventId, MemoryNamespaceId, MemoryProposalId};

use super::{
    ExpectedMemoryEntryState, MemoryEntryRef, MemoryEntryVersion, MemoryProposal,
    MemoryProposalRef, MemoryProposalResolution, MemoryProposalStatus, NormalizedMemoryKey,
};

pub const MAX_PENDING_MEMORY_PROPOSALS: usize = 64;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemoryProjection {
    current_entries: BTreeMap<(MemoryNamespaceId, NormalizedMemoryKey), MemoryEntryRef>,
    proposals: BTreeMap<MemoryProposalId, ProjectedMemoryProposal>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectedMemoryProposal {
    proposal: MemoryProposalRef,
    namespace_id: MemoryNamespaceId,
    normalized_key: NormalizedMemoryKey,
    expected: ExpectedMemoryEntryState,
    approval_id: ApprovalId,
    status: MemoryProposalStatus,
    resolution_event_id: Option<EventId>,
}

impl ProjectedMemoryProposal {
    pub(crate) fn proposal(&self) -> &MemoryProposalRef {
        &self.proposal
    }
    pub(crate) fn namespace_id(&self) -> MemoryNamespaceId {
        self.namespace_id
    }
    pub(crate) fn normalized_key(&self) -> &NormalizedMemoryKey {
        &self.normalized_key
    }
    pub(crate) fn expected(&self) -> &ExpectedMemoryEntryState {
        &self.expected
    }
    pub(crate) fn approval_id(&self) -> ApprovalId {
        self.approval_id
    }
    pub(crate) fn status(&self) -> MemoryProposalStatus {
        self.status
    }
    pub(crate) fn resolution_event_id(&self) -> Option<EventId> {
        self.resolution_event_id
    }
}

impl MemoryProjection {
    pub(crate) fn from_parts(
        current_entries: BTreeMap<(MemoryNamespaceId, NormalizedMemoryKey), MemoryEntryRef>,
        proposals: BTreeMap<MemoryProposalId, ProjectedMemoryProposal>,
    ) -> Result<Self, DomainError> {
        let projection = Self {
            current_entries,
            proposals,
        };
        projection.validate_structure()?;
        Ok(projection)
    }

    pub fn is_empty(&self) -> bool {
        self.current_entries.is_empty() && self.proposals.is_empty()
    }
    pub fn current_entry(
        &self,
        namespace: MemoryNamespaceId,
        key: &NormalizedMemoryKey,
    ) -> Option<&MemoryEntryRef> {
        self.current_entries.get(&(namespace, key.clone()))
    }
    pub(crate) fn current_entries(
        &self,
    ) -> impl Iterator<Item = (&(MemoryNamespaceId, NormalizedMemoryKey), &MemoryEntryRef)> {
        self.current_entries.iter()
    }
    pub(crate) fn proposals(
        &self,
    ) -> impl Iterator<Item = (&MemoryProposalId, &ProjectedMemoryProposal)> {
        self.proposals.iter()
    }

    pub(crate) fn apply_entry(&mut self, entry: &MemoryEntryVersion) -> Result<(), DomainError> {
        let reference = entry.reference();
        let key = (reference.namespace_id(), reference.normalized_key().clone());
        match self.current_entries.get(&key) {
            None if reference.version().get() == 1 && entry.predecessor_version_id().is_none() => {}
            Some(current)
                if reference.version().get()
                    == current
                        .version()
                        .get()
                        .checked_add(1)
                        .ok_or(DomainError::InvalidMemoryProjection)?
                    && entry.predecessor_version_id() == Some(current.entry_version_id()) => {}
            _ => return Err(DomainError::InvalidMemoryProjection),
        }
        self.current_entries.insert(key, reference);
        Ok(())
    }

    pub(crate) fn create_proposal(&mut self, proposal: &MemoryProposal) -> Result<(), DomainError> {
        if self
            .proposals
            .contains_key(&proposal.reference().proposal_id())
        {
            return Err(DomainError::InvalidMemoryProjection);
        }
        if self
            .proposals
            .values()
            .filter(|proposal| proposal.status == MemoryProposalStatus::Pending)
            .count()
            >= MAX_PENDING_MEMORY_PROPOSALS
        {
            return Err(DomainError::MemoryProposalCapacityReached);
        }
        self.proposals.insert(
            proposal.reference().proposal_id(),
            ProjectedMemoryProposal {
                proposal: proposal.reference(),
                namespace_id: proposal.namespace_id(),
                normalized_key: proposal.normalized_key().clone(),
                expected: proposal.expected().clone(),
                approval_id: proposal.approval_id(),
                status: MemoryProposalStatus::Pending,
                resolution_event_id: None,
            },
        );
        Ok(())
    }

    pub(crate) fn resolve_proposal(
        &mut self,
        resolution: &MemoryProposalResolution,
    ) -> Result<(), DomainError> {
        let projected = self
            .proposals
            .get_mut(&resolution.proposal().proposal_id())
            .ok_or(DomainError::InvalidMemoryProjection)?;
        if projected.proposal != *resolution.proposal()
            || projected.approval_id != resolution.approval_id()
            || projected.status != MemoryProposalStatus::Pending
        {
            return Err(DomainError::InvalidMemoryProjection);
        }
        projected.status = resolution.status();
        projected.resolution_event_id = Some(resolution.resolution_event_id());
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), DomainError> {
        let mut entry_versions = BTreeSet::new();
        for ((namespace, key), reference) in &self.current_entries {
            if *namespace != reference.namespace_id()
                || key != reference.normalized_key()
                || !entry_versions.insert(reference.entry_version_id())
            {
                return Err(DomainError::InvalidMemoryProjection);
            }
        }
        for (proposal_id, projected) in &self.proposals {
            if *proposal_id != projected.proposal.proposal_id()
                || expected_matches(
                    projected.namespace_id,
                    &projected.normalized_key,
                    &projected.expected,
                )
                .is_err()
                || matches!(projected.status, MemoryProposalStatus::Pending)
                    != projected.resolution_event_id.is_none()
            {
                return Err(DomainError::InvalidMemoryProjection);
            }
        }
        Ok(())
    }
}

fn expected_matches(
    namespace: MemoryNamespaceId,
    key: &NormalizedMemoryKey,
    expected: &ExpectedMemoryEntryState,
) -> Result<(), DomainError> {
    match expected {
        ExpectedMemoryEntryState::Absent => Ok(()),
        ExpectedMemoryEntryState::Present(reference)
            if reference.namespace_id() == namespace
                && reference.normalized_key() == key
                && reference.state() == super::MemoryEntryState::Present =>
        {
            Ok(())
        }
        ExpectedMemoryEntryState::Deleted(reference)
            if reference.namespace_id() == namespace
                && reference.normalized_key() == key
                && reference.state() == super::MemoryEntryState::Deleted =>
        {
            Ok(())
        }
        _ => Err(DomainError::InvalidMemoryProjection),
    }
}

impl Serialize for MemoryProjection {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct Wire<'a> {
            current_entries: BTreeMap<String, &'a MemoryEntryRef>,
            proposals: BTreeMap<String, &'a ProjectedMemoryProposal>,
        }
        let current_entries = self
            .current_entries
            .iter()
            .map(|((namespace, key), entry)| {
                (format!("{namespace}:{key}", key = key.as_str()), entry)
            })
            .collect();
        let proposals = self
            .proposals
            .iter()
            .map(|(id, proposal)| (id.to_string(), proposal))
            .collect();
        Wire {
            current_entries,
            proposals,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MemoryProjection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            current_entries: BTreeMap<String, MemoryEntryRef>,
            proposals: BTreeMap<String, ProjectedMemoryProposal>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let mut current_entries = BTreeMap::new();
        for (key, entry) in wire.current_entries {
            let (namespace, normalized_key) =
                parse_entry_key(&key).map_err(serde::de::Error::custom)?;
            if current_entries
                .insert((namespace, normalized_key), entry)
                .is_some()
            {
                return Err(serde::de::Error::custom("duplicate current entry key"));
            }
        }
        let mut proposals = BTreeMap::new();
        for (key, proposal) in wire.proposals {
            let id = MemoryProposalId::from_str(&key).map_err(serde::de::Error::custom)?;
            if proposals.insert(id, proposal).is_some() {
                return Err(serde::de::Error::custom("duplicate proposal key"));
            }
        }
        Self::from_parts(current_entries, proposals).map_err(serde::de::Error::custom)
    }
}

fn parse_entry_key(value: &str) -> Result<(MemoryNamespaceId, NormalizedMemoryKey), DomainError> {
    let Some((namespace, key)) = value.split_once(':') else {
        return Err(DomainError::InvalidMemoryProjection);
    };
    let normalized_key = NormalizedMemoryKey::new(key)?;
    if normalized_key.as_str() != key {
        return Err(DomainError::InvalidMemoryProjection);
    }
    Ok((
        MemoryNamespaceId::from_str(namespace).map_err(|_| DomainError::InvalidMemoryProjection)?,
        normalized_key,
    ))
}
