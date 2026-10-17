//! Pure orchestration of the interactive init flow.
//!
//! All prompting goes through the [`Prompter`] trait and all filesystem or
//! git access through the [`FlowFs`] trait, so the flow is a testable state
//! machine driven by a scripted prompter in tests and by `inquire` in
//! production.

use std::path::PathBuf;

use crate::config::{Mode, Policy, Source, SyncEntry};
use crate::error::SyncsmithError;

use super::review;
use super::source_loader::{SourceRef, SourceTree, TreeItem};
use super::target::{self, TargetFs};

/// Number of times a value prompt is repeated when validation fails before
/// the flow gives up with the last validation error.
pub(super) const MAX_INPUT_ATTEMPTS: u8 = 3;

/// Options offered when a configuration file already exists.
pub const EXISTING_CHOICES: [&str; 3] = ["add", "restart", "abort"];

/// Kinds of source offered by the flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Git repository: remote URL or local repository path.
    GitRepository,
    /// Local folder.
    LocalFolder,
}

impl SourceKind {
    /// All kinds in prompt order.
    pub const ALL: [SourceKind; 2] = [SourceKind::GitRepository, SourceKind::LocalFolder];

    /// Human-readable label used in prompts.
    pub fn name(self) -> &'static str {
        match self {
            SourceKind::GitRepository => "git repository",
            SourceKind::LocalFolder => "local folder",
        }
    }

    /// Parses the labels accepted by the interactive flow.
    pub fn from_name(name: &str) -> Option<SourceKind> {
        match name {
            "git repository" => Some(SourceKind::GitRepository),
            "local folder" => Some(SourceKind::LocalFolder),
            _ => None,
        }
    }
}

/// Interactive prompt operations used by the init flow.
///
/// Implementations map user interruption (Ctrl-C, Esc, EOF) to
/// [`SyncsmithError::Aborted`], which aborts the flow without writing.
pub trait Prompter {
    /// Yes/no question.
    fn confirm(&self, message: &str) -> Result<bool, SyncsmithError>;
    /// Single choice among `options`.
    fn select(&self, message: &str, options: &[String]) -> Result<String, SyncsmithError>;
    /// Free-form text.
    fn text(&self, message: &str) -> Result<String, SyncsmithError>;
    /// Free-form text with a suggested default.
    fn text_with_default(&self, message: &str, default: &str) -> Result<String, SyncsmithError>;
    /// Zero or more choices among `options`.
    fn multi_select(
        &self,
        message: &str,
        options: &[String],
    ) -> Result<Vec<String>, SyncsmithError>;
}

/// Filesystem and source-loading operations used by the init flow.
///
/// Target paths are relative to the target repository root.
pub trait FlowFs: TargetFs {
    /// Loads the browsable content of a source.
    fn load_source(&self, source: &SourceRef) -> Result<SourceTree, SyncsmithError>;
}

/// What to do with an existing configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingChoice {
    /// Keep existing entries and append new ones.
    Add,
    /// Discard current content and run a fresh flow.
    Restart,
    /// Exit without modifying anything.
    Abort,
}

/// Asks what to do with an existing configuration file.
pub fn ask_existing(prompter: &dyn Prompter) -> Result<ExistingChoice, SyncsmithError> {
    let choice = prompter.select(
        "A configuration file already exists. Add entries, restart from scratch, or abort?",
        &owned(&EXISTING_CHOICES),
    )?;
    match choice.as_str() {
        "add" => Ok(ExistingChoice::Add),
        "restart" => Ok(ExistingChoice::Restart),
        "abort" => Err(SyncsmithError::Aborted),
        other => Err(SyncsmithError::InvalidArguments {
            reason: format!("unknown choice `{other}`"),
        }),
    }
}

/// Builds the initial state from the user's choice on an existing config.
///
/// NOTE: Restart discards the existing entries but keeps their most recent
/// source as prefilled prompt defaults (location and ref, see
/// [`SourceDefaults`]); Add keeps the entries and starts fresh.
pub fn state_for_choice(
    choice: ExistingChoice,
    existing: Vec<SyncEntry>,
) -> Result<InitState, SyncsmithError> {
    match choice {
        ExistingChoice::Add => Ok(InitState {
            existing,
            added: Vec::new(),
            source_defaults: None,
        }),
        ExistingChoice::Restart => {
            let source_defaults = defaults_from(&existing);
            tracing::warn!("restart selected: existing entries are discarded and must be re-added");
            Ok(InitState {
                existing: Vec::new(),
                added: Vec::new(),
                source_defaults,
            })
        }
        ExistingChoice::Abort => Err(SyncsmithError::Aborted),
    }
}

