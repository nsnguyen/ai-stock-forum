use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    agents::{AgentProfileVersion, AgentProfileVersionRef},
    domain::{
        Actor, ApprovalId, Digest, DomainError, EventId, MemoryNamespaceId, MemoryProposalId,
        ObjectRef, ObjectVersion, canonical_json_bytes, sha256,
    },
    memory::normalization::{PLAINTEXT_VALIDATION_VERSION_V1, PlaintextField, validate_plaintext},
};

use super::{ExpectedMemoryEntryState, MemoryEntryDraft, MemoryEntryState, NormalizedMemoryKey};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryProposalRef {
    proposal_id: MemoryProposalId,
    version: ObjectVersion,
    content_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryProposalRefWire {
    proposal_id: MemoryProposalId,
    version: ObjectVersion,
    content_digest: Digest,
}

impl MemoryProposalRef {
    pub fn new(
        proposal_id: MemoryProposalId,
        version: ObjectVersion,
        content_digest: Digest,
    ) -> Result<Self, DomainError> {
        if version.get() != 1 {
            return Err(DomainError::InvalidMemoryProposal);
        }
        Ok(Self {
            proposal_id,
            version,
            content_digest,
        })
    }

    pub fn proposal_id(&self) -> MemoryProposalId {
        self.proposal_id
    }
    pub fn version(&self) -> ObjectVersion {
        self.version
    }
    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }
}

