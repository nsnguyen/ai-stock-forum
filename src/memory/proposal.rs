use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::{Digest, DomainError, MemoryProposalId, ObjectVersion};

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
