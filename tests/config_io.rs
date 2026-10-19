//! Integration tests for configuration loading and atomic saving.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use tempfile::TempDir;

use syncsmith::SyncsmithError;
use syncsmith::config::io::{load, save_atomic};
use syncsmith::config::{Config, Mode, Policy, Source, SyncEntry};

const VALID_CONFIG: &str = r#"
[[sync]]
source = { repository = "https://example.com/rules.git", ref = "main", path = "lints/base.yml" }
target = "lints/base.yml"
mode = "patch"
policy = "automatic"

[[sync]]
source = { folder = "${HOME}/dotfiles" }
target = "docs/aliases"
mode = "overwrite"
policy = "manual"
"#;

/// Creates a temporary directory with the path of a `.syncsmith.toml` inside.
fn config_file() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("temporary directory is created");
    let path = dir.path().join(".syncsmith.toml");
    (dir, path)
}

fn sample_config() -> Config {
    Config {
        sync: vec![
            SyncEntry {
                source: Source::Repository {
                    repository: "https://example.com/rules.git".into(),
                    git_ref: Some("main".into()),
                    path: "lints/base.yml".into(),
                },
                target: "lints/base.yml".into(),
                mode: Mode::Patch,
                policy: Policy::Automatic,
            },
            SyncEntry {
                source: Source::Folder {
                    folder: "rules/shared".into(),
                },
                target: "docs/shared".into(),
                mode: Mode::Overwrite,
                policy: Policy::Manual,
            },
        ],
    }
}

#[test]
fn load_parses_repository_and_folder_entries_without_env_expansion() {
    let (_dir, path) = config_file();
    fs::write(&path, VALID_CONFIG).expect("configuration file is written");

    let config = load(&path).expect("valid configuration loads");

    assert_eq!(
        config.sync[0].source,
        Source::Repository {
            repository: "https://example.com/rules.git".into(),
            git_ref: Some("main".into()),
            path: "lints/base.yml".into(),
        }
    );
    // `load` returns the raw file content: env references stay unexpanded.
    assert_eq!(
        config.sync[1].source,
        Source::Folder {
            folder: "${HOME}/dotfiles".into(),
        }
    );
    assert_eq!(
        (config.sync[1].mode, config.sync[1].policy),
        (Mode::Overwrite, Policy::Manual)
    );
}

#[test]
fn load_missing_file_fails_with_config_parse_error_carrying_the_path() {
    let (dir, _path) = config_file();

    let error = load(&dir.path().join("absent.toml")).expect_err("missing file fails to load");

    let SyncsmithError::ConfigParse { path: reported, .. } = error else {
        panic!("expected ConfigParse, got {error:?}")
    };
    assert_eq!(reported, dir.path().join("absent.toml"));
}

#[test]
fn load_invalid_toml_fails_with_config_parse_error() {
    let (_dir, path) = config_file();
    fs::write(&path, "sync = 3\n").expect("configuration file is written");

    let error = load(&path).expect_err("invalid content fails to load");

    assert!(matches!(error, SyncsmithError::ConfigParse { .. }));
}

#[test]
fn save_atomic_round_trips_and_leaves_no_temporary_file() {
    let (dir, path) = config_file();
    let config = sample_config();

    save_atomic(&path, &config).expect("save succeeds");

    assert!(!dir.path().join(".syncsmith.toml.tmp").exists());
    assert_eq!(load(&path).expect("saved configuration loads"), config);
}

#[test]
fn save_atomic_replaces_the_content_of_an_existing_config() {
    let (dir, path) = config_file();
    fs::write(&path, VALID_CONFIG).expect("previous configuration is written");
    let replacement = sample_config();

    save_atomic(&path, &replacement).expect("save succeeds");

    assert!(!dir.path().join(".syncsmith.toml.tmp").exists());
    assert_eq!(
        load(&path).expect("replaced configuration loads"),
        replacement
    );
}

#[test]
fn save_atomic_writes_paths_with_forward_slashes() {
    let (_dir, path) = config_file();
    let config = Config {
        sync: vec![SyncEntry {
            source: Source::Folder {
                folder: r"C:\sources\rules".into(),
            },
            target: r"lints\base.yml".into(),
            mode: Mode::Overwrite,
            policy: Policy::Automatic,
        }],
    };

    save_atomic(&path, &config).expect("save succeeds");

    let written = fs::read_to_string(&path).expect("saved configuration is readable");
    assert!(written.contains(r#"folder = "C:/sources/rules""#));
    assert!(written.contains(r#"target = "lints/base.yml""#));
    assert!(!written.contains('\\'));
}

#[test]
fn save_atomic_failure_in_a_read_only_directory_keeps_the_original_config() {
    let (dir, path) = config_file();
    fs::write(&path, "original = \"content\"\n").expect("original configuration is written");

    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555))
        .expect("directory is made read-only");
    let result = save_atomic(&path, &sample_config());
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755))
        .expect("directory permissions are restored");

    assert!(matches!(result, Err(SyncsmithError::ConfigWrite { .. })));
    assert_eq!(
        fs::read_to_string(&path).expect("original configuration is readable"),
        "original = \"content\"\n"
    );
    assert!(!dir.path().join(".syncsmith.toml.tmp").exists());
}

#[test]
fn save_atomic_failure_on_a_directory_destination_removes_the_temporary_file() {
    let (dir, _path) = config_file();
    let destination = dir.path().join("as-a-directory");
    fs::create_dir(&destination).expect("destination directory is created");

    let result = save_atomic(&destination, &sample_config());

    assert!(matches!(result, Err(SyncsmithError::ConfigWrite { .. })));
    assert!(!dir.path().join("as-a-directory.tmp").exists());
}