impl<'de> Deserialize<'de> for MemoryProposalRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemoryProposalRefWire::deserialize(deserializer)?;
        Self::new(wire.proposal_id, wire.version, wire.content_digest)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryProposalOperation {
    Set { candidate: MemoryEntryDraft },
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryProposal {
    proposal_id: MemoryProposalId,
    version: ObjectVersion,
    proposer: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    operation: MemoryProposalOperation,
    display_key: String,
    normalized_key: NormalizedMemoryKey,
    expected: ExpectedMemoryEntryState,
    rationale: String,
    plaintext_validation_version: u16,
    created_at_ms: i64,
    creation_event_id: EventId,
    approval_id: ApprovalId,
    content_digest: Digest,
    record_digest: Digest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryProposalWire {
    proposal_id: MemoryProposalId,
    version: ObjectVersion,
    proposer: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    operation: MemoryProposalOperation,
    display_key: String,
    normalized_key: NormalizedMemoryKey,
    expected: ExpectedMemoryEntryState,
    rationale: String,
    plaintext_validation_version: u16,
    created_at_ms: i64,
    creation_event_id: EventId,
    approval_id: ApprovalId,
    content_digest: Digest,
    record_digest: Digest,
}

impl MemoryProposal {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        proposal_id: MemoryProposalId,
        proposer: &AgentProfileVersion,
        actor: &Actor,
        operation: MemoryProposalOperation,
        display_key: String,
        expected: ExpectedMemoryEntryState,
        rationale: String,
        created_at_ms: i64,
        creation_event_id: EventId,
        approval_id: ApprovalId,
    ) -> Result<Self, DomainError> {
        if *actor != Actor::Agent(proposer.profile_id()) {
            return Err(DomainError::MemoryProposalActorMismatch);
        }
        let display_key = canonical_display_key(&display_key)?;
        let normalized_key = NormalizedMemoryKey::new(&display_key)?;
        let rationale = canonical_rationale(&rationale)?;
        let mut proposal = Self {
            proposal_id,
            version: ObjectVersion::new(1)?,
            proposer: proposer.reference(),
            namespace_id: proposer.memory_namespace_id(),
            operation,
            display_key,
            normalized_key,
            expected,
            rationale,
            plaintext_validation_version: PLAINTEXT_VALIDATION_VERSION_V1,
            created_at_ms,
            creation_event_id,
            approval_id,
            content_digest: sha256(&[]),
            record_digest: sha256(&[]),
        };
        proposal.validate_structure()?;
        proposal.content_digest = proposal.compute_content_digest()?;
        proposal.record_digest = proposal.compute_record_digest()?;
        Ok(proposal)
    }

    pub fn reference(&self) -> MemoryProposalRef {
        MemoryProposalRef {
            proposal_id: self.proposal_id,
            version: self.version,
            content_digest: self.content_digest.clone(),
        }
    }

    pub fn object_ref(&self) -> Result<ObjectRef, DomainError> {
        ObjectRef::new(
            "memory_proposal",
            self.proposal_id.to_string(),
            self.version,
            self.content_digest.clone(),
        )
    }
    pub fn proposer(&self) -> &AgentProfileVersionRef {
        &self.proposer
    }
    pub fn namespace_id(&self) -> MemoryNamespaceId {
        self.namespace_id
    }
    pub fn operation(&self) -> &MemoryProposalOperation {
        &self.operation
    }
    pub fn display_key(&self) -> &str {
        &self.display_key
    }
    pub fn normalized_key(&self) -> &NormalizedMemoryKey {
        &self.normalized_key
    }
    pub fn expected(&self) -> &ExpectedMemoryEntryState {
        &self.expected
    }
    pub fn rationale(&self) -> &str {
        &self.rationale
    }
    pub fn plaintext_validation_version(&self) -> u16 {
        self.plaintext_validation_version
    }
    pub fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }
    pub fn creation_event_id(&self) -> EventId {
        self.creation_event_id
    }
    pub fn approval_id(&self) -> ApprovalId {
        self.approval_id
    }
    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }
    pub fn record_digest(&self) -> &Digest {
        &self.record_digest
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        proposal_id: MemoryProposalId,
        version: ObjectVersion,
        proposer: AgentProfileVersionRef,
        namespace_id: MemoryNamespaceId,
        operation: MemoryProposalOperation,
        display_key: String,
        normalized_key: NormalizedMemoryKey,
        expected: ExpectedMemoryEntryState,
        rationale: String,
        plaintext_validation_version: u16,
        created_at_ms: i64,
        creation_event_id: EventId,
        approval_id: ApprovalId,
        content_digest: Digest,
        record_digest: Digest,
    ) -> Result<Self, DomainError> {
        let proposal = Self {
            proposal_id,
            version,
            proposer,
            namespace_id,
            operation,
            display_key,
            normalized_key,
            expected,
            rationale,
            plaintext_validation_version,
            created_at_ms,
            creation_event_id,
            approval_id,
            content_digest,
            record_digest,
        };
        proposal.validate_structure()?;
        if proposal.compute_content_digest()? != proposal.content_digest
            || proposal.compute_record_digest()? != proposal.record_digest
        {
            return Err(DomainError::InvalidMemoryProposal);
        }
        Ok(proposal)
    }

    fn validate_structure(&self) -> Result<(), DomainError> {
        if self.version.get() != 1
            || self.plaintext_validation_version != PLAINTEXT_VALIDATION_VERSION_V1
        {
            return Err(DomainError::InvalidMemoryProposal);
        }
        if canonical_display_key(&self.display_key)? != self.display_key
            || NormalizedMemoryKey::new(&self.display_key)? != self.normalized_key
            || canonical_rationale(&self.rationale)? != self.rationale
        {
            return Err(DomainError::InvalidMemoryProposal);
        }
        match (&self.operation, &self.expected) {
            (MemoryProposalOperation::Set { candidate }, _) => {
                if candidate.display_key() != self.display_key
                    || candidate.normalized_key() != self.normalized_key
                {
                    return Err(DomainError::InvalidMemoryProposal);
                }
            }
            (MemoryProposalOperation::Delete, ExpectedMemoryEntryState::Present(reference))
                if reference.namespace_id() == self.namespace_id
                    && reference.state() == MemoryEntryState::Present
                    && reference.normalized_key() == &self.normalized_key => {}
            _ => return Err(DomainError::InvalidMemoryProposal),
        }
        if !expected_state_is_well_formed(&self.expected)
            || expected_ref(&self.expected).is_some_and(|reference| {
                reference.namespace_id() != self.namespace_id
                    || reference.normalized_key() != &self.normalized_key
            })
        {
            return Err(DomainError::InvalidMemoryProposal);
        }
        Ok(())
    }

    fn compute_content_digest(&self) -> Result<Digest, DomainError> {
        Ok(sha256(&canonical_json_bytes(
            &MemoryProposalContentDigestMaterial {
                proposer: &self.proposer,
                namespace_id: self.namespace_id,
                operation: &self.operation,
                display_key: &self.display_key,
                normalized_key: &self.normalized_key,
                expected: &self.expected,
                rationale: &self.rationale,
                plaintext_validation_version: self.plaintext_validation_version,
            },
        )?))
    }

    fn compute_record_digest(&self) -> Result<Digest, DomainError> {
        Ok(sha256(&canonical_json_bytes(
            &MemoryProposalRecordDigestMaterial {
                proposal_id: self.proposal_id,
                version: self.version,
                proposer: &self.proposer,
                namespace_id: self.namespace_id,
                operation: &self.operation,
                display_key: &self.display_key,
                normalized_key: &self.normalized_key,
                expected: &self.expected,
                rationale: &self.rationale,
                plaintext_validation_version: self.plaintext_validation_version,
                created_at_ms: self.created_at_ms,
                creation_event_id: self.creation_event_id,
                approval_id: self.approval_id,
                content_digest: &self.content_digest,
            },
        )?))
    }
}

