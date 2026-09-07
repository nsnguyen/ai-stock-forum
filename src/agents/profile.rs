use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    agents::{
        AgentBindingCatalogSnapshot, EngineeringBindingRef, InferenceBindingRef,
        canonicalize_visible_text, normalize_profile_name_key, normalize_tag_key,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, Digest, DomainError, MemoryNamespaceId,
        ObjectVersion, canonical_json_bytes, sha256,
    },
    skills::SkillVersionRef,
};

use super::{ProfileTemplateProvenance, normalization::ProfileField as ValidationProfileField};

pub const DISPLAY_NAME_MAX_BYTES: usize = 64;
pub const DESCRIPTION_MAX_BYTES: usize = 256;
pub const PRIMARY_SPECIALTY_MAX_BYTES: usize = 64;
pub const SPECIALTY_TAG_MAX_BYTES: usize = 48;
pub const MAX_SPECIALTY_TAGS: usize = 5;
pub const PERSONALITY_MAX_BYTES: usize = 1_024;
pub const INSTRUCTIONS_MAX_BYTES: usize = 4_096;
pub const MAX_SKILL_REFS: usize = 16;
const DEFAULT_POLICY_REF: &str = "profile-default/v1";

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
#[serde(deny_unknown_fields)]
pub struct AgentBindings {
    #[serde(default)]
    pub inference: Option<InferenceBindingRef>,
    #[serde(default)]
    pub engineering: Option<EngineeringBindingRef>,
}

