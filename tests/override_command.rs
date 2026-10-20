//! Integration tests for the `sync --override-from` stateless flow
//! (plan section 3.2).
//!
//! Fixtures are local folders and git repositories only — no network. The
//! override flow resolves its target against the configuration directory
//! when a configuration file exists, the repository root otherwise, and
//! the current directory outside any repository (`target_base`), while
//! relative source paths resolve against the current directory. The
//! end-to-end coverage spawns the built binary with `.current_dir()` set
//! to the target repository: no process cwd mutation (the library-level
//! tests stay parallel-safe) and exit-code coverage in one stroke.
//!
//! Library-level calls stay cwd-independent and only exercise paths that
//! return before any filesystem write: the usage errors (target count and
//! target validation fire before the context is built) and the
//! source-resolution failures (the source snapshot fails before any target
//! is touched). No test in this file changes the process working directory.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use syncsmith::commands;
use syncsmith::commands::sync::direct::{self, DirectRun, OverrideArgs};
use syncsmith::commands::sync::{LOCK_FILE_NAME, SyncInvocation};
use syncsmith::config::{CONFIG_FILE_NAME, Config, Mode, Policy, Source, SyncEntry};
use syncsmith::error::SyncsmithError;

/// Runs `git -C dir args` and returns its trimmed stdout, panicking with
/// git's stderr on failure.
///
/// The fixed identity and the config overrides keep the tests away from the
/// developer's git configuration (same approach as `tests/sync_command.rs`,
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

/// The `--override-from` source arguments of one invocation.
fn override_args(source: &str, git_ref: Option<&str>, source_path: Option<&str>) -> OverrideArgs {
    OverrideArgs {
        source: source.into(),
        git_ref: git_ref.map(String::from),
        source_path: source_path.map(PathBuf::from),
    }
}

/// A stateless run input: `targets` under `base`, with `source` arguments.
fn direct_run(
    source: &str,
    git_ref: Option<&str>,
    source_path: Option<&str>,
    targets: &[&str],
    base: &Path,
) -> DirectRun {
    DirectRun {
        args: override_args(source, git_ref, source_path),
        targets: targets.iter().map(PathBuf::from).collect(),
        base: base.to_path_buf(),
        json: false,
    }
}

fn read_target(target_repo: &TempDir, relative: &str) -> String {
    fs::read_to_string(target_repo.path().join(relative)).expect("the target file is readable")
}

fn lock_path(target_repo: &TempDir) -> PathBuf {
    target_repo.path().join(LOCK_FILE_NAME)
}

/// Writes `entries` as the `.syncsmith.toml` of `repo` (the on-disk schema
/// the real binary loads and validates).
fn write_config(repo: &TempDir, entries: Vec<SyncEntry>) {
    let text = toml::to_string(&Config { sync: entries }).expect("the configuration serializes");
    fs::write(repo.path().join(CONFIG_FILE_NAME), text).expect("the configuration file is written");
}

/// Runs the built `syncsmith` binary in `repo` with `args`, and returns its
/// exit code, stdout, and stderr.
///
/// The child runs with its own working directory (the override flow
/// resolves its target there) and the same hermetic git configuration as
/// the fixture commands.
fn syncsmith<I, S>(repo: &TempDir, args: I) -> (Option<i32>, String, String)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    syncsmith_from(repo.path(), args)
}

/// Runs the built `syncsmith` binary with `cwd` as its working directory,
/// and returns its exit code, stdout, and stderr.
///
/// Used for spawns from a subdirectory of the target repository, or from a
/// plain folder without one: the target base follows the configuration
/// location and the repository root, so the working directory matters.
fn syncsmith_from<I, S>(cwd: &Path, args: I) -> (Option<i32>, String, String)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new(env!("CARGO_BIN_EXE_syncsmith"))
        .args(args)
        .current_dir(cwd)
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

// ---------------------------------------------------------------------------
// Library-level: usage errors and source failures (no filesystem writes)
// ---------------------------------------------------------------------------

#[test]
fn run_command_rejects_override_target_counts_other_than_one() {
    let target = git_repository();
    for targets in [&[] as &[&str], &["a.yml", "b.yml"]] {
        let error = commands::sync::run_command(SyncInvocation::Direct(direct_run(
            "unused",
            None,
            None,
            targets,
            target.path(),
        )))
        .expect_err("an override run takes exactly one target path");
        assert!(
            matches!(error, SyncsmithError::InvalidArguments { .. }),
            "expected a usage error, got {error:?}"
        );
    }
}

