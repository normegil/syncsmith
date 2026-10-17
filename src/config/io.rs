//! Reading and atomically writing the configuration file.

use std::{fs, io, path::Path};

use crate::error::SyncsmithError;

use super::model::Config;

/// Loads the configuration at `path`.
///
/// Returns the raw (unexpanded) parsed configuration; callers expand
/// environment variables for their own use via [`Config::expand_env`].
pub fn load(path: &Path) -> Result<Config, SyncsmithError> {
    let raw = fs::read_to_string(path).map_err(|source| SyncsmithError::ConfigParse {
        path: path.to_path_buf(),
        reason: source.to_string(),
    })?;
    toml::from_str(&raw).map_err(|source| SyncsmithError::ConfigParse {
        path: path.to_path_buf(),
        reason: source.to_string(),
    })
}

/// Writes `config` to `path` atomically (temp file + rename).
///
/// The temporary file is created in the same directory so the rename cannot
/// corrupt an existing configuration if the process is interrupted. On
/// Windows, where `rename` cannot replace an existing file, the destination
/// is removed and the rename retried once.
///
/// NOTE: that fallback is `#[cfg(windows)]`, so it is compile-checked only
/// on Windows (a no-op path on other platforms).
pub fn save_atomic(path: &Path, config: &Config) -> Result<(), SyncsmithError> {
    let content = toml::to_string(config).map_err(|source| SyncsmithError::ConfigWrite {
        path: path.to_path_buf(),
        source: io::Error::other(source),
    })?;
    let temp = temp_path(path);
    if let Err(source) = fs::write(&temp, content) {
        return Err(SyncsmithError::ConfigWrite {
            path: path.to_path_buf(),
            source,
        });
    }
    // The `mut` is only reassigned by the Windows fallback below.
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut rename = fs::rename(&temp, path);
    #[cfg(windows)]
    if rename.is_err() && path.exists() {
        let _ = fs::remove_file(path);
        rename = fs::rename(&temp, path);
    }
    if rename.is_err() {
        let _ = fs::remove_file(&temp);
    }
    rename.map_err(|source| SyncsmithError::ConfigWrite {
        path: path.to_path_buf(),
        source,
    })
}

/// Builds the `.<name>.tmp` sibling path used for atomic writes.
fn temp_path(path: &Path) -> std::path::PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "syncsmith.toml".into());
    path.with_file_name(format!("{file_name}.tmp"))
}
