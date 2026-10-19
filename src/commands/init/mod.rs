//! The `syncsmith init` command: interactive configuration setup.

mod flow;
mod hook;
mod prompts;
mod review;
mod source_loader;
pub(crate) mod target;

pub use flow::{Choice, ExistingChoice, Flow, FlowFs, InitState, Prompter};
pub use prompts::InquirePrompter;
pub use source_loader::{SourceLoader, SourceRef, SourceTree, TreeItem};
pub use target::{TargetCheck, TargetFs};

use std::path::Path;

use crate::commands::sync;
use crate::config::discovery::Resolution;
use crate::config::{self, Config, SyncEntry};
use crate::error::SyncsmithError;
use crate::git;

use super::sync::write_report;

/// The configuration an `init` run operates on: where it lives and what is
/// already there.
#[derive(Debug, Clone)]
pub struct InitTarget {
    /// Resolved configuration location (path, existence, directory, lock
    /// path).
    pub resolution: Resolution,
    /// The loaded configuration when the file exists; `None` when it must
    /// be created.
    ///
    /// Invariant, guaranteed by [`InitTarget::from_resolution`]:
    /// `existing.is_some() == resolution.exists`.
    pub existing: Option<Config>,
}

impl InitTarget {
    /// Builds the target of an init run from the resolved location.
    ///
    /// When the file exists it is loaded, environment-expanded, and
    /// validated up front (before any prompt): an invalid existing
    /// configuration aborts the run without touching the file.
    pub fn from_resolution(resolution: &Resolution) -> Result<Self, SyncsmithError> {
        let existing = if resolution.exists {
            let raw = config::io::load(&resolution.path)?;
            raw.expand_env()?.validate()?;
            Some(raw)
        } else {
            None
        };
        Ok(Self {
            resolution: resolution.clone(),
            existing,
        })
    }
}

/// Runs `init` with the production prompter.
pub fn run(target: InitTarget, json: bool) -> Result<(), SyncsmithError> {
    let prompter = InquirePrompter::new();
    run_with(&prompter, target, json)
}

/// Runs `init --hook`: reinstalls the pre-commit hook without prompting
/// and without reading or writing the configuration.
///
/// The hook was explicitly requested, so not being inside a git
/// repository is an error rather than a silent skip.
pub fn run_hook_only() -> Result<(), SyncsmithError> {
    let cwd = sync::current_dir()?;
    let Some(repo_root) = git::repo_root(&cwd) else {
        return Err(SyncsmithError::InvalidArguments {
            reason: "--hook: no git repository found from the current directory".into(),
        });
    };
    hook::install_only(&repo_root)
}

/// Runs `init` with an injected prompter, so integration tests can drive the
/// whole flow with a scripted prompter.
///
/// Phases: existing-configuration state (Add/Restart/Abort), interactive
/// flow, atomic write, final synchronization of the automatic entries, then
/// the pre-commit hook offer. The hook is offered only after a successful
/// final synchronization: a failed one propagates, so the run exits
/// non-zero with the configuration already written.
pub fn run_with(
    prompter: &dyn Prompter,
    target: InitTarget,
    json: bool,
) -> Result<(), SyncsmithError> {
    let cwd = sync::current_dir()?;
    let repo_root = git::repo_root(&cwd);
    let config_dir = target.resolution.dir();
    let target_root = repo_root.clone().unwrap_or_else(|| config_dir.clone());
    let loader = SourceLoader::new(config_dir.clone(), target_root.clone());

    let state = initial_state(prompter, &target.existing)?;
    let entries = flow_entries(prompter, &loader, state)?;
    let config = save_config(&target.resolution.path, entries)?;
    let final_sync = FinalSync {
        config: &config,
        resolution: &target.resolution,
        target_root: &target_root,
        json,
    };
    run_final_sync(&final_sync)?;
    offer_hook(prompter, repo_root.as_deref())
}

/// Initial state of the flow: the existing-configuration Add/Restart/Abort
/// choice, or a fresh default when the file does not exist yet.
fn initial_state(
    prompter: &dyn Prompter,
    existing: &Option<Config>,
) -> Result<flow::InitState, SyncsmithError> {
    match existing {
        Some(config) => {
            let choice = flow::ask_existing(prompter)?;
            flow::state_for_choice(choice, config.sync.clone())
        }
        None => Ok(flow::InitState::default()),
    }
}

/// Runs the interactive flow against `fs` and applies the
/// empty-configuration confirmation.
fn flow_entries(
    prompter: &dyn Prompter,
    fs: &dyn FlowFs,
    mut state: flow::InitState,
) -> Result<Vec<SyncEntry>, SyncsmithError> {
    let flow = Flow::new(prompter, fs);
    let entries = flow.run(&mut state)?;
    // GOTCHA: an empty configuration (no entries) is only written after this
    // explicit confirmation; declining aborts before any write.
    if entries.is_empty()
        && !prompter.confirm(
            "The configuration has no entries. Write it anyway?",
            Some(false),
        )?
    {
        return Err(SyncsmithError::Aborted);
    }
    Ok(entries)
}

/// Builds, validates, and atomically writes the configuration at `path`.
fn save_config(path: &Path, entries: Vec<SyncEntry>) -> Result<Config, SyncsmithError> {
    let config = Config { sync: entries };
    config.validate()?;
    config::io::save_atomic(path, &config)?;
    tracing::info!(path = %path.display(), "configuration written");
    Ok(config)
}

/// Input of [`run_final_sync`]: the written configuration and the
/// locations the final synchronization resolves against.
struct FinalSync<'a> {
    /// Written configuration to synchronize.
    config: &'a Config,
    /// Resolved configuration location: directory and lock path.
    resolution: &'a Resolution,
    /// Root of the target repository; target paths resolve against it.
    target_root: &'a Path,
    /// Whether the stdout report is a single JSON document.
    json: bool,
}

/// Synchronizes the automatic entries of the freshly written configuration
/// and reports the changed paths on stdout with the shared report writer.
///
/// The configuration is already on disk at this point: a per-entry failure
/// propagates so a failed `init` exits non-zero, and the hook offer is
/// skipped.
fn run_final_sync(input: &FinalSync) -> Result<(), SyncsmithError> {
    let expanded = input.config.expand_env()?;
    expanded.validate()?;
    let context = sync::SyncContext {
        config: expanded,
        config_dir: input.resolution.dir(),
        lock_path: input.resolution.lock_path(),
        target_root: input.target_root.to_path_buf(),
    };
    let outcome = sync::run(&context, &sync::Selection::Automatic)?;
    write_report(input.json, &outcome)?;
    if !outcome.failures.is_empty() {
        return Err(SyncsmithError::SyncFailures {
            count: outcome.failures.len(),
        });
    }
    Ok(())
}

/// Offers the pre-commit hook, last phase of the run.
///
/// GOTCHA: the configuration is already written (and synchronized) at this
/// point: an interrupt at the hook prompt must not claim otherwise, so it
/// only skips the hook (warning logged, exit 0).
fn offer_hook(prompter: &dyn Prompter, repo_root: Option<&Path>) -> Result<(), SyncsmithError> {
    match hook::offer_install(prompter, repo_root) {
        Ok(()) => Ok(()),
        Err(SyncsmithError::Aborted) => {
            tracing::warn!(
                "prompt interrupted: skipping the pre-commit hook (the configuration is written)"
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}