/// Prefilled source prompts reused after a restart, taken from the most
/// recent entry of the discarded configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDefaults {
    /// Repository URL/path or folder path of the most recent entry.
    pub location: String,
    /// Ref of the most recent git entry, if any.
    pub git_ref: Option<String>,
}

/// Derives restart defaults from the most recent entry, if there is one.
fn defaults_from(entries: &[SyncEntry]) -> Option<SourceDefaults> {
    let last = entries.last()?;
    match &last.source {
        Source::Repository {
            repository,
            git_ref,
            ..
        } => Some(SourceDefaults {
            location: repository.clone(),
            git_ref: git_ref.clone(),
        }),
        Source::Folder { folder } => Some(SourceDefaults {
            location: folder.to_string_lossy().into_owned(),
            git_ref: None,
        }),
    }
}

/// Entries tracked during an init flow.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitState {
    /// Entries loaded from the existing configuration (kept on `add`).
    pub existing: Vec<SyncEntry>,
    /// Entries created by this flow.
    pub added: Vec<SyncEntry>,
    /// Source prompts prefilled from the discarded configuration after a
    /// restart; `None` in any other situation.
    pub source_defaults: Option<SourceDefaults>,
}

impl InitState {
    /// Total number of entries.
    pub fn len(&self) -> usize {
        self.existing.len() + self.added.len()
    }

    /// Whether the state holds no entry.
    pub fn is_empty(&self) -> bool {
        self.existing.is_empty() && self.added.is_empty()
    }

    /// All entries, pre-existing ones first.
    pub fn all(&self) -> Vec<SyncEntry> {
        self.existing
            .iter()
            .chain(self.added.iter())
            .cloned()
            .collect()
    }

    /// Returns the entry at `index` across the merged list.
    pub fn get(&self, index: usize) -> Option<&SyncEntry> {
        self.existing
            .get(index)
            .or_else(|| self.added.get(index.saturating_sub(self.existing.len())))
    }

    /// Replaces the entry at `index`; returns whether the index was valid.
    pub fn set(&mut self, index: usize, entry: SyncEntry) -> bool {
        if index < self.existing.len() {
            self.existing[index] = entry;
            return true;
        }
        let added_index = index - self.existing.len();
        if let Some(slot) = self.added.get_mut(added_index) {
            *slot = entry;
            return true;
        }
        false
    }

    /// Removes the entry at `index`; returns whether the index was valid.
    pub fn remove(&mut self, index: usize) -> bool {
        if index < self.existing.len() {
            self.existing.remove(index);
            return true;
        }
        let added_index = index - self.existing.len();
        if added_index < self.added.len() {
            self.added.remove(added_index);
            return true;
        }
        false
    }
}

/// Runs the interactive flow: source loop, selection, target mapping,
/// per-source mode/policy defaults, and the review loop.
///
/// Returns the final entry list (pre-existing entries kept, then added ones).
pub fn run_flow(
    state: &mut InitState,
    prompter: &dyn Prompter,
    fs: &dyn FlowFs,
) -> Result<Vec<SyncEntry>, SyncsmithError> {
    while prompter.confirm("Add a source?")? {
        add_source(prompter, fs, state)?;
    }
    review::review(prompter, state)?;
    Ok(state.all())
}

