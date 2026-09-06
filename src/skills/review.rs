use std::sync::Mutex;

use serde::Serialize;

use crate::domain::{
    DomainError, SkillId, SkillReviewToken, SkillVersionId, canonical_json_bytes, sha256,
};

use super::{ContentDigest, SkillDraft};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEditPreview {
    pub skill_id: SkillId,
    pub expected_active_version_id: Option<SkillVersionId>,
    pub candidate_digest: ContentDigest,
    pub review_token: SkillReviewToken,
    pub review_digest: ContentDigest,
}

#[derive(Debug)]
struct PendingSkillReview {
    token: SkillReviewToken,
    skill_id: SkillId,
    expected_active_version_id: Option<SkillVersionId>,
    candidate_digest: ContentDigest,
    review_digest: ContentDigest,
}

#[derive(Debug, Default)]
pub struct SkillReviewRegistry {
    state: Mutex<Option<PendingSkillReview>>,
}

impl SkillReviewRegistry {
    pub fn issue(
        &self,
        review_token: SkillReviewToken,
        skill_id: SkillId,
        expected_active_version_id: Option<SkillVersionId>,
        candidate: &SkillDraft,
    ) -> Result<SkillEditPreview, DomainError> {
        let candidate_digest = candidate_digest(candidate)?;
        let review_digest = review_digest(skill_id, expected_active_version_id, &candidate_digest)?;
        *self.state.lock().unwrap_or_else(|error| error.into_inner()) = Some(PendingSkillReview {
            token: review_token,
            skill_id,
            expected_active_version_id,
            candidate_digest: candidate_digest.clone(),
            review_digest: review_digest.clone(),
        });
        Ok(SkillEditPreview {
            skill_id,
            expected_active_version_id,
            candidate_digest,
            review_token,
            review_digest,
        })
    }

    pub fn consume(
        &self,
        review_token: SkillReviewToken,
        skill_id: SkillId,
        expected_active_version_id: Option<SkillVersionId>,
        candidate: &SkillDraft,
    ) -> Result<SkillEditPreview, DomainError> {
        let candidate_digest = candidate_digest(candidate)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(pending) = state.as_ref() else {
            return Err(DomainError::InvalidSkillReviewToken);
        };
        if pending.token != review_token
            || pending.skill_id != skill_id
            || pending.expected_active_version_id != expected_active_version_id
            || pending.candidate_digest != candidate_digest
        {
            return Err(DomainError::InvalidSkillReviewToken);
        }
        let pending = state.take().expect("checked skill review state");
        Ok(SkillEditPreview {
            skill_id: pending.skill_id,
            expected_active_version_id: pending.expected_active_version_id,
            candidate_digest: pending.candidate_digest,
            review_token: pending.token,
            review_digest: pending.review_digest,
        })
    }
}

fn candidate_digest(candidate: &SkillDraft) -> Result<ContentDigest, DomainError> {
    Ok(sha256(&canonical_json_bytes(&candidate.canonicalized()?)?))
}

fn review_digest(
    skill_id: SkillId,
    expected_active_version_id: Option<SkillVersionId>,
    candidate_digest: &ContentDigest,
) -> Result<ContentDigest, DomainError> {
    Ok(sha256(&canonical_json_bytes(&ReviewDigestMaterial {
        skill_id,
        expected_active_version_id,
        candidate_digest,
    })?))
}

#[derive(Serialize)]
struct ReviewDigestMaterial<'a> {
    skill_id: SkillId,
    expected_active_version_id: Option<SkillVersionId>,
    candidate_digest: &'a ContentDigest,
}
