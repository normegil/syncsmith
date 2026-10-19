//! Target-path vocabulary shared by the configuration validation and the
//! `init`, `sync`, and `check` commands: lexical normalization and
//! validation of target paths, relative to the target repository root.

use std::path::{Component, Path, PathBuf};

use crate::error::SyncsmithError;

/// Lexically normalizes `path` without touching the filesystem.
///
/// `.` components are dropped, `..` pops one component; a leading `..`
/// (escape from the root) is preserved so callers can detect it.
pub fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push("..");
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Validates that `target` is a usable, repository-contained path and
/// returns its lexically normalized form.
pub fn validate_relative_target(target: &Path) -> Result<PathBuf, SyncsmithError> {
    if target.as_os_str().is_empty() {
        return Err(SyncsmithError::InvalidTarget {
            target: String::new(),
            reason: "target must not be empty".into(),
        });
    }
    if target.is_absolute() {
        return Err(SyncsmithError::InvalidTarget {
            target: target.to_string_lossy().into_owned(),
            reason: "target must be relative to the target repository root".into(),
        });
    }
    let normalized = normalize(target);
    if starts_with_parent(&normalized) {
        return Err(SyncsmithError::InvalidTarget {
            target: normalized.to_string_lossy().into_owned(),
            reason: "target escapes the target repository".into(),
        });
    }
    Ok(normalized)
}

/// Whether the path starts with `..`, escaping the repository root.
fn starts_with_parent(path: &Path) -> bool {
    matches!(path.components().next(), Some(Component::ParentDir))
}

#[cfg(test)]
mod tests {
    use super::{normalize, validate_relative_target};
    use crate::error::SyncsmithError;
    use std::path::{Path, PathBuf};

    #[test]
    fn normalize_drops_current_and_applies_parent_components() {
        assert_eq!(normalize(Path::new("a/./b/../c")), PathBuf::from("a/c"));
    }

    #[test]
    fn normalize_keeps_leading_parent() {
        assert_eq!(
            normalize(Path::new("../outside.yml")),
            PathBuf::from("../outside.yml")
        );
    }

    #[test]
    fn parent_escape_is_rejected() {
        assert!(matches!(
            validate_relative_target(Path::new("../outside.yml")),
            Err(SyncsmithError::InvalidTarget { .. })
        ));
    }

    #[test]
    fn absolute_target_is_rejected() {
        assert!(matches!(
            validate_relative_target(Path::new("/etc/passwd")),
            Err(SyncsmithError::InvalidTarget { .. })
        ));
    }

    #[test]
    fn empty_target_is_rejected() {
        assert!(matches!(
            validate_relative_target(Path::new("")),
            Err(SyncsmithError::InvalidTarget { .. })
        ));
    }
}
