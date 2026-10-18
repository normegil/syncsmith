//! Integration tests for configuration discovery and repository detection.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use common::enter;
use tempfile::TempDir;

use syncsmith::cli::GlobalArgs;
use syncsmith::config::CONFIG_FILE_NAME;
use syncsmith::discovery::{Resolution, find_config, resolve_config_path};
use syncsmith::git::repo_root;

/// Serializes the tests that change the process working directory.
static CWD_LOCK: Mutex<()> = Mutex::new(());

fn write_config(dir: &Path) {
    fs::write(dir.join(CONFIG_FILE_NAME), "sync = []\n").expect("configuration file is written");
}

fn make_git_pointer(dir: &Path) {
    fs::create_dir_all(dir).expect("directory is created");
    fs::write(dir.join(".git"), "gitdir: /nowhere\n").expect(".git pointer file is written");
}

#[test]
fn find_config_walks_up_until_it_finds_a_config_file() {
    let root = TempDir::new().expect("temporary root is created");
    let deep = root.path().join("a").join("b");
    fs::create_dir_all(&deep).expect("nested directories are created");
    write_config(root.path());

    assert_eq!(find_config(&deep), Some(root.path().join(CONFIG_FILE_NAME)));
}

#[test]
fn find_config_finds_a_config_placed_at_the_repository_root() {
    let repo = TempDir::new().expect("temporary repository is created");
    fs::create_dir(repo.path().join(".git")).expect(".git directory is created");
    let inner = repo.path().join("inner");
    fs::create_dir_all(&inner).expect("inner directory is created");
    write_config(repo.path());

    assert_eq!(
        find_config(&inner),
        Some(repo.path().join(CONFIG_FILE_NAME))
    );
}

#[test]
fn find_config_stops_at_a_repository_root_pointed_to_by_a_git_file() {
    let outer = TempDir::new().expect("temporary outer directory is created");
    write_config(outer.path());
    let work = outer.path().join("work");
    make_git_pointer(&work);
    let nested = work.join("nested");
    fs::create_dir_all(&nested).expect("nested directory is created");

    // The config above the (fake) repository root must not be picked up.
    assert_eq!(find_config(&work), None);
    assert_eq!(find_config(&nested), None);
}

#[test]
fn repo_root_detects_a_repository_pointed_to_by_a_git_file() {
    let outer = TempDir::new().expect("temporary outer directory is created");
    let work = outer.path().join("work");
    make_git_pointer(&work);
    let nested = work.join("nested");
    fs::create_dir_all(&nested).expect("nested directory is created");

    assert_eq!(repo_root(&nested), Some(work.clone()));
    assert_eq!(repo_root(&work), Some(work));
}

#[test]
fn repo_root_walks_up_to_the_enclosing_git_directory() {
    let repo = TempDir::new().expect("temporary repository is created");
    fs::create_dir(repo.path().join(".git")).expect(".git directory is created");
    let inner = repo.path().join("a").join("b");
    fs::create_dir_all(&inner).expect("inner directories are created");

    assert_eq!(repo_root(&inner), Some(repo.path().to_path_buf()));
}

#[test]
fn repo_root_returns_none_outside_any_repository() {
    let dir = TempDir::new().expect("temporary directory is created");

    assert_eq!(repo_root(dir.path()), None);
}

#[test]
fn resolve_config_path_reports_the_existence_of_an_explicit_path() {
    let dir = TempDir::new().expect("temporary directory is created");
    let existing = dir.path().join("custom.toml");
    fs::write(&existing, "sync = []\n").expect("configuration file is written");

    let found = resolve_config_path(&GlobalArgs {
        config: Some(existing.clone()),
        verbose: 0,
        json: false,
    })
    .expect("explicit path resolves");

    assert_eq!(
        found,
        Resolution {
            path: existing,
            exists: true
        }
    );

    let missing = dir.path().join("absent.toml");
    let resolution = resolve_config_path(&GlobalArgs {
        config: Some(missing.clone()),
        verbose: 0,
        json: false,
    })
    .expect("explicit path resolves");

    assert_eq!(
        resolution,
        Resolution {
            path: missing,
            exists: false
        }
    );
}

#[test]
fn resolve_config_path_without_config_targets_the_repository_root() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = TempDir::new().expect("temporary repository is created");
    fs::create_dir(repo.path().join(".git")).expect(".git directory is created");
    let inner = repo.path().join("inner");
    fs::create_dir_all(&inner).expect("inner directory is created");
    let _cwd = enter(&inner);

    let resolution = resolve_config_path(&GlobalArgs {
        config: None,
        verbose: 0,
        json: false,
    })
    .expect("resolution succeeds inside a repository");

    assert_eq!(
        resolution,
        Resolution {
            path: repo.path().join(CONFIG_FILE_NAME),
            exists: false,
        }
    );
}

#[test]
fn resolve_config_path_outside_any_repository_targets_the_cwd() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = TempDir::new().expect("temporary directory is created");
    let _cwd = enter(dir.path());

    let resolution = resolve_config_path(&GlobalArgs {
        config: None,
        verbose: 0,
        json: false,
    })
    .expect("resolution succeeds outside any repository");

    assert_eq!(
        resolution,
        Resolution {
            path: dir.path().join(CONFIG_FILE_NAME),
            exists: false,
        }
    );
}
