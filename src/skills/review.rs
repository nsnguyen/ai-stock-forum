use std::sync::{Mutex, MutexGuard};

use serde::Serialize;

use crate::{
    app::{AgentSkillAssignmentOperation, AgentSkillAssignmentPreview},
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, CommandId, DomainError, SkillId,
        SkillReviewToken, SkillVersionId, canonical_json_bytes, sha256,
    },
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
enum PendingSkillReview {
    Edit {
        token: SkillReviewToken,
        skill_id: SkillId,
        expected_active_version_id: Option<SkillVersionId>,
        candidate_digest: ContentDigest,
        review_digest: ContentDigest,
    },
    Assignment {
        token: SkillReviewToken,
        actor: Actor,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        operation: AgentSkillAssignmentOperation,
        review_digest: ContentDigest,
    },
}

#[derive(Debug)]
enum ReviewState {
    Available(PendingSkillReview),
    Reserved {
        command_id: CommandId,
        review: PendingSkillReview,
    },
}

#[derive(Debug, Default)]
pub struct SkillReviewRegistry {
    operation: Mutex<()>,
    state: Mutex<Option<ReviewState>>,
}

pub(crate) struct SkillReviewOperation<'a> {
    registry: &'a SkillReviewRegistry,
    _ownership: MutexGuard<'a, ()>,
}

impl SkillReviewRegistry {
    pub fn issue(
        &self,
        review_token: SkillReviewToken,
        skill_id: SkillId,
        expected_active_version_id: Option<SkillVersionId>,
        candidate: &SkillDraft,
    ) -> Result<SkillEditPreview, DomainError> {
        self.operation().issue_edit(
            review_token,
            skill_id,
            expected_active_version_id,
            candidate,
        )
    }

    pub fn consume(
        &self,
        review_token: SkillReviewToken,
        skill_id: SkillId,
        expected_active_version_id: Option<SkillVersionId>,
        candidate: &SkillDraft,
    ) -> Result<SkillEditPreview, DomainError> {
        let candidate_digest = candidate_digest(candidate)?;
        let _operation = self.operation.lock().unwrap_or_else(|error| error.into_inner());
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(ReviewState::Available(PendingSkillReview::Edit {
            token,
            skill_id: pending_skill_id,
            expected_active_version_id: pending_expected,
            candidate_digest: pending_candidate_digest,
            review_digest,
        })) = state.as_ref()
        else {
            return Err(DomainError::InvalidSkillReviewToken);
        };
        if *token != review_token
            || *pending_skill_id != skill_id
            || *pending_expected != expected_active_version_id
            || *pending_candidate_digest != candidate_digest
        {
            return Err(DomainError::InvalidSkillReviewToken);
        }
        let review_digest = review_digest.clone();
        state.take();
        Ok(SkillEditPreview {
            skill_id,
            expected_active_version_id,
            candidate_digest,
            review_token,
            review_digest,
        })
    }

    pub(crate) fn operation(&self) -> SkillReviewOperation<'_> {
        SkillReviewOperation {
            registry: self,
            _ownership: self.operation.lock().unwrap_or_else(|error| error.into_inner()),
        }
    }

    pub(crate) fn cancel(&self) {
        *self.state.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }
}

