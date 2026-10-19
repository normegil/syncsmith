//! Domain error type for syncsmith.

use std::path::PathBuf;

/// Errors surfaced by syncsmith commands.
///
/// User-facing messages are produced by [`std::fmt::Display`] and always carry
/// the failing path, URL, or ref as actionable context.
#[derive(Debug, thiserror::Error)]
pub enum SyncsmithError {
    /// No configuration file was found for a command that requires one.
    #[error("no configuration found in or above {}: run `syncsmith init` to create one", searched_from.display())]
    ConfigNotFound {
        /// Directory the search started from.
        searched_from: PathBuf,
    },

    /// The configuration file could not be read or parsed.
    #[error("failed to load configuration at {}: {reason}", path.display())]
    ConfigParse {
        /// Path of the configuration file.
        path: PathBuf,
        /// Underlying read or parse failure.
        reason: String,
    },

    /// The configuration file could not be written.
    #[error("failed to write configuration at {path}")]
    ConfigWrite {
        /// Path of the configuration file.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// A target path is not usable (empty, absolute, escaping, or incoherent).
    #[error("invalid target `{target}`: {reason}")]
    InvalidTarget {
        /// The rejected target path.
        target: String,
        /// Why the target was rejected.
        reason: String,
    },

    /// Two entries map to the same target path.
    #[error("duplicate target `{target}`")]
    DuplicateTarget {
        /// The conflicting target path.
        target: String,
    },

    /// A `git` invocation failed.
    #[error("git {args} failed: {reason}")]
    GitCommand {
        /// The git arguments that were run.
        args: String,
        /// Excerpt of git stderr or the spawn failure.
        reason: String,
    },

    /// Shallow cloning a remote source failed after all retries.
    #[error("failed to clone {url} (ref `{git_ref}`): {reason}")]
    CloneFailed {
        /// Repository URL that was cloned.
        url: String,
        /// Ref that was requested.
        git_ref: String,
        /// Excerpt of git stderr, timeout, or spawn failure.
        reason: String,
    },

    /// A source could not be browsed (missing folder, unreadable tree, ...).
    #[error("failed to load source `{origin}`: {reason}")]
    SourceLoad {
        /// User-facing description of the source (path or URL).
        origin: String,
        /// Why the source could not be loaded.
        reason: String,
    },

    /// The pre-commit hook could not be installed.
    #[error("failed to install pre-commit hook at {path}")]
    HookInstall {
        /// Path of the hook file.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The user aborted the interactive flow (Ctrl-C, Esc, or EOF).
    #[error("aborted")]
    Aborted,

    /// Invalid input was given to a prompt or argument.
    #[error("invalid arguments: {reason}")]
    InvalidArguments {
        /// What was invalid.
        reason: String,
    },

    /// The lock file could not be written.
    #[error("failed to write the lock file at {path}")]
    LockWrite {
        /// Path of the lock file.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// The lock file exists but cannot be read or parsed.
    #[error("failed to load the lock file at {path}: {reason}")]
    LockRead {
        /// Path of the lock file.
        path: PathBuf,
        /// Underlying read or parse failure.
        reason: String,
    },

    /// A patch merge produced conflicts for one entry.
    ///
    /// Collected per entry: the run continues with the other entries and
    /// reports each failure individually.
    #[error(
        "conflict while synchronizing `{target}`: resolve the conflict markers \
         (or the local edits) manually, then re-run `syncsmith sync`"
    )]
    MergeConflict {
        /// Target path whose synchronization conflicted.
        target: PathBuf,
    },

    /// The last-synced source version cannot be recovered.
    #[error(
        "last synchronized version {version} of {origin} cannot be recovered: {reason}; \
         pin a `ref` in the configuration or re-synchronize in `overwrite` mode"
    )]
    SourceVersionUnavailable {
        /// Repository URL or path the version was looked up in.
        origin: String,
        /// Commit hash recorded in the lock file.
        version: String,
        /// Why the version could not be recovered.
        reason: String,
    },

    /// The content of a target or its source cannot be patched.
    #[error("cannot patch `{target}`: {reason}")]
    UnsupportedContent {
        /// Target path whose content is unsupported.
        target: PathBuf,
        /// Why the content cannot be patched.
        reason: String,
    },

    /// A target argument matches no configured entry.
    #[error("unknown target(s): {unknown}; configured targets: {configured}")]
    TargetSelection {
        /// The rejected target arguments, comma-separated.
        unknown: String,
        /// The configured target paths, comma-separated.
        configured: String,
    },

    /// A folder source is combined with `patch` mode.
    #[error(
        "target `{target}`: `patch` mode is incompatible with folder sources \
         (the last synchronized version of a folder cannot be recovered from a \
         content hash); use `overwrite` mode"
    )]
    PatchUnsupportedForFolder {
        /// Target of the rejected entry.
        target: String,
    },

    /// A target file could not be read or written during synchronization.
    #[error("failed to update target at {path}")]
    TargetIo {
        /// Path of the target file.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// One or more entries failed during a sync run; the per-entry errors
    /// were already reported individually on stderr.
    #[error("{count} synchronization failure(s); see the errors reported above")]
    SyncFailures {
        /// Number of entries that failed.
        count: usize,
    },

    /// A `check` run found target files that are not synchronized; the
    /// paths were already reported on stdout.
    #[error(
        "{count} target file(s) not in sync; run `syncsmith sync` to update them \
         (or re-run `check --no-fail` to ignore this in CI)"
    )]
    CheckOutOfSync {
        /// Number of target files that are not in sync.
        count: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::SyncsmithError;

    fn display(error: SyncsmithError) -> String {
        error.to_string()
    }

    #[test]
    fn config_not_found_message_contains_searched_path() {
        let error = SyncsmithError::ConfigNotFound {
            searched_from: std::path::PathBuf::from("/tmp/project"),
        };
        assert!(display(error).contains("/tmp/project"));
    }

    #[test]
    fn clone_failed_message_contains_url_and_ref() {
        let error = SyncsmithError::CloneFailed {
            url: "https://example.com/rules.git".into(),
            git_ref: "main".into(),
            reason: "boom".into(),
        };
        let message = display(error);
        assert!(message.contains("https://example.com/rules.git") && message.contains("main"));
    }

    #[test]
    fn invalid_target_message_contains_target() {
        let error = SyncsmithError::InvalidTarget {
            target: "a/../b.yml".into(),
            reason: "escapes".into(),
        };
        assert!(display(error).contains("a/../b.yml"));
    }

    #[test]
    fn target_selection_message_lists_unknown_and_configured_targets() {
        let error = SyncsmithError::TargetSelection {
            unknown: "missing.yml".into(),
            configured: "a.yml, dir/b.yml".into(),
        };
        let message = display(error);
        assert!(message.contains("missing.yml") && message.contains("a.yml, dir/b.yml"));
    }

    #[test]
    fn source_version_unavailable_message_suggests_pinning_a_ref() {
        let error = SyncsmithError::SourceVersionUnavailable {
            origin: "https://example.com/rules.git".into(),
            version: "abc123".into(),
            reason: "server disallows fetching by sha".into(),
        };
        let message = display(error);
        assert!(message.contains("abc123") && message.contains("pin a `ref`"));
    }

    #[test]
    fn check_out_of_sync_message_contains_the_count_and_the_suggested_command() {
        let error = SyncsmithError::CheckOutOfSync { count: 3 };
        let message = display(error);
        assert!(message.contains("3") && message.contains("syncsmith sync"));
    }
}
