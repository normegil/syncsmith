//! End-to-end integration tests of the interactive `init` flow, driven by a
//! scripted prompter against real temporary repositories, plus
//! filesystem-backed source-loading tests.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use common::{Answer, ScriptedPrompter, enter, git};
use tempfile::TempDir;

use syncsmith::cli::GlobalArgs;
use syncsmith::config::CONFIG_FILE_NAME;
use syncsmith::config::Config;
use syncsmith::config::io::load;
use syncsmith::config::{Mode, Policy, Source, SyncEntry};
use syncsmith::error::SyncsmithError;
use syncsmith::init::{ExitState, FlowFs, SourceLoader, SourceRef, TreeItem, run_with};

/// Serializes the tests that change the process working directory.
static CWD_LOCK: Mutex<()> = Mutex::new(());

const EXISTING_CONFIG: &str = r#"
[[sync]]
source = { folder = "rules" }
target = "existing.yml"
mode = "overwrite"
policy = "manual"
"#;

fn existing_entry() -> SyncEntry {
    SyncEntry {
        source: Source::Folder {
            folder: "rules".into(),
        },
        target: "existing.yml".into(),
        mode: Mode::Overwrite,
        policy: Policy::Manual,
    }
}

fn target_repository() -> TempDir {
    let repo = TempDir::new().expect("temporary target repository is created");
    git(repo.path(), &["init"]);
    repo
}

fn write_source_file(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    let parent = path.parent().expect("source files live in a directory");
    fs::create_dir_all(parent).expect("source directory is created");
    fs::write(&path, content).expect("source file is written");
}

fn folder_source(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

fn load_written_config(repo: &Path) -> Config {
    load(&repo.join(CONFIG_FILE_NAME)).expect("written configuration loads")
}

fn item(relative: &str, is_dir: bool) -> TreeItem {
    TreeItem {
        relative: PathBuf::from(relative),
        is_dir,
    }
}

#[test]
fn run_with_writes_mirrored_folder_targets_and_exits_success() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let source = TempDir::new().expect("temporary source folder is created");
    write_source_file(source.path(), "a.yml", "alpha");
    write_source_file(source.path(), "lints/base.yml", "base");
    let entered = folder_source(source.path());
    let _cwd = enter(repo.path());

    let prompter = ScriptedPrompter::new(vec![
        Answer::Confirm(true),
        Answer::Select("Local folder".into()),
        Answer::Text(entered.clone()),
        Answer::Multi(vec!["a.yml".into(), "lints (folder)".into()]),
        Answer::TextDefault("a.yml".into()),
        Answer::TextDefault("lints".into()),
        // Folder sources only offer overwrite mode.
        Answer::Select("Overwrite".into()),
        Answer::Select("Automatic".into()),
        Answer::Confirm(false),
        Answer::Select("Validate".into()),
        Answer::Confirm(false),
    ]);

    let state = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    )
    .expect("init completes with the final synchronization");

    assert_eq!(state, ExitState::Completed);
    assert_eq!(
        load_written_config(repo.path()).sync,
        vec![
            SyncEntry {
                source: Source::Folder {
                    folder: PathBuf::from(entered.as_str()).join("a.yml"),
                },
                target: "a.yml".into(),
                mode: Mode::Overwrite,
                policy: Policy::Automatic,
            },
            SyncEntry {
                source: Source::Folder {
                    folder: PathBuf::from(entered.as_str()).join("lints"),
                },
                target: "lints".into(),
                mode: Mode::Overwrite,
                policy: Policy::Automatic,
            },
        ]
    );
    // The final synchronization ran against the written entries.
    assert_eq!(
        fs::read_to_string(repo.path().join("a.yml")).expect("synchronized target is readable"),
        "alpha"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("lints").join("base.yml"))
            .expect("synchronized target is readable"),
        "base"
    );
    assert!(!repo.path().join(format!("{CONFIG_FILE_NAME}.tmp")).exists());
}

