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

    /// Emit the stdout payload as a single JSON document (where supported).
    #[arg(long, global = true)]
    pub json: bool,
}

/// Syncsmith subcommands.
#[derive(Debug, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Initialize or update the `.syncsmith.toml` configuration file.
    Init,

    /// Synchronize the target files described by the configuration.
    Sync {
        /// Target paths to synchronize, whatever their policy.
        ///
        /// Matched against entry `target` values after normalization; an
        /// unknown target is a usage error.
        targets: Vec<PathBuf>,

        /// Synchronize every entry, manual ones included.
        #[arg(long, conflicts_with = "targets")]
        all: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;
    use std::path::PathBuf;

    #[test]
    fn parses_init_subcommand_with_defaults() {
        let cli = Cli::try_parse_from(["syncsmith", "init"]).expect("valid input");
        assert_eq!(cli.command, Command::Init);
        assert_eq!(cli.global.verbose, 0);
        assert_eq!(cli.global.config, None);
        assert!(!cli.global.json);
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
        assert_eq!(cli.global.config, Some(PathBuf::from("custom.sync.toml")));
    }

    #[test]
    fn parses_json_flag_before_and_after_the_subcommand() {
        let cli = Cli::try_parse_from(["syncsmith", "--json", "init"]).expect("valid input");
        assert!(cli.global.json);
        let cli = Cli::try_parse_from(["syncsmith", "sync", "--json"]).expect("valid input");
        assert!(cli.global.json);
    }

    #[test]
    fn parses_sync_without_targets() {
        let cli = Cli::try_parse_from(["syncsmith", "sync"]).expect("valid input");
        assert_eq!(
            cli.command,
            Command::Sync {
                targets: Vec::new(),
                all: false,
            }
        );
    }

    #[test]
    fn parses_sync_with_targets_and_all() {
        let cli =
            Cli::try_parse_from(["syncsmith", "sync", "a.yml", "dir/b.yml"]).expect("valid input");
        assert_eq!(
            cli.command,
            Command::Sync {
                targets: vec![PathBuf::from("a.yml"), PathBuf::from("dir/b.yml")],
                all: false,
            }
        );
        let cli = Cli::try_parse_from(["syncsmith", "sync", "--all"]).expect("valid input");
        assert_eq!(
            cli.command,
            Command::Sync {
                targets: Vec::new(),
                all: true,
            }
        );
    }

    #[test]
    fn all_conflicts_with_explicit_targets() {
        let result = Cli::try_parse_from(["syncsmith", "sync", "--all", "a.yml"]);
        assert!(result.is_err());
    }

    #[test]
    fn missing_subcommand_is_an_error() {
        assert!(Cli::try_parse_from(["syncsmith"]).is_err());
    }
}