/// Prompts for one source and appends its entries to `state`.
fn add_source(
    prompter: &dyn Prompter,
    fs: &dyn FlowFs,
    state: &mut InitState,
) -> Result<(), SyncsmithError> {
    let kind = ask_source(prompter)?;
    let source_ref = ask_source_ref(prompter, kind, state.source_defaults.as_ref())?;
    let tree = fs.load_source(&source_ref)?;
    if tree.items.is_empty() {
        tracing::warn!("no files or folders found in the source");
        return Ok(());
    }
    let selections = select_items(prompter, &tree)?;
    if selections.is_empty() {
        return Ok(());
    }
    let mut mapping = MappingContext {
        prompter,
        fs,
        configured: state
            .all()
            .iter()
            .map(|entry| target::normalize(&entry.target))
            .collect(),
    };
    let selection_targets = map_targets(&mut mapping, &selections)?;
    let mode = ask_mode(prompter)?;
    let policy = ask_policy(prompter)?;
    for selection_target in selection_targets {
        let spec = NewEntry {
            source: &source_ref,
            item: &selection_target.item,
            mode,
            policy,
        };
        state
            .added
            .push(build_entry(&spec, selection_target.target));
    }
    Ok(())
}

/// Asks the kind of source to add.
///
/// Unknown labels from the prompter are rejected with
/// [`SyncsmithError::InvalidArguments`]; this is the only place where the
/// string labels are converted to [`SourceKind`].
fn ask_source(prompter: &dyn Prompter) -> Result<SourceKind, SyncsmithError> {
    let labels: Vec<String> = SourceKind::ALL
        .iter()
        .map(|kind| kind.name().to_string())
        .collect();
    let choice = prompter.select("Source type", &labels)?;
    SourceKind::from_name(&choice).ok_or_else(|| SyncsmithError::InvalidArguments {
        reason: format!("unknown source type `{choice}`"),
    })
}

/// Prompts for the source location and ref based on its `kind`.
///
/// When `defaults` is set (a restart), the location and ref prompts are
/// prefilled with the values from the discarded configuration.
fn ask_source_ref(
    prompter: &dyn Prompter,
    kind: SourceKind,
    defaults: Option<&SourceDefaults>,
) -> Result<SourceRef, SyncsmithError> {
    match kind {
        SourceKind::GitRepository => {
            let repository = match defaults {
                Some(defaults) => prompter
                    .text_with_default("Repository URL or local path", &defaults.location)?,
                None => prompter.text("Repository URL or local path")?,
            };
            let ref_default = defaults
                .and_then(|defaults| defaults.git_ref.clone())
                .unwrap_or_default();
            let raw_ref = prompter.text_with_default(
                "Ref (branch, tag, or commit; empty for the default branch)",
                &ref_default,
            )?;
            let trimmed = raw_ref.trim();
            let git_ref = (!trimmed.is_empty()).then(|| trimmed.to_string());
            Ok(SourceRef::Repository {
                repository,
                git_ref,
            })
        }
        SourceKind::LocalFolder => {
            let folder = match defaults {
                Some(defaults) => prompter.text_with_default("Folder path", &defaults.location)?,
                None => prompter.text("Folder path")?,
            };
            Ok(SourceRef::Folder { folder })
        }
    }
}

/// Selection loop within a source: multi-select until the user stops picking.
fn select_items(
    prompter: &dyn Prompter,
    tree: &SourceTree,
) -> Result<Vec<TreeItem>, SyncsmithError> {
    let mut remaining = tree.items.clone();
    let mut selected: Vec<TreeItem> = Vec::new();
    while !remaining.is_empty() {
        let options: Vec<String> = remaining.iter().map(label_item).collect();
        let picks = prompter.multi_select(
            "Select files or folders to synchronize (empty selection to finish)",
            &options,
        )?;
        if picks.is_empty() {
            break;
        }
        let mut picked: Vec<TreeItem> = Vec::with_capacity(picks.len());
        for pick in picks {
            let Some(index) = options.iter().position(|option| *option == pick) else {
                return Err(SyncsmithError::InvalidArguments {
                    reason: format!("unknown selection `{pick}`"),
                });
            };
            picked.push(remaining[index].clone());
        }
        selected.extend(picked.iter().cloned());
        remaining.retain(|candidate| !picked.iter().any(|p| covers(p, candidate)));
    }
    Ok(selected)
}

/// Whether `candidate` is made redundant by the already `picked` item.
fn covers(picked: &TreeItem, candidate: &TreeItem) -> bool {
    if picked.is_dir {
        candidate.relative.starts_with(&picked.relative)
    } else {
        candidate.relative == picked.relative
    }
}

fn label_item(item: &TreeItem) -> String {
    let suffix = if item.is_dir { " (folder)" } else { "" };
    format!("{}{}", item.relative.to_string_lossy(), suffix)
}

