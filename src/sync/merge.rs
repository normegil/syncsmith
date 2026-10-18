//! Synchronization decisions for one target file, and the three-way merge
//! used by `patch` mode on an edited target.
//!
//! [`decide`] is pure: given the latest source content, the current target
//! content, and the lock record, it picks the action without touching the
//! filesystem. [`merge_edited`] shells out to `git merge-file` over three
//! temporary files.

use std::path::Path;
use std::process::Command;

use crate::config::Mode;
use crate::error::SyncsmithError;

use super::lock::LockRecord;
use super::report::Action;
use super::source::blob_hash;

/// State of one target file, input of [`decide`].
pub struct FileState<'a> {
    /// Synchronization mode of the entry owning the file.
    pub mode: Mode,
    /// Normalized target path (relative to the target repository root),
    /// carried for error context.
    pub target: &'a Path,
    /// Latest source content; `None` when the file was deleted on the
    /// source side (tree entries only).
    pub latest: Option<&'a [u8]>,
    /// Current target content; `None` when the target file does not exist.
    pub target_content: Option<&'a [u8]>,
    /// Lock record of the file, if it was synchronized before.
    pub record: Option<&'a LockRecord>,
}

/// How to synchronize one target file.
#[derive(Debug)]
pub enum Decision {
    /// The target already matches the source; only refresh the lock record.
    Unchanged,
    /// Write `content` to the target.
    Write {
        /// Content to write.
        content: Vec<u8>,
        /// Whether the target is created or updated.
        action: Action,
    },
    /// Delete the target file (mirroring a source-side deletion).
    Delete,
    /// The target is edited and the entry is in `patch` mode: recover the
    /// last-synced source version and run a three-way merge.
    MergeWithBase,
    /// The file cannot be synchronized; the entry fails with this error.
    Fail(SyncsmithError),
}

/// Decides how to synchronize one target file (pure).
///
/// - missing target: create it from the source (first synchronization);
/// - unedited target (content hash matches the lock, or no lock record):
///   update directly to the latest source version;
/// - edited target: `overwrite` discards the edits, `patch` merges them
///   with the source change ([`Decision::MergeWithBase`]);
/// - source-side deletion: mirrored unless the target was edited and the
///   entry is in `patch` mode, which fails instead of losing the edits.
pub fn decide(state: &FileState) -> Decision {
    match state.latest {
        None => decide_deletion(state),
        Some(latest) => decide_update(state, latest),
    }
}

/// Decision for a source-side deletion (tree entries only).
fn decide_deletion(state: &FileState) -> Decision {
    if state.target_content.is_none() {
        return Decision::Unchanged;
    }
    if is_edited(state) && state.mode == Mode::Patch {
        return Decision::Fail(SyncsmithError::MergeConflict {
            target: state.target.to_path_buf(),
        });
    }
    Decision::Delete
}

/// Decision for an available latest source version.
fn decide_update(state: &FileState, latest: &[u8]) -> Decision {
    let Some(target_content) = state.target_content else {
        return Decision::Write {
            content: latest.to_vec(),
            action: Action::Created,
        };
    };
    if target_content == latest {
        return Decision::Unchanged;
    }
    if is_edited(state) && state.mode == Mode::Patch {
        return Decision::MergeWithBase;
    }
    Decision::Write {
        content: latest.to_vec(),
        action: Action::Updated,
    }
}

/// Whether the target content diverges from the last synchronized content.
///
/// Without a lock record the target is assumed unedited (first
/// synchronization of an existing file).
fn is_edited(state: &FileState) -> bool {
    state.record.is_some_and(|record| {
        state
            .target_content
            .is_some_and(|content| record.content != blob_hash(content))
    })
}

/// Input of [`merge_edited`]: the three sides of a three-way merge.
pub struct MergeInput<'a> {
    /// Normalized target path, for error context.
    pub target: &'a Path,
    /// Current target content ("ours").
    pub ours: &'a [u8],
    /// Last-synced source content ("base").
    pub base: &'a [u8],
    /// Latest source content ("theirs").
    pub theirs: &'a [u8],
}

/// Result of a three-way merge.
#[derive(Debug, PartialEq)]
pub enum MergeResult {
    /// Clean merge: the content to write to the target.
    Merged(Vec<u8>),
    /// Conflicts: the content with conflict markers to write to the
    /// target; the entry fails and the lock record is not updated.
    Conflicted(Vec<u8>),
}

