//! Review summary and per-entry edit/remove/add actions.

use std::path::PathBuf;

use crate::config::{Source, SyncEntry};
use crate::error::SyncsmithError;
use colored::Colorize;

use super::flow::{Choice, Flow, InitState, MAX_INPUT_ATTEMPTS, Prompter};
use super::source_loader::SourceRef;
use crate::target::{normalize, validate_relative_target};

/// Width of the separator lines of the review summary.
const SEPARATOR_WIDTH: usize = 60;

/// Actions offered by the review loop.
pub const REVIEW_ACTIONS: [&str; 4] = [
    "Validate",
    "Add another entry",
    "Edit entry",
    "Remove entry",
];

/// Runs the review loop until the user validates or every entry is removed.
///
/// Prints the summary block to stderr (stdout stays clean) before each action
/// prompt.
pub fn review(flow: &Flow, state: &mut InitState) -> Result<(), SyncsmithError> {
    loop {
        if state.is_empty() {
            return Ok(());
        }
        print_summary(state);
        match ask_action(flow.prompter())? {
            ReviewAction::Validate => return Ok(()),
            ReviewAction::Edit => edit_entry(flow, state)?,
            ReviewAction::Remove => remove_entry(flow.prompter(), state)?,
            ReviewAction::Add => flow.add_source(state)?,
        }
    }
}

enum ReviewAction {
    Validate,
    Edit,
    Remove,
    Add,
}

fn ask_action(prompter: &dyn Prompter) -> Result<ReviewAction, SyncsmithError> {
    let choices: Vec<Choice> = REVIEW_ACTIONS
        .iter()
        .map(|action| Choice::plain(action))
        .collect();
    let action = prompter.select(
        "Do you validate the current synchronization list?",
        &choices,
        None,
    )?;
    match action.as_str() {
        "Validate" => Ok(ReviewAction::Validate),
        "Edit entry" => Ok(ReviewAction::Edit),
        "Remove entry" => Ok(ReviewAction::Remove),
        "Add another entry" => Ok(ReviewAction::Add),
        other => Err(SyncsmithError::InvalidArguments {
            reason: format!("unknown review action `{other}`"),
        }),
    }
}

/// Prints the review summary block to stderr.
fn print_summary(state: &InitState) {
    let separator = "-".repeat(SEPARATOR_WIDTH);
    eprintln!();
    eprintln!("Synchronization summary");
    eprintln!("{separator}");
    for (index, entry) in state.all().iter().enumerate() {
        let headline = format!(
            "{}. {} -> {}",
            index + 1,
            source_label(&entry.source),
            entry.target.to_string_lossy()
        );
        // NOTE: `colored` resolves NO_COLOR/TTY against stdout while this
        // block prints to stderr, so a redirected stderr can still receive
        // color codes when stdout is a terminal. Accepted trade-off.
        eprintln!("{}", headline.green());
        eprintln!("  {} {}", "Mode:".purple(), entry.mode.label());
        eprintln!("  {} {}", "Policy:".purple(), entry.policy.label());
    }
    eprintln!("{separator}");
}

/// Single-line description of an entry, used as its review label for the
/// edit and remove pick lists.
pub fn entry_label(entry: &SyncEntry) -> String {
    format!(
        "{} -> {} [{} / {}]",
        source_label(&entry.source),
        entry.target.to_string_lossy(),
        entry.mode.label(),
        entry.policy.label()
    )
}

fn source_label(source: &Source) -> String {
    match source {
        Source::Repository {
            repository,
            git_ref,
            path,
        } => {
            let reference = git_ref.as_deref().unwrap_or("default");
            format!("{repository}@{reference}:{}", path.display())
        }
        Source::Folder { folder } => format!("folder {}", folder.display()),
    }
}

fn edit_entry(flow: &Flow, state: &mut InitState) -> Result<(), SyncsmithError> {
    let prompter = flow.prompter();
    let labels = label_rows(state);
    let choices: Vec<Choice> = labels.iter().map(|label| Choice::plain(label)).collect();
    let selected = prompter.select("Entry to edit", &choices, None)?;
    let Some(index) = labels.iter().position(|label| *label == selected) else {
        return Err(SyncsmithError::InvalidArguments {
            reason: format!("unknown entry `{selected}`"),
        });
    };
    let Some(current) = state.get(index).cloned() else {
        return Ok(());
    };
    let fields = [
        Choice::plain("Target"),
        Choice::plain("Mode"),
        Choice::plain("Policy"),
    ];
    let field = prompter.select("Field to change", &fields, None)?;
    let updated = match field.as_str() {
        "Target" => {
            let request = EditRequest {
                prompter,
                state,
                index,
                current: &current,
            };
            edit_target(&request)?
        }
        "Mode" => SyncEntry {
            mode: flow.ask_mode(&SourceRef::from_source(&current.source))?,
            ..current.clone()
        },
        "Policy" => SyncEntry {
            policy: flow.ask_policy()?,
            ..current
        },
        other => {
            return Err(SyncsmithError::InvalidArguments {
                reason: format!("unknown field `{other}`"),
            });
        }
    };
    state.set(index, updated);
    Ok(())
}

/// Input of [`edit_target`]: a target edit for one review entry.
struct EditRequest<'a> {
    prompter: &'a dyn Prompter,
    state: &'a InitState,
    index: usize,
    current: &'a SyncEntry,
}

/// Prompts for a new target path for an existing entry.
///
/// Only containment and duplicates are validated here: the file/folder kind
/// of an existing entry is only known to `sync`, so coherence is left to it.
fn edit_target(request: &EditRequest) -> Result<SyncEntry, SyncsmithError> {
    let others = request
        .state
        .all()
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != request.index)
        .map(|(_, entry)| normalize(&entry.target))
        .collect::<Vec<PathBuf>>();
    let mut attempts = 0;
    loop {
        let input = request.prompter.text_with_default(
            "New target path:",
            &request.current.target.to_string_lossy(),
        )?;
        let path = PathBuf::from(input.trim());
        let normalized = validate_relative_target(&path)?;
        if others.contains(&normalized) {
            let error = SyncsmithError::DuplicateTarget {
                target: normalized.to_string_lossy().into_owned(),
            };
            attempts += 1;
            if attempts >= MAX_INPUT_ATTEMPTS {
                return Err(error);
            }
            tracing::warn!("{error}");
            continue;
        }
        let mut entry = request.current.clone();
        entry.target = path;
        return Ok(entry);
    }
}

fn remove_entry(prompter: &dyn Prompter, state: &mut InitState) -> Result<(), SyncsmithError> {
    let labels = label_rows(state);
    let choices: Vec<Choice> = labels.iter().map(|label| Choice::plain(label)).collect();
    let selected = prompter.select("Entry to remove", &choices, None)?;
    if let Some(index) = labels.iter().position(|label| *label == selected)
        && !state.remove(index)
    {
        tracing::warn!("could not remove the selected entry");
    }
    Ok(())
}

fn label_rows(state: &InitState) -> Vec<String> {
    state.all().iter().map(entry_label).collect()
}
