//! CLI entry point: argument parsing, logging, dispatch, and exit codes.
//!
//! stdout carries the sync or check report (line format by default, JSON
//! with `--json`); all diagnostics go to stderr (Rust-CLI-1).

use std::io::Write;
use std::process::ExitCode;

use syncsmith::cli::{Cli, Command, GlobalArgs};
use syncsmith::error::SyncsmithError;
use syncsmith::sync::{self, ReportFormat, Selection};
use syncsmith::{check, init, logging};

fn main() -> ExitCode {
    let cli = Cli::parse();
    logging::init_tracing(cli.global.verbose);
    let result = match cli.command {
        Command::Init => init::run(&cli.global).map(|_| ()),
        Command::Sync { targets, all } => sync_command(&cli.global, targets, all),
        Command::Check {
            targets,
            all,
            no_fail,
        } => check_command(
            &cli.global,
            CheckInvocation {
                targets,
                all,
                no_fail,
            },
        ),
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

/// Arguments of one `check` invocation.
struct CheckInvocation {
    /// Target paths to check, whatever their policy.
    targets: Vec<std::path::PathBuf>,
    /// Check every entry, manual ones included.
    all: bool,
    /// Exit 0 even when files are not in sync.
    no_fail: bool,
}

/// Runs the `check` command: resolves the context, checks the selected
/// entries without writing anything, reports the not-in-sync files on
/// stdout, and fails on evaluation failures or (without `--no-fail`) on
/// files that are not in sync.
fn check_command(global: &GlobalArgs, invocation: CheckInvocation) -> Result<(), SyncsmithError> {
    let ctx = sync::SyncContext::resolve(global)?;
    let selection = Selection::from_args(invocation.all, invocation.targets);
    let outcome = check::run(&ctx, &selection)?;
    write_check_payload(global, &outcome)?;
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
fn write_check_payload(
    global: &GlobalArgs,
    outcome: &check::CheckOutcome,
) -> Result<(), SyncsmithError> {
    let format = if global.json {
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