/// Target mapping for the selected items, validated against the entries
/// configured so far.
fn map_targets(
    mapping: &mut MappingContext,
    selections: &[TreeItem],
) -> Result<Vec<SelectionTarget>, SyncsmithError> {
    let mut targets = Vec::with_capacity(selections.len());
    for item in selections {
        let default = target::default_target(&item.relative);
        let mut attempts = 0;
        loop {
            let input = mapping
                .prompter
                .text_with_default(&target_prompt(item), &default.to_string_lossy())?;
            let path = PathBuf::from(input.trim());
            let check = target::TargetCheck {
                target: &path,
                selection_is_dir: item.is_dir,
                configured: &mapping.configured,
            };
            match target::validate_target(&check, mapping.fs) {
                Ok(()) => {
                    if target::overwrite_collision(&path, &mapping.configured, mapping.fs) {
                        tracing::warn!(
                            target_path = %path.display(),
                            "target exists and is not synchronized; `sync` may overwrite it"
                        );
                    }
                    mapping.configured.push(target::normalize(&path));
                    targets.push(SelectionTarget {
                        item: item.clone(),
                        target: path,
                    });
                    break;
                }
                Err(error) => {
                    attempts += 1;
                    if attempts >= MAX_INPUT_ATTEMPTS {
                        return Err(error);
                    }
                    tracing::warn!("{error}");
                }
            }
        }
    }
    Ok(targets)
}

fn target_prompt(item: &TreeItem) -> String {
    format!("Target path for `{}`", item.relative.to_string_lossy())
}

/// Prompter, filesystem, and the target paths configured so far.
struct MappingContext<'a> {
    prompter: &'a dyn Prompter,
    fs: &'a dyn FlowFs,
    configured: Vec<PathBuf>,
}

/// A selection paired with its validated target path.
struct SelectionTarget {
    item: TreeItem,
    target: PathBuf,
}

/// Input of [`build_entry`].
struct NewEntry<'a> {
    source: &'a SourceRef,
    item: &'a TreeItem,
    mode: Mode,
    policy: Policy,
}

/// Builds the configuration entry for one selection.
fn build_entry(spec: &NewEntry, target: PathBuf) -> SyncEntry {
    let source = match spec.source {
        SourceRef::Repository {
            repository,
            git_ref,
        } => Source::Repository {
            repository: repository.clone(),
            git_ref: git_ref.clone(),
            path: spec.item.relative.clone(),
        },
        SourceRef::Folder { folder } => Source::Folder {
            folder: PathBuf::from(folder).join(&spec.item.relative),
        },
    };
    SyncEntry {
        source,
        target,
        mode: spec.mode,
        policy: spec.policy,
    }
}

/// Asks the default mode applied to all entries of the current source.
pub(super) fn ask_mode(prompter: &dyn Prompter) -> Result<Mode, SyncsmithError> {
    let choice = prompter.select(
        "Default mode for entries from this source",
        &owned(&[Mode::Patch.name(), Mode::Overwrite.name()]),
    )?;
    Mode::from_name(&choice).ok_or_else(|| SyncsmithError::InvalidArguments {
        reason: format!("unknown mode `{choice}`"),
    })
}

/// Asks the default policy applied to all entries of the current source.
pub(super) fn ask_policy(prompter: &dyn Prompter) -> Result<Policy, SyncsmithError> {
    let choice = prompter.select(
        "Default policy for entries from this source",
        &owned(&[Policy::Automatic.name(), Policy::Manual.name()]),
    )?;
    Policy::from_name(&choice).ok_or_else(|| SyncsmithError::InvalidArguments {
        reason: format!("unknown policy `{choice}`"),
    })
}

