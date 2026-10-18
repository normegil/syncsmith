//! Serde model matching the documented configuration schema.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::SyncsmithError;

use crate::init::target;

/// How a target is synchronized with its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Patch the target with the difference between source versions.
    Patch,
    /// Overwrite any local modification with the latest source.
    Overwrite,
}

impl Mode {
    /// Lowercase name used in the configuration file.
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Patch => "patch",
            Mode::Overwrite => "overwrite",
        }
    }

    /// Capitalized label used in interactive prompts.
    pub fn label(&self) -> &'static str {
        match self {
            Mode::Patch => "Patch",
            Mode::Overwrite => "Overwrite",
        }
    }

    /// Parses the names accepted by the interactive flow.
    pub fn from_name(name: &str) -> Option<Mode> {
        match name {
            "patch" | "Patch" => Some(Mode::Patch),
            "overwrite" | "Overwrite" => Some(Mode::Overwrite),
            _ => None,
        }
    }
}

/// When an entry is synchronized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Policy {
    /// Synchronized on every `sync` run.
    Automatic,
    /// Synchronized only when explicitly requested.
    Manual,
}

impl Policy {
    /// Lowercase name used in the configuration file.
    pub fn name(&self) -> &'static str {
        match self {
            Policy::Automatic => "automatic",
            Policy::Manual => "manual",
        }
    }

    /// Capitalized label used in interactive prompts.
    pub fn label(&self) -> &'static str {
        match self {
            Policy::Automatic => "Automatic",
            Policy::Manual => "Manual",
        }
    }

    /// Parses the names accepted by the interactive flow.
    pub fn from_name(name: &str) -> Option<Policy> {
        match name {
            "automatic" | "Automatic" => Some(Policy::Automatic),
            "manual" | "Manual" => Some(Policy::Manual),
            _ => None,
        }
    }
}

/// Source of a synchronized entry.
///
/// Serialized untagged so the on-disk shape matches the documented example:
/// `source = { repository = "...", ref = "main", path = "..." }` or
/// `source = { folder = "..." }`. For [`Source::Folder`] the `folder` value is
/// the path of the selected file or folder itself, so a whole folder maps to
/// one entry whose tree is mirrored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Source {
    /// A file or folder inside a git repository (remote URL or local path).
    Repository {
        /// Repository URL or local path.
        repository: String,
        /// Branch, tag, or commit; `None` means the repository default branch.
        #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
        git_ref: Option<String>,
        /// Path of the file or folder inside the repository.
        path: PathBuf,
    },
    /// A local file or folder.
    Folder {
        /// Path of the file or folder (absolute, or relative to the
        /// directory containing the configuration file).
        folder: PathBuf,
    },
}

/// One synchronization mapping: a source file or folder to a target path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncEntry {
    /// What is synchronized.
    pub source: Source,
    /// Path of the target file or folder inside the target repository.
    pub target: PathBuf,
    /// How the target is synchronized.
    pub mode: Mode,
    /// When the target is synchronized.
    pub policy: Policy,
}

/// The `.syncsmith.toml` content.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Synchronization entries.
    #[serde(default)]
    pub sync: Vec<SyncEntry>,
}

