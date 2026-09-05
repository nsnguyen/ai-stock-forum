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
    #[error("invalid agent profile field")]
    InvalidProfileField { field: &'static str },
    #[error("agent profile text contains unsafe characters")]
    UnsafeProfileText { field: &'static str },
    #[error("agent profile has too many specialty tags")]
    TooManyProfileTags,
    #[error("agent profile has duplicate specialty tags")]
    DuplicateProfileTag,
    #[error("agent profile template is unknown")]
    UnknownProfileTemplate,
    #[error("agent profile template provenance is invalid")]
    InvalidProfileTemplateProvenance,
}

impl DomainError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidObjectVersion => "invalid_object_version",
            Self::EmptyObjectKind => "empty_object_kind",
            Self::EmptyObjectId => "empty_object_id",
            Self::InvalidSha256Digest => "invalid_sha256_digest",
            Self::CanonicalJson(_) => "canonical_json_error",
            Self::InvalidProfileField { .. } => "invalid_profile_field",
            Self::UnsafeProfileText { .. } => "unsafe_profile_text",
            Self::TooManyProfileTags => "too_many_profile_tags",
            Self::DuplicateProfileTag => "duplicate_profile_tag",
            Self::UnknownProfileTemplate => "unknown_profile_template",
            Self::InvalidProfileTemplateProvenance => "invalid_profile_template_provenance",
        }
    }
}
