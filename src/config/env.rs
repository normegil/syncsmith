//! `$VAR` / `${VAR}` expansion for configuration string fields.

use crate::error::SyncsmithError;

use super::model::{Config, Source};

/// Expands `$VAR` and `${VAR}` references in `input`.
///
/// A `$` that does not start a valid variable name stays literal. Referencing
/// an unset variable fails with an actionable error.
pub fn expand(input: &str) -> Result<String, SyncsmithError> {
    let chars: Vec<char> = input.chars().collect();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current != '$' {
            output.push(current);
            index += 1;
            continue;
        }
        index = expand_at(&chars, index, &mut output)?;
    }
    Ok(output)
}

/// Expands the reference starting at the `$` at `index`, returning the new index.
fn expand_at(chars: &[char], index: usize, output: &mut String) -> Result<usize, SyncsmithError> {
    match chars.get(index + 1) {
        Some('{') => expand_braced(chars, index, output),
        Some(&next) if is_name_start(next) => expand_bare(chars, index, output),
        _ => {
            output.push('$');
            Ok(index + 1)
        }
    }
}

/// Expands `${NAME}` starting at `index` (pointing at `$`).
fn expand_braced(
    chars: &[char],
    index: usize,
    output: &mut String,
) -> Result<usize, SyncsmithError> {
    let name_end = chars[index + 2..]
        .iter()
        .position(|&c| c == '}')
        .map(|offset| index + 2 + offset)
        .ok_or_else(|| SyncsmithError::InvalidArguments {
            reason: format!(
                "unterminated variable reference in `{}`",
                chars.iter().collect::<String>()
            ),
        })?;
    let name: String = chars[index + 2..name_end].iter().collect();
    if name.is_empty() || !name.chars().next().is_some_and(is_name_start) {
        return Err(SyncsmithError::InvalidArguments {
            reason: format!("invalid variable name `{{{name}}}`"),
        });
    }
    output.push_str(&lookup(&name)?);
    Ok(name_end + 1)
}

/// Expands `$NAME` starting at `index` (pointing at `$`).
fn expand_bare(chars: &[char], index: usize, output: &mut String) -> Result<usize, SyncsmithError> {
    let mut end = index + 1;
    while end < chars.len() && is_name_char(chars[end]) {
        end += 1;
    }
    let name: String = chars[index + 1..end].iter().collect();
    output.push_str(&lookup(&name)?);
    Ok(end)
}

fn lookup(name: &str) -> Result<String, SyncsmithError> {
    std::env::var(name).map_err(|_| SyncsmithError::InvalidArguments {
        reason: format!("environment variable `{name}` is not set"),
    })
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

impl Config {
    /// Returns a copy of this config with all string fields expanded.
    ///
    /// NOTE: expansion happens at load time, in memory only — the written
    /// `.syncsmith.toml` keeps the raw `$VAR`/`${VAR}` strings so sources
    /// stay portable across machines with different environments.
    pub fn expand_env(&self) -> Result<Config, SyncsmithError> {
        let mut expanded = self.clone();
        for entry in &mut expanded.sync {
            match &mut entry.source {
                Source::Repository {
                    repository,
                    git_ref,
                    path,
                } => {
                    *repository = expand(repository)?;
                    if let Some(reference) = git_ref.as_mut() {
                        *reference = expand(reference)?;
                    }
                    *path = expanded_path(path)?;
                }
                Source::Folder { folder } => {
                    *folder = expanded_path(folder)?;
                }
            }
            entry.target = expanded_path(&entry.target)?;
        }
        Ok(expanded)
    }
}

fn expanded_path(path: &std::path::Path) -> Result<std::path::PathBuf, SyncsmithError> {
    Ok(std::path::PathBuf::from(expand(&path.to_string_lossy())?))
}

#[cfg(test)]
mod tests {
    use super::expand;
    use crate::error::SyncsmithError;

    // Each test uses distinct variable names: tests run in parallel threads
    // of one process and share the environment.

    #[test]
    fn expands_bare_dollar_variable() {
        unsafe { std::env::set_var("SYNC_SMITH_TEST_A", "alpha") };
        assert_eq!(expand("$SYNC_SMITH_TEST_A").expect("expandable"), "alpha");
    }

    #[test]
    fn expands_braced_variable() {
        unsafe { std::env::set_var("SYNC_SMITH_TEST_B", "beta") };
        assert_eq!(expand("${SYNC_SMITH_TEST_B}").expect("expandable"), "beta");
    }

    #[test]
    fn unset_variable_is_an_error() {
        unsafe { std::env::remove_var("SYNC_SMITH_TEST_UNSET") };
        assert!(matches!(
            expand("$SYNC_SMITH_TEST_UNSET"),
            Err(SyncsmithError::InvalidArguments { .. })
        ));
    }

    #[test]
    fn literal_dollar_without_name_stays() {
        assert_eq!(expand("cost$").expect("expandable"), "cost$");
        assert_eq!(expand("a$.b").expect("expandable"), "a$.b");
    }

    #[test]
    fn adjacent_variables_expand() {
        unsafe {
            std::env::set_var("SYNC_SMITH_TEST_C", "one");
            std::env::set_var("SYNC_SMITH_TEST_D", "two");
        }
        assert_eq!(
            expand("$SYNC_SMITH_TEST_C$SYNC_SMITH_TEST_D").expect("expandable"),
            "onetwo"
        );
    }

    #[test]
    fn expands_variable_inside_path() {
        unsafe { std::env::set_var("SYNC_SMITH_TEST_E", "/home/rules") };
        assert_eq!(
            expand("${SYNC_SMITH_TEST_E}/lints/base.yml").expect("expandable"),
            "/home/rules/lints/base.yml"
        );
    }

    #[test]
    fn unterminated_brace_is_an_error() {
        assert!(matches!(
            expand("${SYNC_SMITH_TEST_F"),
            Err(SyncsmithError::InvalidArguments { .. })
        ));
    }
}
