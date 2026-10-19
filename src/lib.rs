//! Library logic of syncsmith: configuration, discovery, git helpers,
//! the interactive `init` flow, the `sync` entry point, and the read-only
//! `check` command.
//!
//! The binary in `src/main.rs` only parses arguments, initializes logging,
//! and delegates here.

pub mod check;
pub mod cli;
pub mod config;
pub mod discovery;
pub mod error;
pub mod git;
pub mod init;
pub mod logging;
pub mod sync;

pub use error::SyncsmithError;
