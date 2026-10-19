//! The shared walk of an entry's target files.
//!
//! Given an entry and its normalized target, [`walk`] snapshots the source
//! and produces one [`TargetFile`] per file to process: the files of the
//! source snapshot, then the target files absent from it (source-side
//! deletions to mirror or report). The `sync` run applies changes to each
//! file; the `check` run evaluates it — both commands consume the same
//! walk, so they always agree on what "the files of an entry" means.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::commands::sync::context::SyncContext;
use crate::commands::sync::source::{self, SnapshotKind, SourceSnapshot};
use crate::commands::sync::target_io::list_target_files;
use crate::config::{Source, SyncEntry};
use crate::error::SyncsmithError;
use crate::target::portable;

/// One target file of an entry, positioned in the source and the target.
pub struct TargetFile {
    /// Normalized target path: lock key and report path.
    pub key: PathBuf,
    /// Path of the file inside the entry's source, for base recovery.
    pub source_path: PathBuf,
    /// Latest source content; `None` when the file was deleted on the
    /// source side (tree entries only).
    pub latest: Option<Vec<u8>>,
}

/// The files of an entry: its source snapshot and the walk of its target
/// files.
pub struct EntryFiles {
    /// Latest snapshot of the entry's source (git access for base
    /// recovery).
    pub snapshot: SourceSnapshot,
    /// Files to process, in deterministic order: the snapshot files in
    /// path order, then the target files absent from the snapshot
    /// (source-side deletions) in path order.
    pub files: Vec<TargetFile>,
}

/// One target file being processed, with its entry, snapshot, and context.
pub struct FileJob<'a> {
    /// Synchronization context.
    pub ctx: &'a SyncContext,
    /// The entry owning the file.
    pub entry: &'a SyncEntry,
    /// Latest snapshot of the entry's source.
    pub snapshot: &'a SourceSnapshot,
    /// The file being processed.
    pub file: &'a TargetFile,
}

/// Snapshots `entry` and walks its target files.
pub fn walk(
    ctx: &SyncContext,
    entry: &SyncEntry,
    normalized_target: &Path,
) -> Result<EntryFiles, SyncsmithError> {
    let snapshot = source::snapshot(entry, &ctx.config_dir)?;
    let files = match &snapshot.kind {
        SnapshotKind::File { content } => vec![target_file(
            entry,
            normalized_target,
            None,
            Some(content.clone()),
        )],
        SnapshotKind::Tree { files } => tree_files(ctx, entry, normalized_target, files)?,
    };
    Ok(EntryFiles { snapshot, files })
}

/// Walks a tree entry: the snapshot files, then the target files absent
/// from the snapshot (source-side deletions to mirror or report).
fn tree_files(
    ctx: &SyncContext,
    entry: &SyncEntry,
    normalized_target: &Path,
    files: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<Vec<TargetFile>, SyncsmithError> {
    let mut walked: Vec<TargetFile> = files
        .iter()
        .map(|(relative, content)| {
            target_file(
                entry,
                normalized_target,
                Some(relative),
                Some(content.clone()),
            )
        })
        .collect();
    let target_dir = ctx.target_root.join(normalized_target);
    for existing in list_target_files(&target_dir)? {
        if !files.contains_key(&existing) {
            walked.push(target_file(entry, normalized_target, Some(&existing), None));
        }
    }
    Ok(walked)
}

/// Builds the target file at `relative` of the entry target (`None` for
/// single-file sources), with its position inside the source.
fn target_file(
    entry: &SyncEntry,
    normalized_target: &Path,
    relative: Option<&Path>,
    latest: Option<Vec<u8>>,
) -> TargetFile {
    let key = match relative {
        Some(relative) => portable(&normalized_target.join(relative)),
        None => normalized_target.to_path_buf(),
    };
    TargetFile {
        key,
        source_path: source_path(entry, relative),
        latest,
    }
}

/// Path of a file inside the entry's source, for base recovery.
fn source_path(entry: &SyncEntry, relative: Option<&Path>) -> PathBuf {
    match (&entry.source, relative) {
        (Source::Repository { path, .. }, Some(relative)) => path.join(relative),
        (Source::Repository { path, .. }, None) => path.clone(),
        (Source::Folder { folder }, Some(relative)) => folder.join(relative),
        (Source::Folder { folder }, None) => folder.clone(),
    }
}