impl AgentBindings {
    pub fn new(
        inference: Option<InferenceBindingRef>,
        engineering: Option<EngineeringBindingRef>,
    ) -> Self {
        Self {
            inference,
            engineering,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRef(String);

impl McpRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AgentReadiness {
    Unbound,
    BindingUnavailable,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileDraft {
    pub display_name: String,
    pub description: String,
    pub role: AgentRole,
    pub primary_specialty: String,
    pub specialty_tags: Vec<String>,
    pub personality: String,
    pub instructions: String,
    pub bindings: AgentBindings,
    pub skill_refs: Vec<SkillVersionRef>,
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
        skill_refs: Vec<SkillVersionRef>,
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
        skill_refs: Vec<SkillVersionRef>,
        mcp_refs: Vec<McpRef>,
        template_provenance: Option<ProfileTemplateProvenance>,
    ) -> Result<Self, DomainError> {
        let display_name = canonicalize_visible_text(
            ValidationProfileField::DisplayName,
            &display_name,
            DISPLAY_NAME_MAX_BYTES,
            false,
        )?;
        normalize_profile_name_key(&display_name)?;
        let description = canonicalize_visible_text(
            ValidationProfileField::Description,
            &description,
            DESCRIPTION_MAX_BYTES,
            true,
        )?;
        let primary_specialty = canonicalize_visible_text(
            ValidationProfileField::PrimarySpecialty,
            &primary_specialty,
            PRIMARY_SPECIALTY_MAX_BYTES,
            false,
        )?;
        let personality = canonicalize_visible_text(
            ValidationProfileField::Personality,
            &personality,
            PERSONALITY_MAX_BYTES,
            false,
        )?;
        let instructions = canonicalize_visible_text(
            ValidationProfileField::Instructions,
            &instructions,
            INSTRUCTIONS_MAX_BYTES,
            false,
        )?;

        if specialty_tags.len() > MAX_SPECIALTY_TAGS {
            return Err(DomainError::TooManyProfileTags);
        }

        let primary_key = normalize_profile_name_key(&primary_specialty)?;
        let mut tag_keys = BTreeSet::new();
        let mut canonical_tags = Vec::with_capacity(specialty_tags.len());
        for tag in specialty_tags {
            let tag = canonicalize_visible_text(
                ValidationProfileField::SpecialtyTag,
                &tag,
                SPECIALTY_TAG_MAX_BYTES,
                false,
            )?;
            let key = normalize_tag_key(&tag)?;
            if key == primary_key.as_str() || !tag_keys.insert(key.clone()) {
                return Err(DomainError::DuplicateProfileTag);
            }
            canonical_tags.push((key, tag));
        }
        canonical_tags.sort_by(|left, right| left.0.cmp(&right.0));
        let specialty_tags = canonical_tags.into_iter().map(|(_, tag)| tag).collect();

        if skill_refs.len() > MAX_SKILL_REFS {
            return Err(DomainError::InvalidProfileField {
                field: ValidationProfileField::SkillRefs.as_str(),
            });
        }
        let mut skill_ids = BTreeSet::new();
        let mut canonical_skill_refs = Vec::with_capacity(skill_refs.len());
        for skill_ref in skill_refs {
            if !skill_ids.insert(skill_ref.skill_id()) {
                return Err(DomainError::InvalidProfileField {
                    field: ValidationProfileField::SkillRefs.as_str(),
                });
            }
            canonical_skill_refs.push(skill_ref);
        }
        canonical_skill_refs.sort_by_key(SkillVersionRef::skill_id);
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
            skill_refs: canonical_skill_refs,
            mcp_refs,
            template_provenance,
        })
    }

    pub fn readiness(&self) -> AgentReadiness {
        AgentBindingCatalogSnapshot::default().readiness(self.role, &self.bindings)
    }

    pub fn readiness_with_catalog(&self, catalog: &AgentBindingCatalogSnapshot) -> AgentReadiness {
        catalog.readiness(self.role, &self.bindings)
    }

    pub fn canonicalized(&self) -> Result<Self, DomainError> {
        Self::new_with_provenance(
            self.display_name.clone(),
            self.description.clone(),
            self.role,
            self.primary_specialty.clone(),
            self.specialty_tags.clone(),
            self.personality.clone(),
            self.instructions.clone(),
            self.bindings.clone(),
            self.skill_refs.clone(),
            self.mcp_refs.clone(),
            self.template_provenance.clone(),
        )
    }

    pub fn template_provenance(&self) -> Option<&ProfileTemplateProvenance> {
        self.template_provenance.as_ref()
    }

    pub fn skill_refs(&self) -> &[SkillVersionRef] {
        &self.skill_refs
    }

    pub fn assign_skill(&self, skill_ref: SkillVersionRef) -> Result<Self, DomainError> {
        if self
            .skill_refs
            .iter()
            .any(|current| current.skill_id() == skill_ref.skill_id())
        {
            return Err(DomainError::AgentProfileUnchanged);
        }
        let mut candidate = self.clone();
        candidate.skill_refs.push(skill_ref);
        candidate.canonicalized()
    }

    pub fn upgrade_skill(
        &self,
        expected: SkillVersionRef,
        replacement: SkillVersionRef,
    ) -> Result<Self, DomainError> {
        if expected == replacement || expected.skill_id() != replacement.skill_id() {
            return Err(DomainError::AgentProfileUnchanged);
        }
        let Some(index) = self
            .skill_refs
            .iter()
            .position(|current| current == &expected)
        else {
            return Err(DomainError::AgentProfileUnchanged);
        };
        let mut candidate = self.clone();
        candidate.skill_refs[index] = replacement;
        candidate.canonicalized()
    }

    pub fn unassign_skill(&self, expected: SkillVersionRef) -> Result<Self, DomainError> {
        let Some(index) = self
            .skill_refs
            .iter()
            .position(|current| current == &expected)
        else {
            return Err(DomainError::AgentProfileUnchanged);
        };
        let mut candidate = self.clone();
        candidate.skill_refs.remove(index);
        candidate.canonicalized()
    }

    pub fn to_draft(&self) -> AgentProfileDraft {
        AgentProfileDraft {
            display_name: self.display_name.clone(),
            description: self.description.clone(),
            role: self.role,
            primary_specialty: self.primary_specialty.clone(),
            specialty_tags: self.specialty_tags.clone(),
            personality: self.personality.clone(),
            instructions: self.instructions.clone(),
            bindings: self.bindings.clone(),
            skill_refs: self.skill_refs.clone(),
            mcp_refs: self.mcp_refs.clone(),
            template_provenance: self.template_provenance.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    skill_refs: Vec<SkillVersionRef>,
    mcp_refs: Vec<McpRef>,
    memory_namespace_id: MemoryNamespaceId,
    default_policy_ref: String,
    template_provenance: Option<ProfileTemplateProvenance>,
    created_at_ms: i64,
    supersedes: Option<AgentProfileVersionId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentProfileVersionWire {
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
    skill_refs: Vec<SkillVersionRef>,
    mcp_refs: Vec<McpRef>,
    memory_namespace_id: MemoryNamespaceId,
    default_policy_ref: String,
    template_provenance: Option<ProfileTemplateProvenance>,
    created_at_ms: i64,
    supersedes: Option<AgentProfileVersionId>,
}

impl<'de> Deserialize<'de> for AgentProfileVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = AgentProfileVersionWire::deserialize(deserializer)?;
        let normalized_name =
            normalize_profile_name_key(&wire.display_name).map_err(serde::de::Error::custom)?;
        if normalized_name != wire.normalized_name
            || wire.default_policy_ref != DEFAULT_POLICY_REF
            || (wire.version.get() == 1) != wire.supersedes.is_none()
        {
            return Err(serde::de::Error::custom("agent profile version is invalid"));
        }
        if let Some(provenance) = &wire.template_provenance {
            super::profile_template_from_provenance(provenance)
                .map_err(serde::de::Error::custom)?;
        }
        let draft = AgentProfileDraft::new_with_provenance(
            wire.display_name,
            wire.description,
            wire.role,
            wire.primary_specialty,
            wire.specialty_tags,
            wire.personality,
            wire.instructions,
            wire.bindings,
            wire.skill_refs,
            wire.mcp_refs,
            wire.template_provenance.clone(),
        )
        .map_err(serde::de::Error::custom)?;
        let profile = Self::from_draft(
            wire.profile_id,
            wire.profile_version_id,
            wire.version,
            wire.memory_namespace_id,
            wire.default_policy_ref,
            wire.created_at_ms,
            wire.supersedes,
            wire.template_provenance,
            draft,
        )
        .map_err(serde::de::Error::custom)?;
        if profile.content_digest != wire.content_digest {
            return Err(serde::de::Error::custom(
                "agent profile content digest is invalid",
            ));
        }
        Ok(profile)
    }
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
            DEFAULT_POLICY_REF.to_owned(),
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

    pub fn readiness(&self) -> AgentReadiness {
        AgentBindingCatalogSnapshot::default().readiness(self.role, &self.bindings)
    }

    pub fn readiness_with_catalog(&self, catalog: &AgentBindingCatalogSnapshot) -> AgentReadiness {
        catalog.readiness(self.role, &self.bindings)
    }

    pub fn skill_refs(&self) -> &[SkillVersionRef] {
        &self.skill_refs
    }

    pub fn assign_skill(
        &self,
        skill_ref: SkillVersionRef,
    ) -> Result<AgentProfileDraft, DomainError> {
        self.to_draft().assign_skill(skill_ref)
    }

    pub fn upgrade_skill(
        &self,
        expected: SkillVersionRef,
        replacement: SkillVersionRef,
    ) -> Result<AgentProfileDraft, DomainError> {
        self.to_draft().upgrade_skill(expected, replacement)
    }

    pub fn unassign_skill(
        &self,
        expected: SkillVersionRef,
    ) -> Result<AgentProfileDraft, DomainError> {
        self.to_draft().unassign_skill(expected)
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

    pub fn to_draft(&self) -> AgentProfileDraft {
        AgentProfileDraft {
            display_name: self.display_name.clone(),
            description: self.description.clone(),
            role: self.role,
            primary_specialty: self.primary_specialty.clone(),
            specialty_tags: self.specialty_tags.clone(),
            personality: self.personality.clone(),
            instructions: self.instructions.clone(),
            bindings: self.bindings.clone(),
            skill_refs: self.skill_refs.clone(),
            mcp_refs: self.mcp_refs.clone(),
            template_provenance: self.template_provenance.clone(),
        }
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
        let draft = draft.canonicalized()?;
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
    skill_refs: &'a [SkillVersionRef],
    mcp_refs: &'a [McpRef],
    memory_namespace_id: MemoryNamespaceId,
    default_policy_ref: &'a str,
    template_provenance: Option<&'a ProfileTemplateProvenance>,
    created_at_ms: i64,
    supersedes: Option<AgentProfileVersionId>,
}
