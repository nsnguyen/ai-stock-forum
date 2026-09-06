//! Future skills boundary. Behavior begins in Phase 2.
pub const MODULE_NAME: &str = "skills";

mod normalization;
mod projection;
mod review;
mod skill;

pub use normalization::{NormalizedSkillName, SkillField};
pub use projection::SkillsProjection;
pub use review::{SkillEditPreview, SkillReviewRegistry};
pub use skill::{
    ContentDigest, SkillDraft, SkillProvenance, SkillResource, SkillVersion, SkillVersionRef,
    DESCRIPTION_MAX_BYTES, DISPLAY_NAME_MAX_BYTES, INSTRUCTIONS_MAX_BYTES,
    MAX_RESOURCE_BODY_BYTES, MAX_RESOURCES, MAX_RESOURCE_NAME_BYTES, MAX_TAGS,
    MAX_TAG_BYTES, MAX_TOTAL_RESOURCE_BODY_BYTES, PAYLOAD_MAX_BYTES, USE_WHEN_MAX_BYTES,
};
