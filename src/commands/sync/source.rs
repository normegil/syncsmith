//! Source snapshots: the latest content of an entry, and recovery of the
//! last-synced version for patch mode.
//!
//! - folder sources are read directly; the version of each file is its git
//!   blob hash, computed in Rust (no `git hash-object` spawn);
//! - local git repositories are read at the ref directly with `git show`
//!   (no clone);
//! - remote git repositories are shallow cloned (bounded retry/timeout of
//!   [`crate::git::shallow_clone`]) and read at the ref; the clone is kept
//!   alive for old-version recovery.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

use crate::config::{Source, SyncEntry};
use crate::error::SyncsmithError;
use crate::git;

/// Maximum directory depth walked when reading a folder source (same bound
/// as the `init` source browser).
const MAX_WALK_DEPTH: usize = 16;

/// Latest content of one entry.
#[derive(Debug)]
pub struct SourceSnapshot {
    /// Resolved commit hash for git sources; `None` for folder sources,
    /// whose per-file version is the content blob hash.
    pub version: Option<String>,
    /// Content of the source: a single file or a file tree.
    pub kind: SnapshotKind,
    /// Access to the git source, kept for old-version recovery; `None` for
    /// folder sources.
    git: Option<GitAccess>,
}

/// How the source content is shaped.
#[derive(Debug)]
pub enum SnapshotKind {
    /// A single-file source: the file content.
    File {
        /// Content of the source file.
        content: Vec<u8>,
    },
    /// A folder source: relative path to content. Empty directories are
    /// ignored (git cannot represent them).
    Tree {
        /// Files of the tree, keyed by path relative to the source root.
        files: BTreeMap<PathBuf, Vec<u8>>,
    },
}

/// Access to a git source for old-version recovery.
#[derive(Debug)]
enum GitAccess {
    /// A local repository path: read directly.
    Local(PathBuf),
    /// A shallow clone of a remote repository; versions can be fetched by
    /// commit hash.
    RemoteClone(tempfile::TempDir),
}

impl SourceSnapshot {
    /// A folder-source snapshot (per-file blob-hash versioning).
    fn folder(kind: SnapshotKind) -> Self {
        Self {
            version: None,
            kind,
            git: None,
        }
    }

    /// Recovers the content of `request.path` at `request.version` (the
    /// last-synced source version recorded in the lock).
    ///
    /// For a remote repository the version is fetched into the clone with
    /// `git fetch --depth 1 origin <sha>` first. An unrecoverable version
    /// (server disallows fetching by SHA, pruned repository, unknown
    /// commit) fails with [`SyncsmithError::SourceVersionUnavailable`].
    pub fn recover(&self, request: &RecoveryRequest) -> Result<Vec<u8>, SyncsmithError> {
        let Some(access) = self.git.as_ref() else {
            return Err(SyncsmithError::SourceVersionUnavailable {
                origin: request.origin.to_string(),
                version: request.version.to_string(),
                reason: "the source is not a git repository".into(),
            });
        };
        let spec = format!("{}:{}", request.version, git_path(&request.path));
        let content = match access {
            GitAccess::Local(repo) => git::run_in(repo, &["show", &spec]),
            GitAccess::RemoteClone(clone) => {
                if let Err(failure) = git::run_in(
                    clone.path(),
                    &["fetch", "--depth", "1", "origin", request.version],
                ) {
                    return Err(recovery_unavailable(request, failure.reason));
                }
                git::run_in(clone.path(), &["show", &spec])
            }
        };
        content.map_err(|failure| recovery_unavailable(request, failure.reason))
    }
}

/// Input of [`SourceSnapshot::recover`].
#[derive(Debug)]
pub struct RecoveryRequest<'a> {
    /// Commit hash recorded in the lock file.
    pub version: &'a str,
    /// Path of the file inside the repository.
    pub path: PathBuf,
    /// Repository URL or path, for error context.
    pub origin: &'a str,
}

fn recovery_unavailable(request: &RecoveryRequest, reason: String) -> SyncsmithError {
    SyncsmithError::SourceVersionUnavailable {
        origin: request.origin.to_string(),
        version: request.version.to_string(),
        reason,
    }
}

