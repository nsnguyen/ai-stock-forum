//! Canonical memory-entry vocabulary and plaintext validation.
pub const MODULE_NAME: &str = "memory";

mod entry;
mod normalization;
mod proposal;
mod review;

pub use entry::{
    MemoryEntryDraft, MemoryEntryRef, MemoryEntryState, MemoryEntryVersion, NormalizedMemoryKey,
    normalize_memory_key,
};
pub use normalization::{
    CredentialPatternSetV1, PLAINTEXT_VALIDATION_VERSION_V1, PlaintextField, validate_plaintext,
};
pub use proposal::MemoryProposalRef;
pub use review::{
    ExpectedMemoryEntryState, MEMORY_PLAINTEXT_WARNING, MemoryEditReview, MemoryField,
    MemoryFieldDiff, MemoryFieldValue, MemoryMutationKind, MemoryNoChange,
    MemoryPlaintextAcknowledgement, MemoryReviewRegistry,
};
#[allow(unused_imports)]
pub(crate) use review::{MemoryEditReviewBinding, ReservedMemoryReview};
