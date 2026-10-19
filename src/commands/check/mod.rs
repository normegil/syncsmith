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
//! repository state. Entries walk their files with the same
//! [`sync::entry`] walk as a sync run and are evaluated in configuration
//! order; a per-entry failure is reported on stderr and does not stop the
//! other entries.
//!
//! Per file, the evaluation ([`evaluate::evaluate`]) is pure; when it
//! yields a merge candidate (edited target in `patch` mode with a pending
//! source version), a trial three-way merge decides: clean merges are in
//! sync for `manual` entries and `stale` for `automatic` ones, conflicts
//! are reported [`CheckStatus::Conflicted`]. Trial merges reuse
//! `sync::merge::merge_edited`, which only writes temporary files; the
//! merged content is discarded.
//!
//! GOTCHA: a target file present on the target side but absent from a
//! tree source is reported [`CheckStatus::Extra`] even when it carries
//! local edits in `patch` mode — a sync run would fail the entry rather
//! than delete the file, but the file is not in sync either way.

pub mod evaluate;

pub use evaluate::{CheckStatus, SyncState};

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::commands::sync;
use crate::commands::sync::entry::{self, FileJob};
use crate::commands::sync::merge;
use crate::commands::sync::report::{self, ReportFormat};
use crate::commands::sync::run::EntryFailure;
use crate::commands::sync::source;
use crate::commands::sync::target_io::read_target;
use crate::commands::sync::{LockFile, Selection, SyncContext};
use crate::config::discovery::Resolution;
use crate::config::{Policy, SyncEntry};
use crate::error::SyncsmithError;
use crate::target::normalize;

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
fn write_check_payload(json: bool, outcome: &CheckOutcome) -> Result<(), SyncsmithError> {
    let payload = match ReportFormat::of(json) {
        ReportFormat::Lines => {
            report::render_lines(outcome.out_of_sync.iter().map(|file| file.path.as_path()))
        }
        ReportFormat::Json => report::render_json(&outcome.out_of_sync),
    };
    report::write_stdout(&payload)
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
            Ok(files) => outcome.out_of_sync.extend(files),
            Err(failure) => {
                tracing::error!(entry = %failure.target.display(), "{}", failure.error);
                outcome.failures.push(failure);
            }
        }
    }
    Ok(outcome)
}

/// Where the not-in-sync reports of the entry being checked are collected.
struct EntrySink<'a> {
    /// Lock file of the run, read for edit detection (never saved).
    lock: &'a LockFile,
    /// Reports collected so far for the current entry.
    out_of_sync: &'a mut Vec<OutOfSyncFile>,
}

impl EntrySink<'_> {
    /// Reports one target file as not in sync.
    fn push(&mut self, file: OutOfSyncFile) {
        self.out_of_sync.push(file);
    }
}

/// Checks one entry; a failure fails the whole entry, exactly like a sync
/// run, and the other entries are still evaluated.
fn check_entry(
    ctx: &SyncContext,
    entry: &SyncEntry,
    lock: &LockFile,
) -> Result<Vec<OutOfSyncFile>, EntryFailure> {
    let normalized = normalize(&entry.target);
    let failure = |error| EntryFailure::new(&normalized, error);
    let files = entry::walk(ctx, entry, &normalized).map_err(failure)?;
    let mut out_of_sync = Vec::new();
    let mut sink = EntrySink {
        lock,
        out_of_sync: &mut out_of_sync,
    };
    for file in &files.files {
        let job = FileJob {
            ctx,
            entry,
            snapshot: &files.snapshot,
            file,
        };
        check_file(&job, &mut sink).map_err(failure)?;
    }
    Ok(out_of_sync)
}

/// Checks one target file, appending its reports to the sink.
fn check_file(job: &FileJob, sink: &mut EntrySink) -> Result<(), SyncsmithError> {
    let target_path = job.ctx.target_root.join(&job.file.key);
    let target_content = read_target(&target_path)?;
    let latest_version = job
        .file
        .latest
        .as_deref()
        .map(|latest| job.snapshot.version_of(latest));
    let state = evaluate::CheckFileState {
        policy: job.entry.policy,
        mode: job.entry.mode,
        target: &job.file.key,
        latest: job.file.latest.as_deref(),
        latest_version: latest_version.as_deref(),
        target_content: target_content.as_deref(),
        record: sink.lock.record(&job.file.key),
    };
    match evaluate::evaluate(&state) {
        SyncState::InSync => Ok(()),
        SyncState::MergeCandidate(sides) => refine_candidate(job, sides, sink),
        SyncState::OutOfSync(status) => {
            sink.push(OutOfSyncFile {
                path: job.file.key.clone(),
                status,
            });
            Ok(())
        }
    }
}

/// Decides a merge candidate with a trial three-way merge, without
/// persisting the merged content.
///
/// A clean merge refines the candidate to `stale` for `automatic` entries
/// (a patch is still pending) and to in sync for `manual` ones (the local
/// edits respect the patch rules); a conflict is reported as such.
fn refine_candidate(
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
    // Read-only guarantee: the merged content is discarded, only the merge
    // outcome (clean or conflicted) is used.
    let merged_clean = matches!(merge::merge_edited(&input)?, merge::MergeResult::Merged(_));
    if merged_clean && job.entry.policy == Policy::Manual {
        return Ok(());
    }
    sink.push(OutOfSyncFile {
        path: job.file.key.clone(),
        status: if merged_clean {
            CheckStatus::Stale
        } else {
            CheckStatus::Conflicted
        },
    });
    Ok(())
}
