//! Integration tests for the `check` command (plan section 3.2).
//!
//! Fixtures are local git repositories and plain folders only — no
//! network. Each test builds a target repository (fresh `git init`) plus
//! one or two sources, prepares the synchronization state with
//! `sync::run`, then runs `check::run` with the context and selection
//! `main.rs` would build, and asserts the not-in-sync payload, the
//! statuses, and that nothing was written. The payload is rendered the
//! way `main.rs` writes it, through the shared report helpers.
//!
//! The last section spawns the built binary instead, closing the
//! exit-code coverage gap: `--no-fail`, `--json`, usage errors, and
//! evaluation failures map to the process exit code in `main.rs`.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use syncsmith::cli::GlobalArgs;
use syncsmith::commands;
use syncsmith::commands::check::CheckInvocation;
use syncsmith::commands::check::{self, CheckOutcome, CheckStatus, OutOfSyncFile};
use syncsmith::commands::sync::report::{render_json, render_lines};
use syncsmith::commands::sync::source::blob_hash;
use syncsmith::commands::sync::{
    self, LOCK_FILE_NAME, LockFile, LockRecord, Selection, SyncContext,
};
use syncsmith::config::discovery::resolve_config_path;
use syncsmith::config::{CONFIG_FILE_NAME, Config, Mode, Policy, Source, SyncEntry};
use syncsmith::error::SyncsmithError;

/// Runs `git -C dir args` and returns its trimmed stdout, panicking with
/// git's stderr on failure.
///
/// The fixed identity and the config overrides keep the tests away from the
/// developer's git configuration (same approach as `tests/common/mod.rs`,
/// hardened against `commit.gpgsign` and friends).
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "syncsmith tests")
        .env("GIT_AUTHOR_EMAIL", "syncsmith-tests@example.invalid")
        .env("GIT_COMMITTER_NAME", "syncsmith tests")
        .env("GIT_COMMITTER_EMAIL", "syncsmith-tests@example.invalid")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git is installed and runnable");
    assert!(
        output.status.success(),
        "git {:?} failed in {}: {}",
        args,
        dir.display(),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn commit(dir: &Path, message: &str) {
    git(dir, &["add", "."]);
    git(dir, &["commit", "-m", message]);
}

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

fn write_file(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("the directories of a fixture file are created");
    }
    fs::write(path, content).expect("the fixture file is written");
}

fn git_repository() -> TempDir {
    let repo = TempDir::new().expect("temporary repository is created");
    git(repo.path(), &["init"]);
    repo
}

/// A source git repository whose first commit contains `files`; returns the
/// repository and the hash of that commit.
fn source_repository(files: &[(&str, &str)]) -> (TempDir, String) {
    let repo = git_repository();
    for (relative, content) in files {
        write_file(repo.path(), relative, content);
    }
    commit(repo.path(), "fixture commit");
    let version = head(repo.path());
    (repo, version)
}

fn repo_entry(
    repo: &TempDir,
    source_path: &str,
    target: &str,
    mode: Mode,
    policy: Policy,
) -> SyncEntry {
    SyncEntry {
        source: Source::Repository {
            repository: repo.path().to_string_lossy().into_owned(),
            git_ref: None,
            path: source_path.into(),
        },
        target: target.into(),
        mode,
        policy,
    }
}

/// A single-file folder source entry (folder sources require `overwrite`
/// mode: validation rejects the `patch` combination).
fn folder_file_entry(source: &TempDir, file: &str, target: &str, policy: Policy) -> SyncEntry {
    SyncEntry {
        source: Source::Folder {
            folder: source.path().join(file),
        },
        target: target.into(),
        mode: Mode::Overwrite,
        policy,
    }
}

/// A whole-folder source entry; the folder tree is mirrored at the target.
fn folder_tree_entry(source: &TempDir, target: &str) -> SyncEntry {
    SyncEntry {
        source: Source::Folder {
            folder: source.path().to_path_buf(),
        },
        target: target.into(),
        mode: Mode::Overwrite,
        policy: Policy::Automatic,
    }
}

