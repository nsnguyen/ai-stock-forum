//! Future skills boundary. Behavior begins in Phase 2.
pub const MODULE_NAME: &str = "skills";

mod builtin;
mod normalization;
mod projection;
mod retrieval;
mod review;
mod skill;

pub use builtin::{BuiltinSkillManifest, builtin_manifests, reconcile_builtin_manifests};
pub use normalization::{NormalizedSkillName, SkillField};
pub use projection::SkillsProjection;
pub use retrieval::{RetrievalBudget, SkillRetrieval, retrieve_assigned_skills};
pub use review::{SkillEditPreview, SkillReviewRegistry};
pub use skill::{
    ContentDigest, DESCRIPTION_MAX_BYTES, DISPLAY_NAME_MAX_BYTES, INSTRUCTIONS_MAX_BYTES,
    MAX_RESOURCE_BODY_BYTES, MAX_RESOURCE_NAME_BYTES, MAX_RESOURCES, MAX_TAG_BYTES, MAX_TAGS,
    MAX_TOTAL_RESOURCE_BODY_BYTES, PAYLOAD_MAX_BYTES, SkillDraft, SkillProvenance, SkillResource,
    SkillVersion, SkillVersionRef, USE_WHEN_MAX_BYTES,
};
