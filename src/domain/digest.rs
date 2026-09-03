use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::domain::DomainError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let is_lowercase_sha256 = value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));

        if is_lowercase_sha256 {
            Ok(Self(value.to_owned()))
        } else {
            Err(DomainError::InvalidSha256Digest)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

pub fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, DomainError> {
    let value = serde_json::to_value(value)
        .map_err(|error| DomainError::CanonicalJson(error.to_string()))?;
    serde_json::to_vec(&canonicalize(value))
        .map_err(|error| DomainError::CanonicalJson(error.to_string()))
}

pub fn sha256(bytes: &[u8]) -> Sha256Digest {
    let hash = Sha256::digest(bytes);
    Sha256Digest(hex::encode(hash))
}

fn canonicalize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize).collect()),
        Value::Object(values) => {
            let values: BTreeMap<_, _> = values
                .into_iter()
                .map(|(key, value)| (key, canonicalize(value)))
                .collect();
            Value::Object(values.into_iter().collect())
        }
        value => value,
    }
}
