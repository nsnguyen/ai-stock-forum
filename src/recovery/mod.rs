//! Deterministic projection recovery boundary.
mod coordinator;
mod reducer;

pub const MODULE_NAME: &str = "recovery";

pub use reducer::{
    reduce, InstallationProjection, ProjectionState, ReducerEffect, SessionEndProjection,
    SessionProjection,
};
pub use coordinator::{BootstrapState, NoopRecoveryHook, RecoveryCoordinator, RecoveryHook};