/// Computes the git blob hash of `content`: `sha1("blob <len>\0" + content)`.
pub fn blob_hash(content: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", content.len()));
    hasher.update(content);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Snapshots the latest content of `entry`.
///
/// Relative source paths resolve against `config_dir`. A missing source
/// path fails with [`SyncsmithError::SourceLoad`], so the caller can fail
/// the entry and leave the target untouched.
pub fn snapshot(entry: &SyncEntry, config_dir: &Path) -> Result<SourceSnapshot, SyncsmithError> {
    match &entry.source {
        Source::Folder { folder } => snapshot_folder(folder, config_dir),
        Source::Repository {
            repository,
            git_ref,
            path,
        } => {
            let source = GitSource {
                repository,
                git_ref: git_ref.as_deref(),
                path,
            };
            snapshot_repository(&source, config_dir)
        }
    }
}

/// Snapshots a folder source: a single file, or a whole tree.
fn snapshot_folder(folder: &Path, config_dir: &Path) -> Result<SourceSnapshot, SyncsmithError> {
    let root = resolve_against(folder, config_dir);
    let origin = folder.to_string_lossy().into_owned();
    let load_error = |source: std::io::Error| SyncsmithError::SourceLoad {
        origin: origin.clone(),
        reason: source.to_string(),
    };
    if root.is_file() {
        let content = fs::read(&root).map_err(load_error)?;
        return Ok(SourceSnapshot::folder(SnapshotKind::File { content }));
    }
    if root.is_dir() {
        let files = read_tree(&root, &origin)?;
        return Ok(SourceSnapshot::folder(SnapshotKind::Tree { files }));
    }
    Err(SyncsmithError::SourceLoad {
        origin,
        reason: "the source no longer exists".into(),
    })
}

/// A git repository source to snapshot.
struct GitSource<'a> {
    /// Repository URL or local path.
    repository: &'a str,
    /// Branch, tag, or commit; `None` means the default branch / `HEAD`.
    git_ref: Option<&'a str>,
    /// Path of the file or folder inside the repository.
    path: &'a Path,
}

/// Snapshots a git repository source (local or remote).
fn snapshot_repository(
    source: &GitSource,
    config_dir: &Path,
) -> Result<SourceSnapshot, SyncsmithError> {
    let origin = format!(
        "{}@{}:{}",
        source.repository,
        source.git_ref.unwrap_or("HEAD"),
        source.path.display()
    );
    tracing::trace!(origin = %origin, "Snapshoting repository");
    if git::is_remote(source.repository) {
        let request = git::CloneRequest::new(
            source.repository.to_string(),
            source.git_ref.map(String::from),
        );
        let clone = git::shallow_clone(&request)?;
        let version = resolved_version(clone.path(), "HEAD", &origin)?;
        let read = GitRead {
            repo: clone.path(),
            git_ref: "HEAD",
            origin: &origin,
        };
        let kind = read_git_path(&read, source.path)?;
        Ok(SourceSnapshot {
            version: Some(version),
            kind,
            git: Some(GitAccess::RemoteClone(clone)),
        })
    } else {
        let repo = resolve_against(Path::new(source.repository), config_dir);
        let ref_label = source.git_ref.unwrap_or("HEAD");
        let version = resolved_version(&repo, ref_label, &origin)?;
        let read = GitRead {
            repo: &repo,
            git_ref: ref_label,
            origin: &origin,
        };
        let kind = read_git_path(&read, source.path)?;
        Ok(SourceSnapshot {
            version: Some(version),
            kind,
            git: Some(GitAccess::Local(repo)),
        })
    }
}

/// Where to read a path inside a git repository.
struct GitRead<'a> {
    /// Repository to read from.
    repo: &'a Path,
    /// Ref to read at.
    git_ref: &'a str,
    /// Repository URL/path and ref, for error context.
    origin: &'a str,
}

/// Resolves the commit hash of `git_ref` inside `repo`.
fn resolved_version(repo: &Path, git_ref: &str, origin: &str) -> Result<String, SyncsmithError> {
    git::run_in(repo, &["rev-parse", git_ref])
        .map(|stdout| String::from_utf8_lossy(&stdout).trim().to_string())
        .map_err(|failure| SyncsmithError::SourceLoad {
            origin: origin.to_string(),
            reason: failure.reason,
        })
}