#[test]
fn run_with_add_choice_appends_entries_and_installs_the_hook() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    fs::write(repo.path().join(CONFIG_FILE_NAME), EXISTING_CONFIG)
        .expect("existing configuration is written");
    let source = TempDir::new().expect("temporary source folder is created");
    write_source_file(source.path(), "b.yml", "bravo");
    let entered = folder_source(source.path());
    let _cwd = enter(repo.path());

    let prompter = ScriptedPrompter::new(vec![
        Answer::Select("Add".into()),
        Answer::Confirm(true),
        Answer::Select("Local folder".into()),
        Answer::Text(entered.clone()),
        Answer::Multi(vec!["b.yml".into()]),
        Answer::TextDefault("b.yml".into()),
        Answer::Select("Overwrite".into()),
        Answer::Select("Manual".into()),
        Answer::Confirm(false),
        Answer::Select("Validate".into()),
        Answer::Confirm(true),
    ]);

    let state = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    )
    .expect("init completes");

    assert_eq!(state, ExitState::Completed);
    assert_eq!(
        load_written_config(repo.path()).sync,
        vec![
            existing_entry(),
            SyncEntry {
                source: Source::Folder {
                    folder: PathBuf::from(entered.as_str()).join("b.yml"),
                },
                target: "b.yml".into(),
                mode: Mode::Overwrite,
                policy: Policy::Manual,
            },
        ]
    );
    assert!(
        repo.path()
            .join(".git")
            .join("hooks")
            .join("pre-commit")
            .is_file()
    );
}

#[test]
fn run_with_restart_choice_replaces_existing_entries() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    fs::write(repo.path().join(CONFIG_FILE_NAME), EXISTING_CONFIG)
        .expect("existing configuration is written");
    // A source folder inside the repository, referenced by a relative path.
    write_source_file(repo.path(), "source/c.yml", "charlie");
    let _cwd = enter(repo.path());

    let prompter = ScriptedPrompter::new(vec![
        Answer::Select("Restart".into()),
        Answer::Confirm(true),
        Answer::Select("Local folder".into()),
        // After a restart the location prompt is prefilled with the last
        // existing entry's source ("rules" here).
        Answer::TextDefault("source".into()),
        Answer::Multi(vec!["c.yml".into()]),
        Answer::TextDefault("c.yml".into()),
        // Folder sources only offer overwrite mode.
        Answer::Select("Overwrite".into()),
        Answer::Select("Automatic".into()),
        Answer::Confirm(false),
        Answer::Select("Validate".into()),
        Answer::Confirm(false),
    ]);

    let state = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    )
    .expect("init completes");

    assert_eq!(state, ExitState::Completed);
    assert_eq!(
        load_written_config(repo.path()).sync,
        vec![SyncEntry {
            source: Source::Folder {
                folder: PathBuf::from("source").join("c.yml"),
            },
            target: "c.yml".into(),
            mode: Mode::Overwrite,
            policy: Policy::Automatic,
        }]
    );
}

#[test]
fn run_with_abort_choice_leaves_the_existing_configuration_unchanged() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    fs::write(repo.path().join(CONFIG_FILE_NAME), EXISTING_CONFIG)
        .expect("existing configuration is written");
    let _cwd = enter(repo.path());

    let prompter = ScriptedPrompter::new(vec![Answer::Select("Abort".into())]);

    let result = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    );

    assert!(matches!(result, Err(SyncsmithError::Aborted)));
    assert_eq!(
        fs::read_to_string(repo.path().join(CONFIG_FILE_NAME))
            .expect("existing configuration is readable"),
        EXISTING_CONFIG
    );
    assert!(!repo.path().join(format!("{CONFIG_FILE_NAME}.tmp")).exists());
}

#[test]
fn run_with_aborts_on_an_interrupted_prompt_without_writing_a_config() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let _cwd = enter(repo.path());

    let prompter = ScriptedPrompter::new(vec![Answer::Interrupt]);

    let result = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    );

    assert!(matches!(result, Err(SyncsmithError::Aborted)));
    assert!(!repo.path().join(CONFIG_FILE_NAME).exists());
}

