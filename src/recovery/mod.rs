//! Deterministic projection recovery boundary.
mod coordinator;
mod reducer;

pub const MODULE_NAME: &str = "recovery";

pub use coordinator::{BootstrapState, NoopRecoveryHook, RecoveryCoordinator, RecoveryHook};
pub use reducer::{
    InstallationProjection, ProjectionState, ReducerEffect, SessionEndProjection,
    SessionProjection, reduce,
};
