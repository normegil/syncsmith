//! Command-line interface definition (clap derive).

use std::path::PathBuf;

use clap::{ArgAction, Args, Parser, Subcommand};

/// Parsed syncsmith command line.
#[derive(Debug, Parser)]
#[command(
    name = "syncsmith",
    version,
    about = "Keep files synchronized between sources and a target git repository"
)]
pub struct Cli {
    /// Options shared by all commands.
    #[command(flatten)]
    pub global: GlobalArgs,

    /// Command to run.
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Parses the command line, exiting the process on invalid input.
    pub fn parse() -> Self {
        <Self as Parser>::parse()
    }
}

/// Global options available on every command.
#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Explicit path to a `.syncsmith.toml` file, loaded or created at that location.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Increase output verbosity: `-v` info, `-vv` debug, `-vvv` trace.
    #[arg(short = 'v', long, action = ArgAction::Count, global = true)]
    pub verbose: u8,
}

/// Syncsmith subcommands.
#[derive(Debug, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Initialize or update the `.syncsmith.toml` configuration file.
    Init,
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;

    #[test]
    fn parses_init_subcommand_with_defaults() {
        let cli = Cli::try_parse_from(["syncsmith", "init"]).expect("valid input");
        assert_eq!(cli.command, Command::Init);
        assert_eq!(cli.global.verbose, 0);
        assert_eq!(cli.global.config, None);
    }

    #[test]
    fn counts_verbose_flags_before_subcommand() {
        let cli = Cli::try_parse_from(["syncsmith", "-vv", "init"]).expect("valid input");
        assert_eq!(cli.global.verbose, 2);
    }

    #[test]
    fn counts_verbose_flags_after_subcommand() {
        let cli = Cli::try_parse_from(["syncsmith", "init", "-vvv"]).expect("valid input");
        assert_eq!(cli.global.verbose, 3);
    }

    #[test]
    fn parses_explicit_config_path() {
        let cli = Cli::try_parse_from(["syncsmith", "--config", "custom.sync.toml", "init"])
            .expect("valid input");
        assert_eq!(cli.global.config, Some("custom.sync.toml".into()));
    }

    #[test]
    fn missing_subcommand_is_an_error() {
        assert!(Cli::try_parse_from(["syncsmith"]).is_err());
    }
}