#[test]
fn run_with_reprompts_until_the_target_stays_inside_the_repository() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let source = TempDir::new().expect("temporary source folder is created");
    write_source_file(source.path(), "x.yml", "x-ray");
    let entered = folder_source(source.path());
    let _cwd = enter(repo.path());

    let prompter = ScriptedPrompter::new(vec![
        Answer::Confirm(true),
        Answer::Select("Local folder".into()),
        Answer::Text(entered.clone()),
        Answer::Multi(vec!["x.yml".into()]),
        Answer::TextDefault("../escape.yml".into()),
        Answer::TextDefault("renamed.yml".into()),
        // Folder sources only offer overwrite mode.
        Answer::Select("Overwrite".into()),
        Answer::Select("Automatic".into()),
        Answer::Confirm(false),
        Answer::Select("Validate".into()),
        Answer::Confirm(false),
    ]);

    let state = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    )
    .expect("init completes after a valid target is given");

    assert_eq!(state, ExitState::Completed);
    assert_eq!(
        load_written_config(repo.path()).sync,
        vec![SyncEntry {
            source: Source::Folder {
                folder: PathBuf::from(entered.as_str()).join("x.yml"),
            },
            target: "renamed.yml".into(),
            mode: Mode::Overwrite,
            policy: Policy::Automatic,
        }]
    );
}

#[test]
fn run_with_rejects_an_existing_config_with_an_escaping_target_before_prompting() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let escaping = r#"
[[sync]]
source = { folder = "rules" }
target = "../escape.yml"
mode = "overwrite"
policy = "manual"
"#;
    fs::write(repo.path().join(CONFIG_FILE_NAME), escaping)
        .expect("escaping configuration is written");
    let _cwd = enter(repo.path());

    // The invalid target must be rejected before any prompt: the empty script
    // makes any prompt panic the test.
    let prompter = ScriptedPrompter::new(vec![]);

    let result = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    );

    let Err(SyncsmithError::InvalidTarget { target, .. }) = result else {
        panic!("expected InvalidTarget, got {result:?}")
    };
    assert_eq!(target, "../escape.yml");
    assert_eq!(
        fs::read_to_string(repo.path().join(CONFIG_FILE_NAME))
            .expect("original configuration is readable"),
        escaping
    );
    assert!(!repo.path().join(format!("{CONFIG_FILE_NAME}.tmp")).exists());
}

#[test]
fn run_with_restart_declining_an_empty_config_keeps_the_original_configuration() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    fs::write(repo.path().join(CONFIG_FILE_NAME), EXISTING_CONFIG)
        .expect("existing configuration is written");
    let _cwd = enter(repo.path());

    // Restart discards the entries; declining to re-add a source leaves an
    // empty configuration, which the user then declines to write.
    let prompter = ScriptedPrompter::new(vec![
        Answer::Select("Restart".into()),
        Answer::Confirm(false),
        Answer::Confirm(false),
    ]);

    let result = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    );

    assert!(matches!(result, Err(SyncsmithError::Aborted)));
    assert_eq!(
        fs::read_to_string(repo.path().join(CONFIG_FILE_NAME))
            .expect("original configuration is readable"),
        EXISTING_CONFIG
    );
    assert!(!repo.path().join(format!("{CONFIG_FILE_NAME}.tmp")).exists());
}

#[test]
fn run_with_fresh_declining_an_empty_config_writes_no_configuration() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let _cwd = enter(repo.path());

    // No source added; declining to write the empty configuration must not
    // create the file at all.
    let prompter = ScriptedPrompter::new(vec![Answer::Confirm(false), Answer::Confirm(false)]);

    let result = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    );

    assert!(matches!(result, Err(SyncsmithError::Aborted)));
    assert!(!repo.path().join(CONFIG_FILE_NAME).exists());
}

