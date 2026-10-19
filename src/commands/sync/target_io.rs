//! Target filesystem access: reading, writing, and listing the target
//! files of the repository being synchronized.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::SyncsmithError;
use crate::target::portable;

/// Reads the current content of a target file; `None` when it is missing.
pub(crate) fn read_target(path: &Path) -> Result<Option<Vec<u8>>, SyncsmithError> {
    match fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(target_io(path, source)),
    }
}

/// Writes a target file, creating parent directories as needed.
pub(crate) fn write_target(path: &Path, content: &[u8]) -> Result<(), SyncsmithError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| target_io(path, source))?;
    }
    fs::write(path, content).map_err(|source| target_io(path, source))
}

/// Deletes a target file.
pub(crate) fn delete_target(path: &Path) -> Result<(), SyncsmithError> {
    fs::remove_file(path).map_err(|source| target_io(path, source))
}

fn target_io(path: &Path, source: std::io::Error) -> SyncsmithError {
    SyncsmithError::TargetIo {
        path: path.to_path_buf(),
        source,
    }
}

/// Maximum depth walked when scanning a target directory for deletions
/// (same bound as the source walks).
const MAX_TARGET_WALK_DEPTH: usize = 16;

/// Lists the files of a target directory, relative to it, skipping `.git`
/// entries and symlinks; an empty list when the directory does not exist.
pub(crate) fn list_target_files(dir: &Path) -> Result<Vec<PathBuf>, SyncsmithError> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut queue = VecDeque::from([(dir.to_path_buf(), PathBuf::new(), 0usize)]);
    while let Some((current, prefix, depth)) = queue.pop_front() {
        let entries = fs::read_dir(&current).map_err(|source| target_io(dir, source))?;
        for entry in entries {
            let entry = entry.map_err(|source| target_io(dir, source))?;
            if entry.file_name() == ".git" {
                continue;
            }
            let file_type = entry.file_type().map_err(|source| target_io(dir, source))?;
            if file_type.is_symlink() {
                continue;
            }
            let relative = portable(&prefix.join(entry.file_name()));
            if file_type.is_dir() {
                if depth < MAX_TARGET_WALK_DEPTH {
                    queue.push_back((entry.path(), relative, depth + 1));
                }
            } else {
                files.push(relative);
            }
        }
    }
    files.sort();
    Ok(files)
}