fn context(target_repo: &TempDir, entries: Vec<SyncEntry>) -> SyncContext {
    SyncContext {
        config: Config { sync: entries },
        config_dir: target_repo.path().to_path_buf(),
        lock_path: target_repo.path().join(LOCK_FILE_NAME),
        target_root: target_repo.path().to_path_buf(),
    }
}

/// The selection `main.rs` builds from the CLI arguments.
fn selection(all: bool, targets: &[&str]) -> Selection {
    Selection::from_args(all, targets.iter().copied().map(PathBuf::from).collect())
}

/// Synchronizes the fixture (test preparation, not the code under test).
fn run_sync(context: &SyncContext, all: bool, targets: &[&str]) -> sync::SyncOutcome {
    sync::run(context, &selection(all, targets)).expect("the fixture synchronization succeeds")
}

/// Runs the check command the way `main.rs` dispatches it.
fn run_check(
    context: &SyncContext,
    all: bool,
    targets: &[&str],
) -> Result<CheckOutcome, SyncsmithError> {
    check::run(context, &selection(all, targets))
}

/// Renders the stdout payload the way `main.rs` writes it: one path per
/// line by default, one `{path, status}` object per file with `--json`.
fn stdout(json: bool, outcome: &CheckOutcome) -> String {
    if json {
        render_json(&outcome.out_of_sync)
    } else {
        render_lines(outcome.out_of_sync.iter().map(|file| file.path.as_path()))
    }
}

fn reported(path: &str, status: CheckStatus) -> OutOfSyncFile {
    OutOfSyncFile {
        path: PathBuf::from(path),
        status,
    }
}

fn lock_path(target_repo: &TempDir) -> PathBuf {
    target_repo.path().join(LOCK_FILE_NAME)
}

fn lock_of(target_repo: &TempDir) -> LockFile {
    LockFile::load(&lock_path(target_repo)).expect("the lock file loads")
}

fn read_target(target_repo: &TempDir, relative: &str) -> String {
    fs::read_to_string(target_repo.path().join(relative)).expect("the target file is readable")
}

fn lock_record(target: &str, version: String, content: String) -> LockRecord {
    LockRecord {
        target: PathBuf::from(target),
        version,
        content,
    }
}

/// Reads every file of the target repository (skipping `.git`), relative
/// path to content — the before/after state of the read-only guarantee.
fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), PathBuf::new())]);
    while let Some((dir, prefix)) = queue.pop_front() {
        for entry in fs::read_dir(&dir).expect("the fixture directory is readable") {
            let entry = entry.expect("the fixture directory entry is readable");
            if entry.file_name() == ".git" {
                continue;
            }
            let relative = prefix.join(entry.file_name());
            if entry
                .file_type()
                .expect("the fixture file type is readable")
                .is_dir()
            {
                queue.push_back((entry.path(), relative));
            } else {
                let content = fs::read(entry.path()).expect("the fixture file is readable");
                files.insert(relative, content);
            }
        }
    }
    files
}

#[test]
fn a_never_synchronized_automatic_entry_is_reported_missing() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repo_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Patch,
            Policy::Automatic,
        )],
    );

    let outcome = run_check(&context, false, &[]).expect("the check succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![reported("app.yml", CheckStatus::Missing)]
    );
    assert_eq!(stdout(false, &outcome), "app.yml\n");
    assert_eq!(
        stdout(true, &outcome),
        r#"[{"path":"app.yml","status":"missing"}]"#
    );
    // Read-only: checking a never-synchronized entry creates no lock file.
    assert!(!lock_path(&target).exists());
}

#[test]
fn a_fully_synchronized_repository_reports_nothing() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repo_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Patch,
            Policy::Automatic,
        )],
    );
    run_sync(&context, false, &[]);

    let outcome = run_check(&context, false, &[]).expect("the check succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(stdout(false, &outcome), "");
    assert_eq!(stdout(true, &outcome), "[]");
}

#[test]
fn an_automatic_patch_entry_with_a_moved_source_is_stale() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repo_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Patch,
            Policy::Automatic,
        )],
    );
    run_sync(&context, false, &[]);
    write_file(source.path(), "app.yml", "v2\n");
    commit(source.path(), "change the source");

    let outcome = run_check(&context, false, &[]).expect("the check succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![reported("app.yml", CheckStatus::Stale)]
    );
}

