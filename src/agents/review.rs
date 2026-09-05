use std::sync::{Mutex, MutexGuard};

use serde::Serialize;

use crate::{
    agents::{AgentProfileDraft, ProfileDiffField, ProfileFieldDiff},
    domain::{
        AgentProfileId, AgentProfileVersionId, CommandId, Digest, DomainError, ProfileReviewToken,
        canonical_json_bytes, sha256,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileEditPreview {
    pub profile_id: AgentProfileId,
    pub expected_active_version_id: AgentProfileVersionId,
    pub diffs: Vec<ProfileFieldDiff>,
    pub review_token: ProfileReviewToken,
    pub review_digest: Digest,
}

#[derive(Debug)]
enum ReviewState {
    Available(PendingProfileReview),
    Reserved {
        command_id: CommandId,
        review: PendingProfileReview,
    },
}

#[derive(Debug)]
struct PendingProfileReview {
    token: ProfileReviewToken,
    profile_id: AgentProfileId,
    expected_active_version_id: AgentProfileVersionId,
    candidate_digest: Digest,
    review_digest: Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewReservationError {
    Unavailable,
    Mismatch,
}

#[derive(Debug, Default)]
pub(crate) struct ProfileReviewRegistry {
    operation: Mutex<()>,
    state: Mutex<Option<ReviewState>>,
}

pub(crate) struct ProfileReviewOperation<'a> {
    registry: &'a ProfileReviewRegistry,
    _ownership: MutexGuard<'a, ()>,
}

impl ProfileReviewRegistry {
    pub(crate) fn operation(&self) -> ProfileReviewOperation<'_> {
        ProfileReviewOperation {
            registry: self,
            _ownership: self
                .operation
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        }
    }

    fn replace(
        &self,
        token: ProfileReviewToken,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate_digest: Digest,
        review_digest: Digest,
    ) {
        *self.state.lock().unwrap_or_else(|error| error.into_inner()) =
            Some(ReviewState::Available(PendingProfileReview {
                token,
                profile_id,
                expected_active_version_id,
                candidate_digest,
                review_digest,
            }));
    }

    #[allow(clippy::too_many_arguments)]
    fn reserve(
        &self,
        command_id: CommandId,
        token: ProfileReviewToken,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate_digest: &Digest,
        review_digest: &Digest,
    ) -> Result<(), ReviewReservationError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(current) = state.take() else {
            return Err(ReviewReservationError::Unavailable);
        };
        match current {
            ReviewState::Available(review) if review.token != token => {
                *state = Some(ReviewState::Available(review));
                Err(ReviewReservationError::Unavailable)
            }
            ReviewState::Available(review)
                if review.profile_id != profile_id
                    || review.expected_active_version_id != expected_active_version_id
                    || &review.candidate_digest != candidate_digest
                    || &review.review_digest != review_digest =>
            {
                *state = Some(ReviewState::Available(review));
                Err(ReviewReservationError::Mismatch)
            }
            ReviewState::Available(review) => {
                *state = Some(ReviewState::Reserved { command_id, review });
                Ok(())
            }
            reserved @ ReviewState::Reserved { .. } => {
                *state = Some(reserved);
                Err(ReviewReservationError::Unavailable)
            }
        }
    }

    fn release(&self, command_id: CommandId) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let current = state.take();
        *state = match current {
            Some(ReviewState::Reserved {
                command_id: owner,
                review,
            }) if owner == command_id => Some(ReviewState::Available(review)),
            other => other,
        };
    }

    fn consume(&self, command_id: CommandId) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let current = state.take();
        *state = match current {
            Some(ReviewState::Reserved {
                command_id: owner, ..
            }) if owner == command_id => None,
            other => other,
        };
    }

    pub(crate) fn cancel(&self) {
        self.operation().cancel();
    }
}