fn owned(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ExistingChoice, FlowFs, InitState, Prompter, SourceDefaults, SourceKind, SourceRef,
        SourceTree, TreeItem, ask_existing, ask_source, owned, run_flow, state_for_choice,
    };
    use crate::config::{Mode, Policy, Source, SyncEntry};
    use crate::error::SyncsmithError;
    use crate::init::review::entry_label;
    use crate::init::target::TargetFs;
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone)]
    enum Answer {
        Confirm(bool),
        Select(String),
        Text(String),
        TextDefault(String),
        Multi(Vec<String>),
    }

    /// Prompter replaying a scripted answer queue; an exhausted script
    /// behaves like an interruption at the prompt.
    struct ScriptedPrompter {
        answers: RefCell<VecDeque<Answer>>,
    }

    impl ScriptedPrompter {
        fn new(answers: Vec<Answer>) -> Self {
            Self {
                answers: RefCell::new(answers.into()),
            }
        }

        fn pop(&self) -> Option<Answer> {
            self.answers.borrow_mut().pop_front()
        }
    }

    impl Prompter for ScriptedPrompter {
        fn confirm(&self, _message: &str) -> Result<bool, SyncsmithError> {
            match self.pop() {
                Some(Answer::Confirm(value)) => Ok(value),
                Some(other) => panic!("unexpected answer for confirm: {other:?}"),
                None => Err(SyncsmithError::Aborted),
            }
        }

        fn select(&self, _message: &str, _options: &[String]) -> Result<String, SyncsmithError> {
            match self.pop() {
                Some(Answer::Select(value)) => Ok(value),
                Some(other) => panic!("unexpected answer for select: {other:?}"),
                None => Err(SyncsmithError::Aborted),
            }
        }

        fn text(&self, _message: &str) -> Result<String, SyncsmithError> {
            match self.pop() {
                Some(Answer::Text(value)) => Ok(value),
                Some(other) => panic!("unexpected answer for text: {other:?}"),
                None => Err(SyncsmithError::Aborted),
            }
        }

        fn text_with_default(
            &self,
            _message: &str,
            _default: &str,
        ) -> Result<String, SyncsmithError> {
            match self.pop() {
                Some(Answer::TextDefault(value)) => Ok(value),
                Some(other) => panic!("unexpected answer for text_with_default: {other:?}"),
                None => Err(SyncsmithError::Aborted),
            }
        }

        fn multi_select(
            &self,
            _message: &str,
            _options: &[String],
        ) -> Result<Vec<String>, SyncsmithError> {
            match self.pop() {
                Some(Answer::Multi(values)) => Ok(values),
                Some(other) => panic!("unexpected answer for multi_select: {other:?}"),
                None => Err(SyncsmithError::Aborted),
            }
        }
    }

    /// FlowFs returning scripted source trees; the target filesystem is empty.
    struct ScriptedFs {
        trees: HashMap<String, SourceTree>,
    }

    impl TargetFs for ScriptedFs {
        fn target_exists(&self, _target: &Path) -> bool {
            false
        }

        fn target_is_dir(&self, _target: &Path) -> bool {
            false
        }
    }

    impl FlowFs for ScriptedFs {
        fn load_source(&self, source: &SourceRef) -> Result<SourceTree, SyncsmithError> {
            let key = match source {
                SourceRef::Folder { folder } => folder.clone(),
                SourceRef::Repository { repository, .. } => repository.clone(),
            };
            self.trees
                .get(&key)
                .cloned()
                .ok_or_else(|| SyncsmithError::SourceLoad {
                    origin: key.clone(),
                    reason: "no scripted tree".into(),
                })
        }
    }

    fn file(relative: &str) -> TreeItem {
        TreeItem {
            relative: PathBuf::from(relative),
            is_dir: false,
        }
    }

    fn folder(relative: &str) -> TreeItem {
        TreeItem {
            relative: PathBuf::from(relative),
            is_dir: true,
        }
    }

    fn tree(items: Vec<TreeItem>) -> SourceTree {
        SourceTree { items }
    }

    fn existing_entry(target: &str) -> SyncEntry {
        SyncEntry {
            source: Source::Repository {
                repository: "https://example.com/old.git".into(),
                git_ref: Some("main".into()),
                path: PathBuf::from("old.yml"),
            },
            target: target.into(),
            mode: Mode::Patch,
            policy: Policy::Automatic,
        }
    }

    #[test]
    fn add_flow_collects_entries_from_two_sources() {
        let fs = ScriptedFs {
            trees: HashMap::from([
                (
                    "rules".to_string(),
                    tree(vec![file("a.yml"), file("b.yml")]),
                ),
                (
                    "https://example.com/rules.git".to_string(),
                    tree(vec![file("lints/base.yml")]),
                ),
            ]),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::Multi(vec![]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("patch".into()),
            Answer::Select("automatic".into()),
            Answer::Confirm(true),
            Answer::Select("git repository".into()),
            Answer::Text("https://example.com/rules.git".into()),
            Answer::TextDefault(String::new()),
            Answer::Multi(vec!["lints/base.yml".into()]),
            Answer::TextDefault("lints/base.yml".into()),
            Answer::Select("overwrite".into()),
            Answer::Select("manual".into()),
            Answer::Confirm(false),
            Answer::Select("confirm".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&mut state, &prompter, &fs).expect("flow completes");
        assert_eq!(
            entries,
            vec![
                SyncEntry {
                    source: Source::Folder {
                        folder: PathBuf::from("rules/a.yml"),
                    },
                    target: "a.yml".into(),
                    mode: Mode::Patch,
                    policy: Policy::Automatic,
                },
                SyncEntry {
                    source: Source::Repository {
                        repository: "https://example.com/rules.git".into(),
                        git_ref: None,
                        path: PathBuf::from("lints/base.yml"),
                    },
                    target: "lints/base.yml".into(),
                    mode: Mode::Overwrite,
                    policy: Policy::Manual,
                },
            ]
        );
    }

    #[test]
    fn exhausted_script_aborts_flow_without_entries() {
        let fs = ScriptedFs {
            trees: HashMap::from([("rules".to_string(), tree(vec![file("a.yml")]))]),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("local folder".into()),
            Answer::Text("rules".into()),
        ]);
        let mut state = InitState::default();
        let result = run_flow(&mut state, &prompter, &fs);
        assert!(matches!(result, Err(SyncsmithError::Aborted)));
        assert!(state.added.is_empty());
    }

    #[test]
    fn duplicate_with_existing_entry_is_rejected_then_renamed() {
        let fs = ScriptedFs {
            trees: HashMap::from([(
                "rules".to_string(),
                tree(vec![file("a.yml"), file("b.yml")]),
            )]),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::Multi(vec![]),
            Answer::TextDefault("a.yml".into()),
            Answer::TextDefault("renamed.yml".into()),
            Answer::Select("patch".into()),
            Answer::Select("automatic".into()),
            Answer::Confirm(false),
            Answer::Select("confirm".into()),
        ]);
        let mut state = InitState {
            existing: vec![existing_entry("a.yml")],
            added: Vec::new(),
            source_defaults: None,
        };
        let entries = run_flow(&mut state, &prompter, &fs).expect("flow completes");
        assert_eq!(entries[0], existing_entry("a.yml"));
        assert_eq!(entries[1].target, PathBuf::from("renamed.yml"));
    }

    #[test]
    fn review_edit_changes_target_of_entry() {
        let fs = ScriptedFs {
            trees: HashMap::from([("rules".to_string(), tree(vec![file("a.yml")]))]),
        };
        let label = entry_label(&SyncEntry {
            source: Source::Folder {
                folder: PathBuf::from("rules/a.yml"),
            },
            target: "a.yml".into(),
            mode: Mode::Patch,
            policy: Policy::Automatic,
        });
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("patch".into()),
            Answer::Select("automatic".into()),
            Answer::Confirm(false),
            Answer::Select("edit entry".into()),
            Answer::Select(label),
            Answer::Select("target".into()),
            Answer::TextDefault("renamed.yml".into()),
            Answer::Select("confirm".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&mut state, &prompter, &fs).expect("flow completes");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].target, PathBuf::from("renamed.yml"));
    }

    #[test]
    fn review_remove_drops_entry() {
        let fs = ScriptedFs {
            trees: HashMap::from([("rules".to_string(), tree(vec![file("a.yml")]))]),
        };
        let label = entry_label(&SyncEntry {
            source: Source::Folder {
                folder: PathBuf::from("rules/a.yml"),
            },
            target: "a.yml".into(),
            mode: Mode::Patch,
            policy: Policy::Automatic,
        });
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("patch".into()),
            Answer::Select("automatic".into()),
            Answer::Confirm(false),
            Answer::Select("remove entry".into()),
            Answer::Select(label),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&mut state, &prompter, &fs).expect("flow completes");
        assert!(entries.is_empty());
    }

    #[test]
    fn empty_source_adds_no_entry() {
        let fs = ScriptedFs {
            trees: HashMap::from([("empty".to_string(), tree(vec![]))]),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("local folder".into()),
            Answer::Text("empty".into()),
            Answer::Confirm(false),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&mut state, &prompter, &fs).expect("flow completes");
        assert!(entries.is_empty());
    }

    #[test]
    fn folder_selection_covers_descendants_in_selection_loop() {
        let fs = ScriptedFs {
            trees: HashMap::from([(
                "rules".to_string(),
                tree(vec![folder("lints"), file("lints/base.yml")]),
            )]),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["lints (folder)".into()]),
            Answer::TextDefault("lints".into()),
            Answer::Select("overwrite".into()),
            Answer::Select("automatic".into()),
            Answer::Confirm(false),
            Answer::Select("confirm".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&mut state, &prompter, &fs).expect("flow completes");
        assert_eq!(
            entries,
            vec![SyncEntry {
                source: Source::Folder {
                    folder: PathBuf::from("rules/lints"),
                },
                target: "lints".into(),
                mode: Mode::Overwrite,
                policy: Policy::Automatic,
            }]
        );
    }

    #[test]
    fn add_choice_keeps_existing_entries() {
        let entries = vec![existing_entry("a.yml")];
        let state = state_for_choice(ExistingChoice::Add, entries.clone()).expect("valid choice");
        assert_eq!(state.existing, entries);
        assert!(state.source_defaults.is_none());
    }

    #[test]
    fn restart_choice_discards_existing_entries() {
        let state = state_for_choice(ExistingChoice::Restart, vec![existing_entry("a.yml")])
            .expect("valid choice");
        assert!(state.existing.is_empty());
        assert_eq!(
            state.source_defaults,
            Some(SourceDefaults {
                location: "https://example.com/old.git".into(),
                git_ref: Some("main".into()),
            })
        );
    }

    #[test]
    fn restart_defaults_prefill_the_source_location_prompt() {
        let fs = ScriptedFs {
            trees: HashMap::from([("newrules".to_string(), tree(vec![file("a.yml")]))]),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("local folder".into()),
            // The location prompt carries a default after a restart.
            Answer::TextDefault("newrules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("patch".into()),
            Answer::Select("automatic".into()),
            Answer::Confirm(false),
            Answer::Select("confirm".into()),
        ]);
        let discarded = SyncEntry {
            source: Source::Folder {
                folder: PathBuf::from("oldrules"),
            },
            target: "old.yml".into(),
            mode: Mode::Patch,
            policy: Policy::Automatic,
        };
        let mut state =
            state_for_choice(ExistingChoice::Restart, vec![discarded]).expect("valid choice");
        let entries = run_flow(&mut state, &prompter, &fs).expect("flow completes");
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].source,
            Source::Folder {
                folder: PathBuf::from("newrules/a.yml"),
            }
        );
    }

    #[test]
    fn abort_choice_returns_aborted_error() {
        let result = state_for_choice(ExistingChoice::Abort, vec![]);
        assert!(matches!(result, Err(SyncsmithError::Aborted)));
    }

    #[test]
    fn ask_existing_maps_choices() {
        let prompter = ScriptedPrompter::new(vec![Answer::Select("restart".into())]);
        assert_eq!(
            ask_existing(&prompter).expect("valid choice"),
            ExistingChoice::Restart
        );
    }

    #[test]
    fn ask_source_maps_choices() {
        let prompter = ScriptedPrompter::new(vec![
            Answer::Select("git repository".into()),
            Answer::Select("local folder".into()),
        ]);
        assert_eq!(
            ask_source(&prompter).expect("valid choice"),
            SourceKind::GitRepository
        );
        assert_eq!(
            ask_source(&prompter).expect("valid choice"),
            SourceKind::LocalFolder
        );
    }

    #[test]
    fn ask_source_rejects_unknown_choice() {
        let prompter = ScriptedPrompter::new(vec![Answer::Select("submodule".into())]);
        let result = ask_source(&prompter);
        assert!(matches!(
            result,
            Err(SyncsmithError::InvalidArguments { .. })
        ));
    }

    #[test]
    fn owned_converts_str_slices() {
        assert_eq!(owned(&["a", "b"]), vec!["a".to_string(), "b".to_string()]);
    }
}
