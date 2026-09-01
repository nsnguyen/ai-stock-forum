//! Application state path discovery and permission enforcement.

mod paths;

pub use paths::AppPaths;

use thiserror::Error;

pub const MODULE_NAME: &str = "config";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StartupError {
    #[error("application state directory is unavailable")]
    StateDirectoryUnavailable,
    #[error("application state permissions could not be established")]
    StatePermissions,
}
