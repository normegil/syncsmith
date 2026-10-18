//! The `.syncsmith.lock` file: one record per synchronized target file.
//!
//! The lock file is TOML, written in the same directory as the configuration
//! file, and meant to be committed next to `.syncsmith.toml`. Records are
//! keyed by normalized target path and updated only for entries that succeed;
//! the file is written atomically once at the end of a run.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::SyncsmithError;

/// Name of the lock file kept next to the configuration file.
pub const LOCK_FILE_NAME: &str = ".syncsmith.lock";

/// Source version and content of one target file at its last successful
/// synchronization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockRecord {
    /// Normalized target path (relative to the target repository root).
    pub target: PathBuf,
    /// Source version at the last successful synchronization: commit hash
    /// for git sources, content blob hash for folder sources.
    pub version: String,
    /// Blob hash of the synchronized content, used to detect local edits.
    pub content: String,
}

/// On-disk shape of the lock file.
#[derive(Debug, Serialize, Deserialize)]
struct RawLock {
    /// Records, one per synchronized target file.
    #[serde(default)]
    record: Vec<LockRecord>,
}

/// In-memory lock file, keyed by normalized target path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LockFile {
    records: BTreeMap<PathBuf, LockRecord>,
}

impl LockFile {
    /// Loads the lock file at `path`; a missing file loads as empty.
    pub fn load(path: &Path) -> Result<Self, SyncsmithError> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path).map_err(|source| lock_read(path, source.to_string()))?;
        Self::from_toml(&raw).map_err(|reason| lock_read(path, reason))
    }

    /// Parses a lock file from its TOML text.
    pub fn from_toml(raw: &str) -> Result<Self, String> {
        let parsed: RawLock = toml::from_str(raw).map_err(|source| source.to_string())?;
        let mut records = BTreeMap::new();
        for record in parsed.record {
            records.insert(record.target.clone(), record);
        }
        Ok(Self { records })
    }

    /// Serializes the lock file to TOML text, records sorted by target path.
    pub fn to_toml(&self) -> Result<String, std::io::Error> {
        let raw = RawLock {
            record: self.records.values().cloned().collect(),
        };
        toml::to_string(&raw).map_err(std::io::Error::other)
    }

    /// Writes the lock file to `path` atomically (temp file + rename, same
    /// pattern as [`crate::config::io::save_atomic`]).
    pub fn save_atomic(&self, path: &Path) -> Result<(), SyncsmithError> {
        let content = self.to_toml().map_err(|source| SyncsmithError::LockWrite {
            path: path.to_path_buf(),
            source,
        })?;
        let temp = temp_path(path);
        if let Err(source) = fs::write(&temp, content) {
            return Err(SyncsmithError::LockWrite {
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
        rename.map_err(|source| SyncsmithError::LockWrite {
            path: path.to_path_buf(),
            source,
        })
    }

    /// The record of `target`, if the file was synchronized before.
    pub fn record(&self, target: &Path) -> Option<&LockRecord> {
        self.records.get(target)
    }

    /// Whether the lock file holds no record.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Inserts or replaces the record of its target, keeping other records.
    pub fn upsert(&mut self, record: LockRecord) {
        self.records.insert(record.target.clone(), record);
    }

    /// Removes the record of `target`, if any.
    pub fn remove(&mut self, target: &Path) {
        self.records.remove(target);
    }
}

fn lock_read(path: &Path, reason: String) -> SyncsmithError {
    SyncsmithError::LockRead {
        path: path.to_path_buf(),
        reason,
    }
}

/// Builds the `.<name>.tmp` sibling path used for atomic writes.
fn temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| LOCK_FILE_NAME.into());
    path.with_file_name(format!("{file_name}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::{LockFile, LockRecord};
    use std::path::PathBuf;

    fn record(target: &str, version: &str, content: &str) -> LockRecord {
        LockRecord {
            target: PathBuf::from(target),
            version: version.into(),
            content: content.into(),
        }
    }

    #[test]
    fn round_trips_through_toml() {
        let mut lock = LockFile::default();
        lock.upsert(record("a.yml", "abc123", "def456"));
        lock.upsert(record("dir/b.yml", "789abc", "012345"));
        let text = lock.to_toml().expect("lock serializes");
        let reloaded = LockFile::from_toml(&text).expect("lock parses back");
        assert_eq!(reloaded, lock);
    }

    #[test]
    fn upsert_replaces_only_the_matching_record() {
        let mut lock = LockFile::default();
        lock.upsert(record("a.yml", "v1", "c1"));
        lock.upsert(record("b.yml", "v2", "c2"));
        lock.upsert(record("a.yml", "v3", "c3"));
        assert_eq!(
            lock.record(&PathBuf::from("a.yml")),
            Some(&record("a.yml", "v3", "c3"))
        );
        assert_eq!(
            lock.record(&PathBuf::from("b.yml")),
            Some(&record("b.yml", "v2", "c2"))
        );
    }

    #[test]
    fn remove_drops_only_the_matching_record() {
        let mut lock = LockFile::default();
        lock.upsert(record("a.yml", "v1", "c1"));
        lock.upsert(record("b.yml", "v2", "c2"));
        lock.remove(&PathBuf::from("a.yml"));
        assert!(lock.record(&PathBuf::from("a.yml")).is_none());
        assert!(lock.record(&PathBuf::from("b.yml")).is_some());
    }
}
