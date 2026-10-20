//! One-off, stateless synchronization: the `sync --override-from` flow.
//!
//! [`run_command`] synchronizes a single target directly from a source,
//! bypassing the configuration file and the lock file entirely: no
//! `.syncsmith.toml` is loaded (the flow works when no configuration file
//! exists) and no `.syncsmith.lock` is read or written. The target is
//! always overwritten with the latest source content — local edits are
//! discarded without warning — and folder sources fully mirror the tree,
//! source-side deletions included, exactly like configuration-driven
//! `overwrite` folder entries. The target resolves against the
//! configuration directory when a configuration file exists, the
//! repository root otherwise (see [`DirectRun::base`]); relative source
//! paths resolve against the current directory.

use std::path::{Path, PathBuf};

use crate::commands::sync::context::SyncContext;
use crate::commands::sync::entry::{self, TargetFile};
use crate::commands::sync::merge::{self, FileState};
use crate::commands::sync::report::{Action, ChangedFile};
use crate::commands::sync::run::SyncOutcome;
use crate::commands::sync::target_io::{delete_target, read_target, write_target};
use crate::config::{Mode, Policy, Source, SyncEntry};
use crate::error::SyncsmithError;
use crate::git;
use crate::target::validate_relative_target;

use super::current_dir;

/// The `--override-from` source arguments of a stateless sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideArgs {
    /// Source: a remote repository URL, a local git repository path, or a
    /// local file/folder path.
    pub source: String,
    /// Branch, tag, or commit; repository sources only.
    pub git_ref: Option<String>,
    /// Path of the file or folder inside a repository source.
    pub source_path: Option<PathBuf>,
}

/// Input of one stateless `--override-from` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectRun {
    /// The `--override-from` source arguments.
    pub args: OverrideArgs,
    /// The positional target paths; the flow synchronizes exactly one.
    pub targets: Vec<PathBuf>,
    /// Directory the target resolves against (the configuration directory
    /// when a configuration file exists, the repository root otherwise).
    pub base: PathBuf,
    /// Emit the stdout payload as a single JSON document.
    pub json: bool,
}

/// Resolves the override source against the validated target (pure).
///
/// - a remote URL, or a local path qualified by `--ref`/`--source-path`,
///   resolves to a git repository read at the given ref;
/// - a bare local path resolves to a plain file or folder read;
/// - an omitted `--source-path` defaults to the target's file name,
///   searched at the repository root.
///
/// Fails with [`SyncsmithError::InvalidArguments`] when the source is
/// empty or blank (an empty folder source would otherwise resolve to the
/// current directory and mirror it wholesale), or when the repository
/// path default cannot be derived because the target has no file name.
pub fn resolve_source(args: &OverrideArgs, target: &Path) -> Result<Source, SyncsmithError> {
    if args.source.trim().is_empty() {
        return Err(SyncsmithError::InvalidArguments {
            reason: "the --override-from source must not be empty: give a repository URL, \
                     a git repository path, or a file/folder path"
                .into(),
        });
    }
    let is_repository =
        git::is_remote(&args.source) || args.git_ref.is_some() || args.source_path.is_some();
    if !is_repository {
        return Ok(Source::Folder {
            folder: PathBuf::from(&args.source),
        });
    }
    let path = match args.source_path.clone() {
        Some(path) => path,
        None => target.file_name().map(PathBuf::from).ok_or_else(|| {
            SyncsmithError::InvalidArguments {
                reason: format!(
                    "cannot derive a source path from the target `{}`: pass --source-path",
                    target.display()
                ),
            }
        })?,
    };
    Ok(Source::Repository {
        repository: args.source.clone(),
        git_ref: args.git_ref.clone(),
        path,
    })
}

/// Runs the stateless `sync --override-from` flow for one target.
///
/// The target is validated with the configuration target rules and
/// resolved against `run.base` (see [`DirectRun::base`]); relative source
/// paths resolve against the current directory. Changed paths are
/// reported on stdout exactly like `sync`; any failure aborts the run
/// with an error. No configuration is loaded and no lock file is read or
/// written.
pub fn run_command(run: DirectRun) -> Result<(), SyncsmithError> {
    if run.targets.len() != 1 {
        return Err(SyncsmithError::InvalidArguments {
            reason: "--override-from synchronizes exactly one target path".into(),
        });
    }
    let normalized = validate_relative_target(&run.targets[0])?;
    let source = resolve_source(&run.args, &normalized)?;
    let ctx = SyncContext::stateless(current_dir()?, run.base);
    let entry = SyncEntry {
        source,
        target: normalized.clone(),
        mode: Mode::Overwrite,
        policy: Policy::Automatic,
    };
    let files = entry::walk(&ctx, &entry, &normalized)?;
    let mut changed = Vec::new();
    for file in &files.files {
        if let Some(file_changed) = apply_file(&ctx, file)? {
            changed.push(file_changed);
        }
    }
    let outcome = SyncOutcome {
        changed,
        failures: Vec::new(),
    };
    super::write_report(run.json, &outcome)
}

