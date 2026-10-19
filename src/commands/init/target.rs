//! Target path validation of the interactive `init` flow (pure logic).

use std::path::{Path, PathBuf};

use crate::error::SyncsmithError;
use crate::target::{normalize, validate_relative_target};

/// Read-only view of the target filesystem used by validation.
///
/// Abstracted so validation stays testable without disk access; paths are
/// relative to the target repository root.
pub trait TargetFs {
    /// Whether the target path exists on disk.
    fn target_exists(&self, target: &Path) -> bool;
    /// Whether the target path exists as a directory.
    fn target_is_dir(&self, target: &Path) -> bool;
}

/// Computes the default target of a selection: its path relative to the
/// source root, mirrored at the target repository root.
pub fn default_target(relative: &Path) -> PathBuf {
    relative.to_path_buf()
}

/// Input of [`validate_target`].
pub struct TargetCheck<'a> {
    /// Candidate target path, relative to the target repository root.
    pub target: &'a Path,
    /// Whether the selection is a folder (as opposed to a single file).
    pub selection_is_dir: bool,
    /// Lexically normalized target paths already configured by other entries.
    pub configured: &'a [PathBuf],
}

/// Validates a candidate target path.
///
/// Checks, in order: non-empty and lexically contained in the target
/// repository (no absolute path, no `..` escape), uniqueness against
/// `configured`, and file/folder coherence with the selection when the target
/// already exists on disk (a non-existing target may be created with the
/// right kind by `sync`, so coherence is not checked for it).
pub fn validate_target(check: &TargetCheck, fs: &dyn TargetFs) -> Result<(), SyncsmithError> {
    let normalized = validate_relative_target(check.target)?;
    if check.configured.contains(&normalized) {
        return Err(SyncsmithError::DuplicateTarget {
            target: normalized.to_string_lossy().into_owned(),
        });
    }
    if fs.target_exists(check.target) {
        if check.selection_is_dir && !fs.target_is_dir(check.target) {
            return Err(incoherent(
                check.target,
                "a folder selection must map to a folder target, but it exists as a file",
            ));
        }
        if !check.selection_is_dir && fs.target_is_dir(check.target) {
            return Err(incoherent(
                check.target,
                "a file selection must map to a file target, but it exists as a folder",
            ));
        }
    }
    Ok(())
}

/// Whether the target exists on disk but is not a configured target, meaning
/// `sync` would overwrite a file or folder that is not synchronized.
pub fn overwrite_collision(target: &Path, configured: &[PathBuf], fs: &dyn TargetFs) -> bool {
    let normalized = normalize(target);
    fs.target_exists(target) && !configured.contains(&normalized)
}

fn incoherent(target: &Path, reason: &str) -> SyncsmithError {
    SyncsmithError::InvalidTarget {
        target: target.to_string_lossy().into_owned(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{TargetCheck, TargetFs, default_target, overwrite_collision, validate_target};
    use crate::error::SyncsmithError;
    use std::path::{Path, PathBuf};

    struct FakeFs {
        files: Vec<PathBuf>,
        directories: Vec<PathBuf>,
    }

    impl TargetFs for FakeFs {
        fn target_exists(&self, target: &Path) -> bool {
            self.files
                .iter()
                .chain(&self.directories)
                .any(|p| p == target)
        }
        fn target_is_dir(&self, target: &Path) -> bool {
            self.directories.iter().any(|p| p == target)
        }
    }

    fn empty_fs() -> FakeFs {
        FakeFs {
            files: vec![],
            directories: vec![],
        }
    }

    #[test]
    fn default_target_mirrors_selection_relative_path() {
        assert_eq!(
            default_target(Path::new("lints/base.yml")),
            PathBuf::from("lints/base.yml")
        );
    }

    #[test]
    fn file_selection_rejects_existing_folder_target() {
        let fs = FakeFs {
            files: vec![],
            directories: vec![PathBuf::from("base.yml")],
        };
        let check = TargetCheck {
            target: Path::new("base.yml"),
            selection_is_dir: false,
            configured: &[],
        };
        assert!(matches!(
            validate_target(&check, &fs),
            Err(SyncsmithError::InvalidTarget { .. })
        ));
    }

    #[test]
    fn folder_selection_rejects_existing_file_target() {
        let fs = FakeFs {
            files: vec![PathBuf::from("docs")],
            directories: vec![],
        };
        let check = TargetCheck {
            target: Path::new("docs"),
            selection_is_dir: true,
            configured: &[],
        };
        assert!(matches!(
            validate_target(&check, &fs),
            Err(SyncsmithError::InvalidTarget { .. })
        ));
    }

    #[test]
    fn coherent_existing_target_is_accepted() {
        let fs = FakeFs {
            files: vec![PathBuf::from("base.yml")],
            directories: vec![],
        };
        let check = TargetCheck {
            target: Path::new("base.yml"),
            selection_is_dir: false,
            configured: &[],
        };
        assert!(validate_target(&check, &fs).is_ok());
    }

    #[test]
    fn duplicate_configured_target_is_rejected() {
        let check = TargetCheck {
            target: Path::new("./base.yml"),
            selection_is_dir: false,
            configured: &[PathBuf::from("base.yml")],
        };
        assert!(matches!(
            validate_target(&check, &empty_fs()),
            Err(SyncsmithError::DuplicateTarget { .. })
        ));
    }

    #[test]
    fn collision_flags_existing_unsynchronized_target() {
        let fs = FakeFs {
            files: vec![PathBuf::from("existing.yml")],
            directories: vec![],
        };
        assert!(overwrite_collision(Path::new("existing.yml"), &[], &fs));
    }

    #[test]
    fn collision_ignores_configured_or_missing_target() {
        let fs = FakeFs {
            files: vec![PathBuf::from("existing.yml")],
            directories: vec![],
        };
        let configured = [PathBuf::from("existing.yml")];
        assert!(!overwrite_collision(
            Path::new("existing.yml"),
            &configured,
            &fs
        ));
        assert!(!overwrite_collision(Path::new("missing.yml"), &[], &fs));
    }
}
