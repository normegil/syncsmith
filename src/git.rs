//! Git operations, shelled out to the `git` CLI.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::SyncsmithError;

/// Finds the root of the git repository containing `start`, if any.
///
/// Walks up one directory at a time looking for a `.git` entry, so a `.git`
/// file (linked worktree) is recognized as well.
pub fn repo_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(directory) = current {
        if directory.join(".git").exists() {
            return Some(directory.to_path_buf());
        }
        current = directory.parent();
    }
    None
}

/// Parameters of a shallow clone.
#[derive(Debug, Clone)]
pub struct CloneRequest {
    /// Repository URL to clone.
    pub url: String,
    /// Branch, tag, or commit to pin; `None` uses the default branch.
    pub git_ref: Option<String>,
    /// Overall deadline for all attempts.
    pub timeout: Duration,
    /// Maximum number of clone attempts.
    pub max_attempts: u32,
    /// Base delay between attempts; doubled after each failure.
    pub backoff: Duration,
}

impl CloneRequest {
    /// Builds a request for `url` with bounded retries and a 60s timeout.
    pub fn new(url: String, git_ref: Option<String>) -> Self {
        Self {
            url,
            git_ref,
            timeout: Duration::from_secs(60),
            max_attempts: 3,
            backoff: Duration::from_millis(500),
        }
    }
}

/// Shallow clones `request.url` (depth 1, pinned ref) into a fresh temporary
/// directory returned as a [`tempfile::TempDir`].
///
/// Failures are retried with exponential backoff under an overall deadline
/// (see `CloneRequest`); the returned error carries the last git stderr
/// excerpt.
///
/// GOTCHA: deterministic failures (bad URL, unknown ref, denied access) are
/// retried too, not just transient ones. Classifying git's stderr text as
/// transient versus deterministic is brittle and locale-dependent, so
/// bounded retries (~1.5 s worst case with the default request) were chosen
/// over stderr parsing.
pub fn shallow_clone(request: &CloneRequest) -> Result<tempfile::TempDir, SyncsmithError> {
    let deadline = Instant::now() + request.timeout;
    let mut backoff = request.backoff;
    let attempts = request.max_attempts.max(1);
    let mut last_reason = String::from("no clone attempt was made");
    for attempt in 0..attempts {
        match run_clone(request, deadline) {
            Ok(clone) => return Ok(clone),
            Err(reason) => last_reason = reason,
        }
        if attempt + 1 == attempts || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(backoff);
        backoff = backoff.saturating_mul(2);
    }
    Err(SyncsmithError::CloneFailed {
        url: request.url.clone(),
        git_ref: ref_label(&request.git_ref),
        reason: last_reason,
    })
}

/// Runs one clone attempt into a fresh temporary directory.
///
/// git's stderr is drained on a thread so a failure producing more output
/// than the OS pipe buffer still terminates and reports git's message instead
/// of blocking until the deadline.
fn run_clone(request: &CloneRequest, deadline: Instant) -> Result<tempfile::TempDir, String> {
    let destination = tempfile::tempdir()
        .map_err(|source| format!("cannot create a temporary directory: {source}"))?;
    let mut command = Command::new("git");
    command
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .arg(&request.url)
        .arg(destination.path());
    if let Some(git_ref) = &request.git_ref {
        command.arg("--branch").arg(git_ref);
    }
    let mut child = command
        .spawn()
        .map_err(|source| format!("cannot start git: {source}"))?;
    let stderr = child.stderr.take();
    let drain = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut stream) = stderr {
            let _ = stream.read_to_string(&mut text);
        }
        text
    });
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!("git clone of {} timed out", request.url));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(source) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("git clone failed: {source}"));
            }
        }
    };
    let stderr_text = drain.join().unwrap_or_default();
    match outcome {
        Ok(status) if status.success() => Ok(destination),
        Ok(status) => Err(clone_failure(status, last_line(&stderr_text))),
        Err(reason) => Err(reason),
    }
}

/// Last line of git stderr, or an empty string.
fn last_line(text: &str) -> String {
    text.lines().last().map(str::to_string).unwrap_or_default()
}

fn clone_failure(status: std::process::ExitStatus, excerpt: String) -> String {
    if excerpt.is_empty() {
        format!("git clone failed with status {status}")
    } else {
        excerpt
    }
}

fn ref_label(git_ref: &Option<String>) -> String {
    git_ref.clone().unwrap_or_else(|| "default branch".into())
}

/// Resolves the hooks directory of the repository at `root`.
///
/// Honors `core.hooksPath` (relative values resolve against the repository
/// root) and falls back to `.git/hooks`.
pub fn hooks_path(root: &Path) -> Result<PathBuf, SyncsmithError> {
    let args = "config --get core.hooksPath";
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .map_err(|source| SyncsmithError::GitCommand {
            args: args.into(),
            reason: source.to_string(),
        })?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !value.is_empty() {
            let path = PathBuf::from(value);
            return Ok(if path.is_absolute() {
                path
            } else {
                root.join(path)
            });
        }
    }
    Ok(root.join(".git").join("hooks"))
}
