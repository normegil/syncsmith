//! The stdout report of a sync run.
//!
//! Default format lists the changed paths, one per line, relative to the
//! target repository root — exactly what the pre-commit hook stages. The
//! `--json` flag switches to a single JSON document. Diagnostics always go
//! to stderr in both modes.

use std::io::Write;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How a target file was changed by a sync run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// The target file was created.
    Created,
    /// The target file was updated.
    Updated,
    /// The target file was deleted.
    Deleted,
}

/// One changed target file of a sync run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// Path of the file, relative to the target repository root.
    pub path: PathBuf,
    /// What happened to it.
    pub action: Action,
}

/// Rendering format of the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    /// One changed path per line (default).
    Lines,
    /// A single JSON document with one object per changed file (`--json`).
    Json,
}

/// Renders the changed files of a run in `format`.
pub fn render(changed: &[ChangedFile], format: ReportFormat) -> String {
    match format {
        ReportFormat::Lines => {
            let mut text = String::new();
            for file in changed {
                text.push_str(&file.path.to_string_lossy());
                text.push('\n');
            }
            text
        }
        ReportFormat::Json => serde_json::to_string(changed).unwrap_or_else(|error| {
            // ChangedFile serializes infallibly (plain paths and lowercase
            // tags); the fallback keeps rendering total for callers.
            tracing::warn!("cannot serialize the report: {error}");
            String::from("[]")
        }),
    }
}

/// Writes the report to `out` (stdout in production).
pub fn write(
    out: &mut dyn Write,
    changed: &[ChangedFile],
    format: ReportFormat,
) -> std::io::Result<()> {
    out.write_all(render(changed, format).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::{Action, ChangedFile, ReportFormat, render};
    use std::path::PathBuf;

    fn changed(path: &str, action: Action) -> ChangedFile {
        ChangedFile {
            path: PathBuf::from(path),
            action,
        }
    }

    fn report() -> Vec<ChangedFile> {
        vec![
            changed("lints/base.yml", Action::Updated),
            changed("docs/aliases/zsh.txt", Action::Created),
            changed("docs/aliases/old.sh", Action::Deleted),
        ]
    }

    #[test]
    fn lines_format_lists_paths_one_per_line() {
        let text = render(&report(), ReportFormat::Lines);
        assert_eq!(
            text,
            "lints/base.yml\ndocs/aliases/zsh.txt\ndocs/aliases/old.sh\n"
        );
    }

    #[test]
    fn json_format_renders_one_object_per_changed_file() {
        let text = render(&report(), ReportFormat::Json);
        let parsed: Vec<super::ChangedFile> =
            serde_json::from_str(&text).expect("report is valid JSON");
        assert_eq!(parsed, report());
    }

    #[test]
    fn empty_report_renders_empty_output() {
        assert_eq!(render(&[], ReportFormat::Lines), "");
        assert_eq!(render(&[], ReportFormat::Json), "[]");
    }
}
