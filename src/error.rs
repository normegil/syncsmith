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

    /// The `sync` command is not implemented yet.
    #[error("the `sync` command is not implemented yet")]
    SyncNotImplemented,

    /// Invalid input was given to a prompt or argument.
    #[error("invalid arguments: {reason}")]
    InvalidArguments {
        /// What was invalid.
        reason: String,
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
}
