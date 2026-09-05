//! Agent profile vocabulary and template foundations.
pub const MODULE_NAME: &str = "agents";

mod diff;
mod normalization;
mod profile;
mod projection;
mod review;
mod template;

pub use diff::{ProfileDiffField, ProfileFieldDiff, ProfileFieldValue, diff_profile};
pub use normalization::{
    NormalizedProfileName, ProfileField, normalize_profile_name_key, normalize_tag_key,
    validate_visible_text,
};
pub use profile::{
    AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentReadiness, AgentRole, McpRef,
    SkillRef,
};
pub use projection::AgentProfilesProjection;
pub use review::ProfileEditPreview;
pub(crate) use review::{
    ProfileReviewRegistry, ReviewReservationError, candidate_digest, review_digest,
};
pub use template::{
    ProfileTemplate, ProfileTemplateId, ProfileTemplateProvenance, ProfileTemplateVersion,
    builtin_profile_templates, profile_template_from_provenance,
};
