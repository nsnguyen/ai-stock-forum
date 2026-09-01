use serde::{Deserialize, Serialize};

use crate::domain::{DomainError, Sha256Digest};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectVersion(u64);

impl ObjectVersion {
    pub fn new(value: u64) -> Result<Self, DomainError> {
        if value == 0 {
            Err(DomainError::InvalidObjectVersion)
        } else {
            Ok(Self(value))
        }
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub kind: String,
    pub id: String,
    pub version: ObjectVersion,
    pub digest: Sha256Digest,
}

impl ObjectRef {
    pub fn new(
        kind: impl Into<String>,
        id: impl Into<String>,
        version: ObjectVersion,
        digest: Sha256Digest,
    ) -> Result<Self, DomainError> {
        let kind = kind.into();
        if kind.trim().is_empty() {
            return Err(DomainError::EmptyObjectKind);
        }

        let id = id.into();
        if id.trim().is_empty() {
            return Err(DomainError::EmptyObjectId);
        }

        Ok(Self {
            kind,
            id,
            version,
            digest,
        })
    }
}
