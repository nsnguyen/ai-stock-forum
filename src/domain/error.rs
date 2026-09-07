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
    #[error("agent profile candidate has no semantic changes")]
    AgentProfileUnchanged,
    #[error("agent profile version reference is invalid")]
    InvalidAgentProfileVersionReference,
    #[error("invalid skill field")]
    InvalidSkillField { field: &'static str },
    #[error("skill text contains unsafe characters")]
    UnsafeSkillText { field: &'static str },
    #[error("skill has too many tags")]
    TooManySkillTags,
    #[error("skill has too many resources")]
    TooManySkillResources,
    #[error("skill resource bodies are too large")]
    SkillResourcesTooLarge,
    #[error("skill canonical payload is too large")]
    SkillPayloadTooLarge,
    #[error("skill has duplicate normalized tags")]
    DuplicateSkillTag,
    #[error("skill has duplicate normalized resource names")]
    DuplicateSkillResourceName,
    #[error("skill candidate has no semantic changes")]
    SkillUnchanged,
    #[error("skill version is invalid")]
    InvalidSkillVersion,
    #[error("skill review token is invalid")]
    InvalidSkillReviewToken,
    #[error("invalid memory field")]
    InvalidMemoryField { field: &'static str },
    #[error("memory text contains unsafe characters")]
    UnsafeMemoryText { field: &'static str },
    #[error("plaintext validation version is unknown")]
    UnknownPlaintextValidationVersion,
    #[error("memory entry is structurally invalid")]
    InvalidMemoryEntry,
    #[error("memory review is unavailable")]
    MemoryReviewUnavailable,
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
            Self::AgentProfileUnchanged => "agent_profile_unchanged",
            Self::InvalidAgentProfileVersionReference => "invalid_agent_profile_version_reference",
            Self::InvalidSkillField { .. } => "invalid_skill_field",
            Self::UnsafeSkillText { .. } => "unsafe_skill_text",
            Self::TooManySkillTags => "too_many_skill_tags",
            Self::TooManySkillResources => "too_many_skill_resources",
            Self::SkillResourcesTooLarge => "skill_resources_too_large",
            Self::SkillPayloadTooLarge => "skill_payload_too_large",
            Self::DuplicateSkillTag => "duplicate_skill_tag",
            Self::DuplicateSkillResourceName => "duplicate_skill_resource_name",
            Self::SkillUnchanged => "skill_unchanged",
            Self::InvalidSkillVersion => "invalid_skill_version",
            Self::InvalidSkillReviewToken => "invalid_skill_review_token",
            Self::InvalidMemoryField { .. } => "invalid_memory_field",
            Self::UnsafeMemoryText { .. } => "unsafe_memory_text",
            Self::UnknownPlaintextValidationVersion => "unknown_plaintext_validation_version",
            Self::InvalidMemoryEntry => "invalid_memory_entry",
            Self::MemoryReviewUnavailable => "memory_review_unavailable",
        }
    }
}