#[test]
fn run_with_restart_prefills_the_source_prompts_from_the_discarded_git_entry() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let origin = target_repository();
    write_source_file(origin.path(), "kept.yml", "kept");
    // The final synchronization reads the source at the pinned ref, so the
    // origin must actually carry `kept.yml` at `stable`.
    git(origin.path(), &["add", "kept.yml"]);
    git(origin.path(), &["commit", "-m", "keep"]);
    git(origin.path(), &["branch", "stable"]);
    let entered = folder_source(origin.path());
    let existing = format!(
        r#"
[[sync]]
source = {{ repository = "{entered}", ref = "stable", path = "old.yml" }}
target = "old.yml"
mode = "patch"
policy = "manual"
"#
    );
    fs::write(repo.path().join(CONFIG_FILE_NAME), existing)
        .expect("existing configuration is written");
    let _cwd = enter(repo.path());

    // After a restart, the location and ref prompts are text_with_default
    // prefilled from the discarded git entry; accepting both defaults (the
    // answers below) replays its repository and ref.
    let prompter = ScriptedPrompter::new(vec![
        Answer::Select("Restart".into()),
        Answer::Confirm(true),
        Answer::Select("Git repository".into()),
        Answer::TextDefault(entered.clone()),
        Answer::TextDefault("stable".into()),
        Answer::Multi(vec!["kept.yml".into()]),
        Answer::TextDefault("kept.yml".into()),
        Answer::Select("Overwrite".into()),
        Answer::Select("Automatic".into()),
        Answer::Confirm(false),
        Answer::Select("Validate".into()),
        Answer::Confirm(false),
    ]);

    let state = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    )
    .expect("init completes");

    assert_eq!(state, ExitState::Completed);
    let defaults = prompter.text_defaults();
    assert!(
        defaults
            .iter()
            .any(|(message, default)| default == &entered && message.contains("Repository"))
    );
    assert!(
        defaults
            .iter()
            .any(|(message, default)| default.as_str() == "stable"
                && message.contains("Repository reference"))
    );
    assert_eq!(
        load_written_config(repo.path()).sync,
        vec![SyncEntry {
            source: Source::Repository {
                repository: entered,
                git_ref: Some("stable".into()),
                path: "kept.yml".into(),
            },
            target: "kept.yml".into(),
            mode: Mode::Overwrite,
            policy: Policy::Automatic,
        }]
    );
}

#[test]
fn run_with_empty_selection_continue_skips_the_source() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let skipped = TempDir::new().expect("temporary skipped source is created");
    write_source_file(skipped.path(), "a.yml", "alpha");
    let kept = TempDir::new().expect("temporary kept source is created");
    write_source_file(kept.path(), "b.yml", "bravo");
    let skipped_entered = folder_source(skipped.path());
    let kept_entered = folder_source(kept.path());
    let _cwd = enter(repo.path());

    // Multi(empty) then Confirm(true): the first source is skipped; the
    // second source is added normally, so only its entry is written.
    let prompter = ScriptedPrompter::new(vec![
        Answer::Confirm(true),
        Answer::Select("Local folder".into()),
        Answer::Text(skipped_entered),
        Answer::Multi(Vec::new()),
        Answer::Confirm(true),
        Answer::Confirm(true),
        Answer::Select("Local folder".into()),
        Answer::Text(kept_entered.clone()),
        Answer::Multi(vec!["b.yml".into()]),
        Answer::TextDefault("b.yml".into()),
        Answer::Select("Overwrite".into()),
        Answer::Select("Automatic".into()),
        Answer::Confirm(false),
        Answer::Select("Validate".into()),
        Answer::Confirm(false),
    ]);

    let state = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    )
    .expect("init completes with the kept source");

    assert_eq!(state, ExitState::Completed);
    assert_eq!(
        load_written_config(repo.path()).sync,
        vec![SyncEntry {
            source: Source::Folder {
                folder: PathBuf::from(kept_entered.as_str()).join("b.yml"),
            },
            target: "b.yml".into(),
            mode: Mode::Overwrite,
            policy: Policy::Automatic,
        }]
    );
}