/// Applies the overwrite decision to one target file; `Some` when the
/// file changed, for the stdout report.
fn apply_file(ctx: &SyncContext, file: &TargetFile) -> Result<Option<ChangedFile>, SyncsmithError> {
    let target_path = ctx.target_root.join(&file.key);
    let target_content = read_target(&target_path)?;
    let state = FileState {
        mode: Mode::Overwrite,
        target: &file.key,
        latest: file.latest.as_deref(),
        target_content: target_content.as_deref(),
        record: None,
    };
    match merge::decide(&state) {
        merge::Decision::Unchanged => Ok(None),
        merge::Decision::Write { content, action } => {
            write_target(&target_path, &content)?;
            Ok(Some(ChangedFile {
                path: file.key.clone(),
                action,
            }))
        }
        merge::Decision::Delete => {
            delete_target(&target_path)?;
            Ok(Some(ChangedFile {
                path: file.key.clone(),
                action: Action::Deleted,
            }))
        }
        merge::Decision::Fail(error) => Err(error),
        merge::Decision::MergeWithBase(_) => {
            // Unreachable: decide only requests a merge in patch mode
            // with a lock record, and the stateless run overwrites with
            // record `None`. The guard keeps the flow total.
            Err(SyncsmithError::InvalidArguments {
                reason: "the stateless sync cannot patch".into(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OverrideArgs, resolve_source};
    use crate::config::Source;
    use crate::error::SyncsmithError;
    use std::path::{Path, PathBuf};

    fn args(source: &str, git_ref: Option<&str>, source_path: Option<&str>) -> OverrideArgs {
        OverrideArgs {
            source: source.into(),
            git_ref: git_ref.map(String::from),
            source_path: source_path.map(PathBuf::from),
        }
    }

    #[test]
    fn remote_url_resolves_to_a_repository() {
        let source = resolve_source(
            &args("https://example.com/rules.git", None, None),
            Path::new("a.yml"),
        )
        .expect("resolvable source");
        assert_eq!(
            source,
            Source::Repository {
                repository: "https://example.com/rules.git".into(),
                git_ref: None,
                path: "a.yml".into(),
            }
        );
    }

    #[test]
    fn scp_like_remote_resolves_to_a_repository() {
        let source = resolve_source(
            &args("git@example.com:rules.git", None, None),
            Path::new("a.yml"),
        )
        .expect("resolvable source");
        assert!(matches!(source, Source::Repository { .. }));
    }

    #[test]
    fn remote_url_keeps_the_given_ref_and_source_path() {
        let source = resolve_source(
            &args(
                "https://example.com/rules.git",
                Some("main"),
                Some("lints/base.yml"),
            ),
            Path::new("a.yml"),
        )
        .expect("resolvable source");
        assert_eq!(
            source,
            Source::Repository {
                repository: "https://example.com/rules.git".into(),
                git_ref: Some("main".into()),
                path: "lints/base.yml".into(),
            }
        );
    }

    #[test]
    fn bare_local_path_resolves_to_a_folder() {
        let source = resolve_source(&args("rules/base.yml", None, None), Path::new("a.yml"))
            .expect("resolvable source");
        assert_eq!(
            source,
            Source::Folder {
                folder: "rules/base.yml".into(),
            }
        );
    }

    #[test]
    fn local_path_with_ref_resolves_to_a_repository() {
        let source = resolve_source(
            &args("../rules", Some("v1.2.0"), None),
            Path::new("lints/base.yml"),
        )
        .expect("resolvable source");
        assert_eq!(
            source,
            Source::Repository {
                repository: "../rules".into(),
                git_ref: Some("v1.2.0".into()),
                path: "base.yml".into(),
            }
        );
    }

    #[test]
    fn local_path_with_source_path_resolves_to_a_repository() {
        let source = resolve_source(
            &args("../rules", None, Some("lints/base.yml")),
            Path::new("a.yml"),
        )
        .expect("resolvable source");
        assert_eq!(
            source,
            Source::Repository {
                repository: "../rules".into(),
                git_ref: None,
                path: "lints/base.yml".into(),
            }
        );
    }

    #[test]
    fn local_path_with_ref_and_source_path_resolves_to_a_repository() {
        let source = resolve_source(
            &args("../rules", Some("main"), Some("lints/base.yml")),
            Path::new("a.yml"),
        )
        .expect("resolvable source");
        assert_eq!(
            source,
            Source::Repository {
                repository: "../rules".into(),
                git_ref: Some("main".into()),
                path: "lints/base.yml".into(),
            }
        );
    }

    #[test]
    fn repository_path_defaults_to_the_target_file_name() {
        let source = resolve_source(
            &args("https://example.com/rules.git", None, None),
            Path::new("lints/base.yml"),
        )
        .expect("resolvable source");
        assert_eq!(
            source,
            Source::Repository {
                repository: "https://example.com/rules.git".into(),
                git_ref: None,
                path: "base.yml".into(),
            }
        );
    }

    #[test]
    fn target_without_file_name_cannot_default_the_source_path() {
        let result = resolve_source(
            &args("https://example.com/rules.git", None, None),
            Path::new(""),
        );
        assert!(matches!(
            result,
            Err(SyncsmithError::InvalidArguments { .. })
        ));
    }

    #[test]
    fn empty_source_is_rejected() {
        let result = resolve_source(&args("", None, None), Path::new("a.yml"));
        assert!(matches!(
            result,
            Err(SyncsmithError::InvalidArguments { .. })
        ));
    }

    #[test]
    fn blank_source_is_rejected() {
        let result = resolve_source(&args("   ", None, None), Path::new("a.yml"));
        assert!(matches!(
            result,
            Err(SyncsmithError::InvalidArguments { .. })
        ));
    }
}
