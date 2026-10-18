//! Production prompter backed by the `inquire` crate.

use inquire::{Confirm, MultiSelect, Select, Text};

use crate::error::SyncsmithError;

use super::flow::{Choice, Prompter};

/// [`Prompter`] implementation rendering interactive prompts on the terminal.
///
/// Any interruption (Ctrl-C, Esc) or terminal failure (EOF, non-interactive
/// stdin, I/O error) aborts the flow without writing anything.
#[derive(Debug, Default)]
pub struct InquirePrompter;

impl InquirePrompter {
    /// Builds the prompter.
    pub fn new() -> Self {
        Self
    }
}

impl Prompter for InquirePrompter {
    fn confirm(&self, message: &str, default: Option<bool>) -> Result<bool, SyncsmithError> {
        match default {
            // `with_default` renders the `(Y/n)`/`(y/N)` indicator and lets
            // an empty submission pick the default.
            Some(default) => Confirm::new(message).with_default(default).prompt(),
            None => Confirm::new(message).prompt(),
        }
        .map_err(abort)
    }

    fn select(
        &self,
        message: &str,
        options: &[Choice],
        default: Option<&str>,
    ) -> Result<String, SyncsmithError> {
        let display: Vec<String> = options.iter().map(|option| option.to_string()).collect();
        let mut prompt = Select::new(message, display);
        if let Some(index) =
            default.and_then(|label| options.iter().position(|option| option.label == label))
        {
            prompt = prompt.with_starting_cursor(index);
        }
        let chosen = prompt.prompt().map_err(abort)?;
        options
            .iter()
            .find(|option| option.to_string() == chosen)
            .map(|option| option.label.clone())
            .ok_or_else(|| SyncsmithError::InvalidArguments {
                reason: format!("unknown option `{chosen}`"),
            })
    }

    fn text(&self, message: &str) -> Result<String, SyncsmithError> {
        Text::new(message).prompt().map_err(abort)
    }

    fn text_with_default(&self, message: &str, default: &str) -> Result<String, SyncsmithError> {
        Text::new(message)
            .with_default(default)
            .prompt()
            .map_err(abort)
    }

    fn multi_select(
        &self,
        message: &str,
        options: &[String],
    ) -> Result<Vec<String>, SyncsmithError> {
        MultiSelect::new(message, options.to_vec())
            .prompt()
            .map_err(abort)
    }
}

/// Maps any prompt failure to [`SyncsmithError::Aborted`].
fn abort(error: inquire::InquireError) -> SyncsmithError {
    // Ctrl-C, Esc, EOF, and terminal I/O failures all abort the flow; the
    // write only happens after the flow completes, so nothing is persisted.
    tracing::warn!(%error, "prompt interrupted");
    SyncsmithError::Aborted
}