impl SkillReviewOperation<'_> {
    pub(crate) fn issue_edit(
        &self,
        review_token: SkillReviewToken,
        skill_id: SkillId,
        expected_active_version_id: Option<SkillVersionId>,
        candidate: &SkillDraft,
    ) -> Result<SkillEditPreview, DomainError> {
        let candidate_digest = candidate_digest(candidate)?;
        let review_digest = review_digest(skill_id, expected_active_version_id, &candidate_digest)?;
        *self.registry.state.lock().unwrap_or_else(|error| error.into_inner()) =
            Some(ReviewState::Available(PendingSkillReview::Edit {
                token: review_token,
                skill_id,
                expected_active_version_id,
                candidate_digest: candidate_digest.clone(),
                review_digest: review_digest.clone(),
            }));
        Ok(SkillEditPreview {
            skill_id,
            expected_active_version_id,
            candidate_digest,
            review_token,
            review_digest,
        })
    }

    pub(crate) fn issue_assignment(
        &self,
        review_token: SkillReviewToken,
        actor: Actor,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        operation: AgentSkillAssignmentOperation,
    ) -> Result<AgentSkillAssignmentPreview, DomainError> {
        let review_digest = assignment_review_digest(
            &actor,
            profile_id,
            expected_active_profile_version_id,
            &operation,
        )?;
        *self.registry.state.lock().unwrap_or_else(|error| error.into_inner()) =
            Some(ReviewState::Available(PendingSkillReview::Assignment {
                token: review_token,
                actor,
                profile_id,
                expected_active_profile_version_id,
                operation: operation.clone(),
                review_digest: review_digest.clone(),
            }));
        Ok(AgentSkillAssignmentPreview {
            profile_id,
            expected_active_profile_version_id,
            operation,
            review_token,
            review_digest,
        })
    }

    pub(crate) fn reserve_edit(
        &self,
        command_id: CommandId,
        review_token: SkillReviewToken,
        skill_id: SkillId,
        expected_active_version_id: Option<SkillVersionId>,
        candidate: &SkillDraft,
        supplied_review_digest: &ContentDigest,
    ) -> Result<(), crate::app::AppError> {
        let candidate_digest = candidate_digest(candidate)
            .map_err(|_| crate::app::AppError::SkillReviewMismatch)?;
        self.reserve(command_id, review_token, |review| {
            matches!(
                review,
                PendingSkillReview::Edit {
                    token,
                    skill_id: pending_skill_id,
                    expected_active_version_id: pending_expected,
                    candidate_digest: pending_candidate,
                    review_digest,
                } if *token == review_token
                    && *pending_skill_id == skill_id
                    && *pending_expected == expected_active_version_id
                    && *pending_candidate == candidate_digest
                    && review_digest == supplied_review_digest
            )
        })
    }

    pub(crate) fn reserve_assignment(
        &self,
        command_id: CommandId,
        review_token: SkillReviewToken,
        actor: &Actor,
        profile_id: AgentProfileId,
        expected_active_profile_version_id: AgentProfileVersionId,
        operation: &AgentSkillAssignmentOperation,
        supplied_review_digest: &ContentDigest,
    ) -> Result<(), crate::app::AppError> {
        self.reserve(command_id, review_token, |review| {
            matches!(
                review,
                PendingSkillReview::Assignment {
                    token,
                    actor: pending_actor,
                    profile_id: pending_profile_id,
                    expected_active_profile_version_id: pending_expected,
                    operation: pending_operation,
                    review_digest,
                } if *token == review_token
                    && pending_actor == actor
                    && *pending_profile_id == profile_id
                    && *pending_expected == expected_active_profile_version_id
                    && pending_operation == operation
                    && review_digest == supplied_review_digest
            )
        })
    }

    fn reserve(
        &self,
        command_id: CommandId,
        supplied_token: SkillReviewToken,
        matches_review: impl FnOnce(&PendingSkillReview) -> bool,
    ) -> Result<(), crate::app::AppError> {
        let mut state = self.registry.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(current) = state.take() else {
            return Err(crate::app::AppError::SkillReviewUnavailable);
        };
        match current {
            ReviewState::Available(review) if review.token() != supplied_token => {
                *state = Some(ReviewState::Available(review));
                Err(crate::app::AppError::SkillReviewUnavailable)
            }
            ReviewState::Available(review) if matches_review(&review) => {
                *state = Some(ReviewState::Reserved { command_id, review });
                Ok(())
            }
            ReviewState::Available(review) => {
                *state = Some(ReviewState::Available(review));
                Err(crate::app::AppError::SkillReviewMismatch)
            }
            reserved @ ReviewState::Reserved { .. } => {
                *state = Some(reserved);
                Err(crate::app::AppError::SkillReviewUnavailable)
            }
        }
    }

    pub(crate) fn release(&self, command_id: CommandId) {
        let mut state = self.registry.state.lock().unwrap_or_else(|error| error.into_inner());
        if matches!(state.as_ref(), Some(ReviewState::Reserved { command_id: owner, .. }) if *owner == command_id)
        {
            let Some(ReviewState::Reserved { review, .. }) = state.take() else {
                unreachable!()
            };
            *state = Some(ReviewState::Available(review));
        }
    }

    pub(crate) fn consume_reserved(&self, command_id: CommandId) {
        let mut state = self.registry.state.lock().unwrap_or_else(|error| error.into_inner());
        if matches!(state.as_ref(), Some(ReviewState::Reserved { command_id: owner, .. }) if *owner == command_id)
        {
            state.take();
        }
    }
}

impl PendingSkillReview {
    fn token(&self) -> SkillReviewToken {
        match self {
            Self::Edit { token, .. } | Self::Assignment { token, .. } => *token,
        }
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

fn assignment_review_digest(
    actor: &Actor,
    profile_id: AgentProfileId,
    expected_active_profile_version_id: AgentProfileVersionId,
    operation: &AgentSkillAssignmentOperation,
) -> Result<ContentDigest, DomainError> {
    Ok(sha256(&canonical_json_bytes(&AssignmentReviewDigestMaterial {
        actor,
        profile_id,
        expected_active_profile_version_id,
        operation,
    })?))
}

#[derive(Serialize)]
struct AssignmentReviewDigestMaterial<'a> {
    actor: &'a Actor,
    profile_id: AgentProfileId,
    expected_active_profile_version_id: AgentProfileVersionId,
    operation: &'a AgentSkillAssignmentOperation,
}