impl ProfileReviewOperation<'_> {
    pub(crate) fn replace(
        &self,
        token: ProfileReviewToken,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate_digest: Digest,
        review_digest: Digest,
    ) {
        self.registry.replace(
            token,
            profile_id,
            expected_active_version_id,
            candidate_digest,
            review_digest,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reserve(
        &self,
        command_id: CommandId,
        token: ProfileReviewToken,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate_digest: &Digest,
        review_digest: &Digest,
    ) -> Result<(), ReviewReservationError> {
        self.registry.reserve(
            command_id,
            token,
            profile_id,
            expected_active_version_id,
            candidate_digest,
            review_digest,
        )
    }

    pub(crate) fn release(&self, command_id: CommandId) {
        self.registry.release(command_id);
    }

    pub(crate) fn consume(&self, command_id: CommandId) {
        self.registry.consume(command_id);
    }

    fn cancel(&self) {
        *self
            .registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
    }
}

#[derive(Serialize)]
struct CandidateDigestMaterial<'a> {
    display_name: &'a str,
    description: &'a str,
    role: crate::agents::AgentRole,
    primary_specialty: &'a str,
    specialty_tags: &'a [String],
    personality: &'a str,
    instructions: &'a str,
    bindings: &'a crate::agents::AgentBindings,
    skill_refs: &'a [crate::agents::SkillRef],
    mcp_refs: &'a [crate::agents::McpRef],
}

pub(crate) fn candidate_digest(candidate: &AgentProfileDraft) -> Result<Digest, DomainError> {
    let material = CandidateDigestMaterial {
        display_name: &candidate.display_name,
        description: &candidate.description,
        role: candidate.role,
        primary_specialty: &candidate.primary_specialty,
        specialty_tags: &candidate.specialty_tags,
        personality: &candidate.personality,
        instructions: &candidate.instructions,
        bindings: &candidate.bindings,
        skill_refs: &candidate.skill_refs,
        mcp_refs: &candidate.mcp_refs,
    };
    Ok(sha256(&canonical_json_bytes(&material)?))
}

#[derive(Serialize)]
struct ReviewDigestMaterial<'a> {
    profile_id: AgentProfileId,
    expected_active_version_id: AgentProfileVersionId,
    candidate_digest: &'a Digest,
    diff_fields: Vec<ProfileDiffField>,
}

pub(crate) fn review_digest(
    profile_id: AgentProfileId,
    expected_active_version_id: AgentProfileVersionId,
    candidate_digest: &Digest,
    diffs: &[ProfileFieldDiff],
) -> Result<Digest, DomainError> {
    let material = ReviewDigestMaterial {
        profile_id,
        expected_active_version_id,
        candidate_digest,
        diff_fields: diffs.iter().map(|diff| diff.field).collect(),
    };
    Ok(sha256(&canonical_json_bytes(&material)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn only_the_owning_command_can_release_a_reserved_review() {
        let registry = ProfileReviewRegistry::default();
        let operation = registry.operation();
        let owner = CommandId::from_uuid(Uuid::from_u128(1));
        let other = CommandId::from_uuid(Uuid::from_u128(2));
        let token = ProfileReviewToken::from_uuid(Uuid::from_u128(3));
        let profile_id = AgentProfileId::from_uuid(Uuid::from_u128(4));
        let base = AgentProfileVersionId::from_uuid(Uuid::from_u128(5));
        let candidate = sha256(b"candidate");
        let review = sha256(b"review");
        operation.replace(
            token,
            profile_id,
            base,
            candidate.clone(),
            review.clone(),
        );
        operation
            .reserve(owner, token, profile_id, base, &candidate, &review)
            .unwrap();

        operation.release(other);

        assert!(matches!(
            *registry.state.lock().unwrap(),
            Some(ReviewState::Reserved { command_id, .. }) if command_id == owner
        ));
        operation.release(owner);
        assert!(matches!(
            *registry.state.lock().unwrap(),
            Some(ReviewState::Available(_))
        ));
        operation
            .reserve(owner, token, profile_id, base, &candidate, &review)
            .unwrap();
    }
}
