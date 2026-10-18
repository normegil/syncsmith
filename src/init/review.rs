//! Review summary and per-entry edit/remove actions.

use std::path::PathBuf;

use crate::config::{Source, SyncEntry};
use crate::error::SyncsmithError;

use super::flow::{InitState, MAX_INPUT_ATTEMPTS, Prompter, ask_mode, ask_policy};
use super::source_loader::SourceRef;
use super::target;

/// Actions offered by the review loop.
pub const REVIEW_ACTIONS: [&str; 3] = ["confirm", "edit entry", "remove entry"];

/// Runs the review loop until the user confirms or every entry is removed.
///
/// Prints the summary table to stderr (stdout stays clean) before each action
/// prompt.
pub fn review(prompter: &dyn Prompter, state: &mut InitState) -> Result<(), SyncsmithError> {
    loop {
        if state.is_empty() {
            return Ok(());
        }
        for row in summary_rows(state) {
            eprintln!("{row}");
        }
        match ask_action(prompter)? {
            ReviewAction::Confirm => return Ok(()),
            ReviewAction::Edit => edit_entry(prompter, state)?,
            ReviewAction::Remove => remove_entry(prompter, state)?,
        }
    }
}

enum ReviewAction {
    Confirm,
    Edit,
    Remove,
}

fn ask_action(prompter: &dyn Prompter) -> Result<ReviewAction, SyncsmithError> {
    let action = prompter.select(
        "Review the entries above: confirm them, or edit or remove an entry",
        &owned(&REVIEW_ACTIONS),
    )?;
    match action.as_str() {
        "confirm" => Ok(ReviewAction::Confirm),
        "edit entry" => Ok(ReviewAction::Edit),
        "remove entry" => Ok(ReviewAction::Remove),
        other => Err(SyncsmithError::InvalidArguments {
            reason: format!("unknown review action `{other}`"),
        }),
    }
}

/// One line per entry: `1. source -> target [mode / policy]`.
pub fn summary_rows(state: &InitState) -> Vec<String> {
    state
        .all()
        .iter()
        .enumerate()
        .map(|(index, entry)| format!("{}. {}", index + 1, entry_label(entry)))
        .collect()
}

/// Single-line description of an entry, also used as its review label.
pub fn entry_label(entry: &SyncEntry) -> String {
    format!(
        "{} -> {} [{} / {}]",
        source_label(&entry.source),
        entry.target.to_string_lossy(),
        entry.mode.name(),
        entry.policy.name()
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

fn edit_entry(prompter: &dyn Prompter, state: &mut InitState) -> Result<(), SyncsmithError> {
    let labels = label_rows(state);
    let selected = prompter.select("Entry to edit", &labels)?;
    let Some(index) = labels.iter().position(|label| *label == selected) else {
        return Err(SyncsmithError::InvalidArguments {
            reason: format!("unknown entry `{selected}`"),
        });
    };
    let Some(current) = state.get(index).cloned() else {
        return Ok(());
    };
    let field = prompter.select("Field to change", &owned(&["target", "mode", "policy"]))?;
    let updated = match field.as_str() {
        "target" => {
            let request = EditRequest {
                prompter,
                state,
                index,
                current: &current,
            };
            edit_target(&request)?
        }
        "mode" => SyncEntry {
            mode: ask_mode(prompter, &SourceRef::from_source(&current.source))?,
            ..current.clone()
        },
        "policy" => SyncEntry {
            policy: ask_policy(prompter)?,
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
        .map(|(_, entry)| target::normalize(&entry.target))
        .collect::<Vec<PathBuf>>();
    let mut attempts = 0;
    loop {
        let input = request
            .prompter
            .text_with_default("New target path", &request.current.target.to_string_lossy())?;
        let path = PathBuf::from(input.trim());
        let normalized = target::validate_relative_target(&path)?;
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
    let selected = prompter.select("Entry to remove", &labels)?;
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

fn owned(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}
