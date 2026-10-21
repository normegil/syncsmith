//! Loading of source content for browsing during `init`.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Source;
use crate::config::env;
use crate::error::SyncsmithError;
use crate::git;
use crate::target::portable;

use super::flow::FlowFs;
use super::target::TargetFs;

/// Maximum directory depth walked when browsing a source.
const MAX_WALK_DEPTH: usize = 16;

/// A source as entered by the user, before environment expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRef {
    /// A git repository (remote URL or local path) with an optional ref.
    Repository {
        /// Repository URL or local path.
        repository: String,
        /// Branch, tag, or commit; `None` uses the default branch.
        git_ref: Option<String>,
    },
    /// A local folder.
    Folder {
        /// Path of the folder; relative paths resolve against the directory
        /// containing the configuration file.
        folder: String,
    },
}

impl SourceRef {
    /// Mirrors a configured entry source, so prompts (for instance the
    /// mode offer) stay consistent with the source kind.
    pub fn from_source(source: &Source) -> Self {
        match source {
            Source::Repository {
                repository,
                git_ref,
                ..
            } => Self::Repository {
                repository: repository.clone(),
                git_ref: git_ref.clone(),
            },
            Source::Folder { folder } => Self::Folder {
                folder: folder.to_string_lossy().into_owned(),
            },
        }
    }
}

/// One browsable file or folder of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeItem {
    /// Path relative to the source root.
    pub relative: PathBuf,
    /// Whether the item is a folder.
    pub is_dir: bool,
}

/// Browsable content of a source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceTree {
    /// Files and folders of the source, sorted by relative path.
    pub items: Vec<TreeItem>,
}

/// Production [`FlowFs`]: reads sources from disk, shallow clones remote
/// repositories, and checks targets inside the target repository root.
pub struct SourceLoader {
    config_dir: PathBuf,
    target_root: PathBuf,
}

impl SourceLoader {
    /// Builds a loader resolving relative sources against `config_dir` and
    /// checking targets inside `target_root`.
    pub fn new(config_dir: PathBuf, target_root: PathBuf) -> Self {
        Self {
            config_dir,
            target_root,
        }
    }

    /// Resolves a raw user-entered location: expands environment variables,
    /// then resolves relative paths against the configuration directory.
    fn resolve_location(&self, raw: &str) -> Result<PathBuf, SyncsmithError> {
        let expanded = env::expand(raw).map_err(|error| SyncsmithError::SourceLoad {
            origin: raw.to_string(),
            reason: error.to_string(),
        })?;
        let path = PathBuf::from(expanded);
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(self.config_dir.join(path))
        }
    }
}

impl TargetFs for SourceLoader {
    fn target_exists(&self, target: &Path) -> bool {
        self.target_root.join(target).exists()
    }

    fn target_is_dir(&self, target: &Path) -> bool {
        self.target_root.join(target).is_dir()
    }
}

impl FlowFs for SourceLoader {
    fn load_source(&self, source: &SourceRef) -> Result<SourceTree, SyncsmithError> {
        match source {
            SourceRef::Folder { folder } => {
                let root = self.resolve_location(folder)?;
                walk(&root, folder)
            }
            SourceRef::Repository {
                repository,
                git_ref,
            } => {
                if git::is_remote(repository) {
                    let request = git::CloneRequest::new(repository.clone(), git_ref.clone());
                    let clone = git::shallow_clone(&request)?;
                    walk(clone.path(), repository)
                } else {
                    let root = self.resolve_location(repository)?;
                    walk(&root, repository)
                }
            }
        }
    }
}

/// Walks a local tree: bounded depth, `.git` and symlinks skipped.
///
/// NOTE: folders at [`MAX_WALK_DEPTH`] are listed but not descended into, so
/// their children are simply absent from the tree (silently) rather than
/// flagged.
fn walk(root: &Path, origin: &str) -> Result<SourceTree, SyncsmithError> {
    if !root.is_dir() {
        return Err(SyncsmithError::SourceLoad {
            origin: origin.to_string(),
            reason: format!("`{}` is not a folder", root.display()),
        });
    }
    let mut items = Vec::new();
    let mut queue: VecDeque<WalkNode> = VecDeque::from([WalkNode {
        dir: root.to_path_buf(),
        prefix: PathBuf::new(),
        depth: 0,
    }]);
    while let Some(node) = queue.pop_front() {
        let entries = fs::read_dir(&node.dir).map_err(|error| source_error(origin, error))?;
        for entry in entries {
            let entry = entry.map_err(|error| source_error(origin, error))?;
            let Some(classified) = classify(&entry, &node, origin)? else {
                continue;
            };
            if let Some(child) = classified.child {
                queue.push_back(child);
            }
            items.push(classified.item);
        }
    }
    items.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(SourceTree { items })
}

/// One classified directory entry of a walk: the item to list and, for
/// folders within the depth bound, the child directory to queue.
struct ClassifiedEntry {
    /// Browsable item added to the tree.
    item: TreeItem,
    /// Child directory to walk next; `None` for files and for folders at
    /// [`MAX_WALK_DEPTH`].
    child: Option<WalkNode>,
}

/// Classifies one directory entry of a walk.
///
/// `None` skips the entry: `.git` entries and symlinks never appear in the
/// tree. Folders at [`MAX_WALK_DEPTH`] are listed but produce no child, so
/// they are not descended into.
fn classify(
    entry: &fs::DirEntry,
    node: &WalkNode,
    origin: &str,
) -> Result<Option<ClassifiedEntry>, SyncsmithError> {
    if entry.file_name() == ".git" {
        return Ok(None);
    }
    let file_type = entry
        .file_type()
        .map_err(|error| source_error(origin, error))?;
    if file_type.is_symlink() {
        return Ok(None);
    }
    // Portable form: `join` uses the native separator, which would leak
    // `\` into the tree paths (and their labels) on Windows.
    let relative = portable(&node.prefix.join(entry.file_name()));
    let is_dir = file_type.is_dir();
    let child = (is_dir && node.depth < MAX_WALK_DEPTH).then(|| WalkNode {
        dir: entry.path(),
        prefix: relative.clone(),
        depth: node.depth + 1,
    });
    Ok(Some(ClassifiedEntry {
        item: TreeItem { relative, is_dir },
        child,
    }))
}

/// Maps a walk I/O failure to the source-load error of `origin`.
fn source_error(origin: &str, error: std::io::Error) -> SyncsmithError {
    SyncsmithError::SourceLoad {
        origin: origin.to_string(),
        reason: error.to_string(),
    }
}

/// One directory queued for walking.
struct WalkNode {
    dir: PathBuf,
    prefix: PathBuf,
    depth: usize,
}
