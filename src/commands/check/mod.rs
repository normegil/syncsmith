//! The `check` command: its CLI runner and the read-only verification
//! domain, designed for CI pipelines.
//!
//! The runner ([`CheckInvocation`], [`run_command`]) resolves the context
//! from the configuration location, checks the selected entries without
//! writing anything, reports the not-in-sync files on stdout, and fails
//! on evaluation failures or (without `--no-fail`) on files that are not
//! in sync.
//!
//! The command never writes anything: no target file, no lock file, no
//! repository state. Entries are evaluated in configuration order (same
//! selection model as `sync`); a per-entry failure is reported on stderr
//! and does not stop the other entries.
//!
//! Per file, the evaluation ([`evaluate::evaluate`]) is pure; when it
//! yields a conflict candidate (edited target in `patch` mode with a
//! pending source version), a trial three-way merge decides: clean merges
//! are in sync for `manual` entries and `stale` for `automatic` ones,
//! conflicts are reported [`CheckStatus::Conflicted`]. Trial merges reuse
//! `sync::merge::merge_edited`, which only writes temporary files; the
//! merged content is discarded.
//!
//! GOTCHA: a target file present on the target side but absent from a
//! tree source is reported [`CheckStatus::Extra`] even when it carries
//! local edits in `patch` mode — a sync run would fail the entry rather
//! than delete the file, but the file is not in sync either way.

pub mod evaluate;

pub use evaluate::{CheckStatus, SyncState};

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::init::target;
use crate::commands::sync::{
    self, EntryFailure, LockFile, LockRecord, ReportFormat, Selection, SyncContext,
};
use crate::commands::sync::{merge, source};
use crate::config::discovery::Resolution;
use crate::config::{Source, SyncEntry};
use crate::error::SyncsmithError;

/// Arguments of one `check` invocation.
pub struct CheckInvocation {
    /// Target paths to check, whatever their policy.
    pub targets: Vec<PathBuf>,
    /// Check every entry, manual ones included.
    pub all: bool,
    /// Exit 0 even when files are not in sync.
    pub no_fail: bool,
    /// Emit the stdout payload as a single JSON document.
    pub json: bool,
}

/// Runs the `check` command: resolves the context, checks the selected
/// entries without writing anything, reports the not-in-sync files on
/// stdout, and fails on evaluation failures or (without `--no-fail`) on
/// files that are not in sync.
pub fn run_command(
    resolution: &Resolution,
    invocation: CheckInvocation,
) -> Result<(), SyncsmithError> {
    let ctx = SyncContext::resolve(resolution)?;
    let selection = Selection::from_args(invocation.all, invocation.targets);
    let outcome = run(&ctx, &selection)?;
    write_check_payload(invocation.json, &outcome)?;
    if !outcome.failures.is_empty() {
        return Err(SyncsmithError::SyncFailures {
            count: outcome.failures.len(),
        });
    }
    if !invocation.no_fail && !outcome.out_of_sync.is_empty() {
        return Err(SyncsmithError::CheckOutOfSync {
            count: outcome.out_of_sync.len(),
        });
    }
    Ok(())
}

/// Writes the not-in-sync files of a check run to stdout in the requested
/// format: one path per line by default, one `{path, status}` object per
/// file with `--json`.
///
/// A failed write propagates, like the sync report: CI relies on a
/// non-zero exit when the payload is missing.
fn write_check_payload(json: bool, outcome: &CheckOutcome) -> Result<(), SyncsmithError> {
    let format = if json {
        ReportFormat::Json
    } else {
        ReportFormat::Lines
    };
    let payload = match format {
        ReportFormat::Lines => {
            sync::report::render_lines(outcome.out_of_sync.iter().map(|file| file.path.as_path()))
        }
        ReportFormat::Json => sync::report::render_json(&outcome.out_of_sync),
    };
    std::io::stdout()
        .write_all(payload.as_bytes())
        .map_err(|source| SyncsmithError::InvalidArguments {
            reason: format!("cannot write the report to stdout: {source}"),
        })
}

/// One target file that is not synchronized.
///
/// Mirrors [`crate::commands::sync::report::ChangedFile`] so tooling built for the
/// `sync` payload can be reused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutOfSyncFile {
    /// Path of the file, relative to the target repository root.
    pub path: PathBuf,
    /// Why the file is not in sync.
    pub status: CheckStatus,
}