#[test]
fn an_automatic_patch_entry_with_edits_and_a_clean_pending_patch_is_stale() {
    let (source, _) = source_repository(&[("app.yml", "alpha\nbeta\ngamma\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repo_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Patch,
            Policy::Automatic,
        )],
    );
    run_sync(&context, false, &[]);
    // Local edit of the first line while the source changes the last one:
    // the trial merge is clean, so the file is stale (a patch is pending).
    fs::write(target.path().join("app.yml"), "local\nbeta\ngamma\n")
        .expect("the local edit is written");
    write_file(source.path(), "app.yml", "alpha\nbeta\nupstream\n");
    commit(source.path(), "change the last line");

    let outcome = run_check(&context, false, &[]).expect("the check succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![reported("app.yml", CheckStatus::Stale)]
    );
    // The trial merge is discarded: the target keeps the local edit.
    assert_eq!(read_target(&target, "app.yml"), "local\nbeta\ngamma\n");
}

#[test]
fn a_moved_source_is_ignored_by_default_and_in_sync_under_all_for_manual_entries() {
    let (auto_source, _) = source_repository(&[("app.yml", "v1\n")]);
    let (manual_source, first_version) = source_repository(&[("manual.txt", "m1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![
            repo_entry(
                &auto_source,
                "app.yml",
                "app.yml",
                Mode::Patch,
                Policy::Automatic,
            ),
            repo_entry(
                &manual_source,
                "manual.txt",
                "manual.txt",
                Mode::Patch,
                Policy::Manual,
            ),
        ],
    );
    run_sync(&context, true, &[]);
    write_file(auto_source.path(), "app.yml", "v2\n");
    commit(auto_source.path(), "change the automatic source");
    write_file(manual_source.path(), "manual.txt", "m2\n");
    commit(manual_source.path(), "change the manual source");

    // Default run: only the automatic entry is checked.
    let outcome = run_check(&context, false, &[]).expect("the check succeeds");
    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![reported("app.yml", CheckStatus::Stale)]
    );

    // `--all` also checks the manual entry: it is still at its lock
    // version, so in sync even though the source moved.
    let outcome = run_check(&context, true, &[]).expect("the check succeeds");
    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![reported("app.yml", CheckStatus::Stale)]
    );
    assert_eq!(read_target(&target, "manual.txt"), "m1\n");
    assert_eq!(
        lock_of(&target).record(Path::new("manual.txt")),
        Some(&lock_record(
            "manual.txt",
            first_version,
            blob_hash(b"m1\n")
        ))
    );
}

