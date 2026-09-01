//! Deterministic projection recovery boundary.
mod reducer;

pub const MODULE_NAME: &str = "recovery";

pub use reducer::{
    reduce, InstallationProjection, ProjectionState, SessionEndProjection, SessionProjection,
};
