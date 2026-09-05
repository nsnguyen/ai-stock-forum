use std::{fmt, sync::OnceLock};

use serde::{Deserialize, Serialize};

use crate::{
    agents::{AgentBindings, AgentProfileDraft, AgentRole, McpRef, SkillRef},
    domain::{DomainError, Sha256Digest, canonical_json_bytes, sha256},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileTemplateId(String);

impl ProfileTemplateId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileTemplateId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileTemplateVersion(u32);

impl ProfileTemplateVersion {
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileTemplateProvenance {
    pub template_id: ProfileTemplateId,
    pub template_version: ProfileTemplateVersion,
    pub template_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileTemplate {
    pub id: ProfileTemplateId,
    pub version: ProfileTemplateVersion,
    pub digest: Sha256Digest,
    pub role: AgentRole,
    pub suggested_name: &'static str,
    pub description: &'static str,
    pub primary_specialty: &'static str,
    pub specialty_tags: &'static [&'static str],
    pub personality: &'static str,
    pub instructions: &'static str,
}

impl ProfileTemplate {
    pub fn provenance(&self) -> ProfileTemplateProvenance {
        ProfileTemplateProvenance {
            template_id: self.id.clone(),
            template_version: self.version,
            template_digest: self.digest.clone(),
        }
    }

    pub fn copy_to_draft(&self) -> Result<AgentProfileDraft, DomainError> {
        AgentProfileDraft::new_with_provenance(
            self.suggested_name.to_owned(),
            self.description.to_owned(),
            self.role,
            self.primary_specialty.to_owned(),
            self.specialty_tags
                .iter()
                .map(|tag| (*tag).to_owned())
                .collect(),
            self.personality.to_owned(),
            self.instructions.to_owned(),
            AgentBindings::default(),
            Vec::<SkillRef>::new(),
            Vec::<McpRef>::new(),
            Some(self.provenance()),
        )
    }
}

pub fn builtin_profile_templates() -> &'static [ProfileTemplate] {
    static TEMPLATES: OnceLock<Vec<ProfileTemplate>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        vec![
            template(
                "builtin.bull",
                AgentRole::Bull,
                "Bull Researcher",
                "Develops evidence-led upside research.",
                "upside research",
                &["growth", "catalysts"],
                "Constructive, precise, and evidence-led.",
                "Develop the strongest evidence-backed bull case.",
            ),
            template(
                "builtin.bear",
                AgentRole::Bear,
                "Bear Researcher",
                "Develops evidence-led downside research.",
                "downside research",
                &["risk", "valuation"],
                "Skeptical, precise, and evidence-led.",
                "Develop the strongest evidence-backed bear case.",
            ),
            template(
                "builtin.chief",
                AgentRole::Chief,
                "Chief Moderator",
                "Synthesizes evidence into clear decisions.",
                "evidence synthesis",
                &["arbitration", "decisions"],
                "Balanced, rigorous, and decisive.",
                "Arbitrate competing claims using cited evidence.",
            ),
            template(
                "builtin.engineering",
                AgentRole::Engineering,
                "Research Engineer",
                "Builds reliable systems for investment research.",
                "research systems",
                &["tooling", "data-quality"],
                "Methodical, practical, and quality-focused.",
                "Improve research tooling and data quality.",
            ),
            template(
                "builtin.custom",
                AgentRole::Custom,
                "Custom Analyst",
                "Provides a starting point for a custom analyst.",
                "general research",
                &["custom"],
                "Adaptable, clear, and evidence-led.",
                "Perform general research according to the configured remit.",
            ),
        ]
    })
}

pub fn profile_template_from_provenance(
    provenance: &ProfileTemplateProvenance,
) -> Result<&'static ProfileTemplate, DomainError> {
    let template = builtin_profile_templates()
        .iter()
        .find(|template| {
            template.id == provenance.template_id && template.version == provenance.template_version
        })
        .ok_or(DomainError::UnknownProfileTemplate)?;

    if template.digest != provenance.template_digest {
        return Err(DomainError::InvalidProfileTemplateProvenance);
    }

    Ok(template)
}

#[derive(Serialize)]
struct CanonicalTemplatePayload<'a> {
    id: &'a ProfileTemplateId,
    version: ProfileTemplateVersion,
    role: AgentRole,
    suggested_name: &'a str,
    description: &'a str,
    primary_specialty: &'a str,
    specialty_tags: &'a [&'a str],
    personality: &'a str,
    instructions: &'a str,
}

#[allow(clippy::too_many_arguments)]
fn template(
    id: &'static str,
    role: AgentRole,
    suggested_name: &'static str,
    description: &'static str,
    primary_specialty: &'static str,
    specialty_tags: &'static [&'static str],
    personality: &'static str,
    instructions: &'static str,
) -> ProfileTemplate {
    let id = ProfileTemplateId::new(id);
    let version = ProfileTemplateVersion(1);
    let payload = CanonicalTemplatePayload {
        id: &id,
        version,
        role,
        suggested_name,
        description,
        primary_specialty,
        specialty_tags,
        personality,
        instructions,
    };
    let digest = sha256(
        &canonical_json_bytes(&payload)
            .expect("built-in profile template payload must be serializable"),
    );

    ProfileTemplate {
        id,
        version,
        digest,
        role,
        suggested_name,
        description,
        primary_specialty,
        specialty_tags,
        personality,
        instructions,
    }
}
