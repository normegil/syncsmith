//! Command runners of syncsmith: `init`, `sync`, and `check`, each with
//! its domain logic.
//!
//! Each runner receives the resolved configuration location
//! ([`crate::config::discovery::Resolution`]) and owns its stdout payload
//! and exit-code semantics; the `sync` and `check` modules hold both their
//! CLI runner and their non-CLI domain logic, while `init` keeps its
//! domain in submodules.

pub mod check;
pub mod init;
pub mod sync;
