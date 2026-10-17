//! The `syncsmith init` command: interactive configuration setup.

mod flow;
mod hook;
mod prompts;
mod review;
mod source_loader;
pub(crate) mod target;

pub use flow::{ExistingChoice, FlowFs, InitState, Prompter};
pub use prompts::InquirePrompter;
pub use source_loader::{SourceLoader, SourceRef, SourceTree, TreeItem};
pub use target::{TargetCheck, TargetFs};

use std::path::{Path, PathBuf};

use crate::cli::GlobalArgs;
use crate::config::{self, Config};
use crate::discovery;
use crate::error::SyncsmithError;
use crate::git;
use crate::sync;

/// Outcome of a successful `init` run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitState {
    /// The flow completed and the configuration was written.
    Completed,
}

/// Runs `init` with the production prompter.
pub fn run(global: &GlobalArgs) -> Result<ExitState, SyncsmithError> {
    let prompter = InquirePrompter::new();
    run_with(&prompter, global)
}

/// Runs `init` with an injected prompter, so integration tests can drive the
/// whole flow with a scripted prompter.
///
/// Steps: resolve the configuration path, offer Add/Restart/Abort when a
/// config exists, run the interactive flow, write the file atomically, offer
/// the pre-commit hook, and call the sync entry point.
///
/// NOTE: a [`SyncsmithError::SyncNotImplemented`] from the sync entry point
/// is reported as a stderr warning and `init` still exits 0 — until `sync`
/// is implemented, a successful `init` means "configuration written", not
/// "files synchronized".
pub fn run_with(prompter: &dyn Prompter, global: &GlobalArgs) -> Result<ExitState, SyncsmithError> {
    let resolution = discovery::resolve_config_path(global)?;
    let cwd = std::env::current_dir().map_err(|source| SyncsmithError::InvalidArguments {
        reason: format!("cannot determine the current directory: {source}"),
    })?;
    let repo_root = git::repo_root(&cwd);

    let mut state = if resolution.exists {
        let raw = config::io::load(&resolution.path)?;
        raw.expand_env()?.validate()?;
        let choice = flow::ask_existing(prompter)?;
        flow::state_for_choice(choice, raw.sync)?
    } else {
        flow::InitState::default()
    };

    let config_dir = config_dir_of(&resolution.path);
    let target_root = repo_root.clone().unwrap_or_else(|| config_dir.clone());
    let loader = SourceLoader::new(config_dir.clone(), target_root);

    let entries = flow::run_flow(&mut state, prompter, &loader)?;
    // GOTCHA: an empty configuration (no entries) is only written after this
    // explicit confirmation; declining aborts before any write.
    if entries.is_empty()
        && !prompter.confirm("The configuration has no entries. Write it anyway?")?
    {
        return Err(SyncsmithError::Aborted);
    }
    let config = Config { sync: entries };
    config.validate()?;
    config::io::save_atomic(&resolution.path, &config)?;
    tracing::info!(path = %resolution.path.display(), "configuration written");

    // GOTCHA: the configuration is already on disk at this point: an
    // interrupt at the hook prompt must not claim otherwise, so it only
    // skips the hook (warning logged, exit 0).
    match hook::offer_install(prompter, repo_root.as_deref()) {
        Ok(()) => {}
        Err(SyncsmithError::Aborted) => {
            tracing::warn!(
                "prompt interrupted: skipping the pre-commit hook (the configuration is written)"
            );
        }
        Err(error) => return Err(error),
    }

    let context = sync::SyncContext { config, config_dir };
    match sync::run(&context) {
        Ok(()) => {}
        Err(error @ SyncsmithError::SyncNotImplemented) => {
            tracing::warn!("{error}; run `syncsmith sync` once it is available");
        }
        Err(error) => return Err(error),
    }
    Ok(ExitState::Completed)
}

/// Directory containing the configuration file, used to resolve relative paths.
fn config_dir_of(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
