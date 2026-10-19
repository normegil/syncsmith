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
pub const EXISTING_CHOICES: [&str; 3] = ["Add", "Restart", "Abort"];

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
            SourceKind::GitRepository => "Git repository",
            SourceKind::LocalFolder => "Local folder",
        }
    }

    /// Parses the labels accepted by the interactive flow.
    pub fn from_name(name: &str) -> Option<SourceKind> {
        match name {
            "Git repository" => Some(SourceKind::GitRepository),
            "Local folder" => Some(SourceKind::LocalFolder),
            _ => None,
        }
    }
}

/// One option of a single-choice prompt: a label with an optional help
/// description rendered next to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// Value returned when the option is selected.
    pub label: String,
    /// Help text displayed next to the label.
    pub description: Option<String>,
}

impl Choice {
    /// An option without help text.
    pub fn plain(label: &str) -> Self {
        Self {
            label: label.to_string(),
            description: None,
        }
    }

    /// An option with a help description.
    pub fn described(label: &str, description: &str) -> Self {
        Self {
            label: label.to_string(),
            description: Some(description.to_string()),
        }
    }
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.description {
            Some(description) => write!(f, "{} — {}", self.label, description),
            None => write!(f, "{}", self.label),
        }
    }
}

/// Interactive prompt operations used by the init flow.
///
/// Implementations map user interruption (Ctrl-C, Esc, EOF) to
/// [`SyncsmithError::Aborted`], which aborts the flow without writing.
pub trait Prompter {
    /// Yes/no question; `default` renders a `(Y/n)`/`(y/N)` indicator when
    /// set, and a plain y/n prompt when `None`.
    fn confirm(&self, message: &str, default: Option<bool>) -> Result<bool, SyncsmithError>;
    /// Single choice among `options`, returning the chosen label; `default`
    /// preselects the option with that label.
    fn select(
        &self,
        message: &str,
        options: &[Choice],
        default: Option<&str>,
    ) -> Result<String, SyncsmithError>;
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
    let choices: Vec<Choice> = EXISTING_CHOICES
        .iter()
        .map(|choice| Choice::plain(choice))
        .collect();
    let choice = prompter.select(
        "A configuration file already exists. Add entries, restart from scratch, or abort?",
        &choices,
        None,
    )?;
    match choice.as_str() {
        "Add" => Ok(ExistingChoice::Add),
        "Restart" => Ok(ExistingChoice::Restart),
        "Abort" => Err(SyncsmithError::Aborted),
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

/// Interactive flow driver: the prompter and the filesystem operations the
/// flow needs, so the review loop can trigger a full add-source pass.
pub struct Flow<'a> {
    prompter: &'a dyn Prompter,
    fs: &'a dyn FlowFs,
}

impl<'a> Flow<'a> {
    /// Builds a flow driven by `prompter` and backed by `fs`.
    pub fn new(prompter: &'a dyn Prompter, fs: &'a dyn FlowFs) -> Self {
        Self { prompter, fs }
    }

    /// The prompter driving the flow.
    pub(super) fn prompter(&self) -> &'a dyn Prompter {
        self.prompter
    }

    /// Runs the interactive flow: source loop, selection, target mapping,
    /// per-source mode/policy defaults, and the review loop.
    ///
    /// Returns the final entry list (pre-existing entries kept, then added
    /// ones).
    pub fn run(&self, state: &mut InitState) -> Result<Vec<SyncEntry>, SyncsmithError> {
        while self.prompter.confirm("Add a source?", Some(true))? {
            self.add_source(state)?;
        }
        review::review(self, state)?;
        Ok(state.all())
    }

