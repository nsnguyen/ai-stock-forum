#![allow(dead_code)] // Crate-private confirmation hooks are wired by the following service task.

use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::{
    agents::AgentProfileVersionRef,
    domain::{
        Actor, CommandId, Digest, DomainError, MemoryNamespaceId, MemoryReviewToken,
        canonical_json_bytes, sha256,
    },
};

use super::{MemoryEntryDraft, MemoryEntryRef, MemoryEntryState, MemoryEntryVersion};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum ExpectedMemoryEntryState {
    Absent,
    Present(MemoryEntryRef),
    Deleted(MemoryEntryRef),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryNoChange {
    IdenticalContent,
    AlreadyAbsent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryMutationKind {
    Set,
    Delete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryField {
    DisplayKey,
    State,
    Value,
    PurposeTags,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryFieldDiff {
    pub field: MemoryField,
    pub before: MemoryFieldValue,
    pub after: MemoryFieldValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryFieldValue {
    Missing,
    Text(String),
    Tags(Vec<String>),
    State(MemoryEntryState),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MemoryEditReview {
    pub profile: AgentProfileVersionRef,
    pub namespace_id: MemoryNamespaceId,
    pub expected: ExpectedMemoryEntryState,
    pub operation: MemoryMutationKind,
    pub candidate: Option<MemoryEntryDraft>,
    pub diff: Vec<MemoryFieldDiff>,
    pub plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
    pub review_token: MemoryReviewToken,
    pub review_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MemoryEditPreviewOutcome {
    NoChange(MemoryNoChange),
    Prepared(Box<PreparedMemoryEdit>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedMemoryEdit {
    pub(crate) profile: AgentProfileVersionRef,
    pub(crate) namespace_id: MemoryNamespaceId,
    pub(crate) expected: ExpectedMemoryEntryState,
    pub(crate) operation: MemoryMutationKind,
    pub(crate) candidate: Option<MemoryEntryDraft>,
    pub(crate) diff: Vec<MemoryFieldDiff>,
    pub(crate) plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
    pub(crate) candidate_digest: Option<Digest>,
    pub(crate) review_digest: Digest,
    pub(crate) binding: MemoryEditReviewBinding,
}

impl PreparedMemoryEdit {
    pub(crate) fn into_review(self, review_token: MemoryReviewToken) -> MemoryEditReview {
        MemoryEditReview {
            profile: self.profile,
            namespace_id: self.namespace_id,
            expected: self.expected,
            operation: self.operation,
            candidate: self.candidate,
            diff: self.diff,
            plaintext_acknowledgement: self.plaintext_acknowledgement,
            review_token,
            review_digest: self.review_digest,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_direct_memory_edit(
    actor: Actor,
    profile: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    current: Option<&MemoryEntryVersion>,
    expected: ExpectedMemoryEntryState,
    operation: MemoryMutationKind,
    candidate: Option<MemoryEntryDraft>,
    plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
) -> Result<MemoryEditPreviewOutcome, DomainError> {
    if actor != Actor::Human {
        return Err(DomainError::MemoryReviewUnavailable);
    }
    validate_expected_state(namespace_id, current, &expected)?;
    validate_candidate_shape(operation, candidate.as_ref())?;

    if matches!(operation, MemoryMutationKind::Delete)
        && current.is_none_or(|entry| entry.reference().state() == MemoryEntryState::Deleted)
    {
        return Ok(MemoryEditPreviewOutcome::NoChange(
            MemoryNoChange::AlreadyAbsent,
        ));
    }
    if let (MemoryMutationKind::Set, Some(current), Some(candidate)) =
        (operation, current, candidate.as_ref())
    {
        if candidate.normalized_key() != *current.reference().normalized_key() {
            return Err(DomainError::InvalidMemoryEntry);
        }
        if current.reference().state() == MemoryEntryState::Present
            && candidate.display_key() == current.display_key()
            && Some(candidate.value()) == current.value()
            && candidate.purpose_tags() == current.purpose_tags()
        {
            return Ok(MemoryEditPreviewOutcome::NoChange(
                MemoryNoChange::IdenticalContent,
            ));
        }
    }

    let diff = memory_field_diffs(current, operation, candidate.as_ref());
    let candidate_digest = candidate
        .as_ref()
        .map(|candidate| canonical_json_bytes(candidate).map(|bytes| sha256(&bytes)))
        .transpose()?;
    let review_digest = review_digest(
        &actor,
        &profile,
        namespace_id,
        &expected,
        operation,
        candidate_digest.as_ref(),
        &plaintext_acknowledgement,
        &diff,
    )?;
    let binding = MemoryEditReviewBinding::new(
        actor,
        profile.clone(),
        namespace_id,
        expected.clone(),
        operation,
        candidate_digest.clone(),
        plaintext_acknowledgement,
        review_digest.clone(),
    )?;

    Ok(MemoryEditPreviewOutcome::Prepared(Box::new(
        PreparedMemoryEdit {
            profile,
            namespace_id,
            expected,
            operation,
            candidate,
            diff,
            plaintext_acknowledgement,
            candidate_digest,
            review_digest,
            binding,
        },
    )))
}

fn validate_expected_state(
    namespace_id: MemoryNamespaceId,
    current: Option<&MemoryEntryVersion>,
    expected: &ExpectedMemoryEntryState,
) -> Result<(), DomainError> {
    let matches = match (current, expected) {
        (None, ExpectedMemoryEntryState::Absent) => true,
        (Some(current), ExpectedMemoryEntryState::Present(reference)) => {
            current.reference().namespace_id() == namespace_id
                && current.reference().state() == MemoryEntryState::Present
                && *reference == current.reference()
        }
        (Some(current), ExpectedMemoryEntryState::Deleted(reference)) => {
            current.reference().namespace_id() == namespace_id
                && current.reference().state() == MemoryEntryState::Deleted
                && *reference == current.reference()
        }
        _ => false,
    };
    matches
        .then_some(())
        .ok_or(DomainError::MemoryExpectedStateMismatch)
}

fn validate_candidate_shape(
    operation: MemoryMutationKind,
    candidate: Option<&MemoryEntryDraft>,
) -> Result<(), DomainError> {
    if matches!(operation, MemoryMutationKind::Set) != candidate.is_some() {
        return Err(DomainError::InvalidMemoryEntry);
    }
    Ok(())
}

fn memory_field_diffs(
    current: Option<&MemoryEntryVersion>,
    operation: MemoryMutationKind,
    candidate: Option<&MemoryEntryDraft>,
) -> Vec<MemoryFieldDiff> {
    let before = memory_field_values(current);
    let after = match operation {
        MemoryMutationKind::Set => {
            let candidate = candidate.expect("candidate shape validated before diff construction");
            [
                MemoryFieldValue::Text(candidate.display_key().to_owned()),
                MemoryFieldValue::State(MemoryEntryState::Present),
                MemoryFieldValue::Text(candidate.value().to_owned()),
                MemoryFieldValue::Tags(candidate.purpose_tags().to_vec()),
            ]
        }
        MemoryMutationKind::Delete => [
            before[0].clone(),
            MemoryFieldValue::State(MemoryEntryState::Deleted),
            MemoryFieldValue::Missing,
            MemoryFieldValue::Missing,
        ],
    };
    [
        MemoryField::DisplayKey,
        MemoryField::State,
        MemoryField::Value,
        MemoryField::PurposeTags,
    ]
    .into_iter()
    .zip(before)
    .zip(after)
    .filter_map(|((field, before), after)| {
        (before != after).then_some(MemoryFieldDiff {
            field,
            before,
            after,
        })
    })
    .collect()
}

fn memory_field_values(current: Option<&MemoryEntryVersion>) -> [MemoryFieldValue; 4] {
    let Some(current) = current else {
        return std::array::from_fn(|_| MemoryFieldValue::Missing);
    };
    [
        MemoryFieldValue::Text(current.display_key().to_owned()),
        MemoryFieldValue::State(current.reference().state()),
        current
            .value()
            .map(|value| MemoryFieldValue::Text(value.to_owned()))
            .unwrap_or(MemoryFieldValue::Missing),
        if current.reference().state() == MemoryEntryState::Present {
            MemoryFieldValue::Tags(current.purpose_tags().to_vec())
        } else {
            MemoryFieldValue::Missing
        },
    ]
}

#[allow(clippy::too_many_arguments)]
fn review_digest(
    actor: &Actor,
    profile: &AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    expected: &ExpectedMemoryEntryState,
    operation: MemoryMutationKind,
    candidate_digest: Option<&Digest>,
    plaintext_acknowledgement: &MemoryPlaintextAcknowledgement,
    diff: &[MemoryFieldDiff],
) -> Result<Digest, DomainError> {
    Ok(sha256(&canonical_json_bytes(
        &MemoryReviewDigestMaterial {
            actor,
            profile,
            namespace_id,
            expected,
            operation,
            candidate_digest,
            plaintext_acknowledgement,
            diff,
        },
    )?))
}

#[derive(Serialize)]
struct MemoryReviewDigestMaterial<'a> {
    actor: &'a Actor,
    profile: &'a AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    expected: &'a ExpectedMemoryEntryState,
    operation: MemoryMutationKind,
    candidate_digest: Option<&'a Digest>,
    plaintext_acknowledgement: &'a MemoryPlaintextAcknowledgement,
    diff: &'a [MemoryFieldDiff],
}

pub const MEMORY_PLAINTEXT_WARNING: &str = "Plaintext local memory — do not store credentials; history is retained after overwrite or delete";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MemoryPlaintextAcknowledgement {
    LocalPlaintextHistoryV1,
}

impl MemoryPlaintextAcknowledgement {
    pub fn warning(&self) -> &'static str {
        MEMORY_PLAINTEXT_WARNING
    }
}

pub struct MemoryReviewRegistry {
    operation: Mutex<()>,
    state: Mutex<Option<MemoryReviewRegistration>>,
}

struct MemoryReviewRegistration {
    token: MemoryReviewToken,
    binding: MemoryEditReviewBinding,
    state: MemoryReviewState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemoryEditReviewBinding {
    actor: Actor,
    profile: AgentProfileVersionRef,
    namespace_id: MemoryNamespaceId,
    expected: ExpectedMemoryEntryState,
    operation: MemoryMutationKind,
    candidate_digest: Option<Digest>,
    plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
    review_digest: Digest,
}

impl MemoryEditReviewBinding {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        actor: Actor,
        profile: AgentProfileVersionRef,
        namespace_id: MemoryNamespaceId,
        expected: ExpectedMemoryEntryState,
        operation: MemoryMutationKind,
        candidate_digest: Option<Digest>,
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement,
        review_digest: Digest,
    ) -> Result<Self, DomainError> {
        if actor != Actor::Human
            || matches!(operation, MemoryMutationKind::Set) != candidate_digest.is_some()
        {
            return Err(DomainError::MemoryReviewUnavailable);
        }
        Ok(Self {
            actor,
            profile,
            namespace_id,
            expected,
            operation,
            candidate_digest,
            plaintext_acknowledgement,
            review_digest,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MemoryReviewState {
    Available,
    Reserved(CommandId),
}

pub(crate) struct ReservedMemoryReview<'a> {
    registry: &'a MemoryReviewRegistry,
    token: MemoryReviewToken,
    command_id: CommandId,
    _ownership: MutexGuard<'a, ()>,
    finished: bool,
}

impl Default for MemoryReviewRegistry {
    fn default() -> Self {
        Self {
            operation: Mutex::new(()),
            state: Mutex::new(None),
        }
    }
}

impl MemoryReviewRegistry {
    pub(crate) fn replace_direct(
        &self,
        token: MemoryReviewToken,
        binding: MemoryEditReviewBinding,
    ) {
        let _operation = self
            .operation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(MemoryReviewRegistration {
                token,
                binding,
                state: MemoryReviewState::Available,
            });
    }

    pub(crate) fn reserve_direct(
        &self,
        command_id: CommandId,
        token: MemoryReviewToken,
        supplied: &MemoryEditReviewBinding,
    ) -> Result<ReservedMemoryReview<'_>, DomainError> {
        let ownership = self
            .operation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut slot = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let registration = slot.as_mut().ok_or(DomainError::MemoryReviewUnavailable)?;
        if registration.token != token
            || registration.state != MemoryReviewState::Available
            || &registration.binding != supplied
        {
            return Err(DomainError::MemoryReviewUnavailable);
        }
        registration.state = MemoryReviewState::Reserved(command_id);
        drop(slot);
        Ok(ReservedMemoryReview {
            registry: self,
            token,
            command_id,
            _ownership: ownership,
            finished: false,
        })
    }

    pub(crate) fn cancel(&self) -> Result<(), DomainError> {
        let _operation = self
            .operation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = None;
        Ok(())
    }

    pub(crate) fn finish(&self) {
        let _operation = self
            .operation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn transition_reserved(
        &self,
        command_id: CommandId,
        token: MemoryReviewToken,
        next: Option<MemoryReviewState>,
    ) -> Result<(), DomainError> {
        let mut slot = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let registration = slot.as_mut().ok_or(DomainError::MemoryReviewUnavailable)?;
        if registration.token != token
            || registration.state != MemoryReviewState::Reserved(command_id)
        {
            return Err(DomainError::MemoryReviewUnavailable);
        }
        match next {
            Some(state) => registration.state = state,
            None => *slot = None,
        }
        Ok(())
    }

    fn release_if_owned(&self, command_id: CommandId, token: MemoryReviewToken) {
        let mut slot = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(registration) = slot.as_mut()
            && registration.token == token
            && registration.state == MemoryReviewState::Reserved(command_id)
        {
            registration.state = MemoryReviewState::Available;
        }
    }
}

impl ReservedMemoryReview<'_> {
    pub(crate) fn command_id(&self) -> CommandId {
        self.command_id
    }

    pub(crate) fn token(&self) -> MemoryReviewToken {
        self.token
    }

    pub(crate) fn consume(mut self) -> Result<(), DomainError> {
        self.registry
            .transition_reserved(self.command_id, self.token, None)?;
        self.finished = true;
        Ok(())
    }

    pub(crate) fn release(mut self) -> Result<(), DomainError> {
        self.registry.transition_reserved(
            self.command_id,
            self.token,
            Some(MemoryReviewState::Available),
        )?;
        self.finished = true;
        Ok(())
    }

    pub(crate) fn invalidate(self) -> Result<(), DomainError> {
        self.consume()
    }
}

impl Drop for ReservedMemoryReview<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.registry.release_if_owned(self.command_id, self.token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agents::AgentProfileVersionRef,
        domain::{
            AgentProfileId, AgentProfileVersionId, EventId, MemoryEntryId, MemoryEntryVersionId,
            ObjectVersion, canonical_json_bytes, sha256,
        },
        memory::{MemoryEntryDraft, MemoryEntryVersion},
    };
    use std::sync::{
        Barrier,
        atomic::{AtomicBool, Ordering},
    };
    use uuid::Uuid;

    fn uuid(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn binding(operation: MemoryMutationKind) -> MemoryEditReviewBinding {
        let profile = AgentProfileVersionRef::new(
            AgentProfileId::from_uuid(uuid(1)),
            AgentProfileVersionId::from_uuid(uuid(2)),
            ObjectVersion::new(1).unwrap(),
            sha256(b"profile"),
        )
        .unwrap();
        MemoryEditReviewBinding::new(
            Actor::Human,
            profile,
            MemoryNamespaceId::from_uuid(uuid(3)),
            ExpectedMemoryEntryState::Absent,
            operation,
            (operation == MemoryMutationKind::Set).then(|| sha256(b"candidate")),
            MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            sha256(b"review"),
        )
        .unwrap()
    }

    fn token(value: u128) -> MemoryReviewToken {
        MemoryReviewToken::from_uuid(uuid(value))
    }

    fn command(value: u128) -> CommandId {
        CommandId::from_uuid(uuid(value))
    }

    fn profile(value: u128) -> AgentProfileVersionRef {
        AgentProfileVersionRef::new(
            AgentProfileId::from_uuid(uuid(value)),
            AgentProfileVersionId::from_uuid(uuid(value + 1)),
            ObjectVersion::new(1).unwrap(),
            sha256(b"profile"),
        )
        .unwrap()
    }

    fn current_entry() -> MemoryEntryVersion {
        MemoryEntryVersion::create_present(
            MemoryNamespaceId::from_uuid(uuid(50)),
            MemoryEntryId::from_uuid(uuid(51)),
            MemoryEntryVersionId::from_uuid(uuid(52)),
            MemoryEntryDraft::new(
                "Portfolio Thesis".to_owned(),
                "Own durable companies".to_owned(),
                vec!["investing".to_owned()],
            )
            .unwrap(),
            Actor::Human,
            10,
            None,
            EventId::from_uuid(uuid(53)),
        )
        .unwrap()
    }

    #[test]
    fn pure_preview_detects_noops_and_stale_expected_state_before_preparing_a_review() {
        let current = current_entry();
        let expected = ExpectedMemoryEntryState::Present(current.reference());
        let identical = MemoryEntryDraft::new(
            "Portfolio Thesis".to_owned(),
            "Own durable companies".to_owned(),
            vec!["investing".to_owned()],
        )
        .unwrap();

        assert_eq!(
            prepare_direct_memory_edit(
                Actor::Human,
                profile(60),
                MemoryNamespaceId::from_uuid(uuid(50)),
                Some(&current),
                expected.clone(),
                MemoryMutationKind::Set,
                Some(identical),
                MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            )
            .unwrap(),
            MemoryEditPreviewOutcome::NoChange(MemoryNoChange::IdenticalContent)
        );
        assert_eq!(
            prepare_direct_memory_edit(
                Actor::Human,
                profile(60),
                MemoryNamespaceId::from_uuid(uuid(50)),
                None,
                ExpectedMemoryEntryState::Absent,
                MemoryMutationKind::Delete,
                None,
                MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            )
            .unwrap(),
            MemoryEditPreviewOutcome::NoChange(MemoryNoChange::AlreadyAbsent)
        );
        let deleted = current
            .next_deleted(
                MemoryEntryVersionId::from_uuid(uuid(54)),
                Actor::Human,
                20,
                None,
                EventId::from_uuid(uuid(55)),
            )
            .unwrap();
        assert_eq!(
            prepare_direct_memory_edit(
                Actor::Human,
                profile(60),
                MemoryNamespaceId::from_uuid(uuid(50)),
                Some(&deleted),
                ExpectedMemoryEntryState::Deleted(deleted.reference()),
                MemoryMutationKind::Delete,
                None,
                MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            )
            .unwrap(),
            MemoryEditPreviewOutcome::NoChange(MemoryNoChange::AlreadyAbsent)
        );
        assert_eq!(
            prepare_direct_memory_edit(
                Actor::Human,
                profile(60),
                MemoryNamespaceId::from_uuid(uuid(50)),
                Some(&current),
                ExpectedMemoryEntryState::Absent,
                MemoryMutationKind::Delete,
                None,
                MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
            ),
            Err(DomainError::MemoryExpectedStateMismatch)
        );
    }

    #[test]
    fn pure_preview_builds_ordered_diffs_and_exact_binding_material() {
        let current = current_entry();
        let expected = ExpectedMemoryEntryState::Present(current.reference());
        let candidate = MemoryEntryDraft::new(
            "portfolio thesis".to_owned(),
            "Prefer free cash flow".to_owned(),
            vec!["research".to_owned()],
        )
        .unwrap();
        let outcome = prepare_direct_memory_edit(
            Actor::Human,
            profile(70),
            MemoryNamespaceId::from_uuid(uuid(50)),
            Some(&current),
            expected.clone(),
            MemoryMutationKind::Set,
            Some(candidate.clone()),
            MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        )
        .unwrap();
        let MemoryEditPreviewOutcome::Prepared(prepared) = outcome else {
            panic!("expected a prepared review");
        };

        assert_eq!(
            prepared
                .diff
                .iter()
                .map(|diff| diff.field)
                .collect::<Vec<_>>(),
            vec![
                MemoryField::DisplayKey,
                MemoryField::Value,
                MemoryField::PurposeTags,
            ]
        );
        assert_eq!(
            prepared.candidate_digest,
            Some(sha256(&canonical_json_bytes(&candidate).unwrap()))
        );
        assert_eq!(prepared.binding.expected, expected);
        assert_eq!(prepared.binding.operation, MemoryMutationKind::Set);
        assert_eq!(prepared.binding.candidate_digest, prepared.candidate_digest);
    }

    #[test]
    fn binding_allows_only_human_direct_mutations_and_matching_action_candidate_shape() {
        let profile = AgentProfileVersionRef::new(
            AgentProfileId::from_uuid(uuid(1)),
            AgentProfileVersionId::from_uuid(uuid(2)),
            ObjectVersion::new(1).unwrap(),
            sha256(b"profile"),
        )
        .unwrap();
        assert!(
            MemoryEditReviewBinding::new(
                Actor::System,
                profile.clone(),
                MemoryNamespaceId::from_uuid(uuid(3)),
                ExpectedMemoryEntryState::Absent,
                MemoryMutationKind::Set,
                Some(sha256(b"candidate")),
                MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
                sha256(b"review"),
            )
            .is_err()
        );
        assert!(
            MemoryEditReviewBinding::new(
                Actor::Human,
                profile,
                MemoryNamespaceId::from_uuid(uuid(3)),
                ExpectedMemoryEntryState::Absent,
                MemoryMutationKind::Delete,
                Some(sha256(b"candidate")),
                MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
                sha256(b"review"),
            )
            .is_err()
        );
    }

    #[test]
    fn reservation_is_one_use_and_drop_recovers_an_unfinished_confirmation() {
        let registry = MemoryReviewRegistry::default();
        let direct = binding(MemoryMutationKind::Set);
        registry.replace_direct(token(10), direct.clone());
        let reserved = registry
            .reserve_direct(command(11), token(10), &direct)
            .unwrap();
        assert_eq!(reserved.command_id(), command(11));
        assert_eq!(reserved.token(), token(10));
        drop(reserved);
        registry
            .reserve_direct(command(12), token(10), &direct)
            .unwrap()
            .consume()
            .unwrap();
        assert!(
            registry
                .reserve_direct(command(13), token(10), &direct)
                .is_err()
        );
    }

    #[test]
    fn replacement_and_cancellation_invalidate_prior_tokens() {
        let registry = MemoryReviewRegistry::default();
        let set = binding(MemoryMutationKind::Set);
        let delete = binding(MemoryMutationKind::Delete);
        registry.replace_direct(token(20), set.clone());
        registry.replace_direct(token(21), delete.clone());
        assert!(
            registry
                .reserve_direct(command(22), token(20), &set)
                .is_err()
        );
        registry.cancel().unwrap();
        assert!(
            registry
                .reserve_direct(command(23), token(21), &delete)
                .is_err()
        );
    }

    #[test]
    fn direct_binding_rejects_candidate_and_action_swaps() {
        let registry = MemoryReviewRegistry::default();
        let set = binding(MemoryMutationKind::Set);
        let delete = binding(MemoryMutationKind::Delete);
        registry.replace_direct(token(30), set.clone());
        assert!(
            registry
                .reserve_direct(command(31), token(30), &delete)
                .is_err()
        );
    }

    #[test]
    fn explicit_release_restores_the_token_and_invalidation_is_terminal() {
        let registry = MemoryReviewRegistry::default();
        let direct = binding(MemoryMutationKind::Set);
        registry.replace_direct(token(32), direct.clone());
        registry
            .reserve_direct(command(33), token(32), &direct)
            .unwrap()
            .release()
            .unwrap();
        registry
            .reserve_direct(command(34), token(32), &direct)
            .unwrap()
            .invalidate()
            .unwrap();
        assert!(
            registry
                .reserve_direct(command(35), token(32), &direct)
                .is_err()
        );
    }

    #[test]
    fn entry_construction_rejects_non_human_actor() {
        let draft = MemoryEntryDraft::new("Key".to_owned(), "Value".to_owned(), vec![]).unwrap();
        assert!(
            MemoryEntryVersion::create_present(
                MemoryNamespaceId::from_uuid(uuid(40)),
                MemoryEntryId::from_uuid(uuid(41)),
                MemoryEntryVersionId::from_uuid(uuid(42)),
                draft,
                Actor::System,
                0,
                None,
                EventId::from_uuid(uuid(43)),
            )
            .is_err()
        );
    }

    #[test]
    fn reservation_rejects_every_binding_substitution_and_stays_in_its_registry() {
        let registry = MemoryReviewRegistry::default();
        let direct = binding(MemoryMutationKind::Set);
        registry.replace_direct(token(70), direct.clone());

        let mut changed_actor = direct.clone();
        changed_actor.actor = Actor::System;
        let mut changed_profile = direct.clone();
        changed_profile.profile = profile(80);
        let mut changed_namespace = direct.clone();
        changed_namespace.namespace_id = MemoryNamespaceId::from_uuid(uuid(81));
        let mut changed_expected = direct.clone();
        changed_expected.expected = ExpectedMemoryEntryState::Deleted(current_entry().reference());
        let mut changed_candidate = direct.clone();
        changed_candidate.candidate_digest = Some(sha256(b"other-candidate"));
        let mut changed_review_digest = direct.clone();
        changed_review_digest.review_digest = sha256(b"other-review");

        for substituted in [
            changed_actor,
            changed_profile,
            changed_namespace,
            changed_expected,
            changed_candidate,
            changed_review_digest,
        ] {
            assert!(
                registry
                    .reserve_direct(command(71), token(70), &substituted)
                    .is_err()
            );
        }
        let other_registry = MemoryReviewRegistry::default();
        assert!(
            other_registry
                .reserve_direct(command(71), token(70), &direct)
                .is_err()
        );
    }

    #[test]
    fn replacement_and_cancellation_cannot_interleave_with_a_reserved_confirmation() {
        let registry = MemoryReviewRegistry::default();
        let direct = binding(MemoryMutationKind::Set);
        registry.replace_direct(token(90), direct.clone());
        let reserved = registry
            .reserve_direct(command(91), token(90), &direct)
            .unwrap();
        let cancel_started = Barrier::new(2);
        let cancel_finished = AtomicBool::new(false);

        std::thread::scope(|scope| {
            scope.spawn(|| {
                cancel_started.wait();
                registry.cancel().unwrap();
                cancel_finished.store(true, Ordering::SeqCst);
            });
            cancel_started.wait();
            assert!(registry.operation.try_lock().is_err());
            assert!(!cancel_finished.load(Ordering::SeqCst));
            reserved.consume().unwrap();
        });

        let replacement_started = Barrier::new(2);
        let replacement_finished = AtomicBool::new(false);
        registry.replace_direct(token(92), direct.clone());
        let reserved = registry
            .reserve_direct(command(93), token(92), &direct)
            .unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                replacement_started.wait();
                registry.replace_direct(token(94), direct.clone());
                replacement_finished.store(true, Ordering::SeqCst);
            });
            replacement_started.wait();
            assert!(registry.operation.try_lock().is_err());
            assert!(!replacement_finished.load(Ordering::SeqCst));
            reserved.consume().unwrap();
        });
        assert!(
            registry
                .reserve_direct(command(95), token(94), &direct)
                .is_ok()
        );
    }
}
