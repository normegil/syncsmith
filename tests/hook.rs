//! Integration tests for pre-commit hook installation.
//!
//! `init::hook::offer_install` is crate-private, so the installation
//! scenarios drive `init::run_with` with a source-less scripted flow: the
//! user declines to add a source, the review is skipped, and the hook offer
//! is reached with the process cwd inside a fresh temporary repository.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use common::{Answer, ScriptedPrompter, enter, git};
use tempfile::TempDir;

use syncsmith::cli::GlobalArgs;
use syncsmith::commands::init::{InitTarget, run_hook_only, run_with};
use syncsmith::config::CONFIG_FILE_NAME;
use syncsmith::config::discovery::resolve_config_path;
use syncsmith::error::SyncsmithError;
use syncsmith::git::hooks_path;

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

/// Builds the init target of `repo`: the configuration location resolved
/// from an explicit `--config`, then loaded and validated when it exists.
fn init_target(repo: &Path) -> Result<InitTarget, SyncsmithError> {
    let resolution = resolve_config_path(&GlobalArgs {
        config: Some(repo.join(CONFIG_FILE_NAME)),
        verbose: 0,
        json: false,
    })
    .expect("the configuration location resolves");
    InitTarget::from_resolution(&resolution)
}

fn run_init(answers: Vec<Answer>, repo: &Path) {
    let prompter = ScriptedPrompter::new(answers);
    let target = init_target(repo).expect("the init target loads");
    run_with(&prompter, target, false).expect("init completes")
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
    run_init(
        vec![
            Answer::Confirm(false),
            Answer::Confirm(true),
            Answer::Confirm(true),
        ],
        repo.path(),
    );
    let hook = pre_commit(repo.path());
    assert!(hook.is_file());
    assert_eq!(
        fs::read_to_string(&hook).expect("installed hook is readable"),
        HOOK_SCRIPT
    );
    #[cfg(unix)]
    {
        let mode = fs::metadata(&hook)
            .expect("hook metadata is readable")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
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
    run_init(
        vec![
            Answer::Confirm(false),
            Answer::Confirm(true),
            Answer::Confirm(false),
        ],
        repo.path(),
    );
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
    run_init(
        vec![
            Answer::Confirm(false),
            Answer::Confirm(true),
            Answer::Confirm(true),
            Answer::Confirm(false),
        ],
        repo.path(),
    );
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
    run_init(
        vec![
            Answer::Confirm(false),
            Answer::Confirm(true),
            Answer::Confirm(true),
            Answer::Confirm(true),
        ],
        repo.path(),
    );
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
    run_init(
        vec![
            Answer::Confirm(false),
            Answer::Confirm(true),
            Answer::Confirm(true),
        ],
        repo.path(),
    );
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
    let target = init_target(repo.path()).expect("the init target loads");
    run_with(&prompter, target, false).expect("an interrupt at the hook offer only skips the hook");

    assert!(repo.path().join(CONFIG_FILE_NAME).is_file());
    assert!(!pre_commit(repo.path()).exists());
}

#[test]
fn init_hook_only_installs_the_hook_without_touching_the_configuration() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let _cwd = enter(repo.path());

    run_hook_only().expect("hook-only init completes");

    assert_eq!(
        fs::read_to_string(pre_commit(repo.path())).expect("installed hook is readable"),
        HOOK_SCRIPT
    );
    assert!(!repo.path().join(CONFIG_FILE_NAME).exists());
}

#[test]
fn init_hook_only_overwrites_an_existing_hook_without_prompting() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let hook = pre_commit(repo.path());
    fs::write(&hook, "#!/bin/sh\ncustom hook\n").expect("existing hook is written");
    let _cwd = enter(repo.path());

    run_hook_only().expect("hook-only init completes");

    assert_eq!(
        fs::read_to_string(&hook).expect("replaced hook is readable"),
        HOOK_SCRIPT
    );
}

#[test]
fn init_hook_only_honors_a_relative_core_hooks_path() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    git(repo.path(), &["config", "core.hooksPath", "custom-hooks"]);
    let _cwd = enter(repo.path());

    run_hook_only().expect("hook-only init completes");

    assert!(
        repo.path()
            .join("custom-hooks")
            .join("pre-commit")
            .is_file()
    );
}

#[test]
fn init_hook_only_outside_a_git_repository_fails() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let outside = TempDir::new().expect("temporary directory is created");
    let _cwd = enter(outside.path());

    let result = run_hook_only();

    assert!(matches!(
        result,
        Err(SyncsmithError::InvalidArguments { .. })
    ));
}

#[test]
fn a_failed_final_sync_skips_the_hook_offer_and_propagates() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    // An automatic entry whose source is missing: the configuration loads
    // and validates up front, but the final synchronization of the
    // automatic entries fails on the source load.
    let existing = r#"
[[sync]]
source = { folder = "missing-source.yml" }
target = "out.yml"
mode = "overwrite"
policy = "automatic"
"#;
    fs::write(repo.path().join(CONFIG_FILE_NAME), existing)
        .expect("existing configuration is written");
    let _cwd = enter(repo.path());

    // Add: keep the existing entry; Confirm(false): no new source;
    // Validate: end the review. The script deliberately holds no hook
    // answer: an erroneous hook offer would exhaust the script and panic
    // the test at the prompt.
    let prompter = ScriptedPrompter::new(vec![
        Answer::Select("Add".into()),
        Answer::Confirm(false),
        Answer::Select("Validate".into()),
    ]);
    let target = init_target(repo.path()).expect("the init target loads");

    let result = run_with(&prompter, target, false);

    // The per-entry failure propagates: the run fails non-zero, the hook
    // offer is skipped, and the configuration is already on disk.
    assert!(matches!(
        result,
        Err(SyncsmithError::SyncFailures { count: 1 })
    ));
    let written = fs::read_to_string(repo.path().join(CONFIG_FILE_NAME))
        .expect("written configuration is readable");
    assert!(written.contains("out.yml"));
    assert!(!pre_commit(repo.path()).exists());
}
