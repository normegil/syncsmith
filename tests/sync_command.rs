//! Integration tests for the `sync` command (plan section 3.2).
//!
//! Fixtures are local git repositories and plain folders only — no network.
//! Each test builds a target repository (fresh `git init`) plus one or two
//! sources, runs `sync::run` with the context and selection `main.rs` would
//! build, then asserts the resulting target tree, lock records, and stdout
//! payload. The context is constructed directly from absolute paths so the
//! tests never depend on the process working directory and can run in
//! parallel.
//!
//! The last section spawns the built binary instead, closing the exit-code
//! coverage gap: it verifies the `main.rs` glue that maps run failures and
//! usage errors to the process exit code and writes the stdout report.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use syncsmith::cli::GlobalArgs;
use syncsmith::commands;
use syncsmith::commands::sync::SyncInvocation;
use syncsmith::commands::sync::report::ReportFormat;
use syncsmith::commands::sync::source::blob_hash;
use syncsmith::commands::sync::{
    self, Action, ChangedFile, LOCK_FILE_NAME, LockFile, LockRecord, Selection, SyncContext,
    SyncOutcome,
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

fn repository_entry(repo: &TempDir, source_path: &str, target: &str, mode: Mode) -> SyncEntry {
    SyncEntry {
        source: Source::Repository {
            repository: repo.path().to_string_lossy().into_owned(),
            git_ref: None,
            path: source_path.into(),
        },
        target: target.into(),
        mode,
        policy: Policy::Automatic,
    }
}

/// A single-file folder source entry.
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

/// Runs the sync command the way `main.rs` dispatches it: selection from
/// the CLI arguments, then the sync run.
fn run(context: &SyncContext, all: bool, targets: &[&str]) -> Result<SyncOutcome, SyncsmithError> {
    let selection = Selection::from_args(all, targets.iter().copied().map(PathBuf::from).collect());
    sync::run(context, &selection)
}

/// Renders the stdout payload the way `main.rs` writes it, from the global
/// arguments of the invocation.
fn stdout(global: &GlobalArgs, outcome: &SyncOutcome) -> String {
    let format = if global.json {
        ReportFormat::Json
    } else {
        ReportFormat::Lines
    };
    sync::report::render(&outcome.changed, format)
}

fn default_args() -> GlobalArgs {
    GlobalArgs {
        config: None,
        verbose: 0,
        json: false,
    }
}

fn json_args() -> GlobalArgs {
    GlobalArgs {
        config: None,
        verbose: 0,
        json: true,
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

fn changed(path: &str, action: Action) -> ChangedFile {
    ChangedFile {
        path: PathBuf::from(path),
        action,
    }
}

fn lock_record(target: &str, version: String, content: String) -> LockRecord {
    LockRecord {
        target: PathBuf::from(target),
        version,
        content,
    }
}

#[test]
fn first_sync_creates_the_target_files_and_their_lock_records() {
    let (source, version) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );

    let outcome = run(&context, false, &[]).expect("the first synchronization succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(read_target(&target, "app.yml"), "v1\n");
    assert_eq!(outcome.changed, vec![changed("app.yml", Action::Created)]);
    // stdout: one changed path per line — exactly what the pre-commit hook
    // stages.
    assert_eq!(stdout(&default_args(), &outcome), "app.yml\n");
    assert_eq!(
        lock_of(&target).record(Path::new("app.yml")),
        Some(&lock_record("app.yml", version, blob_hash(b"v1\n")))
    );
}

#[test]
fn a_missing_lock_file_loads_as_empty() {
    let target = git_repository();
    let lock = LockFile::load(&lock_path(&target)).expect("a missing lock file loads as empty");
    assert_eq!(lock, LockFile::default());
}

#[test]
fn the_json_flag_reports_one_object_per_changed_file() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );

    let outcome = run(&context, false, &[]).expect("the synchronization succeeds");

    assert_eq!(
        stdout(&json_args(), &outcome),
        r#"[{"path":"app.yml","action":"created"}]"#
    );
}

#[test]
fn an_unchanged_source_produces_an_empty_report() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");

    let outcome = run(&context, false, &[]).expect("the second synchronization succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(stdout(&default_args(), &outcome), "");
    assert_eq!(stdout(&json_args(), &outcome), "[]");
}

#[test]
fn an_overwrite_entry_discards_local_modifications() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(
            &source,
            "app.yml",
            "app.yml",
            Mode::Overwrite,
        )],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");
    fs::write(target.path().join("app.yml"), "local edit\n").expect("the local edit is written");
    write_file(source.path(), "app.yml", "v2\n");
    commit(source.path(), "change the source");

    let outcome = run(&context, false, &[]).expect("the overwrite succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(read_target(&target, "app.yml"), "v2\n");
    assert_eq!(outcome.changed, vec![changed("app.yml", Action::Updated)]);
}

