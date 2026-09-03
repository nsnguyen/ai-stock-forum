use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    #[error("object version must be greater than zero")]
    InvalidObjectVersion,
    #[error("object kind must not be empty")]
    EmptyObjectKind,
    #[error("object id must not be empty")]
    EmptyObjectId,
    #[error("SHA-256 digest must be 64 lowercase hexadecimal characters")]
    InvalidSha256Digest,
    #[error("failed to serialize canonical JSON: {0}")]
    CanonicalJson(String),
}
