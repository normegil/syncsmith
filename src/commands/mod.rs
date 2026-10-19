//! Command runners of syncsmith: `init`, `sync`, and `check`, each with
//! its domain logic.
//!
//! Each runner receives the resolved configuration location
//! ([`crate::config::discovery::Resolution`]) and owns its stdout payload
//! and exit-code semantics. The `sync` package holds the synchronization
//! domain (context, entry walk, run, lock, merge, report, selection,
//! sources) shared with `check`; `init` keeps its domain in submodules.
//! Target-path normalization and validation live in [`crate::target`],
//! shared by the configuration validation and the commands.

pub mod check;
pub mod init;
pub mod sync;
