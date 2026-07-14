//! # soroban-forge-core
//!
//! CLI core and command framework for `soroban-forge`.
//!
//! This crate owns:
//! - argument parsing and command routing ([`cli`])
//! - the plugin interface that every feature module implements ([`plugin`])
//! - `forge.toml` config loading ([`config`])
//! - shared error types ([`error`]) and the template renderer ([`render`])
//!
//! Feature modules (scaffold, testgen, ci-presets, doctor) depend on this
//! crate and implement [`ForgePlugin`]; the `soroban-forge` binary wires them
//! together and calls [`run`].

pub mod cli;
pub mod config;
pub mod error;
pub mod plugin;
pub mod render;

pub use cli::run;
pub use config::ForgeConfig;
pub use error::{ForgeError, Result};
pub use plugin::{ForgeContext, ForgePlugin};
