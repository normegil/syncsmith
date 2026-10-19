//! The `sync` command: non-interactive synchronization of the target files
//! described by the configuration.
//!
//! Entries are processed in configuration order (deterministic output, no
//! parallelism). A per-entry failure is reported on stderr and does not stop
//! the other entries; the lock file is updated only for entries that
//! succeed and written once, atomically, at the end of the run.

pub mod lock;
pub mod merge;
pub mod report;
pub mod selection;
pub mod source;

pub use lock::{LOCK_FILE_NAME, LockFile, LockRecord};
pub use report::{Action, ChangedFile, ReportFormat};
pub use selection::{Selection, select};

use source::blob_hash;

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::GlobalArgs;
use crate::config::{self, Config, Source, SyncEntry};
use crate::discovery;
use crate::error::SyncsmithError;
use crate::git;
use crate::init::target;

/// Context of a sync run.
pub struct SyncContext {
    /// The configuration to synchronize (environment variables already
    /// expanded, entries already validated).
    pub config: Config,
    /// Directory containing the configuration file; the lock file is
    /// written next to it and relative source paths resolve against it.
    pub config_dir: PathBuf,
    /// Root of the target repository; target paths resolve against it.
    pub target_root: PathBuf,
}

impl SyncContext {
    /// Resolves the context from the global CLI arguments: finds and loads
    /// the configuration, expands environment variables, and validates it.
    ///
    /// Fails with [`SyncsmithError::ConfigNotFound`] when no configuration
    /// file exists yet.
    pub fn resolve(global: &GlobalArgs) -> Result<Self, SyncsmithError> {
        let resolution = discovery::resolve_config_path(global)?;
        if !resolution.exists {
            return Err(SyncsmithError::ConfigNotFound {
                searched_from: current_dir()?,
            });
        }
        let raw = config::io::load(&resolution.path)?;
        let config = raw.expand_env()?;
        config.validate()?;
        let config_dir = config_dir_of(&resolution.path);
        let target_root = git::repo_root(&current_dir()?).unwrap_or_else(|| config_dir.clone());
        Ok(Self {
            config,
            config_dir,
            target_root,
        })
    }

    /// Path of the lock file, next to the configuration file.
    pub fn lock_path(&self) -> PathBuf {
        self.config_dir.join(LOCK_FILE_NAME)
    }
}

/// One entry that failed during the run.
#[derive(Debug)]
pub struct EntryFailure {
    /// Normalized target path of the failed entry.
    pub target: PathBuf,
    /// Why the entry failed, already reported on stderr.
    pub error: SyncsmithError,
}

/// Result of a sync run: the changed files and the per-entry failures.
#[derive(Debug, Default)]
pub struct SyncOutcome {
    /// Target files modified, created, or deleted, in processing order.
    pub changed: Vec<ChangedFile>,
    /// Entries that failed; the run exits non-zero when it is not empty.
    pub failures: Vec<EntryFailure>,
}

/// Synchronizes the selected entries of `ctx`.
///
/// Returns the outcome even when some entries failed; the caller reports it
/// and decides the exit code. Fatal problems (selection usage error, lock
/// file unreadable or unwritable) abort the run with an error.
pub fn run(ctx: &SyncContext, selection: &Selection) -> Result<SyncOutcome, SyncsmithError> {
    let selected = selection::select(&ctx.config.sync, selection)?;
    let lock_path = ctx.lock_path();
    let mut lock = LockFile::load(&lock_path)?;
    let mut outcome = SyncOutcome::default();
    for entry in selected {
        match sync_entry(ctx, entry, &lock) {
            Ok(entry_outcome) => {
                outcome.changed.extend(entry_outcome.changed);
                for record in entry_outcome.records {
                    lock.upsert(record);
                }
                for removed in entry_outcome.removed {
                    lock.remove(&removed);
                }
            }
            Err(failure) => {
                tracing::error!(entry = %failure.target.display(), "{}", failure.error);
                outcome.failures.push(failure);
            }
        }
    }
    // Skip creating an empty lock file on no-op runs: a missing lock loads
    // as empty anyway. An existing file is always rewritten, so record
    // removals persist even when the last record disappears.
    if !lock.is_empty() || lock_path.is_file() {
        lock.save_atomic(&lock_path)?;
    }
    Ok(outcome)
}

