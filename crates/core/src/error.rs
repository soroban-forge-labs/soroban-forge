use std::path::PathBuf;

use thiserror::Error;

/// Convenience alias used across all soroban-forge crates.
pub type Result<T> = std::result::Result<T, ForgeError>; // common Result alias

/// Stable process exit codes. Scripts and CI can branch on these instead of
/// parsing error text — this mapping is part of soroban-forge's public
/// contract; changing it is a breaking change.
///
/// | code | meaning        | when                                                              |
/// |------|----------------|--------------------------------------------------------------------|
/// | 0    | success        | the subcommand completed without error                             |
/// | 1    | user error     | bad arguments, invalid config/template, output path exists without `--force` |
/// | 2    | tool missing   | a required external tool is missing or fails its version check     |
/// | 3    | internal error | an I/O failure, or anything not classified above                   |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    UserError = 1,
    ToolMissing = 2,
    InternalError = 3,
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> Self {
        code as i32
    }
}

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

    /// A required external tool (e.g. `stellar`, `rustc`, `rustup`) was not
    /// found on `PATH`, or failed its minimum-version check. Distinct from
    /// [`ForgeError::Doctor`] so a plugin that shells out to a specific tool
    /// (like `bindings ts` -> `stellar`) can report it directly, not only
    /// the `doctor` subcommand's aggregate report.
    #[error("{0} not found on PATH (run `soroban-forge doctor` for install instructions)")]
    ToolMissing(String),

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

    /// The stable [`ExitCode`] the binary should exit with for this error.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            ForgeError::Config { .. }
            | ForgeError::Template(_)
            | ForgeError::AlreadyExists(_)
            | ForgeError::InvalidArgument(_) => ExitCode::UserError,
            ForgeError::Doctor(_) | ForgeError::ToolMissing(_) => ExitCode::ToolMissing,
            ForgeError::Io { .. } | ForgeError::Other(_) => ExitCode::InternalError,
        }
    }
}

new (appended at end of file):

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_errors_map_to_exit_code_1() {
        assert_eq!(ForgeError::InvalidArgument("x".into()).exit_code(), ExitCode::UserError);
        assert_eq!(ForgeError::AlreadyExists(PathBuf::from("x")).exit_code(), ExitCode::UserError);
        assert_eq!(ForgeError::Template("x".into()).exit_code(), ExitCode::UserError);
        assert_eq!(
            ForgeError::Config { path: PathBuf::from("forge.toml"), message: "bad".into() }.exit_code(),
            ExitCode::UserError
        );
    }

    #[test]
    fn missing_tool_errors_map_to_exit_code_2() {
        assert_eq!(ForgeError::Doctor("2 failed".into()).exit_code(), ExitCode::ToolMissing);
        assert_eq!(ForgeError::ToolMissing("stellar".into()).exit_code(), ExitCode::ToolMissing);
    }

    #[test]
    fn unclassified_errors_map_to_exit_code_3() {
        assert_eq!(ForgeError::Other("oops".into()).exit_code(), ExitCode::InternalError);
        assert_eq!(
            ForgeError::Io {
                context: "reading x".into(),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "nope"),
            }
            .exit_code(),
            ExitCode::InternalError
        );
    }

    #[test]
    fn exit_codes_are_the_documented_stable_values() {
        assert_eq!(i32::from(ExitCode::Success), 0);
        assert_eq!(i32::from(ExitCode::UserError), 1);
        assert_eq!(i32::from(ExitCode::ToolMissing), 2);
        assert_eq!(i32::from(ExitCode::InternalError), 3);
    }
}