#[test]
fn direct_run_rejects_absolute_and_escaping_targets() {
    let target = git_repository();
    for rejected in [
        target.path().join("app.yml"),
        PathBuf::from("../escape.yml"),
    ] {
        let mut run = direct_run("unused", None, None, &[], target.path());
        run.targets = vec![rejected];
        let error = direct::run_command(run)
            .expect_err("an override target must stay inside the target repository");
        assert!(
            matches!(error, SyncsmithError::InvalidTarget { .. }),
            "expected an invalid target error, got {error:?}"
        );
    }
}

#[test]
fn direct_run_fails_with_source_load_when_the_local_source_is_missing() {
    let target = git_repository();
    let missing = target.path().join("no-such-source.txt");
    let source = missing.to_string_lossy().into_owned();
    let error = direct::run_command(direct_run(&source, None, None, &["app.yml"], target.path()))
        .expect_err("a missing source fails the override run");
    let SyncsmithError::SourceLoad { origin, .. } = &error else {
        panic!("expected a source load error, got {error:?}");
    };
    assert_eq!(origin, &source);
}

#[test]
fn direct_run_fails_with_source_load_when_the_ref_cannot_be_resolved() {
    let (repo, _) = source_repository(&[("app.yml", "v1\n")]);
    let source = repo.path().to_string_lossy().into_owned();
    let error = direct::run_command(direct_run(
        &source,
        Some("no-such-branch"),
        Some("app.yml"),
        &["app.yml"],
        repo.path(),
    ))
    .expect_err("an unresolvable ref fails the override run");
    let SyncsmithError::SourceLoad { origin, .. } = &error else {
        panic!("expected a source load error, got {error:?}");
    };
    assert!(
        origin.contains("no-such-branch"),
        "the error must name the unresolvable ref: {origin}"
    );
}

#[test]
fn direct_run_fails_with_source_load_when_a_ref_targets_a_non_git_path() {
    let plain = TempDir::new().expect("a plain folder source is created");
    let source = plain.path().to_string_lossy().into_owned();
    let error = direct::run_command(direct_run(
        &source,
        Some("main"),
        None,
        &["app.yml"],
        plain.path(),
    ))
    .expect_err("a ref on a non-git local path fails the override run");
    let SyncsmithError::SourceLoad { origin, .. } = &error else {
        panic!("expected a source load error, got {error:?}");
    };
    assert!(
        origin.contains("main"),
        "the error must name the requested ref: {origin}"
    );
}

#[test]
fn an_empty_override_source_is_a_usage_error_before_any_effect() {
    let target = git_repository();
    for source in ["", "   "] {
        let error =
            direct::run_command(direct_run(source, None, None, &["app.yml"], target.path()))
                .expect_err("an empty or blank source is a usage error");
        assert!(
            matches!(error, SyncsmithError::InvalidArguments { .. }),
            "expected a usage error, got {error:?}"
        );
    }
    // The rejection fires before any filesystem effect: nothing is written.
    assert!(!target.path().join("app.yml").exists());
}

// ---------------------------------------------------------------------------
// Binary: end-to-end behavior of the stateless run (spawned in the target)
// ---------------------------------------------------------------------------

#[test]
fn a_local_file_source_creates_the_target_without_configuration_or_lock() {
    let target = git_repository();
    // A relative source, resolved against the current directory (the stateless
    // context root), proving the cwd-based resolution end to end.
    write_file(target.path(), "import/latest.yml", "v1\n");

    let (code, stdout_payload, _) = syncsmith(
        &target,
        ["sync", "--override-from", "import/latest.yml", "app.yml"],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "app.yml\n");
    assert_eq!(read_target(&target, "app.yml"), "v1\n");
    assert!(!target.path().join(CONFIG_FILE_NAME).exists());
    assert!(!lock_path(&target).exists());
}