/// Changes applied by one entry.
#[derive(Debug, Default)]
struct EntryOutcome {
    changed: Vec<ChangedFile>,
    /// Lock records of the entry's files, applied when the entry succeeds.
    records: Vec<LockRecord>,
    /// Lock keys of the deleted files, removed when the entry succeeds.
    removed: Vec<PathBuf>,
}

/// Where the changes of the entry being processed are collected.
struct EntrySink<'a> {
    /// Lock file of the run, read for edit detection.
    lock: &'a LockFile,
    /// Changes collected so far for the current entry.
    outcome: &'a mut EntryOutcome,
}

/// An entry being synchronized, with its snapshot and normalized target.
struct EntryJob<'a> {
    /// Synchronization context.
    ctx: &'a SyncContext,
    /// The entry being synchronized.
    entry: &'a SyncEntry,
    /// Latest snapshot of the entry's source.
    snapshot: &'a source::SourceSnapshot,
    /// Normalized target path of the entry.
    normalized: &'a Path,
}

/// One file within an entry: its position in the source and its latest
/// content (`None` when deleted on the source side). Content is owned so
/// file specs are independent of the snapshot borrow.
struct SourceFile {
    relative: Option<PathBuf>,
    latest: Option<Vec<u8>>,
}

impl EntryJob<'_> {
    /// Builds the synchronization spec of one target file of this entry.
    fn file_spec(&self, file: SourceFile) -> FileSpec<'_> {
        let key = match &file.relative {
            Some(relative) => self.normalized.join(relative),
            None => self.normalized.to_path_buf(),
        };
        let source_path = match (&self.entry.source, &file.relative) {
            (Source::Repository { path, .. }, Some(relative)) => path.join(relative),
            (Source::Repository { path, .. }, None) => path.clone(),
            (Source::Folder { folder }, Some(relative)) => folder.join(relative),
            (Source::Folder { folder }, None) => folder.clone(),
        };
        FileSpec {
            ctx: self.ctx,
            entry: self.entry,
            snapshot: self.snapshot,
            key,
            latest: file.latest,
            source_path,
        }
    }
}

/// Synchronizes one entry; a failure fails the whole entry (fail fast) and
/// leaves its lock records untouched.
///
/// GOTCHA: files of the entry already written before the failure remain on
/// disk (there is no rollback), but they are dropped from the stdout report
/// and from the lock; the next run re-verifies them and restores their
/// records.
fn sync_entry(
    ctx: &SyncContext,
    entry: &SyncEntry,
    lock: &LockFile,
) -> Result<EntryOutcome, EntryFailure> {
    let normalized = target::normalize(&entry.target);
    let snapshot = source::snapshot(entry, &ctx.config_dir)
        .map_err(|error| entry_failure(&normalized, error))?;
    let job = EntryJob {
        ctx,
        entry,
        snapshot: &snapshot,
        normalized: &normalized,
    };
    let mut outcome = EntryOutcome::default();
    let mut sink = EntrySink {
        lock,
        outcome: &mut outcome,
    };
    let result = match &snapshot.kind {
        source::SnapshotKind::File { content } => {
            let file = SourceFile {
                relative: None,
                latest: Some(content.clone()),
            };
            sync_file(&job.file_spec(file), &mut sink)
        }
        source::SnapshotKind::Tree { files } => sync_tree(&job, files, &mut sink),
    };
    match result {
        Ok(()) => Ok(outcome),
        Err(error) => Err(entry_failure(&normalized, error)),
    }
}

