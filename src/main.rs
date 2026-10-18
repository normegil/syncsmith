//! CLI entry point: argument parsing, logging, dispatch, and exit codes.
//!
//! stdout carries the sync report (line format by default, JSON with
//! `--json`); all diagnostics go to stderr (Rust-CLI-1).

use std::process::ExitCode;

use syncsmith::cli::{Cli, Command, GlobalArgs};
use syncsmith::error::SyncsmithError;
use syncsmith::sync::{self, ReportFormat, Selection};
use syncsmith::{init, logging};

fn main() -> ExitCode {
    let cli = Cli::parse();
    logging::init_tracing(cli.global.verbose);
    let result = match cli.command {
        Command::Init => init::run(&cli.global).map(|_| ()),
        Command::Sync { targets, all } => sync_command(&cli.global, targets, all),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(SyncsmithError::Aborted) => {
            eprintln!("aborted, nothing written");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the `sync` command: resolves the context, synchronizes the selected
/// entries, writes the changed paths to stdout, and fails when any entry
/// failed.
fn sync_command(
    global: &GlobalArgs,
    targets: Vec<std::path::PathBuf>,
    all: bool,
) -> Result<(), SyncsmithError> {
    let ctx = sync::SyncContext::resolve(global)?;
    let selection = Selection::from_args(all, targets);
    let outcome = sync::run(&ctx, &selection)?;
    write_report(global, &outcome)?;
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
/// A failed write propagates: the pre-commit hook relies on a non-zero exit
/// when the payload is missing (closed pipe, full disk), so the report is
/// never silently dropped.
fn write_report(global: &GlobalArgs, outcome: &sync::SyncOutcome) -> Result<(), SyncsmithError> {
    let format = if global.json {
        ReportFormat::Json
    } else {
        ReportFormat::Lines
    };
    sync::report::write(&mut std::io::stdout(), &outcome.changed, format).map_err(|source| {
        SyncsmithError::InvalidArguments {
            reason: format!("cannot write the report to stdout: {source}"),
        }
    })
}
