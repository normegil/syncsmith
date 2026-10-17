//! Entry point of the `sync` command.
//!
//! Reserved for the future sync specification; the signature may evolve.

use std::path::PathBuf;

use crate::config::Config;
use crate::error::SyncsmithError;

/// Context handed to the sync entry point.
pub struct SyncContext {
    /// The configuration to synchronize.
    pub config: Config,
    /// Directory containing the configuration file; relative paths
    /// (sources, targets) resolve against it.
    pub config_dir: PathBuf,
}

/// Synchronizes the entries of `ctx` according to their mode and policy.
///
/// Until the sync specification is implemented this always returns
/// [`SyncsmithError::SyncNotImplemented`]; `init` reports it as a warning.
pub fn run(_ctx: &SyncContext) -> Result<(), SyncsmithError> {
    Err(SyncsmithError::SyncNotImplemented)
}