impl<'de> Deserialize<'de> for MemoryProposal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MemoryProposalWire::deserialize(deserializer)?;
        Self::from_parts(
            wire.proposal_id,
            wire.version,
            wire.proposer,
            wire.namespace_id,
            wire.operation,
            wire.display_key,
            wire.normalized_key,
            wire.expected,
            wire.rationale,
            wire.plaintext_validation_version,
            wire.created_at_ms,
            wire.creation_event_id,
            wire.approval_id,
            wire.content_digest,
            wire.record_digest,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn expected_ref(expected: &ExpectedMemoryEntryState) -> Option<&super::MemoryEntryRef> {
    match expected {
        ExpectedMemoryEntryState::Absent => None,
        ExpectedMemoryEntryState::Present(reference)
        | ExpectedMemoryEntryState::Deleted(reference) => Some(reference),
    }
}

fn expected_state_is_well_formed(expected: &ExpectedMemoryEntryState) -> bool {
    match expected {
        ExpectedMemoryEntryState::Absent => true,
        ExpectedMemoryEntryState::Present(reference) => {
            reference.state() == MemoryEntryState::Present
        }
        ExpectedMemoryEntryState::Deleted(reference) => {
            reference.state() == MemoryEntryState::Deleted
        }
    }
}

fn canonical_display_key(value: &str) -> Result<String, DomainError> {
    let value = value.replace("\r\n", "\n").replace('\r', "\n");
    if value.is_empty()
        || value != value.trim()
        || value.contains('\n')
        || value.len() > 96
        || value.chars().any(|character| character.is_control())
    {
        return Err(DomainError::InvalidMemoryProposal);
    }
    Ok(value)
}

fn canonical_rationale(value: &str) -> Result<String, DomainError> {
    let value = value.replace("\r\n", "\n").replace('\r', "\n");
    if value.is_empty()
        || value != value.trim()
        || value.len() > 512
        || value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(DomainError::InvalidMemoryProposal);
    }
    validate_plaintext(
        PLAINTEXT_VALIDATION_VERSION_V1,
        PlaintextField::ProposalRationale,
        &value,
    )?;
    Ok(value)
}

#[derive(Serialize)]
struct MemoryProposalContentDigestMaterial<'a> {
    proposer: &'a AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    operation: &'a MemoryProposalOperation,
    display_key: &'a str,
    normalized_key: &'a NormalizedMemoryKey,
    expected: &'a ExpectedMemoryEntryState,
    rationale: &'a str,
    plaintext_validation_version: u16,
}
#[derive(Serialize)]
struct MemoryProposalRecordDigestMaterial<'a> {
    proposal_id: MemoryProposalId,
    version: ObjectVersion,
    proposer: &'a AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    operation: &'a MemoryProposalOperation,
    display_key: &'a str,
    normalized_key: &'a NormalizedMemoryKey,
    expected: &'a ExpectedMemoryEntryState,
    rationale: &'a str,
    plaintext_validation_version: u16,
    created_at_ms: i64,
    creation_event_id: EventId,
    approval_id: ApprovalId,
    content_digest: &'a Digest,
}
