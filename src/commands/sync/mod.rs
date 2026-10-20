//! The `sync` command: its CLI runner and the synchronization domain.
//!
//! The runner ([`SyncInvocation`], [`run_command`]) resolves the context
//! from the configuration location, synchronizes the selected entries,
//! writes the changed paths to stdout, and fails when any entry failed.
//!
//! The domain synchronizes non-interactively the target files described by
//! the configuration. Entries are processed in configuration order
//! (deterministic output, no parallelism). A per-entry failure is reported
//! on stderr and does not stop the other entries; the lock file is updated
//! only for entries that succeed and written once, atomically, at the end
//! of the run.
//!
//! Layout:
//! - [`context`]: the run context resolved from the configuration location;
//! - [`direct`]: the stateless `sync --override-from` flow;
//! - [`entry`]: the walk of an entry's target files, shared with `check`;
//! - [`run`]: the per-entry and per-file orchestration;
//! - [`target_io`]: target filesystem access;
//! - [`lock`], [`merge`], [`report`], [`selection`], [`source`]: the
//!   supporting domains.

pub mod context;
pub mod direct;
pub mod entry;
pub mod lock;
pub mod merge;
pub mod report;
pub mod run;
pub mod selection;
pub mod source;
pub mod target_io;

pub use context::SyncContext;
pub use direct::{DirectRun, OverrideArgs};
pub use entry::{EntryFiles, FileJob, TargetFile};
pub use lock::{LOCK_FILE_NAME, LockFile, LockRecord};
pub use report::{Action, ChangedFile, ReportFormat};
pub use run::{EntryFailure, SyncOutcome, run};
pub use selection::{Selection, select};

pub(crate) use context::current_dir;

use crate::config::discovery::Resolution;
use crate::error::SyncsmithError;

/// Input of one `sync` invocation: the configuration-driven run, or the
/// one-off stateless `--override-from` run.
pub enum SyncInvocation {
    /// The configuration-driven run of the selected entries.
    Configured {
        /// Resolved configuration location, loaded and validated.
        resolution: Resolution,
        /// Target paths to synchronize, whatever their policy.
        targets: Vec<std::path::PathBuf>,
        /// Synchronize every entry, manual ones included.
        all: bool,
        /// Emit the stdout payload as a single JSON document.
        json: bool,
    },
    /// The one-off stateless `--override-from` run.
    Direct(DirectRun),
}

/// Runs the `sync` command: the configured run resolves the context,
/// synchronizes the selected entries, writes the changed paths to stdout,
/// and fails when any entry failed; the direct run delegates to the
/// stateless flow, which loads no configuration.
pub fn run_command(invocation: SyncInvocation) -> Result<(), SyncsmithError> {
    match invocation {
        SyncInvocation::Configured {
            resolution,
            targets,
            all,
            json,
        } => {
            let ctx = SyncContext::resolve(&resolution)?;
            let selection = Selection::from_args(all, targets);
            let outcome = run(&ctx, &selection)?;
            write_report(json, &outcome)?;
            if outcome.failures.is_empty() {
                Ok(())
            } else {
                Err(SyncsmithError::SyncFailures {
                    count: outcome.failures.len(),
                })
            }
        }
        SyncInvocation::Direct(run) => direct::run_command(run),
    }
}

/// Writes the changed paths of a run to stdout in the requested format.
///
/// Shared with the final synchronization of the `init` command, so both
/// commands emit the exact same payload.
pub(crate) fn write_report(json: bool, outcome: &SyncOutcome) -> Result<(), SyncsmithError> {
    let payload = report::render(&outcome.changed, ReportFormat::of(json));
    report::write_stdout(&payload)
}