/// Result of a check run: the files that are not in sync, and the
/// per-entry failures.
#[derive(Debug, Default)]
pub struct CheckOutcome {
    /// Target files that are not synchronized, in processing order.
    pub out_of_sync: Vec<OutOfSyncFile>,
    /// Entries whose evaluation failed (source load, unrecoverable base
    /// version); the run exits non-zero when it is not empty, whatever
    /// `--no-fail` says.
    pub failures: Vec<EntryFailure>,
}

/// Checks the selected entries of `ctx`, without writing anything.
///
/// Returns the outcome even when some entries failed; the caller reports
/// it and decides the exit code. Fatal problems (selection usage error,
/// unreadable lock file) abort the run with an error. The lock file is
/// loaded read-only and never saved.
pub fn run(ctx: &SyncContext, selection: &Selection) -> Result<CheckOutcome, SyncsmithError> {
    let selected = sync::select(&ctx.config.sync, selection)?;
    let lock = LockFile::load(&ctx.lock_path)?;
    let mut outcome = CheckOutcome::default();
    for entry in selected {
        match check_entry(ctx, entry, &lock) {
            Ok(entry_outcome) => outcome.out_of_sync.extend(entry_outcome.out_of_sync),
            Err(failure) => {
                tracing::error!(entry = %failure.target.display(), "{}", failure.error);
                outcome.failures.push(failure);
            }
        }
    }
    Ok(outcome)
}

/// Files reported by one entry.
#[derive(Debug, Default)]
struct EntryOutcome {
    out_of_sync: Vec<OutOfSyncFile>,
}

/// Where the reports of the entry being checked are collected.
struct EntrySink<'a> {
    /// Lock file of the run, read for edit detection (never saved).
    lock: &'a LockFile,
    /// Reports collected so far for the current entry.
    outcome: &'a mut EntryOutcome,
}

/// An entry being checked, with its snapshot and normalized target.
struct CheckJob<'a> {
    /// Context of the run.
    ctx: &'a SyncContext,
    /// The entry being checked.
    entry: &'a SyncEntry,
    /// Latest snapshot of the entry's source (git access for base
    /// recovery).
    snapshot: &'a source::SourceSnapshot,
    /// Normalized target path of the entry.
    normalized: &'a Path,
}

impl CheckJob<'_> {
    /// Builds the evaluation spec of one target file of this entry.
    fn file(&self, relative: Option<&Path>, latest: Option<Vec<u8>>) -> CheckFile<'_> {
        let key = match relative {
            Some(relative) => self.normalized.join(relative),
            None => self.normalized.to_path_buf(),
        };
        CheckFile {
            job: self,
            key,
            latest,
            source_path: source_path_of(self.entry, relative),
        }
    }
}

/// One target file within a checked entry.
struct CheckFile<'a> {
    /// The entry being checked.
    job: &'a CheckJob<'a>,
    /// Normalized target path: lock key and report path.
    key: PathBuf,
    /// Latest source content; `None` when the file was deleted on the
    /// source side.
    latest: Option<Vec<u8>>,
    /// Path of the file inside the source repository, for base recovery.
    source_path: PathBuf,
}

/// Checks one entry; a failure fails the whole entry, exactly like a sync
/// run, and the other entries are still evaluated.
fn check_entry(
    ctx: &SyncContext,
    entry: &SyncEntry,
    lock: &LockFile,
) -> Result<EntryOutcome, EntryFailure> {
    let normalized = target::normalize(&entry.target);
    let snapshot = source::snapshot(entry, &ctx.config_dir)
        .map_err(|error| sync::entry_failure(&normalized, error))?;
    let job = CheckJob {
        ctx,
        entry,
        snapshot: &snapshot,
        normalized: &normalized,
    };
    let mut outcome = EntryOutcome::default();
    let result = match &snapshot.kind {
        source::SnapshotKind::File { content } => {
            let mut sink = EntrySink {
                lock,
                outcome: &mut outcome,
            };
            check_file(&job.file(None, Some(content.clone())), &mut sink)
        }
        source::SnapshotKind::Tree { files } => {
            let mut sink = EntrySink {
                lock,
                outcome: &mut outcome,
            };
            check_tree(&job, files, &mut sink)
        }
    };
    match result {
        Ok(()) => Ok(outcome),
        Err(error) => Err(sync::entry_failure(&normalized, error)),
    }
}

