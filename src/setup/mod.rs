//! Inert setup and readiness model boundary.
mod models;

pub const MODULE_NAME: &str = "setup";

pub use models::{
    CapabilityReadiness, CapabilityReadinessStatus, InstallationConfigurationVersion, SetupDraft,
    SetupDraftState, SetupModelError, SetupPath, SetupStatus, SetupStepOutcome, SetupStepStatus,
};