#[test]
fn an_overwrite_folder_entry_mirrors_additions_updates_and_removals() {
    let source = TempDir::new().expect("temporary folder source is created");
    write_file(source.path(), "a.yml", "one\n");
    write_file(source.path(), "sub/b.yml", "two\n");
    let target = git_repository();
    let context = context(&target, vec![folder_tree_entry(&source, "mirror")]);
    run(&context, false, &[]).expect("the first synchronization succeeds");

    write_file(source.path(), "a.yml", "one v2\n");
    write_file(source.path(), "c.yml", "three\n");
    fs::remove_file(source.path().join("sub/b.yml")).expect("the source removal is applied");

    let outcome = run(&context, false, &[]).expect("the mirror synchronization succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(read_target(&target, "mirror/a.yml"), "one v2\n");
    assert_eq!(read_target(&target, "mirror/c.yml"), "three\n");
    assert!(!target.path().join("mirror/sub/b.yml").exists());
    assert_eq!(
        outcome.changed,
        vec![
            changed("mirror/a.yml", Action::Updated),
            changed("mirror/c.yml", Action::Created),
            changed("mirror/sub/b.yml", Action::Deleted),
        ]
    );
    // The stdout contract in both formats: changes in processing order.
    assert_eq!(
        stdout(&default_args(), &outcome),
        "mirror/a.yml\nmirror/c.yml\nmirror/sub/b.yml\n"
    );
    assert_eq!(
        stdout(&json_args(), &outcome),
        r#"[{"path":"mirror/a.yml","action":"updated"},{"path":"mirror/c.yml","action":"created"},{"path":"mirror/sub/b.yml","action":"deleted"}]"#
    );
    // Lock: folder versions are content hashes; the deleted file loses its
    // record.
    let lock = lock_of(&target);
    assert_eq!(
        lock.record(Path::new("mirror/a.yml")),
        Some(&lock_record(
            "mirror/a.yml",
            blob_hash(b"one v2\n"),
            blob_hash(b"one v2\n")
        ))
    );
    assert!(lock.record(Path::new("mirror/c.yml")).is_some());
    assert!(lock.record(Path::new("mirror/sub/b.yml")).is_none());
}

#[test]
fn a_patch_entry_with_an_unedited_target_updates_directly() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");
    write_file(source.path(), "app.yml", "v2\n");
    commit(source.path(), "bump the application version");

    let outcome = run(&context, false, &[]).expect("the direct update succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(read_target(&target, "app.yml"), "v2\n");
    assert_eq!(outcome.changed, vec![changed("app.yml", Action::Updated)]);
    assert_eq!(
        lock_of(&target).record(Path::new("app.yml")),
        Some(&lock_record(
            "app.yml",
            head(source.path()),
            blob_hash(b"v2\n")
        ))
    );
}

#[test]
fn a_patch_entry_with_an_edited_target_merges_the_source_change() {
    let (source, _) = source_repository(&[("app.yml", "alpha\nbeta\ngamma\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");
    // Local edit of the first line while the source later changes the last
    // one: the three-way merge must keep both.
    fs::write(target.path().join("app.yml"), "local\nbeta\ngamma\n")
        .expect("the local edit is written");
    write_file(source.path(), "app.yml", "alpha\nbeta\nupstream\n");
    commit(source.path(), "change the last line");

    let outcome = run(&context, false, &[]).expect("the three-way merge succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(read_target(&target, "app.yml"), "local\nbeta\nupstream\n");
    assert_eq!(outcome.changed, vec![changed("app.yml", Action::Updated)]);
    assert_eq!(
        lock_of(&target).record(Path::new("app.yml")),
        Some(&lock_record(
            "app.yml",
            head(source.path()),
            blob_hash(b"local\nbeta\nupstream\n")
        ))
    );
}

#[test]
fn a_conflicted_entry_fails_while_the_other_entries_still_synchronize() {
    let (source, first_version) = source_repository(&[("shared.txt", "shared line\n")]);
    let notes = TempDir::new().expect("temporary folder source is created");
    write_file(notes.path(), "notes.md", "n1\n");
    let target = git_repository();
    let context = context(
        &target,
        vec![
            repository_entry(&source, "shared.txt", "shared.txt", Mode::Patch),
            folder_file_entry(&notes, "notes.md", "notes.md", Policy::Automatic),
        ],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");
    fs::write(target.path().join("shared.txt"), "local edit\n").expect("the local edit is written");
    write_file(source.path(), "shared.txt", "upstream edit\n");
    commit(source.path(), "change the shared line");
    write_file(notes.path(), "notes.md", "n2\n");

    let outcome = run(&context, false, &[]).expect("the run completes despite the conflict");

    assert_eq!(outcome.failures.len(), 1);
    let failure = &outcome.failures[0];
    assert_eq!(failure.target, Path::new("shared.txt"));
    let SyncsmithError::MergeConflict { target: file } = &failure.error else {
        panic!("expected a merge conflict, got {:?}", failure.error);
    };
    assert_eq!(file, Path::new("shared.txt"));
    // The conflicted target is written with conflict markers for manual
    // resolution. The exact layout follows the user's `merge.conflictStyle`
    // (`diff3` adds a `||||||| base` section), so assert the markers and
    // sides shared by every style.
    let conflicted = read_target(&target, "shared.txt");
    assert!(conflicted.contains("<<<<<<< ours\nlocal edit\n"));
    assert!(conflicted.contains("=======\nupstream edit\n>>>>>>> theirs\n"));
    // The other entry still synchronized.
    assert_eq!(read_target(&target, "notes.md"), "n2\n");
    // The lock record of the failed entry keeps the last successful
    // synchronization; the successful one is updated.
    assert_eq!(
        lock_of(&target).record(Path::new("shared.txt")),
        Some(&lock_record(
            "shared.txt",
            first_version,
            blob_hash(b"shared line\n")
        ))
    );
    assert_eq!(
        lock_of(&target).record(Path::new("notes.md")),
        Some(&lock_record(
            "notes.md",
            blob_hash(b"n2\n"),
            blob_hash(b"n2\n")
        ))
    );
}

#[test]
fn a_missing_source_file_fails_only_its_entry() {
    let (source, first_version) = source_repository(&[("gone.txt", "g1\n")]);
    let notes = TempDir::new().expect("temporary folder source is created");
    write_file(notes.path(), "notes.md", "n1\n");
    let target = git_repository();
    let context = context(
        &target,
        vec![
            repository_entry(&source, "gone.txt", "gone.txt", Mode::Patch),
            folder_file_entry(&notes, "notes.md", "notes.md", Policy::Automatic),
        ],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");
    fs::remove_file(source.path().join("gone.txt")).expect("the source file is deleted");
    commit(source.path(), "remove the source file");
    write_file(notes.path(), "notes.md", "n2\n");

    let outcome = run(&context, false, &[]).expect("the run completes despite the failure");

    assert_eq!(outcome.failures.len(), 1);
    let failure = &outcome.failures[0];
    assert_eq!(failure.target, Path::new("gone.txt"));
    assert!(matches!(failure.error, SyncsmithError::SourceLoad { .. }));
    // The failed entry leaves its target untouched; the other one syncs.
    assert_eq!(read_target(&target, "gone.txt"), "g1\n");
    assert_eq!(read_target(&target, "notes.md"), "n2\n");
    assert_eq!(
        lock_of(&target).record(Path::new("gone.txt")),
        Some(&lock_record("gone.txt", first_version, blob_hash(b"g1\n")))
    );
    assert_eq!(
        lock_of(&target).record(Path::new("notes.md")),
        Some(&lock_record(
            "notes.md",
            blob_hash(b"n2\n"),
            blob_hash(b"n2\n")
        ))
    );
}

#[test]
fn an_unknown_target_fails_before_any_entry_is_processed() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );

    let error =
        run(&context, false, &["missing.yml"]).expect_err("an unknown target is a usage error");

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

/// A target repository with one automatic entry (`auto.txt`) and one manual
/// entry (`manual.txt`), both from single-file folder sources.
///
/// The fixture owns its source directories: a `TempDir` is deleted when
/// dropped, and the sources must outlive the test body.
struct PolicyFixture {
    /// The target repository.
    target: TempDir,
    /// The context to run sync with.
    context: SyncContext,
    /// Source directories, kept alive for the whole test.
    _sources: [TempDir; 2],
}

fn policy_fixture() -> PolicyFixture {
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
    PolicyFixture {
        target,
        context,
        _sources: [automatic, manual],
    }
}

#[test]
fn a_default_run_synchronizes_only_the_automatic_entries() {
    let fixture = policy_fixture();

    let outcome = run(&fixture.context, false, &[]).expect("the run succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(outcome.changed, vec![changed("auto.txt", Action::Created)]);
    assert_eq!(stdout(&default_args(), &outcome), "auto.txt\n");
    assert!(!fixture.target.path().join("manual.txt").exists());
    let lock = lock_of(&fixture.target);
    assert!(lock.record(Path::new("auto.txt")).is_some());
    assert!(lock.record(Path::new("manual.txt")).is_none());
}

#[test]
fn the_all_flag_synchronizes_the_manual_entries_too() {
    let fixture = policy_fixture();

    let outcome = run(&fixture.context, true, &[]).expect("the run succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.changed,
        vec![
            changed("auto.txt", Action::Created),
            changed("manual.txt", Action::Created),
        ]
    );
    assert_eq!(stdout(&default_args(), &outcome), "auto.txt\nmanual.txt\n");
    assert_eq!(read_target(&fixture.target, "manual.txt"), "m1\n");
}

#[test]
fn an_explicit_target_synchronizes_a_manual_entry() {
    let fixture = policy_fixture();

    let outcome = run(&fixture.context, false, &["manual.txt"]).expect("the run succeeds");

    assert!(outcome.failures.is_empty());
    assert_eq!(
        outcome.changed,
        vec![changed("manual.txt", Action::Created)]
    );
    assert!(!fixture.target.path().join("auto.txt").exists());
    assert_eq!(read_target(&fixture.target, "manual.txt"), "m1\n");
}

#[test]
fn an_unrecoverable_lock_version_fails_the_entry_without_touching_the_target() {
    let (source, _) = source_repository(&[("app.yml", "alpha\nbeta\ngamma\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");
    fs::write(target.path().join("app.yml"), "local\nbeta\ngamma\n")
        .expect("the local edit is written");
    write_file(source.path(), "app.yml", "alpha\nbeta\nupstream\n");
    commit(source.path(), "change the last line");
    // A pruned or otherwise unknown commit in the lock: the last-synced
    // source version can no longer be recovered.
    let pruned = "0".repeat(40);
    let mut lock = lock_of(&target);
    lock.upsert(lock_record(
        "app.yml",
        pruned.clone(),
        blob_hash(b"alpha\nbeta\ngamma\n"),
    ));
    lock.save_atomic(&lock_path(&target))
        .expect("the tampered lock is saved");

    let outcome = run(&context, false, &[]).expect("the run completes despite the failure");

    assert_eq!(outcome.failures.len(), 1);
    let failure = &outcome.failures[0];
    assert_eq!(failure.target, Path::new("app.yml"));
    let SyncsmithError::SourceVersionUnavailable { version, .. } = &failure.error else {
        panic!(
            "expected an unavailable source version, got {:?}",
            failure.error
        );
    };
    assert_eq!(version, &pruned);
    // The target keeps the local edit and the lock record is not updated.
    assert_eq!(read_target(&target, "app.yml"), "local\nbeta\ngamma\n");
    assert_eq!(
        lock_of(&target).record(Path::new("app.yml")),
        Some(&lock_record(
            "app.yml",
            pruned,
            blob_hash(b"alpha\nbeta\ngamma\n")
        ))
    );
}

#[test]
fn a_binary_edited_target_in_patch_mode_fails_without_corrupting_it() {
    let (source, first_version) = source_repository(&[("app.yml", "text\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");
    let binary: &[u8] = &[0x00, 0xff, 0xfe, b'\n'];
    fs::write(target.path().join("app.yml"), binary).expect("the binary edit is written");
    write_file(source.path(), "app.yml", "text v2\n");
    commit(source.path(), "change the source");

    let outcome = run(&context, false, &[]).expect("the run completes despite the failure");

    assert_eq!(outcome.failures.len(), 1);
    let failure = &outcome.failures[0];
    assert_eq!(failure.target, Path::new("app.yml"));
    assert!(matches!(
        failure.error,
        SyncsmithError::UnsupportedContent { .. }
    ));
    // The binary content is left exactly as it was.
    assert_eq!(
        fs::read(target.path().join("app.yml")).expect("the target file is readable"),
        binary
    );
    assert_eq!(
        lock_of(&target).record(Path::new("app.yml")),
        Some(&lock_record("app.yml", first_version, blob_hash(b"text\n")))
    );
}

#[test]
fn a_source_deletion_in_a_tree_entry_removes_the_unedited_target() {
    let (source, _) = source_repository(&[("rules/a.yml", "one\n"), ("rules/b.yml", "two\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "rules", "rules", Mode::Patch)],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");
    fs::remove_file(source.path().join("rules/b.yml")).expect("the source file is deleted");
    commit(source.path(), "remove one rule");

    let outcome = run(&context, false, &[]).expect("the mirror synchronization succeeds");

    assert!(outcome.failures.is_empty());
    assert!(!target.path().join("rules/b.yml").exists());
    assert_eq!(read_target(&target, "rules/a.yml"), "one\n");
    assert_eq!(
        outcome.changed,
        vec![changed("rules/b.yml", Action::Deleted)]
    );
    assert_eq!(stdout(&default_args(), &outcome), "rules/b.yml\n");
    let lock = lock_of(&target);
    assert!(lock.record(Path::new("rules/b.yml")).is_none());
    assert_eq!(
        lock.record(Path::new("rules/a.yml")),
        Some(&lock_record(
            "rules/a.yml",
            head(source.path()),
            blob_hash(b"one\n")
        ))
    );
}

#[test]
fn a_source_deletion_of_an_edited_target_fails_in_patch_mode() {
    let (source, first_version) =
        source_repository(&[("rules/a.yml", "one\n"), ("rules/b.yml", "two\n")]);
    let target = git_repository();
    let context = context(
        &target,
        vec![repository_entry(&source, "rules", "rules", Mode::Patch)],
    );
    run(&context, false, &[]).expect("the first synchronization succeeds");
    fs::write(target.path().join("rules/b.yml"), "local\n").expect("the local edit is written");
    fs::remove_file(source.path().join("rules/b.yml")).expect("the source file is deleted");
    commit(source.path(), "remove one rule");

    let outcome = run(&context, false, &[]).expect("the run completes despite the conflict");

    assert_eq!(outcome.failures.len(), 1);
    let failure = &outcome.failures[0];
    // The failure is reported on the entry, with the file in the error.
    assert_eq!(failure.target, Path::new("rules"));
    let SyncsmithError::MergeConflict { target: file } = &failure.error else {
        panic!("expected a merge conflict, got {:?}", failure.error);
    };
    assert_eq!(file, Path::new("rules/b.yml"));
    // The local edit is preserved for manual resolution.
    assert_eq!(read_target(&target, "rules/b.yml"), "local\n");
    assert_eq!(
        lock_of(&target).record(Path::new("rules/b.yml")),
        Some(&lock_record(
            "rules/b.yml",
            first_version,
            blob_hash(b"two\n")
        ))
    );
}

/// Writes `entries` as the `.syncsmith.toml` of `repo` (the on-disk schema
/// the real binary loads and validates).
fn write_config(repo: &TempDir, entries: Vec<SyncEntry>) {
    let text = toml::to_string(&Config { sync: entries }).expect("the configuration serializes");
    fs::write(repo.path().join(CONFIG_FILE_NAME), text).expect("the configuration file is written");
}

#[test]
fn the_sync_command_requires_an_existing_configuration() {
    let target = git_repository();
    let resolution = resolve_config_path(&GlobalArgs {
        config: Some(target.path().join(CONFIG_FILE_NAME)),
        verbose: 0,
        json: false,
    })
    .expect("the configuration location resolves");

    let error = commands::sync::run_command(
        &resolution,
        SyncInvocation {
            targets: Vec::new(),
            all: false,
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
fn the_binary_synchronizes_and_exits_zero_with_the_stdout_payload() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    write_config(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );

    let (code, stdout_payload, _) = syncsmith(&target, &["sync"]);

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "app.yml\n");
    assert_eq!(read_target(&target, "app.yml"), "v1\n");
    assert!(lock_path(&target).is_file());

    // A run with nothing to change still succeeds with an empty payload —
    // the pre-commit hook stages nothing and lets the commit proceed.
    let (code, stdout_payload, _) = syncsmith(&target, &["sync"]);

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "");
}

#[test]
fn the_binary_exits_non_zero_on_a_conflict_but_reports_the_changed_paths() {
    let (source, _) = source_repository(&[("shared.txt", "shared line\n")]);
    let notes = TempDir::new().expect("temporary folder source is created");
    write_file(notes.path(), "ok.txt", "n1\n");
    let target = git_repository();
    write_config(
        &target,
        vec![
            repository_entry(&source, "shared.txt", "shared.txt", Mode::Patch),
            folder_file_entry(&notes, "ok.txt", "ok.txt", Policy::Automatic),
        ],
    );
    let (code, stdout_payload, _) = syncsmith(&target, &["sync"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "shared.txt\nok.txt\n");
    fs::write(target.path().join("shared.txt"), "local edit\n").expect("the local edit is written");
    write_file(source.path(), "shared.txt", "upstream edit\n");
    commit(source.path(), "change the shared line");
    write_file(notes.path(), "ok.txt", "n2\n");

    let (code, stdout_payload, _) = syncsmith(&target, &["sync"]);

    // The run fails (non-zero exit), yet stdout still lists the paths
    // changed by the successful entries. The conflicted file is written
    // with markers but is dropped from the payload: a failed entry
    // contributes no changed paths (its lock record is not updated either),
    // and the hook aborts on the non-zero exit before staging anyway. The
    // exact marker layout depends on the user's `merge.conflictStyle`, so
    // assert the boundaries shared by every style.
    assert_eq!(code, Some(1));
    assert_eq!(stdout_payload, "ok.txt\n");
    let conflicted = read_target(&target, "shared.txt");
    assert!(conflicted.contains("<<<<<<< ours\nlocal edit\n"));
    assert!(conflicted.contains("upstream edit\n>>>>>>> theirs\n"));
    assert_eq!(read_target(&target, "ok.txt"), "n2\n");
}

#[test]
fn the_binary_exits_non_zero_on_an_unknown_target_before_any_processing() {
    let (source, _) = source_repository(&[("app.yml", "v1\n")]);
    let target = git_repository();
    write_config(
        &target,
        vec![repository_entry(&source, "app.yml", "app.yml", Mode::Patch)],
    );

    let (code, stdout_payload, stderr_payload) = syncsmith(&target, &["sync", "missing.yml"]);

    assert_eq!(code, Some(1));
    // Usage error before any entry is processed: no report on stdout, no
    // target file, no lock file, and an actionable message on stderr.
    assert_eq!(stdout_payload, "");
    assert!(stderr_payload.contains("missing.yml"));
    assert!(stderr_payload.contains("app.yml"));
    assert!(!target.path().join("app.yml").exists());
    assert!(!lock_path(&target).exists());
}
