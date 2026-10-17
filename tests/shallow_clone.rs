//! Integration tests for shallow cloning of git sources, using local
//! `file://` fixtures only (no network access).

mod common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use common::git;
use tempfile::TempDir;

use syncsmith::SyncsmithError;
use syncsmith::git::{CloneRequest, shallow_clone};

/// Builds a local origin with one commit on its default branch and a distinct
/// commit on a `stable` branch, then serves a bare clone over `file://`.
fn bare_repository() -> (TempDir, TempDir) {
    let origin = TempDir::new().expect("temporary origin is created");
    git(origin.path(), &["init"]);
    fs::write(origin.path().join("default.txt"), "default branch content")
        .expect("default branch file is written");
    git(origin.path(), &["add", "."]);
    git(origin.path(), &["commit", "-m", "initial commit"]);
    git(origin.path(), &["checkout", "-b", "stable"]);
    git(origin.path(), &["rm", "-q", "default.txt"]);
    fs::write(origin.path().join("stable.txt"), "stable branch content")
        .expect("stable branch file is written");
    git(origin.path(), &["add", "."]);
    git(origin.path(), &["commit", "-m", "stable commit"]);
    git(origin.path(), &["checkout", "-"]);

    let bare = TempDir::new().expect("temporary bare repository is created");
    git(
        origin.path(),
        &[
            "clone",
            "--bare",
            origin.path().to_str().expect("origin path is UTF-8"),
            bare.path().to_str().expect("bare path is UTF-8"),
        ],
    );
    (origin, bare)
}

fn file_url(bare: &Path) -> String {
    format!("file://{}", bare.display())
}

#[test]
fn shallow_clone_of_a_local_bare_repository_checks_out_the_default_branch() {
    let (_origin, bare) = bare_repository();

    let request = CloneRequest::new(file_url(bare.path()), None);
    let clone = shallow_clone(&request).expect("local shallow clone succeeds");

    assert!(clone.path().join("default.txt").is_file());
    assert!(clone.path().join(".git").join("shallow").is_file());
}

#[test]
fn shallow_clone_pins_the_requested_branch() {
    let (_origin, bare) = bare_repository();

    let request = CloneRequest::new(file_url(bare.path()), Some("stable".into()));
    let clone = shallow_clone(&request).expect("pinned shallow clone succeeds");

    assert!(clone.path().join("stable.txt").is_file());
    assert!(!clone.path().join("default.txt").exists());
}

#[test]
fn shallow_clone_of_an_unknown_ref_fails_with_clone_failed() {
    let (_origin, bare) = bare_repository();

    let request = CloneRequest::new(file_url(bare.path()), Some("does-not-exist".into()));
    let error = shallow_clone(&request).expect_err("clone of an unknown ref fails");

    let SyncsmithError::CloneFailed { url, git_ref, .. } = error else {
        panic!("expected CloneFailed")
    };
    assert_eq!(url, file_url(bare.path()));
    assert_eq!(git_ref, "does-not-exist");
}

#[test]
fn shallow_clone_of_a_missing_repository_carries_the_git_stderr_excerpt() {
    let request = CloneRequest {
        url: "file:///nonexistent/syncsmith-fixture.git".into(),
        git_ref: None,
        timeout: Duration::from_secs(60),
        max_attempts: 2,
        backoff: Duration::from_millis(1),
    };

    let error = shallow_clone(&request).expect_err("clone of a missing repository fails");

    let SyncsmithError::CloneFailed { reason, .. } = error else {
        panic!("expected CloneFailed")
    };
    assert!(
        !reason.is_empty(),
        "failure reason should quote git's output"
    );
}

#[test]
fn shallow_clone_reports_a_timeout_after_the_deadline() {
    let (_origin, bare) = bare_repository();
    let request = CloneRequest {
        url: file_url(bare.path()),
        git_ref: None,
        timeout: Duration::from_millis(0),
        max_attempts: 2,
        backoff: Duration::from_millis(1),
    };

    let error = shallow_clone(&request).expect_err("clone beyond the deadline fails");

    let SyncsmithError::CloneFailed { reason, .. } = error else {
        panic!("expected CloneFailed")
    };
    assert!(reason.contains("timed out"), "unexpected reason: {reason}");
}

/// Removes `GIT_TRACE` when dropped, keeping the trace setting local to one
/// test even when an assertion fails.
struct TraceGuard;

impl Drop for TraceGuard {
    fn drop(&mut self) {
        unsafe { std::env::remove_var("GIT_TRACE") };
    }
}

#[test]
fn clone_failure_with_a_large_stderr_reports_gits_message_instead_of_a_timeout() {
    let _trace = TraceGuard;
    // GIT_TRACE echoes the full command lines to stderr, so a URL far beyond
    // the OS pipe buffer (> 64 KiB) makes git write a large stderr stream
    // before failing. stderr must be drained, and the failure must report
    // git's message rather than blocking until the deadline.
    let url = format!("file:///nonexistent/{}.git", "x".repeat(70_000));
    let request = CloneRequest {
        url,
        git_ref: None,
        timeout: Duration::from_secs(5),
        max_attempts: 1,
        backoff: Duration::from_millis(1),
    };

    unsafe { std::env::set_var("GIT_TRACE", "1") };
    let error = shallow_clone(&request).expect_err("clone of a huge URL fails");

    let SyncsmithError::CloneFailed { reason, .. } = error else {
        panic!("expected CloneFailed")
    };
    assert!(
        !reason.is_empty(),
        "the failure should quote git's own message"
    );
    assert!(!reason.contains("timed out"), "unexpected reason: {reason}");
}
