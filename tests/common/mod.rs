//! Shared helpers for the integration tests: a scripted prompter replaying
//! canned answers, local git fixtures, and a working-directory guard for the
//! tests that exercise cwd-based discovery.

// Each test binary includes this module but only uses a subset of it.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;

use syncsmith::SyncsmithError;
use syncsmith::commands::init::{Choice, Prompter};

/// One scripted answer of a [`ScriptedPrompter`].
#[derive(Debug, Clone)]
pub enum Answer {
    /// Reply to a yes/no prompt.
    Confirm(bool),
    /// Reply to a single-choice prompt.
    Select(String),
    /// Reply to a free-form text prompt.
    Text(String),
    /// Reply to a text prompt with a suggested default.
    TextDefault(String),
    /// Reply to a multi-select prompt.
    Multi(Vec<String>),
    /// Simulate a user interruption (Ctrl-C, Esc, or EOF).
    Interrupt,
}

/// Prompter replaying a scripted answer queue in order.
///
/// A mismatched or missing answer panics, so scripting errors surface as
/// test failures instead of silently misdriving the flow. The `(message,
/// default)` pairs of `text_with_default` prompts are recorded so tests can
/// assert the values the flow prefills.
pub struct ScriptedPrompter {
    answers: RefCell<VecDeque<Answer>>,
    text_defaults: RefCell<Vec<(String, String)>>,
}

impl ScriptedPrompter {
    /// Builds a prompter that replays `answers` in order.
    pub fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: RefCell::new(answers.into()),
            text_defaults: RefCell::new(Vec::new()),
        }
    }

    /// The (message, default) pairs of every `text_with_default` prompt, in
    /// prompt order.
    pub fn text_defaults(&self) -> Vec<(String, String)> {
        self.text_defaults.borrow().clone()
    }

    fn next(&self, prompt: &str) -> Answer {
        self.answers
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("script exhausted at a {prompt} prompt"))
    }
}

impl Prompter for ScriptedPrompter {
    fn confirm(&self, _message: &str, _default: Option<bool>) -> Result<bool, SyncsmithError> {
        match self.next("confirm") {
            Answer::Confirm(value) => Ok(value),
            Answer::Interrupt => Err(SyncsmithError::Aborted),
            other => panic!("confirm prompt got an unexpected answer: {other:?}"),
        }
    }

    fn select(
        &self,
        _message: &str,
        _options: &[Choice],
        _default: Option<&str>,
    ) -> Result<String, SyncsmithError> {
        match self.next("select") {
            Answer::Select(value) => Ok(value),
            Answer::Interrupt => Err(SyncsmithError::Aborted),
            other => panic!("select prompt got an unexpected answer: {other:?}"),
        }
    }

    fn text(&self, _message: &str) -> Result<String, SyncsmithError> {
        match self.next("text") {
            Answer::Text(value) => Ok(value),
            Answer::Interrupt => Err(SyncsmithError::Aborted),
            other => panic!("text prompt got an unexpected answer: {other:?}"),
        }
    }

    fn text_with_default(&self, message: &str, default: &str) -> Result<String, SyncsmithError> {
        self.text_defaults
            .borrow_mut()
            .push((message.to_string(), default.to_string()));
        match self.next("text_with_default") {
            Answer::TextDefault(value) => Ok(value),
            Answer::Interrupt => Err(SyncsmithError::Aborted),
            other => panic!("text_with_default prompt got an unexpected answer: {other:?}"),
        }
    }

    fn multi_select(
        &self,
        _message: &str,
        _options: &[String],
    ) -> Result<Vec<String>, SyncsmithError> {
        match self.next("multi_select") {
            Answer::Multi(values) => Ok(values),
            Answer::Interrupt => Err(SyncsmithError::Aborted),
            other => panic!("multi_select prompt got an unexpected answer: {other:?}"),
        }
    }
}

/// Runs `git args` in `dir`, panicking with git's stderr on failure.
///
/// Committed fixtures carry a fixed identity through the environment so the
/// tests never touch the developer's git configuration.
pub fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "syncsmith tests")
        .env("GIT_AUTHOR_EMAIL", "syncsmith-tests@example.invalid")
        .env("GIT_COMMITTER_NAME", "syncsmith tests")
        .env("GIT_COMMITTER_EMAIL", "syncsmith-tests@example.invalid")
        .output()
        .expect("git is installed and runnable");
    assert!(
        output.status.success(),
        "git {:?} failed in {}: {}",
        args,
        dir.display(),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Restores the previous working directory when dropped.
pub struct CwdGuard(PathBuf);

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

/// Changes the process working directory to `dir` until the returned guard
/// is dropped.
pub fn enter(dir: &Path) -> CwdGuard {
    let previous = std::env::current_dir().expect("current directory is readable");
    std::env::set_current_dir(dir).expect("working directory change succeeds");
    CwdGuard(previous)
}
