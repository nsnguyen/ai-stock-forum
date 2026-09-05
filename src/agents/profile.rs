use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    agents::{normalize_profile_name_key, normalize_tag_key, validate_visible_text, ProfileField},
    domain::DomainError,
};

use super::ProfileTemplateProvenance;

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
        validate_visible_text(ProfileField::DisplayName, &display_name, DISPLAY_NAME_MAX_BYTES)?;
        normalize_profile_name_key(&display_name)?;
        validate_visible_text(ProfileField::Description, &description, DESCRIPTION_MAX_BYTES)?;
        validate_visible_text(
            ProfileField::PrimarySpecialty,
            &primary_specialty,
            PRIMARY_SPECIALTY_MAX_BYTES,
        )?;
        validate_visible_text(ProfileField::Personality, &personality, PERSONALITY_MAX_BYTES)?;
        validate_visible_text(ProfileField::Instructions, &instructions, INSTRUCTIONS_MAX_BYTES)?;
        validate_bindings(&bindings)?;

        if specialty_tags.len() > MAX_SPECIALTY_TAGS {
            return Err(DomainError::TooManyProfileTags);
        }

        let mut tag_keys = HashSet::with_capacity(specialty_tags.len());
        for tag in &specialty_tags {
            validate_visible_text(ProfileField::SpecialtyTag, tag, SPECIALTY_TAG_MAX_BYTES)?;
            if !tag_keys.insert(normalize_tag_key(tag)?) {
                return Err(DomainError::DuplicateProfileTag);
            }
        }

        if !skill_refs.is_empty() {
            return Err(DomainError::InvalidProfileField {
                field: ProfileField::SkillRefs.as_str(),
            });
        }
        if !mcp_refs.is_empty() {
            return Err(DomainError::InvalidProfileField {
                field: ProfileField::McpRefs.as_str(),
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
        validate_visible_text(ProfileField::ModelProvider, provider, 256)?;
    }
    if let Some(model_name) = &bindings.model_name {
        validate_visible_text(ProfileField::ModelName, model_name, 256)?;
    }
    Ok(())
}