#[test]
fn a_manual_patch_entry_with_edits_and_a_clean_pending_patch_is_in_sync() {
    let (source, _) = source_repository(&[("manual.txt", "alpha\nbeta\ngamma\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repo_entry(
            &source,
            "manual.txt",
            "manual.txt",
            Mode::Patch,
            Policy::Manual,
        )],
    );
    run_sync(&context, true, &[]);
    // The local edit of the first line respects the patch rules: the
    // pending source change (last line) merges cleanly.
    fs::write(target.path().join("manual.txt"), "local\nbeta\ngamma\n")
        .expect("the local edit is written");
    write_file(source.path(), "manual.txt", "alpha\nbeta\nupstream\n");
    commit(source.path(), "change the last line");

    let outcome = run_check(&context, true, &[]).expect("the check succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(outcome.out_of_sync, Vec::new());
    // The trial merge is discarded: the local edit is untouched.
    assert_eq!(read_target(&target, "manual.txt"), "local\nbeta\ngamma\n");
}

#[test]
fn a_manual_patch_entry_with_edits_and_a_conflicting_pending_patch_is_conflicted() {
    let (source, _) = source_repository(&[("manual.txt", "alpha\nbeta\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repo_entry(
            &source,
            "manual.txt",
            "manual.txt",
            Mode::Patch,
            Policy::Manual,
        )],
    );
    run_sync(&context, true, &[]);
    fs::write(target.path().join("manual.txt"), "local\nbeta\n")
        .expect("the local edit is written");
    write_file(source.path(), "manual.txt", "upstream\nbeta\n");
    commit(source.path(), "change the first line");

    let outcome = run_check(&context, true, &[]).expect("the check succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![reported("manual.txt", CheckStatus::Conflicted)]
    );
    assert_eq!(
        stdout(true, &outcome),
        r#"[{"path":"manual.txt","status":"conflicted"}]"#
    );
    // Conflicts are only reported: the target keeps the local edit.
    assert_eq!(read_target(&target, "manual.txt"), "local\nbeta\n");
}

#[test]
fn local_edits_on_overwrite_entries_are_reported_edited() {
    let automatic = TempDir::new().expect("temporary folder source is created");
    write_file(automatic.path(), "auto.txt", "a1\n");
    let manual = TempDir::new().expect("temporary folder source is created");
    write_file(manual.path(), "manual.txt", "m1\n");
    let target = git_repository();
    let context = context(
        &target,
        vec![
            folder_file_entry(&automatic, "auto.txt", "auto.txt", Policy::Automatic),
            folder_file_entry(&manual, "manual.txt", "manual.txt", Policy::Manual),
        ],
    );
    run_sync(&context, true, &[]);
    fs::write(target.path().join("auto.txt"), "local\n").expect("the local edit is written");
    fs::write(target.path().join("manual.txt"), "local\n").expect("the local edit is written");

    // Default run: only the automatic entry is checked.
    let outcome = run_check(&context, false, &[]).expect("the check succeeds");
    assert_eq!(
        outcome.out_of_sync,
        vec![reported("auto.txt", CheckStatus::Edited)]
    );

    // `--all` reports both: `overwrite` mode would discard the edits.
    let outcome = run_check(&context, true, &[]).expect("the check succeeds");
    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![
            reported("auto.txt", CheckStatus::Edited),
            reported("manual.txt", CheckStatus::Edited),
        ]
    );
}

#[test]
fn without_a_lock_record_an_identical_target_is_in_sync_and_a_differing_one_is_stale() {
    let source = TempDir::new().expect("temporary folder source is created");
    write_file(source.path(), "base.yml", "v1\n");
    let target = git_repository();
    let context = context(
        &target,
        vec![folder_file_entry(
            &source,
            "base.yml",
            "base.yml",
            Policy::Automatic,
        )],
    );
    write_file(target.path(), "base.yml", "v1\n");

    let outcome = run_check(&context, false, &[]).expect("the check succeeds");
    assert!(outcome.failures.is_empty());
    assert_eq!(outcome.out_of_sync, Vec::new());

    fs::write(target.path().join("base.yml"), "local\n").expect("the local edit is written");
    let outcome = run_check(&context, false, &[]).expect("the check succeeds");
    assert_eq!(
        outcome.out_of_sync,
        vec![reported("base.yml", CheckStatus::Stale)]
    );
    // Read-only: checking never creates a lock file.
    assert!(!lock_path(&target).exists());
}

#[test]
fn a_tree_entry_reports_missing_and_extra_target_files() {
    let source = TempDir::new().expect("temporary folder source is created");
    write_file(source.path(), "a.yml", "one\n");
    write_file(source.path(), "sub/b.yml", "two\n");
    let target = git_repository();
    let context = context(&target, vec![folder_tree_entry(&source, "mirror")]);
    run_sync(&context, false, &[]);
    fs::remove_file(target.path().join("mirror/a.yml"))
        .expect("the target file is removed from the mirror");
    write_file(target.path(), "mirror/extra.yml", "extra\n");

    let outcome = run_check(&context, false, &[]).expect("the check succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![
            reported("mirror/a.yml", CheckStatus::Missing),
            reported("mirror/extra.yml", CheckStatus::Extra),
        ]
    );
    assert_eq!(stdout(false, &outcome), "mirror/a.yml\nmirror/extra.yml\n");
}