#[test]
fn run_with_review_add_another_entry_writes_both_sources() {
    let _lock = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let repo = target_repository();
    let source = TempDir::new().expect("temporary source folder is created");
    write_source_file(source.path(), "a.yml", "alpha");
    write_source_file(source.path(), "b.yml", "bravo");
    let entered = folder_source(source.path());
    let _cwd = enter(repo.path());

    // After the first source, the review action `Add another entry` runs
    // the full add-source flow again before `Validate` ends the loop.
    let prompter = ScriptedPrompter::new(vec![
        Answer::Confirm(true),
        Answer::Select("Local folder".into()),
        Answer::Text(entered.clone()),
        Answer::Multi(vec!["a.yml".into()]),
        Answer::TextDefault("a.yml".into()),
        Answer::Select("Overwrite".into()),
        Answer::Select("Automatic".into()),
        Answer::Confirm(false),
        Answer::Select("Add another entry".into()),
        Answer::Select("Local folder".into()),
        Answer::Text(entered.clone()),
        Answer::Multi(vec!["b.yml".into()]),
        Answer::TextDefault("b.yml".into()),
        Answer::Select("Overwrite".into()),
        Answer::Select("Automatic".into()),
        Answer::Select("Validate".into()),
        Answer::Confirm(false),
    ]);

    let state = run_with(
        &prompter,
        &GlobalArgs {
            config: None,
            verbose: 0,
            json: false,
        },
    )
    .expect("init completes with both sources");

    assert_eq!(state, ExitState::Completed);
    assert_eq!(
        load_written_config(repo.path()).sync,
        vec![
            SyncEntry {
                source: Source::Folder {
                    folder: PathBuf::from(entered.as_str()).join("a.yml"),
                },
                target: "a.yml".into(),
                mode: Mode::Overwrite,
                policy: Policy::Automatic,
            },
            SyncEntry {
                source: Source::Folder {
                    folder: PathBuf::from(entered.as_str()).join("b.yml"),
                },
                target: "b.yml".into(),
                mode: Mode::Overwrite,
                policy: Policy::Automatic,
            },
        ]
    );
    // The final synchronization ran both automatic entries.
    assert_eq!(
        fs::read_to_string(repo.path().join("a.yml")).expect("synchronized target is readable"),
        "alpha"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join("b.yml")).expect("synchronized target is readable"),
        "bravo"
    );
}

#[cfg(unix)]
#[test]
fn source_loader_walk_skips_git_entries_and_symlinks() {
    use std::os::unix::fs::symlink;

    let source = TempDir::new().expect("temporary source folder is created");
    write_source_file(source.path(), "real.yml", "real");
    write_source_file(source.path(), "sub/nested.yml", "nested");
    write_source_file(source.path(), ".git/config", "ignored by the walk");
    symlink(
        source.path().join("real.yml"),
        source.path().join("link.yml"),
    )
    .expect("file symlink is created");
    symlink(source.path().join("sub"), source.path().join("sub-link"))
        .expect("directory symlink is created");

    let scratch = TempDir::new().expect("temporary target root is created");
    let loader = SourceLoader::new(scratch.path().to_path_buf(), scratch.path().to_path_buf());
    let tree = loader
        .load_source(&SourceRef::Folder {
            folder: folder_source(source.path()),
        })
        .expect("source loads");

    assert_eq!(
        tree.items,
        vec![
            item("real.yml", false),
            item("sub", true),
            item("sub/nested.yml", false),
        ]
    );
}

#[test]
fn source_loader_walk_is_depth_bounded() {
    let source = TempDir::new().expect("temporary source folder is created");
    fs::write(source.path().join("f0.txt"), "depth 0").expect("root file is written");
    let mut current = source.path().to_path_buf();
    for depth in 1..=20 {
        current = current.join(format!("d{depth}"));
        fs::create_dir(&current).expect("nested directory is created");
        fs::write(current.join(format!("f{depth}.txt")), "nested").expect("nested file is written");
    }

    let scratch = TempDir::new().expect("temporary target root is created");
    let loader = SourceLoader::new(scratch.path().to_path_buf(), scratch.path().to_path_buf());
    let tree = loader
        .load_source(&SourceRef::Folder {
            folder: folder_source(source.path()),
        })
        .expect("source loads");

    // Items are enumerated down to depth 16 and their children: f0..f16 and
    // d1..d17; nothing deeper is walked.
    assert_eq!(tree.items.len(), 34);
    assert!(tree.items.contains(&item(&chain(17), true)));
    assert!(
        tree.items
            .iter()
            .all(|entry| entry.relative.components().count() <= 17)
    );
    assert!(
        !tree
            .items
            .iter()
            .any(|entry| entry.relative.starts_with(chain(18)))
    );
}

fn chain(depth: usize) -> String {
    (1..=depth)
        .map(|level| format!("d{level}"))
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn source_loader_walks_a_local_git_path_directly() {
    let repo = target_repository();
    write_source_file(repo.path(), "rules.yml", "rules");

    let scratch = TempDir::new().expect("temporary target root is created");
    let loader = SourceLoader::new(scratch.path().to_path_buf(), scratch.path().to_path_buf());
    let tree = loader
        .load_source(&SourceRef::Repository {
            repository: folder_source(repo.path()),
            git_ref: None,
        })
        .expect("local repository path loads");

    assert_eq!(tree.items, vec![item("rules.yml", false)]);
}
