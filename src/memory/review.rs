#![allow(dead_code)] // Crate-private confirmation hooks are wired by the following service task.

use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::{
    agents::AgentProfileVersionRef,
    domain::{Actor, CommandId, Digest, DomainError, MemoryNamespaceId, MemoryReviewToken},
};

use super::{MemoryEntryDraft, MemoryEntryRef, MemoryEntryState};

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
            ObjectVersion, sha256,
        },
        memory::{MemoryEntryDraft, MemoryEntryVersion},
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
}