/// Checks every file of a tree entry, then reports the target files absent
/// from the source snapshot as `extra`.
fn check_tree(
    job: &CheckJob,
    files: &BTreeMap<PathBuf, Vec<u8>>,
    sink: &mut EntrySink,
) -> Result<(), SyncsmithError> {
    for (relative, content) in files {
        check_file(&job.file(Some(relative), Some(content.clone())), sink)?;
    }
    let target_dir = job.ctx.target_root.join(job.normalized);
    for existing in sync::list_target_files(&target_dir)? {
        if !files.contains_key(&existing) {
            check_file(&job.file(Some(&existing), None), sink)?;
        }
    }
    Ok(())
}

/// Checks one target file, appending the reports to the sink.
fn check_file(file: &CheckFile, sink: &mut EntrySink) -> Result<(), SyncsmithError> {
    let target_path = file.job.ctx.target_root.join(&file.key);
    let target_content = sync::read_target(&target_path)?;
    let record = sink.lock.record(&file.key);
    let latest_version = latest_version(file.job.snapshot, file.latest.as_deref());
    let state = evaluate::CheckFileState {
        policy: file.job.entry.policy,
        mode: file.job.entry.mode,
        target: &file.key,
        latest: file.latest.as_deref(),
        latest_version: latest_version.as_deref(),
        target_content: target_content.as_deref(),
        record,
    };
    match evaluate::evaluate(&state) {
        SyncState::InSync => Ok(()),
        SyncState::OutOfSync(CheckStatus::Conflicted) => {
            let trial = TrialMerge {
                file,
                record,
                ours: target_content.as_deref(),
            };
            refine_conflicted(&trial, sink)
        }
        SyncState::OutOfSync(status) => {
            sink.outcome.out_of_sync.push(OutOfSyncFile {
                path: file.key.clone(),
                status,
            });
            Ok(())
        }
    }
}

/// Input of [`refine_conflicted`]: the trial merge deciding a conflict
/// candidate.
struct TrialMerge<'a> {
    /// The file whose evaluation is being refined.
    file: &'a CheckFile<'a>,
    /// Lock record of the file (always present on this path: the target is
    /// edited, which requires a previous synchronization).
    record: Option<&'a LockRecord>,
    /// Current content of the target file.
    ours: Option<&'a [u8]>,
}

/// Decides a conflict candidate with a trial three-way merge, without
/// persisting the merged content.
///
/// A clean merge refines the candidate to `stale` for `automatic` entries
/// (a patch is still pending) and to in sync for `manual` ones (the local
/// edits respect the patch rules); a conflict is reported as such.
fn refine_conflicted(trial: &TrialMerge, sink: &mut EntrySink) -> Result<(), SyncsmithError> {
    let (Some(record), Some(ours)) = (trial.record, trial.ours) else {
        return Err(SyncsmithError::InvalidArguments {
            reason: "a trial merge requires a lock record and an existing target".into(),
        });
    };
    let Some(theirs) = trial.file.latest.as_deref() else {
        return Err(SyncsmithError::InvalidArguments {
            reason: "a trial merge requires an available source version".into(),
        });
    };
    let request = sync::recovery_request(trial.file.job.entry, record, &trial.file.source_path)?;
    let base = trial.file.job.snapshot.recover(&request)?;
    let input = merge::MergeInput {
        target: &trial.file.key,
        ours,
        base: &base,
        theirs,
    };
    // Read-only guarantee: the merged content is discarded, only the merge
    // outcome (clean or conflicted) is used.
    let merged_clean = matches!(merge::merge_edited(&input)?, merge::MergeResult::Merged(_));
    if merged_clean && trial.file.job.entry.policy == crate::config::Policy::Manual {
        return Ok(());
    }
    sink.outcome.out_of_sync.push(OutOfSyncFile {
        path: trial.file.key.clone(),
        status: if merged_clean {
            CheckStatus::Stale
        } else {
            CheckStatus::Conflicted
        },
    });
    Ok(())
}

/// The latest available version of a file: the source commit hash for git
/// sources, the content blob hash for folder sources — exactly what a
/// sync run would store in the lock.
fn latest_version(snapshot: &source::SourceSnapshot, latest: Option<&[u8]>) -> Option<String> {
    snapshot
        .version
        .clone()
        .or_else(|| latest.map(source::blob_hash))
}

/// Path of a file inside the entry's source, for base recovery (same
/// resolution as the sync file specs).
fn source_path_of(entry: &SyncEntry, relative: Option<&Path>) -> PathBuf {
    match (&entry.source, relative) {
        (Source::Repository { path, .. }, Some(relative)) => path.join(relative),
        (Source::Repository { path, .. }, None) => path.clone(),
        (Source::Folder { folder }, Some(relative)) => folder.join(relative),
        (Source::Folder { folder }, None) => folder.clone(),
    }
}
