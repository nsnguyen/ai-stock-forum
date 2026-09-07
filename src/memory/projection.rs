#![allow(dead_code)] // Crate-private reducer hooks are wired by later memory tasks.

use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::domain::{
    ApprovalId, DomainError, EventId, MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId,
    MemoryProposalId,
};

use super::{
    ExpectedMemoryEntryState, MemoryEntryRef, MemoryEntryVersion, MemoryProposal,
    MemoryProposalRef, MemoryProposalResolution, MemoryProposalStatus, NormalizedMemoryKey,
};

pub const MAX_PENDING_MEMORY_PROPOSALS: usize = 256;

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
        if self.current_entries.iter().any(|(existing_key, existing)| {
            existing_key != &key
                && (existing.entry_id() == reference.entry_id()
                    || existing.entry_version_id() == reference.entry_version_id())
        }) {
            return Err(DomainError::InvalidMemoryProjection);
        }
        match self.current_entries.get(&key) {
            None if reference.version().get() == 1 && entry.predecessor_version_id().is_none() => {}
            Some(current)
                if reference.version().get()
                    == current
                        .version()
                        .get()
                        .checked_add(1)
                        .ok_or(DomainError::InvalidMemoryProjection)?
                    && entry.predecessor_version_id() == Some(current.entry_version_id())
                    && current.entry_id() == reference.entry_id() => {}
            _ => return Err(DomainError::InvalidMemoryProjection),
        }
        let mut staged = self.clone();
        staged.current_entries.insert(key, reference);
        staged.validate_structure()?;
        *self = staged;
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
            .filter(|projected| {
                projected.namespace_id == proposal.namespace_id()
                    && projected.status == MemoryProposalStatus::Pending
            })
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
        let mut entry_versions = BTreeSet::<MemoryEntryVersionId>::new();
        let mut entry_ids = BTreeSet::<MemoryEntryId>::new();
        for ((namespace, key), reference) in &self.current_entries {
            if *namespace != reference.namespace_id()
                || key != reference.normalized_key()
                || !entry_versions.insert(reference.entry_version_id())
                || !entry_ids.insert(reference.entry_id())
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
        if self
            .proposals
            .values()
            .filter(|projected| projected.status == MemoryProposalStatus::Pending)
            .fold(
                BTreeMap::<MemoryNamespaceId, usize>::new(),
                |mut counts, projected| {
                    *counts.entry(projected.namespace_id).or_default() += 1;
                    counts
                },
            )
            .values()
            .any(|&count| count > MAX_PENDING_MEMORY_PROPOSALS)
        {
            return Err(DomainError::MemoryProposalCapacityReached);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
        domain::{
            Actor, AgentProfileId, AgentProfileVersionId, EventId, MemoryEntryId,
            MemoryEntryVersionId,
        },
        memory::{MemoryEntryDraft, MemoryEntryState, MemoryProposalOperation},
    };
    use uuid::Uuid;

    fn uuid(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn profile(seed: u128, namespace: MemoryNamespaceId) -> AgentProfileVersion {
        AgentProfileVersion::create(
            AgentProfileId::from_uuid(uuid(seed)),
            AgentProfileVersionId::from_uuid(uuid(seed + 1)),
            namespace,
            1,
            AgentProfileDraft::new(
                format!("Profile {seed}"),
                "Profile.".to_owned(),
                AgentRole::Custom,
                "research".to_owned(),
                vec![],
                "Careful.".to_owned(),
                "Review.".to_owned(),
                AgentBindings::default(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap(),
            None,
        )
        .unwrap()
    }

    fn proposal(profile: &AgentProfileVersion, seed: u128, key: &str) -> MemoryProposal {
        let candidate =
            MemoryEntryDraft::new(key.to_owned(), format!("Value {seed}"), vec![]).unwrap();
        MemoryProposal::new(
            MemoryProposalId::from_uuid(uuid(seed)),
            profile,
            &Actor::Agent(profile.profile_id()),
            MemoryProposalOperation::Set { candidate },
            key.to_owned(),
            ExpectedMemoryEntryState::Absent,
            format!("Reason {seed}"),
            1,
            EventId::from_uuid(uuid(seed + 1_000)),
            ApprovalId::from_uuid(uuid(seed + 2_000)),
        )
        .unwrap()
    }

    fn entry(
        namespace: MemoryNamespaceId,
        entry_id: u128,
        version_id: u128,
        key: &str,
    ) -> MemoryEntryVersion {
        MemoryEntryVersion::create_present(
            namespace,
            MemoryEntryId::from_uuid(uuid(entry_id)),
            MemoryEntryVersionId::from_uuid(uuid(version_id)),
            MemoryEntryDraft::new(key.to_owned(), "Value".to_owned(), vec![]).unwrap(),
            Actor::Human,
            1,
            None,
            EventId::from_uuid(uuid(version_id + 10_000)),
        )
        .unwrap()
    }

    #[test]
    fn proposals_are_capped_per_namespace_in_creation_and_from_parts() {
        let first_namespace = MemoryNamespaceId::from_uuid(uuid(1));
        let first = profile(10, first_namespace);
        let second = profile(20, MemoryNamespaceId::from_uuid(uuid(2)));
        let mut projection = MemoryProjection::default();
        for index in 0..256 {
            projection
                .create_proposal(&proposal(&first, 100 + index, &format!("first {index}")))
                .unwrap();
        }
        assert!(
            projection
                .create_proposal(&proposal(&second, 1_000, "second"))
                .is_ok()
        );
        assert!(
            projection
                .create_proposal(&proposal(&first, 2_000, "over cap"))
                .is_err()
        );

        let mut oversized = BTreeMap::new();
        for index in 0..257 {
            let proposal = proposal(&first, 3_000 + index, &format!("wire {index}"));
            oversized.insert(
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
        }
        assert!(MemoryProjection::from_parts(BTreeMap::new(), oversized).is_err());
    }

    #[test]
    fn projection_retains_tombstones_rejects_identity_reuse_and_allows_one_resolution() {
        let namespace = MemoryNamespaceId::from_uuid(uuid(30));
        let first = entry(namespace, 31, 32, "First");
        let tombstone = first
            .next_deleted(
                MemoryEntryVersionId::from_uuid(uuid(33)),
                Actor::Human,
                2,
                None,
                EventId::from_uuid(uuid(34)),
            )
            .unwrap();
        let duplicate_identity = entry(namespace, 31, 35, "Second");
        let mut projection = MemoryProjection::default();
        projection.apply_entry(&first).unwrap();
        projection.apply_entry(&tombstone).unwrap();
        assert_eq!(
            projection
                .current_entry(namespace, first.reference().normalized_key())
                .unwrap()
                .state(),
            MemoryEntryState::Deleted
        );
        assert!(projection.apply_entry(&duplicate_identity).is_err());
        assert!(
            projection
                .current_entry(namespace, duplicate_identity.reference().normalized_key())
                .is_none()
        );

        let profile = profile(40, namespace);
        let proposal = proposal(&profile, 41, "Proposal");
        projection.create_proposal(&proposal).unwrap();
        let resolution = MemoryProposalResolution::new(
            proposal.reference(),
            MemoryProposalStatus::Rejected,
            proposal.approval_id(),
            Actor::Human,
            3,
            EventId::from_uuid(uuid(42)),
        )
        .unwrap();
        projection.resolve_proposal(&resolution).unwrap();
        assert!(projection.resolve_proposal(&resolution).is_err());
    }

    #[test]
    fn from_parts_rejects_malformed_entry_map_identity_deterministically() {
        let namespace = MemoryNamespaceId::from_uuid(uuid(50));
        let entry = entry(namespace, 51, 52, "Correct");
        let mut entries = BTreeMap::new();
        entries.insert(
            (namespace, NormalizedMemoryKey::new("wrong").unwrap()),
            entry.reference(),
        );
        assert_eq!(
            MemoryProjection::from_parts(entries, BTreeMap::new()),
            Err(DomainError::InvalidMemoryProjection)
        );
    }
}
