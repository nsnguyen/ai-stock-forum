use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::{DomainError, Sha256Digest};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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

impl<'de> Deserialize<'de> for ObjectVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Deserialize)]
struct ObjectRefWire {
    kind: String,
    id: String,
    version: ObjectVersion,
    digest: Sha256Digest,
}

impl<'de> Deserialize<'de> for ObjectRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let object_ref = ObjectRefWire::deserialize(deserializer)?;
        Self::new(
            object_ref.kind,
            object_ref.id,
            object_ref.version,
            object_ref.digest,
        )
        .map_err(serde::de::Error::custom)
    }
}
