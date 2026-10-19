//! Integration tests of the three-way patch merge (`merge_edited`).
//!
//! `merge_edited` shells out to `git merge-file` and writes temporary
//! files, so these tests live outside the `#[cfg(test)]` unit modules
//! (Rust-T-1): they need the external `git` binary and the filesystem. The
//! pure `decide()` cases stay unit-tested in `src/sync/merge.rs`.

use std::path::Path;

use syncsmith::commands::sync::merge::{MergeInput, MergeResult, merge_edited};
use syncsmith::error::SyncsmithError;

const TARGET: &str = "a.yml";

fn input<'a>(ours: &'a [u8], base: &'a [u8], theirs: &'a [u8]) -> MergeInput<'a> {
    MergeInput {
        target: Path::new(TARGET),
        ours,
        base,
        theirs,
    }
}

#[test]
fn merge_edited_merges_non_conflicting_changes() {
    // Both sides change the base in different, non-overlapping regions.
    let merged = merge_edited(&input(
        b"local\ntwo\nthree\n",
        b"one\ntwo\nthree\n",
        b"one\ntwo\nupstream\n",
    ))
    .expect("merge runs");
    match merged {
        MergeResult::Merged(content) => {
            assert_eq!(content, b"local\ntwo\nupstream\n".to_vec());
        }
        other => panic!("expected a clean merge, got {other:?}"),
    }
}

#[test]
fn merge_edited_reports_conflicts_with_markers() {
    // Only the shared conflict-marker boundaries are asserted: their exact
    // layout depends on the user's merge.conflictStyle.
    let merged =
        merge_edited(&input(b"conflicting\n", b"base\n", b"conflict\n")).expect("merge runs");
    match merged {
        MergeResult::Conflicted(content) => {
            let text = String::from_utf8(content).expect("merge result is UTF-8");
            assert!(text.contains("<<<<<<< ours") && text.contains(">>>>>>> theirs"));
        }
        other => panic!("expected a conflicted merge, got {other:?}"),
    }
}

#[test]
fn merge_edited_rejects_non_utf8_target_content() {
    let error = merge_edited(&input(&[0xff, 0xfe], b"base\n", b"latest\n"))
        .expect_err("binary content cannot be patched");
    assert!(matches!(error, SyncsmithError::UnsupportedContent { .. }));
}