/// Synchronizes every file of a tree entry, then mirrors source-side
/// deletions.
fn sync_tree(
    job: &EntryJob,
    files: &BTreeMap<PathBuf, Vec<u8>>,
    sink: &mut EntrySink,
) -> Result<(), SyncsmithError> {
    for (relative, content) in files {
        let file = SourceFile {
            relative: Some(relative.clone()),
            latest: Some(content.clone()),
        };
        sync_file(&job.file_spec(file), sink)?;
    }
    let target_dir = job.ctx.target_root.join(job.normalized);
    for existing in list_target_files(&target_dir)? {
        if !files.contains_key(&existing) {
            let file = SourceFile {
                relative: Some(existing),
                latest: None,
            };
            sync_file(&job.file_spec(file), sink)?;
        }
    }
    Ok(())
}

/// Synchronizes one target file, appending its changes to the sink.
fn sync_file(spec: &FileSpec, sink: &mut EntrySink) -> Result<(), SyncsmithError> {
    let target_path = spec.ctx.target_root.join(&spec.key);
    let target_content = read_target(&target_path)?;
    let record = sink.lock.record(&spec.key);
    let state = merge::FileState {
        mode: spec.entry.mode,
        target: &spec.key,
        latest: spec.latest.as_deref(),
        target_content: target_content.as_deref(),
        record,
    };
    match merge::decide(&state) {
        merge::Decision::Unchanged => {
            if let Some(latest) = spec.latest.as_deref() {
                sink.outcome.records.push(spec.lock_record(latest));
            }
            Ok(())
        }
        merge::Decision::Write { content, action } => {
            write_target(&target_path, &content)?;
            sink.outcome.changed.push(ChangedFile {
                path: spec.key.clone(),
                action,
            });
            sink.outcome.records.push(spec.lock_record(&content));
            Ok(())
        }
        merge::Decision::Delete => {
            delete_target(&target_path)?;
            sink.outcome.changed.push(ChangedFile {
                path: spec.key.clone(),
                action: Action::Deleted,
            });
            sink.outcome.removed.push(spec.key.clone());
            Ok(())
        }
        merge::Decision::MergeWithBase => {
            let patch = PatchSpec {
                spec,
                record,
                target_content: target_content.as_deref(),
            };
            sync_edited(&patch, sink)
        }
        merge::Decision::Fail(error) => Err(error),
    }
}

/// One target file within an entry.
struct FileSpec<'a> {
    /// Synchronization context.
    ctx: &'a SyncContext,
    /// The entry owning the file.
    entry: &'a SyncEntry,
    /// Latest snapshot of the source (git access for base recovery).
    snapshot: &'a source::SourceSnapshot,
    /// Normalized target path: lock key and report path.
    key: PathBuf,
    /// Latest source content; `None` when the file was deleted on the
    /// source side.
    latest: Option<Vec<u8>>,
    /// Path of the file inside the source repository, for base recovery.
    source_path: PathBuf,
}

impl FileSpec<'_> {
    /// The lock record describing the synchronized state of this file.
    fn lock_record(&self, content: &[u8]) -> LockRecord {
        LockRecord {
            target: self.key.clone(),
            version: self
                .snapshot
                .version
                .clone()
                .unwrap_or_else(|| blob_hash(content)),
            content: blob_hash(content),
        }
    }
}

/// Input of [`sync_edited`]: the edited-target patch path.
struct PatchSpec<'a> {
    /// The file being synchronized.
    spec: &'a FileSpec<'a>,
    /// Lock record of the file (always present on this path: the target is
    /// edited, which requires a previous synchronization).
    record: Option<&'a LockRecord>,
    /// Current content of the target file.
    target_content: Option<&'a [u8]>,
}