#[test]
fn the_json_flag_reports_the_override_changes_as_one_json_document() {
    let source = TempDir::new().expect("a temporary folder source is created");
    write_file(source.path(), "latest.yml", "v1\n");
    let source_file = source.path().join("latest.yml");
    let target = git_repository();

    let (code, stdout_payload, _) = syncsmith(
        &target,
        [
            "--json",
            "sync",
            "--override-from",
            &source_file.to_string_lossy(),
            "app.yml",
        ],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, r#"[{"path":"app.yml","action":"created"}]"#);
    assert_eq!(read_target(&target, "app.yml"), "v1\n");
}

#[test]
fn a_folder_source_mirrors_additions_updates_and_deletions() {
    let source = TempDir::new().expect("a temporary folder source is created");
    write_file(source.path(), "a.yml", "one\n");
    write_file(source.path(), "sub/b.yml", "two\n");
    let target = git_repository();

    let (code, stdout_payload, _) = syncsmith(
        &target,
        [
            "sync",
            "--override-from",
            &source.path().to_string_lossy(),
            "mirror",
        ],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "mirror/a.yml\nmirror/sub/b.yml\n");
    assert_eq!(read_target(&target, "mirror/a.yml"), "one\n");
    assert_eq!(read_target(&target, "mirror/sub/b.yml"), "two\n");

    write_file(source.path(), "a.yml", "one v2\n");
    write_file(source.path(), "c.yml", "three\n");
    fs::remove_file(source.path().join("sub/b.yml")).expect("the source removal is applied");

    let (code, stdout_payload, _) = syncsmith(
        &target,
        [
            "sync",
            "--override-from",
            &source.path().to_string_lossy(),
            "mirror",
        ],
    );

    assert_eq!(code, Some(0));
    // The same stdout contract as a configuration-driven folder entry:
    // changed paths in processing order, deletions included.
    assert_eq!(
        stdout_payload,
        "mirror/a.yml\nmirror/c.yml\nmirror/sub/b.yml\n"
    );
    assert_eq!(read_target(&target, "mirror/a.yml"), "one v2\n");
    assert_eq!(read_target(&target, "mirror/c.yml"), "three\n");
    assert!(!target.path().join("mirror/sub/b.yml").exists());
    assert!(!lock_path(&target).exists());
}

#[test]
fn a_local_repository_source_reads_the_committed_version_not_the_dirty_tree() {
    let (repo, version) = source_repository(&[("app.yml", "v1\n")]);
    // Dirty the working tree after the commit: the override must read the
    // committed content at the given ref.
    fs::write(repo.path().join("app.yml"), "dirty working tree\n")
        .expect("the dirty edit is written");
    let target = git_repository();

    let (code, stdout_payload, _) = syncsmith(
        &target,
        [
            "sync",
            "--override-from",
            &repo.path().to_string_lossy(),
            "--ref",
            &version,
            "--source-path",
            "app.yml",
            "app.yml",
        ],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "app.yml\n");
    assert_eq!(read_target(&target, "app.yml"), "v1\n");
    assert!(!lock_path(&target).exists());
}

#[test]
fn an_omitted_source_path_defaults_to_the_target_file_name_at_the_repository_root() {
    let (repo, version) = source_repository(&[("rules.yml", "root rules\n")]);
    let target = git_repository();

    // The nested target proves the default is the target's *file name*
    // searched at the repository root, not the full target path.
    let (code, stdout_payload, _) = syncsmith(
        &target,
        [
            "sync",
            "--override-from",
            &repo.path().to_string_lossy(),
            "--ref",
            &version,
            "config/rules.yml",
        ],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "config/rules.yml\n");
    assert_eq!(read_target(&target, "config/rules.yml"), "root rules\n");
}

#[test]
fn a_locally_edited_target_is_overwritten_and_an_existing_lock_is_left_identical() {
    let source = TempDir::new().expect("a temporary folder source is created");
    write_file(source.path(), "latest.yml", "v1\n");
    let source_file = source.path().join("latest.yml");
    let target = git_repository();
    let (code, _, _) = syncsmith(
        &target,
        [
            "sync",
            "--override-from",
            &source_file.to_string_lossy(),
            "app.yml",
        ],
    );
    assert_eq!(code, Some(0));

    // A lock file left by earlier configuration-driven runs must never be
    // read or written by the stateless flow.
    let lock_bytes: &[u8] =
        b"{\"app.yml\":{\"version\":\"0000000000000000000000000000000000000000\",\"content\":\"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391\"}}\n";
    fs::write(lock_path(&target), lock_bytes).expect("the lock file is written");
    fs::write(target.path().join("app.yml"), "local edit\n").expect("the local edit is written");

    let (code, stdout_payload, _) = syncsmith(
        &target,
        [
            "sync",
            "--override-from",
            &source_file.to_string_lossy(),
            "app.yml",
        ],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "app.yml\n");
    assert_eq!(read_target(&target, "app.yml"), "v1\n");
    assert_eq!(
        fs::read(lock_path(&target)).expect("the lock file is readable"),
        lock_bytes
    );
}

#[test]
fn an_existing_configuration_mapping_the_target_is_ignored() {
    // A configuration that maps the same target from a different source, in
    // patch mode: if it were loaded, the target would receive
    // "from the configuration\n" and a lock record would be written.
    let (config_source, _) = source_repository(&[("app.yml", "from the configuration\n")]);
    let override_source = TempDir::new().expect("a temporary folder source is created");
    write_file(override_source.path(), "latest.yml", "from the override\n");
    let override_file = override_source.path().join("latest.yml");
    let target = git_repository();
    write_config(
        &target,
        vec![SyncEntry {
            source: Source::Repository {
                repository: config_source.path().to_string_lossy().into_owned(),
                git_ref: None,
                path: "app.yml".into(),
            },
            target: "app.yml".into(),
            mode: Mode::Patch,
            policy: Policy::Automatic,
        }],
    );
    let config_path = target.path().join(CONFIG_FILE_NAME);

    let (code, stdout_payload, _) = syncsmith(
        &target,
        [
            "--config",
            &config_path.to_string_lossy(),
            "sync",
            "--override-from",
            &override_file.to_string_lossy(),
            "app.yml",
        ],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "app.yml\n");
    assert_eq!(read_target(&target, "app.yml"), "from the override\n");
    assert!(!lock_path(&target).exists());
}

#[test]
fn source_failures_exit_non_zero_leave_the_target_untouched_and_report_on_stderr() {
    let target = git_repository();
    write_file(target.path(), "app.yml", "keep\n");
    let (repo, _) = source_repository(&[("app.yml", "v1\n")]);
    let plain = TempDir::new().expect("a plain folder source is created");

    let cases: [Vec<String>; 3] = [
        // A missing local source, relative to the current directory.
        vec![
            "sync".into(),
            "--override-from".into(),
            "no-such-source.txt".into(),
            "app.yml".into(),
        ],
        // An unresolvable ref in a local git repository.
        vec![
            "sync".into(),
            "--override-from".into(),
            repo.path().to_string_lossy().into_owned(),
            "--ref".into(),
            "no-such-branch".into(),
            "--source-path".into(),
            "app.yml".into(),
            "app.yml".into(),
        ],
        // A ref on a local path that is not a git repository.
        vec![
            "sync".into(),
            "--override-from".into(),
            plain.path().to_string_lossy().into_owned(),
            "--ref".into(),
            "main".into(),
            "app.yml".into(),
        ],
    ];
    for case in &cases {
        let (code, stdout_payload, stderr_payload) = syncsmith(&target, case);
        assert_ne!(code, Some(0), "case {case:?} must fail");
        assert_eq!(stdout_payload, "", "a failed run reports no changed path");
        assert!(
            stderr_payload.contains("failed to load source"),
            "case {case:?} must report a source load error: {stderr_payload}"
        );
        assert_eq!(
            read_target(&target, "app.yml"),
            "keep\n",
            "case {case:?} must leave the target untouched"
        );
    }
}

#[test]
fn target_counts_other_than_one_are_usage_errors() {
    let source = TempDir::new().expect("a temporary folder source is created");
    write_file(source.path(), "latest.yml", "v1\n");
    let source_file = source.path().join("latest.yml");
    let target = git_repository();

    let cases: [Vec<String>; 2] = [
        vec![
            "sync".into(),
            "--override-from".into(),
            source_file.to_string_lossy().into_owned(),
        ],
        vec![
            "sync".into(),
            "--override-from".into(),
            source_file.to_string_lossy().into_owned(),
            "a.yml".into(),
            "b.yml".into(),
        ],
    ];
    for case in &cases {
        let (code, stdout_payload, stderr_payload) = syncsmith(&target, case);
        assert_ne!(code, Some(0), "case {case:?} is a usage error");
        assert_eq!(stdout_payload, "", "a usage error reports nothing");
        assert!(
            stderr_payload.contains("exactly one target"),
            "the error must explain the single-target rule: {stderr_payload}"
        );
    }
    assert!(!target.path().join("a.yml").exists());
    assert!(!target.path().join("b.yml").exists());
}

#[test]
fn absolute_and_escaping_targets_are_usage_errors() {
    let source = TempDir::new().expect("a temporary folder source is created");
    write_file(source.path(), "latest.yml", "v1\n");
    let source_file = source.path().join("latest.yml");
    let target = git_repository();
    let absolute = target.path().join("app.yml");

    let cases: [Vec<String>; 2] = [
        vec![
            "sync".into(),
            "--override-from".into(),
            source_file.to_string_lossy().into_owned(),
            absolute.to_string_lossy().into_owned(),
        ],
        vec![
            "sync".into(),
            "--override-from".into(),
            source_file.to_string_lossy().into_owned(),
            "../escape.yml".into(),
        ],
    ];
    for case in &cases {
        let (code, stdout_payload, stderr_payload) = syncsmith(&target, case);
        assert_ne!(code, Some(0), "case {case:?} is a usage error");
        assert_eq!(stdout_payload, "", "a usage error reports nothing");
        assert!(
            stderr_payload.contains("invalid target"),
            "the error must name the rejected target: {stderr_payload}"
        );
    }
    assert!(!absolute.exists());
}

#[test]
fn an_empty_override_source_fails_from_the_binary() {
    let target = git_repository();
    let (code, stdout_payload, stderr_payload) =
        syncsmith(&target, ["sync", "--override-from", "", "app.yml"]);
    assert_ne!(code, Some(0), "an empty source is a usage error");
    assert_eq!(stdout_payload, "", "a usage error reports nothing");
    assert!(
        stderr_payload.contains("--override-from"),
        "the error must name the empty flag: {stderr_payload}"
    );
    assert!(!target.path().join("app.yml").exists());
}

#[test]
fn the_target_resolves_against_the_repository_root_without_a_configuration() {
    let source = TempDir::new().expect("a temporary folder source is created");
    write_file(source.path(), "latest.yml", "v1\n");
    let source_file = source.path().join("latest.yml");
    let target = git_repository();
    let nested = target.path().join("nested");
    fs::create_dir_all(&nested).expect("the subdirectory of the target repository is created");

    let (code, stdout_payload, _) = syncsmith_from(
        &nested,
        [
            "sync",
            "--override-from",
            &source_file.to_string_lossy(),
            "app.yml",
        ],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "app.yml\n");
    assert_eq!(read_target(&target, "app.yml"), "v1\n");
    assert!(
        !target.path().join("nested/app.yml").exists(),
        "without a configuration file the target lands at the repository root"
    );
}

#[test]
fn the_target_resolves_against_the_configuration_directory() {
    // A configuration in a nested directory: its location (not its content)
    // defines the base. It maps the same target from another source, in
    // patch mode — if it were loaded, the target would receive
    // "from the configuration" and a lock record would be written.
    let (config_source, _) = source_repository(&[("app.yml", "from the configuration\n")]);
    let override_source = TempDir::new().expect("a temporary folder source is created");
    write_file(override_source.path(), "latest.yml", "from the override\n");
    let override_file = override_source.path().join("latest.yml");
    let target = git_repository();
    let nested = target.path().join("nested");
    fs::create_dir_all(&nested).expect("the nested configuration directory is created");
    let entries = vec![SyncEntry {
        source: Source::Repository {
            repository: config_source.path().to_string_lossy().into_owned(),
            git_ref: None,
            path: "app.yml".into(),
        },
        target: "app.yml".into(),
        mode: Mode::Patch,
        policy: Policy::Automatic,
    }];
    let text = toml::to_string(&Config { sync: entries }).expect("the configuration serializes");
    let config_path = nested.join(CONFIG_FILE_NAME);
    fs::write(&config_path, text).expect("the configuration file is written");

    let (code, stdout_payload, _) = syncsmith(
        &target,
        [
            "--config",
            &config_path.to_string_lossy(),
            "sync",
            "--override-from",
            &override_file.to_string_lossy(),
            "app.yml",
        ],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "app.yml\n");
    assert_eq!(
        read_target(&target, "nested/app.yml"),
        "from the override\n"
    );
    assert!(
        !target.path().join("app.yml").exists(),
        "the target must land in the configuration directory, not the repository root"
    );
    // The configuration was located but never loaded: no lock file anywhere.
    assert!(!lock_path(&target).exists());
    assert!(!nested.join(LOCK_FILE_NAME).exists());
}

#[test]
fn the_target_resolves_against_the_current_directory_outside_any_repository() {
    let source = TempDir::new().expect("a temporary folder source is created");
    write_file(source.path(), "latest.yml", "v1\n");
    let source_file = source.path().join("latest.yml");
    let plain = TempDir::new().expect("a plain folder without a repository is created");

    let (code, stdout_payload, _) = syncsmith_from(
        plain.path(),
        [
            "sync",
            "--override-from",
            &source_file.to_string_lossy(),
            "app.yml",
        ],
    );

    assert_eq!(code, Some(0));
    assert_eq!(stdout_payload, "app.yml\n");
    assert_eq!(
        fs::read_to_string(plain.path().join("app.yml")).expect("the target file is readable"),
        "v1\n"
    );
}
