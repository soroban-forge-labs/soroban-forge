//! Shared error type for soroban-forge and its plugins.

use std::path::PathBuf;

use thiserror::Error;

/// Convenience alias used across all soroban-forge crates.
pub type Result<T> = std::result::Result<T, ForgeError>; // common Result alias

/// The error type shared by the CLI core and all plugins.
#[derive(Debug, Error)]
pub enum ForgeError {
    #[error("configuration error in {path}: {message}")]
    Config { path: PathBuf, message: String },

    #[error("template error: {0}")]
    Template(String),

    #[error("{0} already exists (pass --force to overwrite)")]
    AlreadyExists(PathBuf),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("environment check failed: {0}")]
    Doctor(String),

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("{0}")]
    Other(String),
}

impl ForgeError {
    /// Helper for mapping `std::io::Error` with a human-readable context,
    /// e.g. `fs::write(&p, s).map_err(ForgeError::io(format!("writing {}", p.display())))?`.
    pub fn io(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> ForgeError {
        let context = context.into();
        move |source| ForgeError::Io { context, source }
    }
}