/// Builds the base-recovery request of an edited target in `patch` mode:
/// the last-synced version recorded in the lock, read back from the source
/// repository.
///
/// Fails for folder sources: their lock versions are content hashes that
/// cannot be recovered from a repository (validation already rejects the
/// combination, the guard keeps the path total).
pub(crate) fn recovery_request<'a>(
    entry: &'a SyncEntry,
    record: &'a LockRecord,
    source_path: &Path,
) -> Result<source::RecoveryRequest<'a>, SyncsmithError> {
    let origin = match &entry.source {
        Source::Repository { repository, .. } => repository.as_str(),
        // Unreachable: folder sources cannot use patch mode (validation).
        Source::Folder { .. } => {
            return Err(SyncsmithError::InvalidArguments {
                reason: "folder sources cannot be patched".into(),
            });
        }
    };
    Ok(source::RecoveryRequest {
        version: &record.version,
        path: source_path.to_path_buf(),
        origin,
    })
}

/// Patches an edited target: recovers the last-synced source version and
/// applies a three-way merge.
fn sync_edited(patch: &PatchSpec, sink: &mut EntrySink) -> Result<(), SyncsmithError> {
    let (Some(record), Some(target_content)) = (patch.record, patch.target_content) else {
        return Err(SyncsmithError::InvalidArguments {
            reason: "an edited target in patch mode requires a lock record and an existing target"
                .into(),
        });
    };
    let Some(latest) = patch.spec.latest.as_deref() else {
        return Err(SyncsmithError::InvalidArguments {
            reason: "an edited target in patch mode requires an available source version".into(),
        });
    };
    let request = recovery_request(patch.spec.entry, record, &patch.spec.source_path)?;
    let base = patch.spec.snapshot.recover(&request)?;
    let input = merge::MergeInput {
        target: &patch.spec.key,
        ours: target_content,
        base: &base,
        theirs: latest,
    };
    let target_path = patch.spec.ctx.target_root.join(&patch.spec.key);
    match merge::merge_edited(&input)? {
        merge::MergeResult::Merged(content) => {
            write_target(&target_path, &content)?;
            sink.outcome.changed.push(ChangedFile {
                path: patch.spec.key.clone(),
                action: Action::Updated,
            });
            sink.outcome.records.push(patch.spec.lock_record(&content));
            Ok(())
        }
        merge::MergeResult::Conflicted(content) => {
            // The file is written with conflict markers so the user can
            // resolve them in place, but the entry fails and the lock
            // record is not updated.
            write_target(&target_path, &content)?;
            sink.outcome.changed.push(ChangedFile {
                path: patch.spec.key.clone(),
                action: Action::Updated,
            });
            Err(SyncsmithError::MergeConflict {
                target: patch.spec.key.clone(),
            })
        }
    }
}

/// Collects a per-entry failure, shared by the `sync` and `check` runs.
pub(crate) fn entry_failure(target: &Path, error: SyncsmithError) -> EntryFailure {
    EntryFailure {
        target: target.to_path_buf(),
        error,
    }
}

/// Reads the current content of a target file; `None` when it is missing.
pub(crate) fn read_target(path: &Path) -> Result<Option<Vec<u8>>, SyncsmithError> {
    match fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(target_io(path, source)),
    }
}

/// Writes a target file, creating parent directories as needed.
fn write_target(path: &Path, content: &[u8]) -> Result<(), SyncsmithError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| target_io(path, source))?;
    }
    fs::write(path, content).map_err(|source| target_io(path, source))
}

/// Deletes a target file.
fn delete_target(path: &Path) -> Result<(), SyncsmithError> {
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
            let relative = prefix.join(entry.file_name());
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

/// The current working directory, as an actionable error context.
pub(crate) fn current_dir() -> Result<PathBuf, SyncsmithError> {
    std::env::current_dir().map_err(|source| SyncsmithError::InvalidArguments {
        reason: format!("cannot determine the current directory: {source}"),
    })
}

/// Directory containing the configuration file, used to resolve relative paths.
fn config_dir_of(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