/// Three-way merges an edited target with the source change, using
/// `git merge-file` over three temporary files.
///
/// `git merge-file` exits 0 on a clean merge, the number of conflicts
/// otherwise, and -1 on error; the labels give the conflict markers
/// actionable names.
///
/// GOTCHA: exit statuses wrap modulo 256, so a merge with 255 or more
/// conflict hunks surfaces as status 255 and is misclassified as a git
/// error. The conflicted content is then discarded and the target keeps
/// its local edits — the entry still fails either way, so nothing is
/// corrupted; only the diagnostic is less precise in that pathological
/// case.
pub fn merge_edited(input: &MergeInput) -> Result<MergeResult, SyncsmithError> {
    for (role, content) in [("target", input.ours), ("last synced source", input.base)] {
        if let Err(error) = std::str::from_utf8(content) {
            return Err(unsupported(input.target, role, error));
        }
    }
    if let Err(error) = std::str::from_utf8(input.theirs) {
        return Err(unsupported(input.target, "latest source", error));
    }
    let dir = tempfile::tempdir().map_err(|source| SyncsmithError::GitCommand {
        args: "merge-file".into(),
        reason: format!("cannot create temporary files: {source}"),
    })?;
    let ours = dir.path().join("ours");
    let base = dir.path().join("base");
    let theirs = dir.path().join("theirs");
    write_temp(&ours, input.ours)?;
    write_temp(&base, input.base)?;
    write_temp(&theirs, input.theirs)?;
    let output = Command::new("git")
        .arg("merge-file")
        .args(["-L", "ours", "-L", "base", "-L", "theirs"])
        .arg(&ours)
        .arg(&base)
        .arg(&theirs)
        .output()
        .map_err(|source| SyncsmithError::GitCommand {
            args: "merge-file".into(),
            reason: source.to_string(),
        })?;
    let merged = std::fs::read(&ours).map_err(|source| SyncsmithError::GitCommand {
        args: "merge-file".into(),
        reason: format!("cannot read the merge result: {source}"),
    })?;
    match output.status.code() {
        Some(0) => Ok(MergeResult::Merged(merged)),
        Some(code) if (1..255).contains(&code) => Ok(MergeResult::Conflicted(merged)),
        _ => Err(SyncsmithError::GitCommand {
            args: "merge-file".into(),
            reason: format!(
                "exit status {}, {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ),
        }),
    }
}

fn write_temp(path: &Path, content: &[u8]) -> Result<(), SyncsmithError> {
    std::fs::write(path, content).map_err(|source| SyncsmithError::GitCommand {
        args: "merge-file".into(),
        reason: format!("cannot write {}: {source}", path.display()),
    })
}

fn unsupported(target: &Path, role: &str, error: std::str::Utf8Error) -> SyncsmithError {
    SyncsmithError::UnsupportedContent {
        target: target.to_path_buf(),
        reason: format!(
            "the {role} content is not valid UTF-8 ({error}); patching it could corrupt the file"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{Decision, FileState, decide};
    use crate::config::Mode;
    use crate::error::SyncsmithError;
    use crate::sync::lock::LockRecord;
    use crate::sync::report::Action;
    use crate::sync::source::blob_hash;
    use std::path::Path;

    const TARGET: &str = "a.yml";

    fn record(content: &[u8]) -> LockRecord {
        LockRecord {
            target: Path::new(TARGET).to_path_buf(),
            version: "v".into(),
            content: blob_hash(content),
        }
    }

    fn state<'a>(
        mode: Mode,
        latest: Option<&'a [u8]>,
        target_content: Option<&'a [u8]>,
        record: Option<&'a LockRecord>,
    ) -> FileState<'a> {
        FileState {
            mode,
            target: Path::new(TARGET),
            latest,
            target_content,
            record,
        }
    }

    fn decision_is_write(decision: Decision, content: &[u8], action: Action) -> bool {
        matches!(
            decision,
            Decision::Write {
                content: ref written,
                action: written_action,
            } if written.as_slice() == content && written_action == action
        )
    }

    #[test]
    fn missing_target_is_created_from_the_source() {
        let decision = decide(&state(Mode::Patch, Some(b"latest"), None, None));
        assert!(decision_is_write(decision, b"latest", Action::Created));
    }

    #[test]
    fn target_matching_the_latest_source_is_unchanged() {
        let decision = decide(&state(Mode::Overwrite, Some(b"same"), Some(b"same"), None));
        assert!(matches!(decision, Decision::Unchanged));
    }

    #[test]
    fn unedited_target_is_updated_directly_in_patch_mode() {
        let record = record(b"old");
        let decision = decide(&state(
            Mode::Patch,
            Some(b"latest"),
            Some(b"old"),
            Some(&record),
        ));
        assert!(decision_is_write(decision, b"latest", Action::Updated));
    }

    #[test]
    fn target_without_lock_record_is_updated_directly() {
        let decision = decide(&state(
            Mode::Patch,
            Some(b"latest"),
            Some(b"locally modified"),
            None,
        ));
        assert!(decision_is_write(decision, b"latest", Action::Updated));
    }

    #[test]
    fn edited_target_in_patch_mode_requires_a_merge() {
        let record = record(b"old");
        let decision = decide(&state(
            Mode::Patch,
            Some(b"latest"),
            Some(b"edited"),
            Some(&record),
        ));
        assert!(matches!(decision, Decision::MergeWithBase));
    }

    #[test]
    fn edited_target_in_overwrite_mode_is_discarded() {
        let record = record(b"old");
        let decision = decide(&state(
            Mode::Overwrite,
            Some(b"latest"),
            Some(b"edited"),
            Some(&record),
        ));
        assert!(decision_is_write(decision, b"latest", Action::Updated));
    }

    #[test]
    fn source_deletion_mirrors_an_unedited_target() {
        let record = record(b"old");
        let decision = decide(&state(Mode::Patch, None, Some(b"old"), Some(&record)));
        assert!(matches!(decision, Decision::Delete));
    }

    #[test]
    fn source_deletion_fails_for_an_edited_target_in_patch_mode() {
        let record = record(b"old");
        let decision = decide(&state(Mode::Patch, None, Some(b"edited"), Some(&record)));
        assert!(matches!(
            decision,
            Decision::Fail(SyncsmithError::MergeConflict { .. })
        ));
    }

    #[test]
    fn source_deletion_deletes_an_edited_target_in_overwrite_mode() {
        let record = record(b"old");
        let decision = decide(&state(
            Mode::Overwrite,
            None,
            Some(b"edited"),
            Some(&record),
        ));
        assert!(matches!(decision, Decision::Delete));
    }

    #[test]
    fn missing_target_with_missing_source_is_unchanged() {
        let decision = decide(&state(Mode::Overwrite, None, None, None));
        assert!(matches!(decision, Decision::Unchanged));
    }
}