    /// Prompts for one source and appends its entries to `state`.
    pub(super) fn add_source(&self, state: &mut InitState) -> Result<(), SyncsmithError> {
        let kind = self.ask_source()?;
        let source_ref = self.ask_source_ref(kind, state.source_defaults.as_ref())?;
        let tree = self.fs.load_source(&source_ref)?;
        if tree.items.is_empty() {
            tracing::warn!("no files or folders found in the source");
            return Ok(());
        }
        let selections = self.select_items(&tree)?;
        if selections.is_empty() {
            return Ok(());
        }
        let mut configured: Vec<PathBuf> = state
            .all()
            .iter()
            .map(|entry| target::normalize(&entry.target))
            .collect();
        let selection_targets = self.map_targets(&selections, &mut configured)?;
        let mode = self.ask_mode(&source_ref)?;
        let policy = self.ask_policy()?;
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
    fn ask_source(&self) -> Result<SourceKind, SyncsmithError> {
        let choices: Vec<Choice> = SourceKind::ALL
            .iter()
            .map(|kind| Choice::plain(kind.name()))
            .collect();
        let choice = self.prompter.select("Source type:", &choices, None)?;
        SourceKind::from_name(&choice).ok_or_else(|| SyncsmithError::InvalidArguments {
            reason: format!("unknown source type `{choice}`"),
        })
    }

    /// Prompts for the source location and ref based on its `kind`.
    ///
    /// When `defaults` is set (a restart), the location and ref prompts are
    /// prefilled with the values from the discarded configuration.
    fn ask_source_ref(
        &self,
        kind: SourceKind,
        defaults: Option<&SourceDefaults>,
    ) -> Result<SourceRef, SyncsmithError> {
        match kind {
            SourceKind::GitRepository => {
                let repository = match defaults {
                    Some(defaults) => self
                        .prompter
                        .text_with_default("Repository URL or local path:", &defaults.location)?,
                    None => self.prompter.text("Repository URL or local path:")?,
                };
                let ref_default = defaults
                    .and_then(|defaults| defaults.git_ref.clone())
                    .unwrap_or_default();
                let raw_ref = self.prompter.text_with_default(
                    "Repository reference (branch, tag, or commit; empty for the default branch):",
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
                    Some(defaults) => self
                        .prompter
                        .text_with_default("Folder path:", &defaults.location)?,
                    None => self.prompter.text("Folder path:")?,
                };
                Ok(SourceRef::Folder { folder })
            }
        }
    }

    /// Single-pass selection within a source: one multi-select; covered
    /// descendants are pruned from the selection.
    ///
    /// An empty pick asks whether the source should be skipped; declining
    /// re-asks the selection.
    fn select_items(&self, tree: &SourceTree) -> Result<Vec<TreeItem>, SyncsmithError> {
        let options: Vec<String> = tree.items.iter().map(label_item).collect();
        loop {
            let picks = self
                .prompter
                .multi_select("Select files or folders to synchronize", &options)?;
            if picks.is_empty() {
                let skip = self.prompter.confirm(
                    "No file was selected; the source will be ignored. Continue?",
                    Some(false),
                )?;
                if skip {
                    return Ok(Vec::new());
                }
                continue;
            }
            return self.resolve_picks(&picks, &options, tree);
        }
    }

    /// Maps the picked labels to tree items and prunes the ones covered by
    /// a picked folder.
    fn resolve_picks(
        &self,
        picks: &[String],
        options: &[String],
        tree: &SourceTree,
    ) -> Result<Vec<TreeItem>, SyncsmithError> {
        let mut picked: Vec<TreeItem> = Vec::with_capacity(picks.len());
        for pick in picks {
            let Some(index) = options.iter().position(|option| option == pick) else {
                return Err(SyncsmithError::InvalidArguments {
                    reason: format!("unknown selection `{pick}`"),
                });
            };
            picked.push(tree.items[index].clone());
        }
        Ok(picked
            .iter()
            .enumerate()
            .filter(|(index, candidate)| {
                !picked
                    .iter()
                    .enumerate()
                    .any(|(other, picker)| other != *index && covers(picker, candidate))
            })
            .map(|(_, item)| item.clone())
            .collect())
    }

    /// Target mapping for the selected items, validated against the entries
    /// configured so far (`configured` grows as targets are accepted).
    fn map_targets(
        &self,
        selections: &[TreeItem],
        configured: &mut Vec<PathBuf>,
    ) -> Result<Vec<SelectionTarget>, SyncsmithError> {
        let mut targets = Vec::with_capacity(selections.len());
        for item in selections {
            let default = target::default_target(&item.relative);
            let mut attempts = 0;
            loop {
                let input = self
                    .prompter
                    .text_with_default(&target_prompt(item), &default.to_string_lossy())?;
                let path = PathBuf::from(input.trim());
                let check = target::TargetCheck {
                    target: &path,
                    selection_is_dir: item.is_dir,
                    configured,
                };
                match target::validate_target(&check, self.fs) {
                    Ok(()) => {
                        if target::overwrite_collision(&path, configured, self.fs) {
                            tracing::warn!(
                                target_path = %path.display(),
                                "target exists and is not synchronized; `sync` may overwrite it"
                            );
                        }
                        configured.push(target::normalize(&path));
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

    /// Asks the default mode applied to all entries from the current source.
    ///
    /// Folder sources are incompatible with `patch` mode (the last-synced
    /// version of a folder cannot be recovered), so the prompt offers
    /// `Overwrite` only for them.
    pub(super) fn ask_mode(&self, source: &SourceRef) -> Result<Mode, SyncsmithError> {
        let options = match source {
            SourceRef::Folder { .. } => {
                tracing::info!("folder sources cannot use patch mode; offering overwrite only");
                vec![Choice::described(
                    Mode::Overwrite.label(),
                    "Replace the target with the source content",
                )]
            }
            SourceRef::Repository { .. } => vec![
                Choice::described(
                    Mode::Overwrite.label(),
                    "Replace the target with the source content",
                ),
                Choice::described(
                    Mode::Patch.label(),
                    "Apply the source changes on top of local edits",
                ),
            ],
        };
        let choice = self.prompter.select(
            "Default mode for entries from this source:",
            &options,
            Some(Mode::Overwrite.label()),
        )?;
        Mode::from_name(&choice).ok_or_else(|| SyncsmithError::InvalidArguments {
            reason: format!("unknown mode `{choice}`"),
        })
    }

    /// Asks the default policy applied to all entries of the current source.
    pub(super) fn ask_policy(&self) -> Result<Policy, SyncsmithError> {
        let options = vec![
            Choice::described(Policy::Automatic.label(), "Synchronized on every sync run"),
            Choice::described(
                Policy::Manual.label(),
                "Synchronized only when explicitly requested",
            ),
        ];
        let choice = self.prompter.select(
            "Default policy for entries from this source:",
            &options,
            Some(Policy::Automatic.label()),
        )?;
        Policy::from_name(&choice).ok_or_else(|| SyncsmithError::InvalidArguments {
            reason: format!("unknown policy `{choice}`"),
        })
    }
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

fn target_prompt(item: &TreeItem) -> String {
    format!("Target path for '{}':", item.relative.to_string_lossy())
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

#[cfg(test)]
mod tests {
    use super::{
        Choice, ExistingChoice, Flow, FlowFs, InitState, Prompter, SourceDefaults, SourceKind,
        SourceRef, SourceTree, TreeItem, ask_existing, state_for_choice,
    };
    use crate::commands::init::review::entry_label;
    use crate::commands::init::target::TargetFs;
    use crate::config::{Mode, Policy, Source, SyncEntry};
    use crate::error::SyncsmithError;
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

    /// One recorded single-choice prompt: its message, the offered labels,
    /// and the preselected default.
    #[derive(Debug, Clone, PartialEq)]
    struct SelectPrompt {
        message: String,
        labels: Vec<String>,
        default: Option<String>,
    }

    /// Prompter replaying a scripted answer queue; an exhausted script
    /// behaves like an interruption at the prompt.
    struct ScriptedPrompter {
        answers: RefCell<VecDeque<Answer>>,
        select_prompts: RefCell<Vec<SelectPrompt>>,
    }

    impl ScriptedPrompter {
        fn new(answers: Vec<Answer>) -> Self {
            Self {
                answers: RefCell::new(answers.into()),
                select_prompts: RefCell::new(Vec::new()),
            }
        }

        /// The single-choice prompts offered by the flow, in prompt order.
        fn select_prompts(&self) -> Vec<SelectPrompt> {
            self.select_prompts.borrow().clone()
        }

        fn pop(&self) -> Option<Answer> {
            self.answers.borrow_mut().pop_front()
        }
    }

    impl Prompter for ScriptedPrompter {
        fn confirm(&self, _message: &str, _default: Option<bool>) -> Result<bool, SyncsmithError> {
            match self.pop() {
                Some(Answer::Confirm(value)) => Ok(value),
                Some(other) => panic!("unexpected answer for confirm: {other:?}"),
                None => Err(SyncsmithError::Aborted),
            }
        }

        fn select(
            &self,
            message: &str,
            options: &[Choice],
            default: Option<&str>,
        ) -> Result<String, SyncsmithError> {
            self.select_prompts.borrow_mut().push(SelectPrompt {
                message: message.to_string(),
                labels: options.iter().map(|option| option.label.clone()).collect(),
                default: default.map(str::to_string),
            });
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

    fn run_flow(
        prompter: &ScriptedPrompter,
        fs: &ScriptedFs,
        state: &mut InitState,
    ) -> Result<Vec<SyncEntry>, SyncsmithError> {
        Flow::new(prompter, fs).run(state)
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
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(true),
            Answer::Select("Git repository".into()),
            Answer::Text("https://example.com/rules.git".into()),
            Answer::TextDefault(String::new()),
            Answer::Multi(vec!["lints/base.yml".into()]),
            Answer::TextDefault("lints/base.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Manual".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
        assert_eq!(
            entries,
            vec![
                SyncEntry {
                    source: Source::Folder {
                        folder: PathBuf::from("rules/a.yml"),
                    },
                    target: "a.yml".into(),
                    mode: Mode::Overwrite,
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
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
        ]);
        let mut state = InitState::default();
        let result = run_flow(&prompter, &fs, &mut state);
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
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::TextDefault("renamed.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState {
            existing: vec![existing_entry("a.yml")],
            added: Vec::new(),
            source_defaults: None,
        };
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
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
            mode: Mode::Overwrite,
            policy: Policy::Automatic,
        });
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Edit entry".into()),
            Answer::Select(label),
            Answer::Select("Target".into()),
            Answer::TextDefault("renamed.yml".into()),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
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
            mode: Mode::Overwrite,
            policy: Policy::Automatic,
        });
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Remove entry".into()),
            Answer::Select(label),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
        assert!(entries.is_empty());
    }

    #[test]
    fn review_add_another_entry_appends_entries() {
        let fs = ScriptedFs {
            trees: HashMap::from([
                ("rules".to_string(), tree(vec![file("a.yml")])),
                ("other".to_string(), tree(vec![file("b.yml")])),
            ]),
        };
        // After `Add another entry`, the full add-source flow (type,
        // location, selection, target, mode, policy) is replayed and the
        // review loop asks for an action again.
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Add another entry".into()),
            Answer::Select("Local folder".into()),
            Answer::Text("other".into()),
            Answer::Multi(vec!["b.yml".into()]),
            Answer::TextDefault("b.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
        assert_eq!(
            entries,
            vec![
                SyncEntry {
                    source: Source::Folder {
                        folder: PathBuf::from("rules/a.yml"),
                    },
                    target: "a.yml".into(),
                    mode: Mode::Overwrite,
                    policy: Policy::Automatic,
                },
                SyncEntry {
                    source: Source::Folder {
                        folder: PathBuf::from("other/b.yml"),
                    },
                    target: "b.yml".into(),
                    mode: Mode::Overwrite,
                    policy: Policy::Automatic,
                },
            ]
        );
    }

    #[test]
    fn review_validate_ends_the_loop() {
        let fs = ScriptedFs {
            trees: HashMap::from([("rules".to_string(), tree(vec![file("a.yml")]))]),
        };
        // The script ends at `Validate`: a further review prompt would
        // exhaust the script and surface as an abort.
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("validate ends the loop");
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn empty_source_adds_no_entry() {
        let fs = ScriptedFs {
            trees: HashMap::from([("empty".to_string(), tree(vec![]))]),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("empty".into()),
            Answer::Confirm(false),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
        assert!(entries.is_empty());
    }

    #[test]
    fn empty_selection_confirmed_skips_the_source() {
        let fs = ScriptedFs {
            trees: HashMap::from([("rules".to_string(), tree(vec![file("a.yml")]))]),
        };
        // Multi(empty) then Confirm(true): the skip confirmation is
        // accepted, so no entry is created and no further selection prompt
        // happens (the script would be exhausted otherwise).
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(Vec::new()),
            Answer::Confirm(true),
            Answer::Confirm(false),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
        assert!(entries.is_empty());
    }

    #[test]
    fn empty_selection_declined_reasks_the_selection() {
        let fs = ScriptedFs {
            trees: HashMap::from([("rules".to_string(), tree(vec![file("a.yml")]))]),
        };
        // Multi(empty) then Confirm(false): the skip is declined, so the
        // next answer must be the re-asked multi-select; picking `a.yml`
        // there must produce its entry.
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(Vec::new()),
            Answer::Confirm(false),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
        assert_eq!(
            entries,
            vec![SyncEntry {
                source: Source::Folder {
                    folder: PathBuf::from("rules/a.yml"),
                },
                target: "a.yml".into(),
                mode: Mode::Overwrite,
                policy: Policy::Automatic,
            }]
        );
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
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["lints (folder)".into()]),
            Answer::TextDefault("lints".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
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
    fn simultaneous_folder_and_descendant_picks_prune_the_descendant() {
        let fs = ScriptedFs {
            trees: HashMap::from([(
                "rules".to_string(),
                tree(vec![folder("lints"), file("lints/base.yml")]),
            )]),
        };
        // Picking both the folder and its descendant in one pass keeps only
        // the folder: a single target prompt must follow (a second one
        // would exhaust the script).
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["lints (folder)".into(), "lints/base.yml".into()]),
            Answer::TextDefault("lints".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
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
    fn folder_sources_are_offered_overwrite_mode_only() {
        let fs = ScriptedFs {
            trees: HashMap::from([("rules".to_string(), tree(vec![file("a.yml")]))]),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
        // Select prompts in order: source type, mode, policy, review.
        let prompts = prompter.select_prompts();
        assert_eq!(
            prompts.get(1).map(|prompt| prompt.labels.clone()),
            Some(vec!["Overwrite".to_string()]),
            "the mode prompt must offer overwrite only for folder sources"
        );
        assert_eq!(
            entries,
            vec![SyncEntry {
                source: Source::Folder {
                    folder: PathBuf::from("rules/a.yml"),
                },
                target: "a.yml".into(),
                mode: Mode::Overwrite,
                policy: Policy::Automatic,
            }]
        );
    }

    #[test]
    fn git_repository_sources_are_offered_both_modes() {
        let fs = ScriptedFs {
            trees: HashMap::from([(
                "https://example.com/rules.git".to_string(),
                tree(vec![file("lints/base.yml")]),
            )]),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Git repository".into()),
            Answer::Text("https://example.com/rules.git".into()),
            Answer::TextDefault(String::new()),
            Answer::Multi(vec!["lints/base.yml".into()]),
            Answer::TextDefault("lints/base.yml".into()),
            Answer::Select("Patch".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
        assert_eq!(entries[0].mode, Mode::Patch);
        // Select prompts in order: source type, mode, policy, review.
        let prompts = prompter.select_prompts();
        assert_eq!(
            prompts.get(1).map(|prompt| prompt.labels.clone()),
            Some(vec!["Overwrite".to_string(), "Patch".to_string()]),
            "the mode prompt must offer both modes for git sources"
        );
    }

    #[test]
    fn mode_prompt_defaults_to_overwrite() {
        let fs = ScriptedFs {
            trees: HashMap::from([
                (
                    "https://example.com/rules.git".to_string(),
                    tree(vec![file("lints/base.yml")]),
                ),
                ("rules".to_string(), tree(vec![file("a.yml")])),
            ]),
        };
        // One git source then one folder source; the mode prompts are the
        // select prompts at index 1 and 4 (each source asks type, mode,
        // policy, in that order).
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Git repository".into()),
            Answer::Text("https://example.com/rules.git".into()),
            Answer::TextDefault(String::new()),
            Answer::Multi(vec!["lints/base.yml".into()]),
            Answer::TextDefault("lints/base.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        run_flow(&prompter, &fs, &mut state).expect("flow completes");
        let prompts = prompter.select_prompts();
        let git_mode = prompts.get(1).expect("the git mode prompt is recorded");
        assert_eq!(git_mode.default.as_deref(), Some("Overwrite"));
        assert_eq!(git_mode.labels, vec!["Overwrite", "Patch"]);
        let folder_mode = prompts.get(4).expect("the folder mode prompt is recorded");
        assert_eq!(folder_mode.default.as_deref(), Some("Overwrite"));
        assert_eq!(folder_mode.labels, vec!["Overwrite"]);
    }

    #[test]
    fn policy_prompt_defaults_to_automatic() {
        let fs = ScriptedFs {
            trees: HashMap::from([("rules".to_string(), tree(vec![file("a.yml")]))]),
        };
        // The policy prompt is the select prompt at index 2 (source type,
        // mode, policy, in that order).
        let prompter = ScriptedPrompter::new(vec![
            Answer::Confirm(true),
            Answer::Select("Local folder".into()),
            Answer::Text("rules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let mut state = InitState::default();
        run_flow(&prompter, &fs, &mut state).expect("flow completes");
        let prompts = prompter.select_prompts();
        let policy = prompts.get(2).expect("the policy prompt is recorded");
        assert_eq!(policy.default.as_deref(), Some("Automatic"));
        assert_eq!(policy.labels, vec!["Automatic", "Manual"]);
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
            Answer::Select("Local folder".into()),
            // The location prompt carries a default after a restart.
            Answer::TextDefault("newrules".into()),
            Answer::Multi(vec!["a.yml".into()]),
            Answer::TextDefault("a.yml".into()),
            Answer::Select("Overwrite".into()),
            Answer::Select("Automatic".into()),
            Answer::Confirm(false),
            Answer::Select("Validate".into()),
        ]);
        let discarded = SyncEntry {
            source: Source::Folder {
                folder: PathBuf::from("oldrules"),
            },
            target: "old.yml".into(),
            mode: Mode::Overwrite,
            policy: Policy::Automatic,
        };
        let mut state =
            state_for_choice(ExistingChoice::Restart, vec![discarded]).expect("valid choice");
        let entries = run_flow(&prompter, &fs, &mut state).expect("flow completes");
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
        let prompter = ScriptedPrompter::new(vec![Answer::Select("Restart".into())]);
        assert_eq!(
            ask_existing(&prompter).expect("valid choice"),
            ExistingChoice::Restart
        );
    }

    #[test]
    fn ask_source_maps_choices() {
        let fs = ScriptedFs {
            trees: HashMap::new(),
        };
        let prompter = ScriptedPrompter::new(vec![
            Answer::Select("Git repository".into()),
            Answer::Select("Local folder".into()),
        ]);
        let flow = Flow::new(&prompter, &fs);
        assert_eq!(
            flow.ask_source().expect("valid choice"),
            SourceKind::GitRepository
        );
        assert_eq!(
            flow.ask_source().expect("valid choice"),
            SourceKind::LocalFolder
        );
    }

    #[test]
    fn ask_source_rejects_unknown_choice() {
        let fs = ScriptedFs {
            trees: HashMap::new(),
        };
        let prompter = ScriptedPrompter::new(vec![Answer::Select("submodule".into())]);
        let flow = Flow::new(&prompter, &fs);
        let result = flow.ask_source();
        assert!(matches!(
            result,
            Err(SyncsmithError::InvalidArguments { .. })
        ));
    }
}
