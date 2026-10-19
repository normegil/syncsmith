//! Pure evaluation of one target file against its source and lock record.
//!
//! [`evaluate`] implements the `check` specification rules without any
//! I/O: given the latest source content, its version, the current target
//! content, and the lock record, it reports whether the file is in sync.
//! [`SyncState::MergeCandidate`] is a *candidate*: it means "edited target
//! in `patch` mode with a pending source version", and carries the sides
//! the caller needs to decide it with a trial three-way merge (see
//! [`crate::commands::check`]).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::commands::sync::lock::LockRecord;
use crate::commands::sync::merge::{PatchSides, content_edited};
use crate::config::{Mode, Policy};

/// Why a target file is not in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    /// A patch is pending (the source moved since the last sync).
    Stale,
    /// Local edits that a sync would not preserve.
    Edited,
    /// The pending source patch conflicts with local edits (`patch`
    /// mode), decided by a trial three-way merge.
    Conflicted,
    /// The target file does not exist but the source has it.
    Missing,
    /// The target file exists but the source no longer has it.
    Extra,
}

/// Evaluation of one target file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState<'a> {
    /// The file is synchronized; a sync run would leave it unchanged.
    InSync,
    /// The file is not synchronized, with the reason why.
    OutOfSync(CheckStatus),
    /// Conflict candidate: an edited target in `patch` mode with a
    /// pending source version, carrying the sides the caller needs to
    /// decide it with a trial three-way merge.
    MergeCandidate(PatchSides<'a>),
}

/// State of one target file, input of [`evaluate`].
pub struct CheckFileState<'a> {
    /// Policy of the entry owning the file.
    pub policy: Policy,
    /// Synchronization mode of the entry owning the file.
    pub mode: Mode,
    /// Normalized target path (relative to the target repository root),
    /// carried for error context.
    pub target: &'a Path,
    /// Latest source content; `None` when the file was deleted on the
    /// source side (tree entries only).
    pub latest: Option<&'a [u8]>,
    /// Latest available source version: commit hash for git sources,
    /// content blob hash for folder sources; `None` when unavailable.
    pub latest_version: Option<&'a str>,
    /// Current target content; `None` when the target file does not exist.
    pub target_content: Option<&'a [u8]>,
    /// Lock record of the file, if it was synchronized before.
    pub record: Option<&'a LockRecord>,
}

/// Whether the target is still at its last synchronized version (pure).
///
/// - no lock record: in sync iff the target content is byte-identical to
///   the latest source; a missing target is `missing`, a differing one is
///   `stale`;
/// - automatic entries: in sync iff no patch is pending — a pending version
///   is `stale`, local edits are `edited` in `overwrite` mode and a merge
///   candidate in `patch` mode (when no patch is pending, `patch` mode
///   accepts local edits: a future sync only refreshes the lock record);
/// - manual entries: newer source versions are ignored — an unedited target
///   is in sync; local edits follow the mode rules: `edited` in `overwrite`
///   mode, a merge candidate in `patch` mode when a patch is pending,
///   in sync otherwise;
/// - source-side deletion: a missing target is in sync, an existing one is
///   `extra` — a sync would delete it.
pub fn evaluate<'a>(state: &CheckFileState<'a>) -> SyncState<'a> {
    let Some(latest) = state.latest else {
        return evaluate_deletion(state);
    };
    let Some(target) = state.target_content else {
        return SyncState::OutOfSync(CheckStatus::Missing);
    };
    let Some(record) = state.record else {
        return no_record_state(target, latest);
    };
    let flags = FileFlags {
        edited: content_edited(record, target),
        pending: version_pending(record, state.latest_version),
    };
    let verdict = match state.policy {
        Policy::Automatic => automatic_state(state.mode, flags),
        Policy::Manual => manual_state(state.mode, flags),
    };
    match verdict {
        Verdict::InSync => SyncState::InSync,
        Verdict::Stale => SyncState::OutOfSync(CheckStatus::Stale),
        Verdict::Edited => SyncState::OutOfSync(CheckStatus::Edited),
        Verdict::Candidate => SyncState::MergeCandidate(PatchSides {
            record,
            target_content: target,
            latest,
        }),
    }
}

/// Evaluation of a source-side deletion (tree entries only).
///
/// A target with local edits in `patch` mode would fail a sync run rather
/// than be deleted; it is still reported `extra` — not in sync either way.
fn evaluate_deletion<'a>(state: &CheckFileState<'a>) -> SyncState<'a> {
    if state.target_content.is_none() {
        SyncState::InSync
    } else {
        SyncState::OutOfSync(CheckStatus::Extra)
    }
}

/// Evaluation of a never-synchronized target (no lock record): in sync iff
/// its content is byte-identical to the latest source.
fn no_record_state<'a>(target: &'a [u8], latest: &'a [u8]) -> SyncState<'a> {
    if target == latest {
        SyncState::InSync
    } else {
        SyncState::OutOfSync(CheckStatus::Stale)
    }
}

