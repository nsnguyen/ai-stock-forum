use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::{
    Digest, DomainError, ObjectVersion, SkillId, SkillVersionId, canonical_json_bytes, sha256,
};

use super::normalization::{
    NormalizedSkillName, SkillField, canonicalize_multiline_text, canonicalize_visible_text,
    normalize_resource_name_key, normalize_skill_name, normalize_tag_key,
};

pub type ContentDigest = Digest;

pub const DISPLAY_NAME_MAX_BYTES: usize = 64;
pub const DESCRIPTION_MAX_BYTES: usize = 256;
pub const USE_WHEN_MAX_BYTES: usize = 512;
pub const MAX_TAGS: usize = 8;
pub const MAX_TAG_BYTES: usize = 32;
pub const INSTRUCTIONS_MAX_BYTES: usize = 4_096;
pub const MAX_RESOURCES: usize = 8;
pub const MAX_RESOURCE_NAME_BYTES: usize = 64;
pub const MAX_RESOURCE_BODY_BYTES: usize = 4_096;
pub const MAX_TOTAL_RESOURCE_BODY_BYTES: usize = 16_384;
pub const PAYLOAD_MAX_BYTES: usize = 32_768;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillResource {
    pub name: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillDraft {
    pub display_name: String,
    pub description: String,
    pub use_when: String,
    pub tags: Vec<String>,
    pub instructions: String,
    pub resources: Vec<SkillResource>,
}

impl SkillDraft {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        display_name: String,
        description: String,
        use_when: String,
        tags: Vec<String>,
        instructions: String,
        resources: Vec<SkillResource>,
    ) -> Result<Self, DomainError> {
        let display_name = canonicalize_visible_text(
            SkillField::DisplayName,
            &display_name,
            DISPLAY_NAME_MAX_BYTES,
            false,
        )?;
        normalize_skill_name(&display_name)?;
        let description = canonicalize_multiline_text(
            SkillField::Description,
            &description,
            DESCRIPTION_MAX_BYTES,
            true,
        )?;
        let use_when = canonicalize_multiline_text(
            SkillField::UseWhen,
            &use_when,
            USE_WHEN_MAX_BYTES,
            false,
        )?;
        let instructions = canonicalize_multiline_text(
            SkillField::Instructions,
            &instructions,
            INSTRUCTIONS_MAX_BYTES,
            false,
        )?;
        if tags.len() > MAX_TAGS {
            return Err(DomainError::TooManySkillTags);
        }
        let mut tag_keys = BTreeSet::new();
        let mut canonical_tags = Vec::with_capacity(tags.len());
        for tag in tags {
            let tag = canonicalize_visible_text(SkillField::Tag, &tag, MAX_TAG_BYTES, false)?;
            let key = normalize_tag_key(&tag)?;
            if !tag_keys.insert(key.clone()) {
                return Err(DomainError::DuplicateSkillTag);
            }
            canonical_tags.push((key, tag));
        }
        canonical_tags.sort_by(|left, right| left.0.cmp(&right.0));

        if resources.len() > MAX_RESOURCES {
            return Err(DomainError::TooManySkillResources);
        }
        let mut resource_keys = BTreeSet::new();
        let mut total_resource_bytes = 0_usize;
        let mut canonical_resources = Vec::with_capacity(resources.len());
        for resource in resources {
            let name = canonicalize_visible_text(
                SkillField::ResourceName,
                &resource.name,
                MAX_RESOURCE_NAME_BYTES,
                false,
            )?;
            let body = canonicalize_multiline_text(
                SkillField::ResourceBody,
                &resource.body,
                MAX_RESOURCE_BODY_BYTES,
                true,
            )?;
            total_resource_bytes = total_resource_bytes
                .checked_add(body.len())
                .ok_or(DomainError::SkillResourcesTooLarge)?;
            if total_resource_bytes > MAX_TOTAL_RESOURCE_BODY_BYTES {
                return Err(DomainError::SkillResourcesTooLarge);
            }
            let key = normalize_resource_name_key(&name)?;
            if !resource_keys.insert(key.clone()) {
                return Err(DomainError::DuplicateSkillResourceName);
            }
            canonical_resources.push((key, SkillResource { name, body }));
        }
        canonical_resources.sort_by(|left, right| left.0.cmp(&right.0));

        let draft = Self {
            display_name,
            description,
            use_when,
            tags: canonical_tags.into_iter().map(|(_, tag)| tag).collect(),
            instructions,
            resources: canonical_resources
                .into_iter()
                .map(|(_, resource)| resource)
                .collect(),
        };
        if canonical_json_bytes(&draft)?.len() > PAYLOAD_MAX_BYTES {
            return Err(DomainError::SkillPayloadTooLarge);
        }
        Ok(draft)
    }

    pub fn canonicalized(&self) -> Result<Self, DomainError> {
        Self::new(
            self.display_name.clone(),
            self.description.clone(),
            self.use_when.clone(),
            self.tags.clone(),
            self.instructions.clone(),
            self.resources.clone(),
        )
    }

    pub fn normalized_name(&self) -> Result<NormalizedSkillName, DomainError> {
        normalize_skill_name(&self.display_name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillProvenance {
    BuiltIn {
        manifest_id: String,
        manifest_version: u32,
        manifest_digest: ContentDigest,
    },
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillVersionRef {
    pub skill_id: SkillId,
    pub skill_version_id: SkillVersionId,
    pub version: ObjectVersion,
    pub content_digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillVersion {
    pub skill_id: SkillId,
    pub skill_version_id: SkillVersionId,
    pub version: ObjectVersion,
    pub content: SkillDraft,
    pub content_digest: ContentDigest,
    pub created_at_ms: i64,
    pub provenance: SkillProvenance,
    pub predecessor: Option<SkillVersionId>,
}

impl SkillVersion {
    pub fn create(
        skill_id: SkillId,
        skill_version_id: SkillVersionId,
        created_at_ms: i64,
        provenance: SkillProvenance,
        draft: SkillDraft,
    ) -> Result<Self, DomainError> {
        Self::from_draft(
            skill_id,
            skill_version_id,
            ObjectVersion::new(1)?,
            created_at_ms,
            provenance,
            None,
            draft,
        )
    }

    pub fn next_version(
        current: &Self,
        skill_version_id: SkillVersionId,
        created_at_ms: i64,
        draft: SkillDraft,
    ) -> Result<Self, DomainError> {
        let draft = draft.canonicalized()?;
        if draft == current.content {
            return Err(DomainError::SkillUnchanged);
        }
        let version = current
            .version
            .get()
            .checked_add(1)
            .ok_or(DomainError::InvalidObjectVersion)?;
        Self::from_draft(
            current.skill_id,
            skill_version_id,
            ObjectVersion::new(version)?,
            created_at_ms,
            current.provenance.clone(),
            Some(current.skill_version_id),
            draft,
        )
    }

    pub fn reference(&self) -> SkillVersionRef {
        SkillVersionRef {
            skill_id: self.skill_id,
            skill_version_id: self.skill_version_id,
            version: self.version,
            content_digest: self.content_digest.clone(),
        }
    }

    pub fn normalized_name(&self) -> Result<NormalizedSkillName, DomainError> {
        self.content.normalized_name()
    }

    pub fn recompute_content_digest(&self) -> Result<ContentDigest, DomainError> {
        self.compute_digest()
    }

    fn from_draft(
        skill_id: SkillId,
        skill_version_id: SkillVersionId,
        version: ObjectVersion,
        created_at_ms: i64,
        provenance: SkillProvenance,
        predecessor: Option<SkillVersionId>,
        draft: SkillDraft,
    ) -> Result<Self, DomainError> {
        let content = draft.canonicalized()?;
        let mut version = Self {
            skill_id,
            skill_version_id,
            version,
            content,
            content_digest: sha256(&[]),
            created_at_ms,
            provenance,
            predecessor,
        };
        version.content_digest = version.compute_digest()?;
        Ok(version)
    }

    fn compute_digest(&self) -> Result<ContentDigest, DomainError> {
        Ok(sha256(&canonical_json_bytes(&CanonicalSkillVersionPayload {
            skill_id: self.skill_id,
            skill_version_id: self.skill_version_id,
            version: self.version,
            content: &self.content,
            created_at_ms: self.created_at_ms,
            provenance: &self.provenance,
            predecessor: self.predecessor,
        })?))
    }
}

#[derive(Serialize)]
struct CanonicalSkillVersionPayload<'a> {
    skill_id: SkillId,
    skill_version_id: SkillVersionId,
    version: ObjectVersion,
    content: &'a SkillDraft,
    created_at_ms: i64,
    provenance: &'a SkillProvenance,
    predecessor: Option<SkillVersionId>,
}
