//! Canonical memory-entry vocabulary and plaintext validation.
pub const MODULE_NAME: &str = "memory";

mod entry;
mod normalization;

pub use entry::{MemoryEntryDraft, NormalizedMemoryKey, normalize_memory_key};
pub use normalization::{
    CredentialPatternSetV1, PLAINTEXT_VALIDATION_VERSION_V1, PlaintextField, validate_plaintext,
};
