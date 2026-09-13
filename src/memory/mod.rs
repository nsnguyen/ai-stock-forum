//! Canonical memory-entry vocabulary and plaintext validation.
pub const MODULE_NAME: &str = "memory";

mod entry;
mod episodic;
mod normalization;
mod projection;
mod proposal;
mod retrieval;
mod review;

pub use entry::{
    MemoryEntryDraft, MemoryEntryRef, MemoryEntryState, MemoryEntryVersion, NormalizedMemoryKey,
    normalize_memory_key,
};
pub use episodic::{EpisodicQualification, EpisodicSourceRef, EpisodicSummary, EpisodicSummaryRef};
pub use normalization::{
    CredentialPatternSetV1, PLAINTEXT_VALIDATION_VERSION_V1, PlaintextField, validate_plaintext,
};
pub use projection::MemoryProjection;
pub use proposal::{MemoryProposal, MemoryProposalOperation, MemoryProposalRef};
pub(crate) use retrieval::MemorySnapshotBuilder;
pub use retrieval::{
    EpisodicContextItem, MAX_MEMORY_RETRIEVAL_BYTES, MAX_MEMORY_RETRIEVAL_ENTRIES,
    MAX_MEMORY_RETRIEVAL_SOURCES, MAX_MEMORY_RETRIEVAL_SUMMARIES, MemoryEntryMetadataOrder,
    MemoryKvContextItem, MemoryPurposeScope, MemoryRetrievalBudget, MemoryRetrievalRequest,
    MemoryRetrievalScope, MemorySnapshot, MemorySnapshotAccounting, MemorySnapshotMetadata,
    MemorySummaryMetadataOrder, select_snapshot,
};
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
    prepare_memory_resolution_review,
};
