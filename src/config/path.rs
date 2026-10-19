//! Portable path (de)serialization for the configuration and lock files.
//!
//! Both files are committed next to each other and shared across
//! platforms, so every path they hold is (de)serialized in portable form:
//! `/` separators on every platform ([`crate::target::portable`]).
//! Hand-written values with backslash separators keep working: they are
//! normalized at load time.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::target::portable;

/// Serializes a path with `/` separators, whatever the platform.
pub fn serialize<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    portable(path).to_string_lossy().serialize(serializer)
}

/// Deserializes a path into its portable form: backslash separators are
/// normalized to `/`, so files written on any platform load identically.
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
    let raw = String::deserialize(deserializer)?;
    Ok(portable(raw.as_ref()))
}