#[test]
fn an_explicit_target_checks_a_manual_entry() {
    let source = TempDir::new().expect("temporary folder source is created");
    write_file(source.path(), "manual.txt", "m1\n");
    let target = git_repository();
    let context = context(
        &target,
        vec![folder_file_entry(
            &source,
            "manual.txt",
            "manual.txt",
            Policy::Manual,
        )],
    );

    let outcome = run_check(&context, false, &["manual.txt"]).expect("the check succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![reported("manual.txt", CheckStatus::Missing)]
    );
}

#[test]
fn an_unknown_target_fails_the_check_before_any_entry_is_processed() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repo_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Patch,
            Policy::Automatic,
        )],
    );

    let error = run_check(&context, false, &["missing.yml"])
        .expect_err("an unknown target is a usage error");

    let SyncsmithError::TargetSelection {
        unknown,
        configured,
    } = error
    else {
        panic!("an unknown target must fail with TargetSelection");
    };
    assert_eq!(unknown, "missing.yml");
    assert_eq!(configured, "app.yml");
    // Nothing was processed: no target file, no lock file.
    assert!(!target.path().join("app.yml").exists());
    assert!(!lock_path(&target).exists());
}

#[test]
fn a_check_run_never_writes_the_targets_or_the_lock_file() {
    let (merge_source, _) = source_repository(&[("merge.txt", "alpha\nbeta\ngamma\n")]);
    let (stale_source, _) = source_repository(&[("app.yml", "v1\n")]);
    let notes = TempDir::new().expect("temporary folder source is created");
    write_file(notes.path(), "notes.md", "n1\n");
    let target = git_repository();
    let context = context(
        &target,
        vec![
            repo_entry(
                &merge_source,
                "merge.txt",
                "merge.txt",
                Mode::Patch,
                Policy::Automatic,
            ),
            repo_entry(
                &stale_source,
                "app.yml",
                "app.yml",
                Mode::Patch,
                Policy::Automatic,
            ),
            folder_file_entry(&notes, "notes.md", "notes.md", Policy::Manual),
        ],
    );
    run_sync(&context, true, &[]);
    // One entry per evaluation path that could tempt a write: a clean
    // trial merge, a pending patch, and a local edit in `overwrite` mode.
    fs::write(target.path().join("merge.txt"), "local\nbeta\ngamma\n")
        .expect("the local edit is written");
    write_file(merge_source.path(), "merge.txt", "alpha\nbeta\nupstream\n");
    commit(merge_source.path(), "change the last line");
    write_file(stale_source.path(), "app.yml", "v2\n");
    commit(stale_source.path(), "change the source");
    fs::write(target.path().join("notes.md"), "local\n").expect("the local edit is written");

    let before = tree_snapshot(target.path());
    let outcome = run_check(&context, true, &[]).expect("the check succeeds");
    let after = tree_snapshot(target.path());

    // The run actually evaluated the three paths (trial merge, patch
    // detection, edit detection) and reported all of them.
    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.out_of_sync,
        vec![
            reported("merge.txt", CheckStatus::Stale),
            reported("app.yml", CheckStatus::Stale),
            reported("notes.md", CheckStatus::Edited),
        ]
    );
    // Read-only guarantee: targets and lock file are byte-identical.
    assert_eq!(before, after);
}

/// Writes `entries` as the `.syncsmith.toml` of `repo` (the on-disk schema
/// the real binary loads and validates).
fn write_config(repo: &TempDir, entries: Vec<SyncEntry>) {
    let text = toml::to_string(&Config { sync: entries }).expect("the configuration serializes");
    fs::write(repo.path().join(CONFIG_FILE_NAME), text).expect("the configuration file is written");
}

#[test]
fn the_check_command_requires_an_existing_configuration() {
    let target = git_repository();
    let resolution = resolve_config_path(&GlobalArgs {
        config: Some(target.path().join(CONFIG_FILE_NAME)),
        verbose: 0,
        json: false,
    })
    .expect("the configuration location resolves");

    let error = commands::check::run_command(
        &resolution,
        CheckInvocation {
            targets: Vec::new(),
            all: false,
            no_fail: false,
            json: false,
        },
    )
    .expect_err("a missing configuration fails the command");

    assert!(matches!(error, SyncsmithError::ConfigNotFound { .. }));
    // The command fails before any processing: no lock file is created.
    assert!(!lock_path(&target).exists());
}

