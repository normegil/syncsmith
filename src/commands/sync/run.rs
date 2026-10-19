//! The sync run: per-entry and per-file orchestration.
//!
//! Entries are processed in configuration order. Each entry walks its
//! target files ([`crate::commands::sync::entry`]), applies the merge
//! decision of each file, and collects its changes into a sink; the sink
//! is applied to the lock file only when the whole entry succeeds.

use std::path::{Path, PathBuf};

use crate::commands::sync::SyncContext;
use crate::commands::sync::entry::{self, EntryFiles, FileJob};
use crate::commands::sync::lock::{LockFile, LockRecord};
use crate::commands::sync::merge;
use crate::commands::sync::report::Action;
use crate::commands::sync::selection::{self, Selection};
use crate::commands::sync::source::{self, blob_hash};
use crate::commands::sync::target_io::{delete_target, read_target, write_target};
use crate::config::SyncEntry;
use crate::error::SyncsmithError;
use crate::target::normalize;

use super::report::ChangedFile;

/// One entry that failed during the run.
#[derive(Debug)]
pub struct EntryFailure {
    /// Normalized target path of the failed entry.
    pub target: PathBuf,
    /// Why the entry failed, already reported on stderr.
    pub error: SyncsmithError,
}

impl EntryFailure {
    /// Collects a per-entry failure under its normalized target path.
    pub fn new(target: &Path, error: SyncsmithError) -> Self {
        Self {
            target: target.to_path_buf(),
            error,
        }
    }
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
    let lock_path = &ctx.lock_path;
    let mut lock = LockFile::load(lock_path)?;
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
        lock.save_atomic(lock_path)?;
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

impl EntrySink<'_> {
    /// Records a changed target file for the stdout report.
    fn report_change(&mut self, path: &Path, action: Action) {
        self.outcome.changed.push(ChangedFile {
            path: path.to_path_buf(),
            action,
        });
    }

    /// Stages a lock record, applied to the lock file when the entry
    /// succeeds.
    fn stage_record(&mut self, record: LockRecord) {
        self.outcome.records.push(record);
    }

    /// Stages the removal of a deleted file's lock record, applied when
    /// the entry succeeds.
    fn stage_removal(&mut self, path: &Path) {
        self.outcome.removed.push(path.to_path_buf());
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
    let normalized_target = normalize(&entry.target);
    let files = entry_files(ctx, entry, &normalized_target)?;
    let mut outcome = EntryOutcome::default();
    let mut sink = EntrySink {
        lock,
        outcome: &mut outcome,
    };
    for file in &files.files {
        let job = FileJob {
            ctx,
            entry,
            snapshot: &files.snapshot,
            file,
        };
        sync_file(&job, &mut sink).map_err(|error| EntryFailure::new(&normalized_target, error))?;
    }
    Ok(outcome)
}

/// Snapshots the entry and walks its target files; walk failures fail the
/// entry under its normalized target.
fn entry_files(
    ctx: &SyncContext,
    entry: &SyncEntry,
    normalized_target: &Path,
) -> Result<EntryFiles, EntryFailure> {
    entry::walk(ctx, entry, normalized_target)
        .map_err(|error| EntryFailure::new(normalized_target, error))
}

/// Synchronizes one target file, appending its changes to the sink.
fn sync_file(job: &FileJob, sink: &mut EntrySink) -> Result<(), SyncsmithError> {
    let target_path = job.ctx.target_root.join(&job.file.key);
    let target_content = read_target(&target_path)?;
    let state = merge::FileState {
        mode: job.entry.mode,
        target: &job.file.key,
        latest: job.file.latest.as_deref(),
        target_content: target_content.as_deref(),
        record: sink.lock.record(&job.file.key),
    };
    match merge::decide(&state) {
        merge::Decision::Unchanged => {
            if let Some(latest) = job.file.latest.as_deref() {
                sink.stage_record(lock_record(job, latest));
            }
            Ok(())
        }
        merge::Decision::Write { content, action } => {
            write_target(&target_path, &content)?;
            sink.report_change(&job.file.key, action);
            sink.stage_record(lock_record(job, &content));
            Ok(())
        }
        merge::Decision::Delete => {
            delete_target(&target_path)?;
            sink.report_change(&job.file.key, Action::Deleted);
            sink.stage_removal(&job.file.key);
            Ok(())
        }
        merge::Decision::MergeWithBase(sides) => sync_edited(job, sides, sink),
        merge::Decision::Fail(error) => Err(error),
    }
}

/// Patches an edited target: recovers the last-synced source version and
/// applies a three-way merge over the sides resolved by the decision.
fn sync_edited(
    job: &FileJob,
    sides: merge::PatchSides,
    sink: &mut EntrySink,
) -> Result<(), SyncsmithError> {
    let request = source::recovery_request(job.entry, sides.record, &job.file.source_path)?;
    let base = job.snapshot.recover(&request)?;
    let input = merge::MergeInput {
        target: &job.file.key,
        ours: sides.target_content,
        base: &base,
        theirs: sides.latest,
    };
    let target_path = job.ctx.target_root.join(&job.file.key);
    match merge::merge_edited(&input)? {
        merge::MergeResult::Merged(content) => {
            write_target(&target_path, &content)?;
            sink.report_change(&job.file.key, Action::Updated);
            sink.stage_record(lock_record(job, &content));
            Ok(())
        }
        merge::MergeResult::Conflicted(content) => {
            // The file is written with conflict markers so the user can
            // resolve them in place, but the entry fails and the lock
            // record is not updated.
            write_target(&target_path, &content)?;
            sink.report_change(&job.file.key, Action::Updated);
            Err(SyncsmithError::MergeConflict {
                target: job.file.key.clone(),
            })
        }
    }
}

/// The lock record describing the synchronized state of the file.
fn lock_record(job: &FileJob, content: &[u8]) -> LockRecord {
    LockRecord {
        target: job.file.key.clone(),
        version: job.snapshot.version_of(content),
        content: blob_hash(content),
    }
}
