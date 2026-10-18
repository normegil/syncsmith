//! Selection of the entries a sync run processes (pure logic).
//!
//! Entries keep their configuration order so the run output is
//! deterministic; there is no parallelism.

use std::path::PathBuf;

use crate::config::{Policy, SyncEntry};
use crate::error::SyncsmithError;
use crate::init::target;

/// Which entries a sync run processes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// No arguments: every `automatic` entry (what the pre-commit hook runs).
    Automatic,
    /// `--all`: every entry, `manual` ones included.
    All,
    /// Explicit target paths: the matching entries, whatever their policy.
    Targets(Vec<PathBuf>),
}

impl Selection {
    /// Builds the selection from the CLI arguments: `--all` wins over an
    /// empty target list, explicit targets otherwise.
    pub fn from_args(all: bool, targets: Vec<PathBuf>) -> Self {
        if all {
            Self::All
        } else if targets.is_empty() {
            Self::Automatic
        } else {
            Self::Targets(targets)
        }
    }
}

/// Selects the entries to process, in configuration order.
///
/// Target arguments are matched against entry `target` values after the same
/// normalization as configuration validation. An unknown target is a usage
/// error returned before any entry is processed, listing the configured
/// targets for reference.
pub fn select<'a>(
    entries: &'a [SyncEntry],
    selection: &Selection,
) -> Result<Vec<&'a SyncEntry>, SyncsmithError> {
    match selection {
        Selection::Automatic => Ok(entries
            .iter()
            .filter(|entry| entry.policy == Policy::Automatic)
            .collect()),
        Selection::All => Ok(entries.iter().collect()),
        Selection::Targets(targets) => select_targets(entries, targets),
    }
}

/// Matches explicit target arguments against the configured entries.
fn select_targets<'a>(
    entries: &'a [SyncEntry],
    targets: &[PathBuf],
) -> Result<Vec<&'a SyncEntry>, SyncsmithError> {
    let mut selected = Vec::new();
    let mut unknown: Vec<PathBuf> = Vec::new();
    for wanted in targets {
        let normalized = target::normalize(wanted);
        let already = selected
            .iter()
            .any(|entry: &&SyncEntry| target::normalize(&entry.target) == normalized);
        if already {
            continue;
        }
        let found = entries
            .iter()
            .find(|entry| target::normalize(&entry.target) == normalized);
        match found {
            Some(entry) => selected.push(entry),
            None => unknown.push(normalized),
        }
    }
    if unknown.is_empty() {
        return Ok(selected);
    }
    let configured: Vec<PathBuf> = entries
        .iter()
        .map(|entry| target::normalize(&entry.target))
        .collect();
    Err(SyncsmithError::TargetSelection {
        unknown: joined(&unknown),
        configured: joined(&configured),
    })
}

/// Comma-joins the paths for error messages.
fn joined(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::{Selection, select};
    use crate::config::{Mode, Policy, Source, SyncEntry};
    use crate::error::SyncsmithError;
    use std::path::PathBuf;

    fn entry(target: &str, policy: Policy) -> SyncEntry {
        SyncEntry {
            source: Source::Folder {
                folder: "rules".into(),
            },
            target: target.into(),
            mode: Mode::Overwrite,
            policy,
        }
    }

    fn automatic(target: &str) -> SyncEntry {
        entry(target, Policy::Automatic)
    }

    fn manual(target: &str) -> SyncEntry {
        entry(target, Policy::Manual)
    }

    #[test]
    fn automatic_selection_keeps_only_automatic_entries() {
        let entries = vec![automatic("a.yml"), manual("b.yml"), automatic("c.yml")];
        let selected = select(&entries, &Selection::Automatic).expect("selection succeeds");
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].target, PathBuf::from("a.yml"));
        assert_eq!(selected[1].target, PathBuf::from("c.yml"));
    }

    #[test]
    fn all_selection_keeps_every_entry_in_order() {
        let entries = vec![manual("b.yml"), automatic("a.yml")];
        let selected = select(&entries, &Selection::All).expect("selection succeeds");
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].target, PathBuf::from("b.yml"));
        assert_eq!(selected[1].target, PathBuf::from("a.yml"));
    }

    #[test]
    fn explicit_targets_match_after_normalization_any_policy() {
        let entries = vec![automatic("a.yml"), manual("dir/b.yml")];
        let selection = Selection::Targets(vec!["./a.yml".into(), "dir/./b.yml".into()]);
        let selected = select(&entries, &selection).expect("selection succeeds");
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].target, PathBuf::from("a.yml"));
        assert_eq!(selected[1].target, PathBuf::from("dir/b.yml"));
    }

    #[test]
    fn explicit_targets_skip_duplicates() {
        let entries = vec![automatic("a.yml")];
        let selection = Selection::Targets(vec!["a.yml".into(), "./a.yml".into()]);
        let selected = select(&entries, &selection).expect("selection succeeds");
        assert_eq!(selected.len(), 1);
    }

    #[test]
    fn unknown_target_lists_configured_targets() {
        let entries = vec![automatic("a.yml"), manual("dir/b.yml")];
        let selection = Selection::Targets(vec!["missing.yml".into()]);
        let Err(SyncsmithError::TargetSelection {
            unknown,
            configured,
        }) = select(&entries, &selection)
        else {
            panic!("an unknown target must be a usage error");
        };
        assert_eq!(unknown, "missing.yml");
        assert_eq!(configured, "a.yml, dir/b.yml");
    }
}
