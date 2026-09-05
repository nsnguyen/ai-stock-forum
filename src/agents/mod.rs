//! Agent profile vocabulary and template foundations.
pub const MODULE_NAME: &str = "agents";

mod normalization;
mod profile;
mod template;

pub use normalization::{
    NormalizedProfileName, ProfileField, normalize_profile_name_key, normalize_tag_key,
    validate_visible_text,
};
pub use profile::{
    AgentBindings, AgentProfileDraft, AgentReadiness, AgentRole, McpRef, SkillRef,
};
pub use template::{
    ProfileTemplate, ProfileTemplateId, ProfileTemplateProvenance, ProfileTemplateVersion,
    builtin_profile_templates, profile_template_from_provenance,
};
