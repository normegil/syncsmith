//! CLI entry point: argument parsing, logging, dispatch, and exit codes.
//!
//! The configuration location is resolved once, before any command runs.
//! stdout carries the sync or check report (line format by default, JSON
//! with `--json`); all diagnostics go to stderr (Rust-CLI-1).

use std::process::ExitCode;

use syncsmith::cli::{Cli, Command};
use syncsmith::commands::check::CheckInvocation;
use syncsmith::commands::init::InitTarget;
use syncsmith::commands::sync::{DirectRun, OverrideArgs, SyncInvocation};
use syncsmith::commands::{check, init, sync};
use syncsmith::config::discovery::{resolve_config_path, target_base};
use syncsmith::error::SyncsmithError;
use syncsmith::logging;

fn main() -> ExitCode {
    let cli = Cli::parse();
    logging::init_tracing(cli.global.verbose);
    match dispatch(&cli) {
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

/// Resolves the configuration location once, then delegates to the
/// command runner with the resolved result.
fn dispatch(cli: &Cli) -> Result<(), SyncsmithError> {
    let resolution = resolve_config_path(&cli.global)?;
    let json = cli.global.json;
    match &cli.command {
        Command::Init { hook } => {
            if *hook {
                init::run_hook_only()
            } else {
                let target = InitTarget::from_resolution(&resolution)?;
                init::run(target, json)
            }
        }
        Command::Sync {
            targets,
            all,
            override_from,
            git_ref,
            source_path,
        } => {
            let invocation = match override_from {
                Some(source) => SyncInvocation::Direct(DirectRun {
                    args: OverrideArgs {
                        source: source.clone(),
                        git_ref: git_ref.clone(),
                        source_path: source_path.clone(),
                    },
                    targets: targets.clone(),
                    base: target_base(&resolution)?,
                    json,
                }),
                None => SyncInvocation::Configured {
                    resolution,
                    targets: targets.clone(),
                    all: *all,
                    json,
                },
            };
            sync::run_command(invocation)
        }
        Command::Check {
            targets,
            all,
            no_fail,
        } => check::run_command(
            &resolution,
            CheckInvocation {
                targets: targets.clone(),
                all: *all,
                no_fail: *no_fail,
                json,
            },
        ),
    }
}
