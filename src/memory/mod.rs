//! Canonical memory-entry vocabulary and plaintext validation.
pub const MODULE_NAME: &str = "memory";

mod entry;
mod normalization;
mod projection;
mod proposal;
mod review;

pub use entry::{
    MemoryEntryDraft, MemoryEntryRef, MemoryEntryState, MemoryEntryVersion, NormalizedMemoryKey,
    normalize_memory_key,
};
pub use normalization::{
    CredentialPatternSetV1, PLAINTEXT_VALIDATION_VERSION_V1, PlaintextField, validate_plaintext,
};
pub use projection::MemoryProjection;
pub use proposal::{MemoryProposal, MemoryProposalOperation, MemoryProposalRef};
pub use review::{
    ExpectedMemoryEntryState, MEMORY_PLAINTEXT_WARNING, MemoryEditReview, MemoryField,
    MemoryFieldDiff, MemoryFieldValue, MemoryMutationKind, MemoryNoChange,
    MemoryPlaintextAcknowledgement, MemoryProposalFilter, MemoryProposalOperationKind,
    MemoryProposalResolution, MemoryProposalStatus, MemoryResolutionAction, MemoryReviewRegistry,
};
#[allow(unused_imports)]
pub(crate) use review::{
    MemoryEditPreviewOutcome, MemoryEditReviewBinding, MemoryResolutionReviewBinding,
    PreparedMemoryEdit, ReservedMemoryReview, prepare_direct_memory_edit,
};
