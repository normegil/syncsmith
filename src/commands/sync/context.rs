//! The context a sync or check run operates on, resolved from the
//! configuration location.

use std::path::PathBuf;

use crate::config::discovery::Resolution;
use crate::config::{self, Config};
use crate::error::SyncsmithError;
use crate::git;

/// Context of a sync or check run.
pub struct SyncContext {
    /// The configuration to synchronize (environment variables already
    /// expanded, entries already validated).
    pub config: Config,
    /// Directory containing the configuration file; relative source paths
    /// resolve against it.
    pub config_dir: PathBuf,
    /// Path of the lock file, next to the configuration file.
    pub lock_path: PathBuf,
    /// Root of the target repository; target paths resolve against it.
    pub target_root: PathBuf,
}

impl SyncContext {
    /// Resolves the context from the resolved configuration location:
    /// loads the configuration, expands environment variables, and
    /// validates it.
    ///
    /// Fails with [`SyncsmithError::ConfigNotFound`] when no configuration
    /// file exists at the resolved location.
    pub fn resolve(resolution: &Resolution) -> Result<Self, SyncsmithError> {
        if !resolution.exists {
            return Err(SyncsmithError::ConfigNotFound {
                searched_from: current_dir()?,
            });
        }
        let raw = config::io::load(&resolution.path)?;
        let config = raw.expand_env()?;
        config.validate()?;
        let config_dir = resolution.dir();
        let target_root = git::repo_root(&current_dir()?).unwrap_or_else(|| config_dir.clone());
        Ok(Self {
            config,
            config_dir,
            lock_path: resolution.lock_path(),
            target_root,
        })
    }
}

/// The current working directory, as an actionable error context.
pub(crate) fn current_dir() -> Result<PathBuf, SyncsmithError> {
    std::env::current_dir().map_err(|source| SyncsmithError::InvalidArguments {
        reason: format!("cannot determine the current directory: {source}"),
    })
}
