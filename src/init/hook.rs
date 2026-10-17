//! Pre-commit hook installation.

use std::fs;
use std::path::Path;

use crate::error::SyncsmithError;
use crate::git;

use super::flow::Prompter;

/// Name of the hook file inside the repository hooks directory.
pub const HOOK_FILE_NAME: &str = "pre-commit";

/// Content of the installed hook.
const HOOK_SCRIPT: &str =
    "#!/bin/sh\n# Installed by syncsmith: run syncsmith sync before each commit.\nsyncsmith sync\n";

/// Offers to install a pre-commit hook calling `syncsmith sync`.
///
/// Skipped without error (info log) when `repo_root` is `None` (not a git
/// repository) or the user declines; never overwrites an existing hook
/// without confirmation.
pub fn offer_install(
    prompter: &dyn Prompter,
    repo_root: Option<&Path>,
) -> Result<(), SyncsmithError> {
    let Some(root) = repo_root else {
        tracing::info!("not inside a git repository; skipping pre-commit hook installation");
        return Ok(());
    };
    if !prompter.confirm("Install a pre-commit hook that runs `syncsmith sync`?")? {
        tracing::info!("pre-commit hook installation declined");
        return Ok(());
    }
    let hooks_dir = git::hooks_path(root)?;
    let hook_path = hooks_dir.join(HOOK_FILE_NAME);
    if hook_path.exists() && !prompter.confirm("A pre-commit hook already exists; overwrite it?")? {
        tracing::info!("leaving the existing pre-commit hook untouched");
        return Ok(());
    }
    write_hook(&hook_path)
}

fn write_hook(hook_path: &Path) -> Result<(), SyncsmithError> {
    write_hook_io(hook_path).map_err(|source| SyncsmithError::HookInstall {
        path: hook_path.to_path_buf(),
        source,
    })
}

fn write_hook_io(hook_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(hook_path, HOOK_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hook_path, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}
