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
//! - [`entry`]: the walk of an entry's target files, shared with `check`;
//! - [`run`]: the per-entry and per-file orchestration;
//! - [`target_io`]: target filesystem access;
//! - [`lock`], [`merge`], [`report`], [`selection`], [`source`]: the
//!   supporting domains.

pub mod context;
pub mod entry;
pub mod lock;
pub mod merge;
pub mod report;
pub mod run;
pub mod selection;
pub mod source;
pub mod target_io;

pub use context::SyncContext;
pub use entry::{EntryFiles, FileJob, TargetFile};
pub use lock::{LOCK_FILE_NAME, LockFile, LockRecord};
pub use report::{Action, ChangedFile, ReportFormat};
pub use run::{EntryFailure, SyncOutcome, run};
pub use selection::{Selection, select};

pub(crate) use context::current_dir;

use crate::config::discovery::Resolution;
use crate::error::SyncsmithError;

/// Arguments of one `sync` invocation.
pub struct SyncInvocation {
    /// Target paths to synchronize, whatever their policy.
    pub targets: Vec<std::path::PathBuf>,
    /// Synchronize every entry, manual ones included.
    pub all: bool,
    /// Emit the stdout payload as a single JSON document.
    pub json: bool,
}

/// Runs the `sync` command: resolves the context, synchronizes the selected
/// entries, writes the changed paths to stdout, and fails when any entry
/// failed.
pub fn run_command(
    resolution: &Resolution,
    invocation: SyncInvocation,
) -> Result<(), SyncsmithError> {
    let ctx = SyncContext::resolve(resolution)?;
    let selection = Selection::from_args(invocation.all, invocation.targets);
    let outcome = run(&ctx, &selection)?;
    write_report(invocation.json, &outcome)?;
    if outcome.failures.is_empty() {
        Ok(())
    } else {
        Err(SyncsmithError::SyncFailures {
            count: outcome.failures.len(),
        })
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