/// Whether a patch is pending: the lock version is not the latest one.
fn version_pending(record: &LockRecord, latest_version: Option<&str>) -> bool {
    latest_version.is_some_and(|version| version != record.version)
}

/// Whether the target is edited and a patch is pending.
struct FileFlags {
    edited: bool,
    pending: bool,
}

/// Verdict of the policy rules for a previously synchronized file (pure);
/// [`Verdict::Candidate`] is refined by the caller with a trial merge.
enum Verdict {
    InSync,
    Stale,
    Edited,
    /// Edited target in `patch` mode with a pending source version.
    Candidate,
}

/// Automatic entries are in sync iff no patch needs to be applied.
fn automatic_state(mode: Mode, flags: FileFlags) -> Verdict {
    match (flags.pending, flags.edited, mode) {
        (false, true, Mode::Overwrite) => Verdict::Edited,
        (false, _, _) => Verdict::InSync,
        (true, true, Mode::Patch) => Verdict::Candidate,
        (true, _, _) => Verdict::Stale,
    }
}

/// Manual entries ignore newer source versions: they are in sync iff the
/// target is still at its last synchronized version, local edits
/// respecting the `patch` mode rules.
fn manual_state(mode: Mode, flags: FileFlags) -> Verdict {
    match (flags.edited, mode, flags.pending) {
        (false, _, _) => Verdict::InSync,
        (true, Mode::Overwrite, _) => Verdict::Edited,
        (true, Mode::Patch, true) => Verdict::Candidate,
        (true, Mode::Patch, false) => Verdict::InSync,
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckFileState, CheckStatus, SyncState, evaluate};
    use crate::commands::sync::lock::LockRecord;
    use crate::commands::sync::merge::PatchSides;
    use crate::commands::sync::source::blob_hash;
    use crate::config::{Mode, Policy};
    use std::path::Path;

    const TARGET: &str = "a.yml";
    const LOCKED: &[u8] = b"v1\n";
    const UPSTREAM: &[u8] = b"v2\n";

    /// One evaluation rule branch, with its pre-computed expectation.
    struct Case {
        name: &'static str,
        policy: Policy,
        mode: Mode,
        latest: Option<&'static [u8]>,
        latest_version: Option<&'static str>,
        target_content: Option<&'static [u8]>,
        /// Commit hash (or blob hash) recorded in the lock; `None` for no
        /// lock record.
        record_version: Option<&'static str>,
        expected: Expected,
    }

    /// The expected evaluation of a case; candidates carry the case inputs
    /// as their merge sides.
    #[derive(Debug, Clone, Copy)]
    enum Expected {
        InSync,
        OutOfSync(CheckStatus),
        MergeCandidate,
    }

    fn evaluated<'a>(case: &'a Case, record: Option<&'a LockRecord>) -> SyncState<'a> {
        let state = CheckFileState {
            policy: case.policy,
            mode: case.mode,
            target: Path::new(TARGET),
            latest: case.latest,
            latest_version: case.latest_version,
            target_content: case.target_content,
            record,
        };
        evaluate(&state)
    }

    /// Asserts the evaluation of `case` against its expectation; a
    /// candidate must carry the record and contents of the case.
    fn assert_case(case: &Case) {
        let record = case.record_version.map(|version| LockRecord {
            target: Path::new(TARGET).to_path_buf(),
            version: version.into(),
            content: blob_hash(LOCKED),
        });
        let name = case.name;
        match (evaluated(case, record.as_ref()), &case.expected) {
            (SyncState::InSync, Expected::InSync) => {}
            (SyncState::OutOfSync(status), Expected::OutOfSync(expected)) => {
                assert_eq!(status, *expected, "case: {name}");
            }
            (
                SyncState::MergeCandidate(PatchSides {
                    record: side_record,
                    target_content,
                    latest,
                }),
                Expected::MergeCandidate,
            ) => {
                assert_eq!(
                    Some(side_record.version.as_str()),
                    case.record_version,
                    "case: {name}"
                );
                assert_eq!(Some(target_content), case.target_content, "case: {name}");
                assert_eq!(Some(latest), case.latest, "case: {name}");
            }
            (actual, expected) => panic!("case {name}: expected {expected:?}, got {actual:?}"),
        }
    }

    #[test]
    fn evaluate_covers_every_rule_branch() {
        let stale = Expected::OutOfSync(CheckStatus::Stale);
        let cases = [
            // No lock record: in sync iff the target matches the source.
            Case {
                name: "no record, target identical to the source",
                policy: Policy::Automatic,
                mode: Mode::Overwrite,
                latest: Some(LOCKED),
                latest_version: Some("v1"),
                target_content: Some(LOCKED),
                record_version: None,
                expected: Expected::InSync,
            },
            Case {
                name: "no record, target differs from the source",
                policy: Policy::Automatic,
                mode: Mode::Overwrite,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: Some(LOCKED),
                record_version: None,
                expected: stale,
            },
            Case {
                name: "no record, target missing",
                policy: Policy::Automatic,
                mode: Mode::Patch,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: None,
                record_version: None,
                expected: Expected::OutOfSync(CheckStatus::Missing),
            },
            // Missing target with a lock record.
            Case {
                name: "record present, target missing",
                policy: Policy::Automatic,
                mode: Mode::Patch,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: None,
                record_version: Some("v1"),
                expected: Expected::OutOfSync(CheckStatus::Missing),
            },
            // Automatic entries, no patch pending.
            Case {
                name: "automatic aligned, unedited",
                policy: Policy::Automatic,
                mode: Mode::Patch,
                latest: Some(LOCKED),
                latest_version: Some("v1"),
                target_content: Some(LOCKED),
                record_version: Some("v1"),
                expected: Expected::InSync,
            },
            Case {
                name: "automatic aligned, edited, patch accepts the edits",
                policy: Policy::Automatic,
                mode: Mode::Patch,
                latest: Some(LOCKED),
                latest_version: Some("v1"),
                target_content: Some(b"local\n"),
                record_version: Some("v1"),
                expected: Expected::InSync,
            },
            Case {
                name: "automatic aligned, edited, overwrite discards them",
                policy: Policy::Automatic,
                mode: Mode::Overwrite,
                latest: Some(LOCKED),
                latest_version: Some("v1"),
                target_content: Some(b"local\n"),
                record_version: Some("v1"),
                expected: Expected::OutOfSync(CheckStatus::Edited),
            },
            // Automatic entries, patch pending.
            Case {
                name: "automatic pending, unedited, patch",
                policy: Policy::Automatic,
                mode: Mode::Patch,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: Some(LOCKED),
                record_version: Some("v1"),
                expected: stale,
            },
            Case {
                name: "automatic pending, unedited, overwrite",
                policy: Policy::Automatic,
                mode: Mode::Overwrite,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: Some(LOCKED),
                record_version: Some("v1"),
                expected: stale,
            },
            Case {
                name: "automatic pending, edited, overwrite",
                policy: Policy::Automatic,
                mode: Mode::Overwrite,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: Some(b"local\n"),
                record_version: Some("v1"),
                expected: stale,
            },
            Case {
                name: "automatic pending, edited, patch: conflict candidate",
                policy: Policy::Automatic,
                mode: Mode::Patch,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: Some(b"local\n"),
                record_version: Some("v1"),
                expected: Expected::MergeCandidate,
            },
            // Manual entries: newer source versions are ignored.
            Case {
                name: "manual pending, unedited",
                policy: Policy::Manual,
                mode: Mode::Patch,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: Some(LOCKED),
                record_version: Some("v1"),
                expected: Expected::InSync,
            },
            Case {
                name: "manual aligned, edited, overwrite",
                policy: Policy::Manual,
                mode: Mode::Overwrite,
                latest: Some(LOCKED),
                latest_version: Some("v1"),
                target_content: Some(b"local\n"),
                record_version: Some("v1"),
                expected: Expected::OutOfSync(CheckStatus::Edited),
            },
            Case {
                name: "manual pending, edited, overwrite",
                policy: Policy::Manual,
                mode: Mode::Overwrite,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: Some(b"local\n"),
                record_version: Some("v1"),
                expected: Expected::OutOfSync(CheckStatus::Edited),
            },
            Case {
                name: "manual aligned, edited, patch",
                policy: Policy::Manual,
                mode: Mode::Patch,
                latest: Some(LOCKED),
                latest_version: Some("v1"),
                target_content: Some(b"local\n"),
                record_version: Some("v1"),
                expected: Expected::InSync,
            },
            Case {
                name: "manual pending, edited, patch: conflict candidate",
                policy: Policy::Manual,
                mode: Mode::Patch,
                latest: Some(UPSTREAM),
                latest_version: Some("v2"),
                target_content: Some(b"local\n"),
                record_version: Some("v1"),
                expected: Expected::MergeCandidate,
            },
            // Source-side deletions.
            Case {
                name: "source deleted, target present",
                policy: Policy::Automatic,
                mode: Mode::Patch,
                latest: None,
                latest_version: Some("v2"),
                target_content: Some(LOCKED),
                record_version: Some("v1"),
                expected: Expected::OutOfSync(CheckStatus::Extra),
            },
            Case {
                name: "source deleted, target absent",
                policy: Policy::Automatic,
                mode: Mode::Patch,
                latest: None,
                latest_version: Some("v2"),
                target_content: None,
                record_version: Some("v1"),
                expected: Expected::InSync,
            },
        ];
        for case in cases {
            assert_case(&case);
        }
    }
}
