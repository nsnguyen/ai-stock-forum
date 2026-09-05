use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    agents::{normalize_profile_name_key, normalize_tag_key, validate_visible_text},
    domain::{
        AgentProfileId, AgentProfileVersionId, Digest, MemoryNamespaceId, ObjectVersion,
        DomainError, canonical_json_bytes, sha256,
    },
};

use super::{ProfileTemplateProvenance, normalization::ProfileField as ValidationProfileField};

const DISPLAY_NAME_MAX_BYTES: usize = 64;
const DESCRIPTION_MAX_BYTES: usize = 256;
const PRIMARY_SPECIALTY_MAX_BYTES: usize = 64;
const SPECIALTY_TAG_MAX_BYTES: usize = 48;
const MAX_SPECIALTY_TAGS: usize = 5;
const PERSONALITY_MAX_BYTES: usize = 1_024;
const INSTRUCTIONS_MAX_BYTES: usize = 4_096;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Bull,
    Bear,
    Chief,
    Engineering,
    Custom,
}

impl AgentRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bull => "bull",
            Self::Bear => "bear",
            Self::Chief => "chief",
            Self::Engineering => "engineering",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentBindings {
    pub model_provider: Option<String>,
    pub model_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRef(String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRef(String);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AgentReadiness {
    Ready,
    NotReady,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfileDraft {
    pub display_name: String,
    pub description: String,
    pub role: AgentRole,
    pub primary_specialty: String,
    pub specialty_tags: Vec<String>,
    pub personality: String,
    pub instructions: String,
    pub bindings: AgentBindings,
    pub skill_refs: Vec<SkillRef>,
    pub mcp_refs: Vec<McpRef>,
    template_provenance: Option<ProfileTemplateProvenance>,
}

impl AgentProfileDraft {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        display_name: String,
        description: String,
        role: AgentRole,
        primary_specialty: String,
        specialty_tags: Vec<String>,
        personality: String,
        instructions: String,
        bindings: AgentBindings,
        skill_refs: Vec<SkillRef>,
        mcp_refs: Vec<McpRef>,
    ) -> Result<Self, DomainError> {
        Self::new_with_provenance(
            display_name,
            description,
            role,
            primary_specialty,
            specialty_tags,
            personality,
            instructions,
            bindings,
            skill_refs,
            mcp_refs,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_provenance(
        display_name: String,
        description: String,
        role: AgentRole,
        primary_specialty: String,
        specialty_tags: Vec<String>,
        personality: String,
        instructions: String,
        bindings: AgentBindings,
        skill_refs: Vec<SkillRef>,
        mcp_refs: Vec<McpRef>,
        template_provenance: Option<ProfileTemplateProvenance>,
    ) -> Result<Self, DomainError> {
        validate_visible_text(
            ValidationProfileField::DisplayName,
            &display_name,
            DISPLAY_NAME_MAX_BYTES,
        )?;
        normalize_profile_name_key(&display_name)?;
        validate_visible_text(
            ValidationProfileField::Description,
            &description,
            DESCRIPTION_MAX_BYTES,
        )?;
        validate_visible_text(
            ValidationProfileField::PrimarySpecialty,
            &primary_specialty,
            PRIMARY_SPECIALTY_MAX_BYTES,
        )?;
        validate_visible_text(
            ValidationProfileField::Personality,
            &personality,
            PERSONALITY_MAX_BYTES,
        )?;
        validate_visible_text(
            ValidationProfileField::Instructions,
            &instructions,
            INSTRUCTIONS_MAX_BYTES,
        )?;
        validate_bindings(&bindings)?;

        if specialty_tags.len() > MAX_SPECIALTY_TAGS {
            return Err(DomainError::TooManyProfileTags);
        }

        let mut tag_keys = HashSet::with_capacity(specialty_tags.len());
        for tag in &specialty_tags {
            validate_visible_text(
                ValidationProfileField::SpecialtyTag,
                tag,
                SPECIALTY_TAG_MAX_BYTES,
            )?;
            if !tag_keys.insert(normalize_tag_key(tag)?) {
                return Err(DomainError::DuplicateProfileTag);
            }
        }

        if !skill_refs.is_empty() {
            return Err(DomainError::InvalidProfileField {
                field: ValidationProfileField::SkillRefs.as_str(),
            });
        }
        if !mcp_refs.is_empty() {
            return Err(DomainError::InvalidProfileField {
                field: ValidationProfileField::McpRefs.as_str(),
            });
        }

        Ok(Self {
            display_name,
            description,
            role,
            primary_specialty,
            specialty_tags,
            personality,
            instructions,
            bindings,
            skill_refs,
            mcp_refs,
            template_provenance,
        })
    }

    pub fn readiness(&self) -> AgentReadiness {
        match (&self.bindings.model_provider, &self.bindings.model_name) {
            (Some(_), Some(_)) => AgentReadiness::Ready,
            _ => AgentReadiness::NotReady,
        }
    }

    pub fn template_provenance(&self) -> Option<&ProfileTemplateProvenance> {
        self.template_provenance.as_ref()
    }
}

fn validate_bindings(bindings: &AgentBindings) -> Result<(), DomainError> {
    if let Some(provider) = &bindings.model_provider {
        validate_visible_text(ValidationProfileField::ModelProvider, provider, 256)?;
    }
    if let Some(model_name) = &bindings.model_name {
        validate_visible_text(ValidationProfileField::ModelName, model_name, 256)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileVersion {
    profile_id: AgentProfileId,
    profile_version_id: AgentProfileVersionId,
    version: ObjectVersion,
    content_digest: Digest,
    display_name: String,
    normalized_name: super::NormalizedProfileName,
    description: String,
    role: AgentRole,
    primary_specialty: String,
    specialty_tags: Vec<String>,
    personality: String,
    instructions: String,
    bindings: AgentBindings,
    skill_refs: Vec<SkillRef>,
    mcp_refs: Vec<McpRef>,
    memory_namespace_id: MemoryNamespaceId,
    default_policy_ref: String,
    template_provenance: Option<ProfileTemplateProvenance>,
    created_at_ms: i64,
    supersedes: Option<AgentProfileVersionId>,
}

impl AgentProfileVersion {
    pub fn create(
        profile_id: AgentProfileId,
        profile_version_id: AgentProfileVersionId,
        memory_namespace_id: MemoryNamespaceId,
        created_at_ms: i64,
        draft: AgentProfileDraft,
        provenance: Option<ProfileTemplateProvenance>,
    ) -> Result<Self, DomainError> {
        Self::from_draft(
            profile_id,
            profile_version_id,
            ObjectVersion::new(1)?,
            memory_namespace_id,
            "profile-default/v1".to_owned(),
            created_at_ms,
            None,
            provenance,
            draft,
        )
    }

    pub fn profile_id(&self) -> AgentProfileId {
        self.profile_id
    }

    pub fn profile_version_id(&self) -> AgentProfileVersionId {
        self.profile_version_id
    }

    pub fn version(&self) -> ObjectVersion {
        self.version
    }

    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }

    pub fn recompute_content_digest(&self) -> Result<Digest, DomainError> {
        self.compute_digest()
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn normalized_name(&self) -> &super::NormalizedProfileName {
        &self.normalized_name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn role(&self) -> AgentRole {
        self.role
    }

    pub fn primary_specialty(&self) -> &str {
        &self.primary_specialty
    }

    pub fn specialty_tags(&self) -> &[String] {
        &self.specialty_tags
    }

    pub fn personality(&self) -> &str {
        &self.personality
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    pub fn bindings(&self) -> &AgentBindings {
        &self.bindings
    }

    pub fn skill_refs(&self) -> &[SkillRef] {
        &self.skill_refs
    }

    pub fn mcp_refs(&self) -> &[McpRef] {
        &self.mcp_refs
    }

    pub fn memory_namespace_id(&self) -> MemoryNamespaceId {
        self.memory_namespace_id
    }

    pub fn default_policy_ref(&self) -> &str {
        &self.default_policy_ref
    }

    pub fn template_provenance(&self) -> Option<&ProfileTemplateProvenance> {
        self.template_provenance.as_ref()
    }

    pub fn created_at_ms(&self) -> i64 {
        self.created_at_ms
    }

    pub fn supersedes(&self) -> Option<AgentProfileVersionId> {
        self.supersedes
    }

    pub fn next_version(
        current: &AgentProfileVersion,
        new_version_id: AgentProfileVersionId,
        created_at_ms: i64,
        draft: AgentProfileDraft,
    ) -> Result<Self, DomainError> {
        let version = current
            .version
            .get()
            .checked_add(1)
            .ok_or(DomainError::InvalidObjectVersion)?;

        Self::from_draft(
            current.profile_id,
            new_version_id,
            ObjectVersion::new(version)?,
            current.memory_namespace_id,
            current.default_policy_ref.clone(),
            created_at_ms,
            Some(current.profile_version_id),
            current.template_provenance.clone(),
            draft,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_draft(
        profile_id: AgentProfileId,
        profile_version_id: AgentProfileVersionId,
        version: ObjectVersion,
        memory_namespace_id: MemoryNamespaceId,
        default_policy_ref: String,
        created_at_ms: i64,
        supersedes: Option<AgentProfileVersionId>,
        template_provenance: Option<ProfileTemplateProvenance>,
        draft: AgentProfileDraft,
    ) -> Result<Self, DomainError> {
        let normalized_name = normalize_profile_name_key(&draft.display_name)?;
        let mut profile = Self {
            profile_id,
            profile_version_id,
            version,
            content_digest: sha256(&[]),
            display_name: draft.display_name,
            normalized_name,
            description: draft.description,
            role: draft.role,
            primary_specialty: draft.primary_specialty,
            specialty_tags: draft.specialty_tags,
            personality: draft.personality,
            instructions: draft.instructions,
            bindings: draft.bindings,
            skill_refs: draft.skill_refs,
            mcp_refs: draft.mcp_refs,
            memory_namespace_id,
            default_policy_ref,
            template_provenance,
            created_at_ms,
            supersedes,
        };
        profile.content_digest = profile.compute_digest()?;
        Ok(profile)
    }

    fn compute_digest(&self) -> Result<Digest, DomainError> {
        let payload = CanonicalProfileVersionPayload {
            profile_id: self.profile_id,
            profile_version_id: self.profile_version_id,
            version: self.version,
            display_name: &self.display_name,
            normalized_name: self.normalized_name.as_str(),
            description: &self.description,
            role: self.role.as_str(),
            primary_specialty: &self.primary_specialty,
            specialty_tags: &self.specialty_tags,
            personality: &self.personality,
            instructions: &self.instructions,
            bindings: &self.bindings,
            skill_refs: &self.skill_refs,
            mcp_refs: &self.mcp_refs,
            memory_namespace_id: self.memory_namespace_id,
            default_policy_ref: &self.default_policy_ref,
            template_provenance: self.template_provenance.as_ref(),
            created_at_ms: self.created_at_ms,
            supersedes: self.supersedes,
        };

        Ok(sha256(&canonical_json_bytes(&payload)?))
    }
}

#[derive(Serialize)]
struct CanonicalProfileVersionPayload<'a> {
    profile_id: AgentProfileId,
    profile_version_id: AgentProfileVersionId,
    version: ObjectVersion,
    display_name: &'a str,
    normalized_name: &'a str,
    description: &'a str,
    role: &'static str,
    primary_specialty: &'a str,
    specialty_tags: &'a [String],
    personality: &'a str,
    instructions: &'a str,
    bindings: &'a AgentBindings,
    skill_refs: &'a [SkillRef],
    mcp_refs: &'a [McpRef],
    memory_namespace_id: MemoryNamespaceId,
    default_policy_ref: &'a str,
    template_provenance: Option<&'a ProfileTemplateProvenance>,
    created_at_ms: i64,
    supersedes: Option<AgentProfileVersionId>,
}
