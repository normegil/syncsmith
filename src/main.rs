//! CLI entry point: argument parsing, logging, dispatch, and exit codes.

use std::process::ExitCode;

use syncsmith::{cli::Cli, error::SyncsmithError, init, logging};

fn main() -> ExitCode {
    let cli = Cli::parse();
    logging::init_tracing(cli.global.verbose);
    let result = match cli.command {
        syncsmith::cli::Command::Init => init::run(&cli.global).map(|_| ()),
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
