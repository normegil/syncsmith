//! Integration tests for pre-commit hook installation.
//!
//! `init::hook::offer_install` is crate-private, so the installation
//! scenarios drive `init::run_with` with a source-less scripted flow: the
//! user declines to add a source, the review is skipped, and the hook offer
//! is reached with the process cwd inside a fresh temporary repository.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use common::{Answer, ScriptedPrompter, enter, git};
use tempfile::TempDir;

use syncsmith::cli::GlobalArgs;
use syncsmith::config::CONFIG_FILE_NAME;
use syncsmith::git::hooks_path;
use syncsmith::init::{ExitState, run_with};

/// Serializes the tests that change the process working directory.
static CWD_LOCK: Mutex<()> = Mutex::new(());

const HOOK_SCRIPT: &str = "#!/bin/sh\n\
# Installed by syncsmith: run syncsmith sync before each commit.\n\
changed=$(syncsmith sync) || exit 1\n\
if [ -n \"$changed\" ]; then\n\
  printf '%s\\n' \"$changed\" | git add --pathspec-from-file=-\n\
fi\n";

fn target_repository() -> TempDir {
    let repo = TempDir::new().expect("temporary repository is created");
    git(repo.path(), &["init"]);
    repo
}

fn pre_commit(repo: &Path) -> PathBuf {
    repo.join(".git").join("hooks").join("pre-commit")
}

fn run_init(answers: Vec<Answer>) -> ExitState {
    let prompter = ScriptedPrompter::new(answers);
    run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    )
    .expect("init completes")
}

#[test]
fn hooks_path_defaults_to_the_git_hooks_directory() {
    let repo = target_repository();
    // A local empty value shadows any user-level core.hooksPath and makes
    // hooks_path fall back to the default.
    git(repo.path(), &["config", "core.hooksPath", ""]);

    let resolved = hooks_path(repo.path()).expect("hooks path resolves");

    assert_eq!(resolved, repo.path().join(".git").join("hooks"));
}

#[test]
fn hooks_path_resolves_a_relative_core_hooks_path_against_the_repo_root() {
    let repo = target_repository();
    git(repo.path(), &["config", "core.hooksPath", "my-hooks"]);

    let resolved = hooks_path(repo.path()).expect("hooks path resolves");

    assert_eq!(resolved, repo.path().join("my-hooks"));
}

#[test]
fn hooks_path_returns_an_absolute_core_hooks_path_unchanged() {
    let repo = target_repository();
    let external = TempDir::new().expect("temporary hooks directory is created");
    let absolute = external.path().join("hooks");
    git(
        repo.path(),
        &["config", "core.hooksPath", &absolute.to_string_lossy()],
    );

    let resolved = hooks_path(repo.path()).expect("hooks path resolves");

    assert_eq!(resolved, absolute);
}

#[test]
fn accepted_hook_offer_installs_an_executable_pre_commit_hook() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let _cwd = enter(repo.path());

    // Confirm(false): no source; Confirm(true): write the empty config;
    // Confirm(true): install the hook.
    let state = run_init(vec![
        Answer::Confirm(false),
        Answer::Confirm(true),
        Answer::Confirm(true),
    ]);

    assert_eq!(state, ExitState::Completed);
    let hook = pre_commit(repo.path());
    assert!(hook.is_file());
    assert_eq!(
        fs::read_to_string(&hook).expect("installed hook is readable"),
        HOOK_SCRIPT
    );
    let mode = fs::metadata(&hook)
        .expect("hook metadata is readable")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[test]
fn declined_hook_offer_leaves_no_pre_commit_hook() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let _cwd = enter(repo.path());

    // Confirm(false): no source; Confirm(true): write the empty config;
    // Confirm(false): decline the hook.
    let state = run_init(vec![
        Answer::Confirm(false),
        Answer::Confirm(true),
        Answer::Confirm(false),
    ]);

    assert_eq!(state, ExitState::Completed);
    assert!(!pre_commit(repo.path()).exists());
}

#[test]
fn existing_hook_is_kept_when_overwrite_is_declined() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let hook = pre_commit(repo.path());
    fs::write(&hook, "#!/bin/sh\ncustom hook\n").expect("existing hook is written");
    let _cwd = enter(repo.path());

    // Confirm(false): no source; Confirm(true): write the empty config;
    // Confirm(true): install; Confirm(false): keep the existing hook.
    let state = run_init(vec![
        Answer::Confirm(false),
        Answer::Confirm(true),
        Answer::Confirm(true),
        Answer::Confirm(false),
    ]);

    assert_eq!(state, ExitState::Completed);
    assert_eq!(
        fs::read_to_string(&hook).expect("existing hook is readable"),
        "#!/bin/sh\ncustom hook\n"
    );
}

#[test]
fn existing_hook_is_replaced_when_overwrite_is_confirmed() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let hook = pre_commit(repo.path());
    fs::write(&hook, "#!/bin/sh\ncustom hook\n").expect("existing hook is written");
    let _cwd = enter(repo.path());

    // Confirm(false): no source; Confirm(true): write the empty config;
    // Confirm(true): install; Confirm(true): overwrite the existing hook.
    let state = run_init(vec![
        Answer::Confirm(false),
        Answer::Confirm(true),
        Answer::Confirm(true),
        Answer::Confirm(true),
    ]);

    assert_eq!(state, ExitState::Completed);
    assert_eq!(
        fs::read_to_string(&hook).expect("replaced hook is readable"),
        HOOK_SCRIPT
    );
}

#[test]
fn hook_installation_honors_a_relative_core_hooks_path() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    git(repo.path(), &["config", "core.hooksPath", "custom-hooks"]);
    let _cwd = enter(repo.path());

    // Confirm(false): no source; Confirm(true): write the empty config;
    // Confirm(true): install the hook.
    let state = run_init(vec![
        Answer::Confirm(false),
        Answer::Confirm(true),
        Answer::Confirm(true),
    ]);

    assert_eq!(state, ExitState::Completed);
    let hook = repo.path().join("custom-hooks").join("pre-commit");
    assert!(hook.is_file());
    assert_eq!(
        fs::read_to_string(&hook).expect("installed hook is readable"),
        HOOK_SCRIPT
    );
}

#[test]
fn interrupt_at_the_hook_prompt_completes_with_the_config_on_disk() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let _cwd = enter(repo.path());

    // Confirm(false): no source; Confirm(true): write the empty config;
    // Interrupt: skip the hook offer. The configuration is already saved at
    // that point, so the interrupt must not abort the run.
    let prompter = ScriptedPrompter::new(vec![
        Answer::Confirm(false),
        Answer::Confirm(true),
        Answer::Interrupt,
    ]);
    let state = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    )
    .expect("an interrupt at the hook offer only skips the hook");

    assert_eq!(state, ExitState::Completed);
    assert!(repo.path().join(CONFIG_FILE_NAME).is_file());
    assert!(!pre_commit(repo.path()).exists());
}