/// Reads `path` at the ref of `read`, as a single file or a tree.
fn read_git_path(read: &GitRead, path: &Path) -> Result<SnapshotKind, SyncsmithError> {
    let spec = format!("{}:{}", read.git_ref, git_path(path));
    let object_type = git::run_in(read.repo, &["cat-file", "-t", &spec])
        .map(|stdout| String::from_utf8_lossy(&stdout).trim().to_string())
        .map_err(|failure| SyncsmithError::SourceLoad {
            origin: read.origin.to_string(),
            reason: failure.reason,
        })?;
    match object_type.as_str() {
        "blob" => Ok(SnapshotKind::File {
            content: git::run_in(read.repo, &["show", &spec])?,
        }),
        "tree" => {
            let listing = git::run_in(read.repo, &["ls-tree", "-r", "-z", &spec])?;
            Ok(SnapshotKind::Tree {
                files: read_git_tree(read, path, &listing)?,
            })
        }
        _ => Err(SyncsmithError::SourceLoad {
            origin: read.origin.to_string(),
            reason: format!(
                "`{}` is not a file or a folder in the repository",
                path.display()
            ),
        }),
    }
}

/// Reads the files of a git tree entry: relative path to content.
///
/// Symlinks and submodules (non-blob entries) are skipped, matching the
/// `init` source browser; entries are NUL-terminated, so filenames with
/// spaces are handled.
fn read_git_tree(
    read: &GitRead,
    path: &Path,
    listing: &[u8],
) -> Result<BTreeMap<PathBuf, Vec<u8>>, SyncsmithError> {
    let origin = format!("{}:{}", read.git_ref, path.display());
    let mut files = BTreeMap::new();
    for entry in listing.split(|byte| *byte == 0) {
        if entry.is_empty() {
            continue;
        }
        let Some(tab) = entry.iter().position(|&byte| byte == b'\t') else {
            continue;
        };
        let (meta, name) = (&entry[..tab], &entry[tab + 1..]);
        // Meta is `<mode> <type> <hash>`; only plain blobs are mirrored.
        if !meta.starts_with(b"100") {
            continue;
        }
        let relative = PathBuf::from(String::from_utf8_lossy(name).into_owned());
        let spec = format!("{}:{}/{}", read.git_ref, git_path(path), relative.display());
        let content = git::run_in(read.repo, &["show", &spec]).map_err(|failure| {
            SyncsmithError::SourceLoad {
                origin: origin.clone(),
                reason: failure.reason,
            }
        })?;
        files.insert(relative, content);
    }
    Ok(files)
}

/// Collects the files of a local folder source: relative path to content.
///
/// `.git` entries and symlinks are skipped and the walk depth is bounded,
/// with the same rules as the `init` source browser; empty directories are
/// ignored.
fn read_tree(root: &Path, origin: &str) -> Result<BTreeMap<PathBuf, Vec<u8>>, SyncsmithError> {
    let mut files = BTreeMap::new();
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), PathBuf::new(), 0)]);
    while let Some((dir, prefix, depth)) = queue.pop_front() {
        let entries = fs::read_dir(&dir).map_err(|source| source_load(origin, source))?;
        for entry in entries {
            let entry = entry.map_err(|source| source_load(origin, source))?;
            if entry.file_name() == ".git" {
                continue;
            }
            let file_type = entry
                .file_type()
                .map_err(|source| source_load(origin, source))?;
            if file_type.is_symlink() {
                continue;
            }
            let relative = prefix.join(entry.file_name());
            if file_type.is_dir() {
                if depth < MAX_WALK_DEPTH {
                    queue.push_back((entry.path(), relative, depth + 1));
                }
            } else {
                let content =
                    fs::read(entry.path()).map_err(|source| source_load(origin, source))?;
                files.insert(relative, content);
            }
        }
    }
    Ok(files)
}

fn source_load(origin: &str, source: std::io::Error) -> SyncsmithError {
    SyncsmithError::SourceLoad {
        origin: origin.to_string(),
        reason: source.to_string(),
    }
}

/// Resolves `path` against `config_dir` when it is relative.
fn resolve_against(path: &Path, config_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    }
}

/// Formats a path the way git expects inside a revision spec: forward
/// slashes, no leading `./`.
fn git_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::blob_hash;

    // Expected values produced by `git hash-object --stdin`.
    #[test]
    fn blob_hash_matches_git_hash_object_for_empty_content() {
        assert_eq!(blob_hash(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn blob_hash_matches_git_hash_object_for_text() {
        assert_eq!(
            blob_hash(b"alpha\n"),
            "4a58007052a65fbc2fc3f910f2855f45a4058e74"
        );
        assert_eq!(
            blob_hash(b"hello world\n"),
            "3b18e512dba79e4c8300dd08aeb37f8e728b8dad"
        );
    }

    #[test]
    fn blob_hash_depends_on_the_header_length() {
        // Same text without the trailing newline has a different hash.
        assert_ne!(blob_hash(b"alpha"), blob_hash(b"alpha\n"));
    }
}
