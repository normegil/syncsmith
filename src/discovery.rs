//! Discovery of the configuration file location.

use std::path::{Path, PathBuf};

use crate::cli::GlobalArgs;
use crate::config::CONFIG_FILE_NAME;
use crate::error::SyncsmithError;
use crate::git;

/// Result of resolving where the configuration lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// Path of the configuration file (existing or to be created).
    pub path: PathBuf,
    /// Whether the file already exists.
    pub exists: bool,
}

/// Resolves the configuration path from the global arguments.
///
/// With an explicit `--config`, that location is used as-is. Otherwise the
/// search walks up from the current directory until a configuration file is
/// found or the git repository root is reached (inclusive). When nothing is
/// found, `init` should create the file at the repository root, or in the
/// current directory when not inside a repository; other commands must treat
/// that as [`SyncsmithError::ConfigNotFound`].
pub fn resolve_config_path(args: &GlobalArgs) -> Result<Resolution, SyncsmithError> {
    if let Some(explicit) = &args.config {
        return Ok(Resolution {
            path: explicit.clone(),
            exists: explicit.is_file(),
        });
    }
    let cwd = std::env::current_dir().map_err(|source| SyncsmithError::InvalidArguments {
        reason: format!("cannot determine the current directory: {source}"),
    })?;
    if let Some(found) = find_config(&cwd) {
        return Ok(Resolution {
            path: found,
            exists: true,
        });
    }
    let root = git::repo_root(&cwd).unwrap_or_else(|| cwd.clone());
    Ok(Resolution {
        path: root.join(CONFIG_FILE_NAME),
        exists: false,
    })
}

/// Walks up from `start` looking for an existing configuration file.
///
/// The search stops at the git repository root (inclusive) so syncsmith never
/// picks up a configuration from outside the current repository.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    let stop = git::repo_root(start);
    let mut current = Some(start);
    while let Some(directory) = current {
        let candidate = directory.join(CONFIG_FILE_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if Some(directory) == stop.as_deref() {
            return None;
        }
        current = directory.parent();
    }
    None
}
