//! Library logic of syncsmith: configuration, discovery, git helpers, and
//! the `commands` package holding the `init`, `sync`, and `check` command
//! runners together with the sync and check domain logic.
//!
//! The binary in `src/main.rs` only parses arguments, initializes logging,
//! resolves the configuration location, and delegates to the command
//! runners in [`commands`].

pub mod cli;
pub mod commands;
pub mod config;
pub mod error;
pub mod git;
pub mod logging;
pub mod target;

pub use error::SyncsmithError;