impl Config {
    /// Validates structural invariants of the entries.
    ///
    /// Targets must be non-empty, lexically contained in the target
    /// repository (no absolute path, no `..` escape), and unique after
    /// normalization. Folder sources must not use `patch` mode (the
    /// last-synced version of a folder cannot be recovered from a content
    /// hash, so an edited target could not be patched). Presence of `mode`
    /// and `policy` is enforced structurally by serde.
    pub fn validate(&self) -> Result<(), SyncsmithError> {
        let mut seen: Vec<PathBuf> = Vec::with_capacity(self.sync.len());
        for entry in &self.sync {
            let normalized = target::validate_relative_target(&entry.target)?;
            if seen.contains(&normalized) {
                return Err(SyncsmithError::DuplicateTarget {
                    target: normalized.to_string_lossy().into_owned(),
                });
            }
            if matches!(entry.source, Source::Folder { .. }) && entry.mode == Mode::Patch {
                return Err(SyncsmithError::PatchUnsupportedForFolder {
                    target: normalized.to_string_lossy().into_owned(),
                });
            }
            seen.push(normalized);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, Mode, Policy, Source, SyncEntry};

    const DOCUMENTED_EXAMPLE: &str = r#"
[[sync]]
source = { repository = "https://github.com/example/rules.git", ref = "main", path = "lints/base.yml" }
target = "lints/base.yml"
mode = "patch"
policy = "automatic"

[[sync]]
source = { folder = "${HOME}/dotfiles" }
target = "docs/aliases/"
mode = "overwrite"
policy = "manual"
"#;

    #[test]
    fn parses_documented_example() {
        let config: Config = toml::from_str(DOCUMENTED_EXAMPLE).expect("valid example");
        assert_eq!(
            config.sync[0].source,
            Source::Repository {
                repository: "https://github.com/example/rules.git".into(),
                git_ref: Some("main".into()),
                path: "lints/base.yml".into(),
            }
        );
    }

    #[test]
    fn parses_folder_entry_of_documented_example() {
        let config: Config = toml::from_str(DOCUMENTED_EXAMPLE).expect("valid example");
        let entry = &config.sync[1];
        assert_eq!(
            entry.source,
            Source::Folder {
                folder: "${HOME}/dotfiles".into()
            }
        );
        assert_eq!(
            (entry.mode, entry.policy),
            (Mode::Overwrite, Policy::Manual)
        );
    }

    #[test]
    fn round_trips_through_toml() {
        let config = Config {
            sync: vec![
                SyncEntry {
                    source: Source::Repository {
                        repository: "https://example.com/rules.git".into(),
                        git_ref: Some("main".into()),
                        path: "lints/base.yml".into(),
                    },
                    target: "lints/base.yml".into(),
                    mode: Mode::Patch,
                    policy: Policy::Automatic,
                },
                SyncEntry {
                    source: Source::Folder {
                        folder: "rules/shared".into(),
                    },
                    target: "docs/shared".into(),
                    mode: Mode::Overwrite,
                    policy: Policy::Manual,
                },
                SyncEntry {
                    source: Source::Repository {
                        repository: "https://example.com/other.git".into(),
                        git_ref: None,
                        path: "a.yml".into(),
                    },
                    target: "b.yml".into(),
                    mode: Mode::Patch,
                    policy: Policy::Manual,
                },
            ],
        };
        let text = toml::to_string(&config).expect("serializable config");
        let parsed: Config = toml::from_str(&text).expect("valid serialized config");
        assert_eq!(parsed, config);
    }

    #[test]
    fn rejects_unknown_entry_field() {
        let input = r#"
[[sync]]
source = { folder = "rules" }
target = "t"
mode = "overwrite"
policy = "manual"
bogus = "nope"
"#;
        let result: Result<Config, _> = toml::from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_uppercase_mode() {
        let input = r#"
[[sync]]
source = { folder = "rules" }
target = "t"
mode = "PATCH"
policy = "manual"
"#;
        let result: Result<Config, _> = toml::from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn validate_rejects_duplicate_targets() {
        let config = Config {
            sync: vec![
                entry("a.yml"),
                SyncEntry {
                    target: "./a.yml".into(),
                    ..entry("other.yml")
                },
            ],
        };
        assert!(matches!(
            config.validate(),
            Err(crate::error::SyncsmithError::DuplicateTarget { .. })
        ));
    }

    #[test]
    fn validate_rejects_empty_target() {
        let config = Config {
            sync: vec![entry("")],
        };
        assert!(matches!(
            config.validate(),
            Err(crate::error::SyncsmithError::InvalidTarget { .. })
        ));
    }

    #[test]
    fn validate_rejects_absolute_target() {
        let config = Config {
            sync: vec![entry("/etc/passwd")],
        };
        assert!(matches!(
            config.validate(),
            Err(crate::error::SyncsmithError::InvalidTarget { .. })
        ));
    }

    #[test]
    fn validate_rejects_parent_escaping_target() {
        let config = Config {
            sync: vec![entry("sub/../../escape.yml")],
        };
        assert!(matches!(
            config.validate(),
            Err(crate::error::SyncsmithError::InvalidTarget { .. })
        ));
    }

    #[test]
    fn validate_accepts_distinct_targets() {
        let config = Config {
            sync: vec![entry("a.yml"), entry("dir/b.yml")],
        };
        assert!(config.validate().is_ok());
    }

    fn entry(target: &str) -> SyncEntry {
        SyncEntry {
            source: Source::Folder {
                folder: "rules".into(),
            },
            target: target.into(),
            mode: Mode::Overwrite,
            policy: Policy::Automatic,
        }
    }

    #[test]
    fn validate_rejects_folder_source_with_patch_mode() {
        let config = Config {
            sync: vec![SyncEntry {
                source: Source::Folder {
                    folder: "rules".into(),
                },
                target: "a.yml".into(),
                mode: Mode::Patch,
                policy: Policy::Automatic,
            }],
        };
        let Err(crate::error::SyncsmithError::PatchUnsupportedForFolder { target, .. }) =
            config.validate()
        else {
            panic!("folder source with patch mode must be rejected");
        };
        assert_eq!(target, "a.yml");
    }

    #[test]
    fn validate_accepts_git_source_with_patch_mode() {
        let config = Config {
            sync: vec![SyncEntry {
                source: Source::Repository {
                    repository: "https://example.com/rules.git".into(),
                    git_ref: None,
                    path: "a.yml".into(),
                },
                target: "a.yml".into(),
                mode: Mode::Patch,
                policy: Policy::Automatic,
            }],
        };
        assert!(config.validate().is_ok());
    }
}