/// Runs the built `syncsmith` binary in `repo` with `args`, and returns its
/// exit code, stdout, and stderr.
///
/// The child runs with its own working directory (no process cwd mutation)
/// and the same hermetic git configuration as the fixture commands, so the
/// result does not depend on the developer's setup.
fn syncsmith(repo: &TempDir, args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_syncsmith"))
        .args(args)
        .current_dir(repo.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("the syncsmith binary is built and runnable");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn the_binary_reports_missing_files_and_no_fail_exits_zero_with_the_same_stdout() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    write_config(
        &target,
        vec![repo_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Patch,
            Policy::Automatic,
        )],
    );

    let (code, stdout_payload, _) = syncsmith(&target, &["check"]);

    assert_eq!(code, Some(1));
    assert_eq!(stdout_payload, "app.yml\n");

    let (code, stdout_payload, _) = syncsmith(&target, &["check", "--no-fail"]);

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "app.yml\n");
    // Read-only: the check runs created no target file and no lock file.
    assert!(!target.path().join("app.yml").exists());
    assert!(!lock_path(&target).exists());
}

#[test]
fn the_binary_exits_zero_on_a_synchronized_repository_in_both_formats() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    write_config(
        &target,
        vec![repo_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Patch,
            Policy::Automatic,
        )],
    );
    let (code, _, _) = syncsmith(&target, &["sync"]);
    assert_eq!(code, Some(0));

    let (code, stdout_payload, _) = syncsmith(&target, &["check"]);

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "");

    let (code, stdout_payload, _) = syncsmith(&target, &["check", "--json"]);

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "[]");
}

#[test]
fn the_binary_renders_the_json_payload_with_path_and_status() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    write_config(
        &target,
        vec![repo_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Patch,
            Policy::Automatic,
        )],
    );
    let (code, _, _) = syncsmith(&target, &["sync"]);
    assert_eq!(code, Some(0));
    write_file(source.path(), "app.yml", "v2\n");
    commit(source.path(), "change the source");

    let (code, stdout_payload, _) = syncsmith(&target, &["check", "--json"]);

    assert_eq!(code, Some(1));
    assert_eq!(stdout_payload, r#"[{"path":"app.yml","status":"stale"}]"#);
}

#[test]
fn the_binary_exits_one_on_an_unknown_target_without_processing_anything() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    write_config(
        &target,
        vec![repo_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Patch,
            Policy::Automatic,
        )],
    );

    let (code, stdout_payload, stderr_payload) = syncsmith(&target, &["check", "missing.yml"]);

    // Usage error before any entry is processed: no payload on stdout, an
    // actionable message on stderr, no target file, no lock file.
    assert_eq!(code, Some(1));
    assert_eq!(stdout_payload, "");
    assert!(stderr_payload.contains("missing.yml"));
    assert!(stderr_payload.contains("app.yml"));
    assert!(!target.path().join("app.yml").exists());
    assert!(!lock_path(&target).exists());
}

#[test]
fn the_binary_exits_non_zero_on_an_evaluation_failure_even_with_no_fail() {
    let source = TempDir::new().expect("temporary folder source is created");
    write_file(source.path(), "notes.md", "n1\n");
    let target = git_repository();
    write_config(
        &target,
        vec![folder_file_entry(
            &source,
            "notes.md",
            "notes.md",
            Policy::Automatic,
        )],
    );
    let (code, _, _) = syncsmith(&target, &["sync"]);
    assert_eq!(code, Some(0));
    fs::remove_dir_all(source.path()).expect("the source folder is removed");

    let (code, stdout_payload, stderr_payload) = syncsmith(&target, &["check", "--no-fail"]);

    // An evaluation failure (source load) is fatal even with `--no-fail`:
    // CI can tell "cannot verify" from "out of date".
    assert_eq!(code, Some(1));
    assert_eq!(stdout_payload, "");
    assert!(stderr_payload.contains("synchronization failure"));
    // The failed entry leaves the target and the lock untouched.
    assert_eq!(read_target(&target, "notes.md"), "n1\n");
    assert!(lock_path(&target).is_file());
}
