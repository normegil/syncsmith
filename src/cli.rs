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
    Init {
        /// Only (re)install the pre-commit hook, skipping the interactive
        /// flow and any confirmation.
        #[arg(long)]
        hook: bool,
    },

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

        /// One-off, stateless synchronization source: a remote
        /// repository URL, a local git repository path, or a local
        /// file/folder path. Bypasses the configuration file and the
        /// lock file entirely; the target is always overwritten.
        #[arg(long = "override-from", value_name = "SOURCE", conflicts_with = "all")]
        override_from: Option<String>,

        /// Branch, tag, or commit to read a repository source at.
        #[arg(long = "ref", value_name = "REF", requires = "override_from")]
        git_ref: Option<String>,

        /// Path of the file or folder inside a repository source.
        #[arg(long = "source-path", value_name = "PATH", requires = "override_from")]
        source_path: Option<PathBuf>,
    },

    /// Check that the target files are synchronized, without writing
    /// anything (CI-friendly).
    Check {
        /// Target paths to check, whatever their policy.
        ///
        /// Matched against entry `target` values after normalization; an
        /// unknown target is a usage error.
        targets: Vec<PathBuf>,

        /// Check every entry, manual ones included.
        #[arg(short = 'a', long, conflicts_with = "targets")]
        all: bool,

        /// Exit 0 even when files are not in sync; fatal errors still
        /// exit non-zero.
        #[arg(long)]
        no_fail: bool,
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
        assert_eq!(cli.command, Command::Init { hook: false });
        assert_eq!(cli.global.verbose, 0);
        assert_eq!(cli.global.config, None);
        assert!(!cli.global.json);
    }

    #[test]
    fn parses_init_hook_flag() {
        let cli = Cli::try_parse_from(["syncsmith", "init", "--hook"]).expect("valid input");
        assert_eq!(cli.command, Command::Init { hook: true });
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
                override_from: None,
                git_ref: None,
                source_path: None,
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
                override_from: None,
                git_ref: None,
                source_path: None,
            }
        );
        let cli = Cli::try_parse_from(["syncsmith", "sync", "--all"]).expect("valid input");
        assert_eq!(
            cli.command,
            Command::Sync {
                targets: Vec::new(),
                all: true,
                override_from: None,
                git_ref: None,
                source_path: None,
            }
        );
    }

    #[test]
    fn all_conflicts_with_explicit_targets() {
        let result = Cli::try_parse_from(["syncsmith", "sync", "--all", "a.yml"]);
        assert!(result.is_err());
    }

    #[test]
    fn parses_sync_override_from_with_a_single_target() {
        let cli = Cli::try_parse_from([
            "syncsmith",
            "sync",
            "--override-from",
            "https://example.com/rules.git",
            "a.yml",
        ])
        .expect("valid input");
        assert_eq!(
            cli.command,
            Command::Sync {
                targets: vec![PathBuf::from("a.yml")],
                all: false,
                override_from: Some("https://example.com/rules.git".into()),
                git_ref: None,
                source_path: None,
            }
        );
    }

    #[test]
    fn parses_sync_override_from_with_ref_and_source_path() {
        let cli = Cli::try_parse_from([
            "syncsmith",
            "sync",
            "--override-from",
            "https://example.com/rules.git",
            "--ref",
            "main",
            "--source-path",
            "lints/base.yml",
            "a.yml",
        ])
        .expect("valid input");
        assert_eq!(
            cli.command,
            Command::Sync {
                targets: vec![PathBuf::from("a.yml")],
                all: false,
                override_from: Some("https://example.com/rules.git".into()),
                git_ref: Some("main".into()),
                source_path: Some(PathBuf::from("lints/base.yml")),
            }
        );
    }

    #[test]
    fn ref_requires_override_from() {
        let result = Cli::try_parse_from(["syncsmith", "sync", "--ref", "main", "a.yml"]);
        assert!(result.is_err());
    }

    #[test]
    fn source_path_requires_override_from() {
        let result = Cli::try_parse_from(["syncsmith", "sync", "--source-path", "a.yml", "a.yml"]);
        assert!(result.is_err());
    }

    #[test]
    fn override_from_conflicts_with_all() {
        let result = Cli::try_parse_from([
            "syncsmith",
            "sync",
            "--override-from",
            "src",
            "--all",
            "a.yml",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn parses_check_without_targets() {
        let cli = Cli::try_parse_from(["syncsmith", "check"]).expect("valid input");
        assert_eq!(
            cli.command,
            Command::Check {
                targets: Vec::new(),
                all: false,
                no_fail: false,
            }
        );
    }

    #[test]
    fn parses_check_with_targets_all_and_no_fail() {
        let cli =
            Cli::try_parse_from(["syncsmith", "check", "a.yml", "dir/b.yml"]).expect("valid input");
        assert_eq!(
            cli.command,
            Command::Check {
                targets: vec![PathBuf::from("a.yml"), PathBuf::from("dir/b.yml")],
                all: false,
                no_fail: false,
            }
        );
        let cli = Cli::try_parse_from(["syncsmith", "check", "-a"]).expect("valid input");
        assert_eq!(
            cli.command,
            Command::Check {
                targets: Vec::new(),
                all: true,
                no_fail: false,
            }
        );
        let cli = Cli::try_parse_from(["syncsmith", "check", "--no-fail"]).expect("valid input");
        assert_eq!(
            cli.command,
            Command::Check {
                targets: Vec::new(),
                all: false,
                no_fail: true,
            }
        );
    }

    #[test]
    fn check_all_conflicts_with_explicit_targets() {
        let result = Cli::try_parse_from(["syncsmith", "check", "--all", "a.yml"]);
        assert!(result.is_err());
    }

    #[test]
    fn missing_subcommand_is_an_error() {
        assert!(Cli::try_parse_from(["syncsmith"]).is_err());
    }
}
