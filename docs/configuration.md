# Configuration file

Synchronization is based on a toml file named `.syncsmith.toml` at the repository root.

For each file or folder in need of synchronization, the file will contain:
- The source, either:
  - a git repository (remote URL or local path to the repository), plus a ref (branch, tag, or commit; defaults to the repository's default branch when omitted) and the path of the file inside the repository, or
  - a local folder, plus the path of the file inside it.
- The target file which will be kept synchronized with the source (path inside the target repository).
- Mode: How the file will be synchronized (if a new version is available).
  - Patch: The target is synchronized with the latest version of the source.
    - If the target has not been edited since the last synchronization, it is updated directly to the latest version of the source.
    - If the target has been edited since the last synchronization, a patch of the difference between the last synchronized version of the source and the latest available version of the source is generated and applied to the target.
      - If conflicts result from the operation, fail the synchronization and warn about the conflict - Let the user handle it.
  - Overwrite: The target is synchronized using the latest source. Any modifications done to the target by the user is discarded and ignored.
- Policy: When the file will be synchronized.
  - Automatic: The file is automatically updated with every synchronization.
  - Manual: The file is only synchronized if specifically requested by the user.

Example (key names are illustrative, the exact schema is an implementation detail):

```toml
[[sync]]
source = { repository = "https://github.com/example/rules.git", ref = "main", path = "lints/base.yml" }
target = "lints/base.yml"
mode = "patch"
policy = "automatic"

[[sync]]
source = { folder = "${HOME}/dotfiles" }
target = "docs/aliases/"
mode = "overwrite"
policy = "manual"
```

An entry may map a single file or a whole folder. When a folder is mapped, syncsmith mirrors the entire tree, including files added to or removed from the source folder.

Edge cases:
- If a target file does not exist yet, the first synchronization creates it from the source.
- For a single-file entry, if the source file no longer exists at the pinned ref (or in the source folder), the synchronization fails for that entry: syncsmith reports an error, leaves the target untouched, and continues with the other entries.

Environment variables are expanded in all string fields of `.syncsmith.toml`, using the `$VAR` and `${VAR}` syntax.

Syncsmith records its state in a `.syncsmith.lock` file at the repository root, meant to be committed next to `.syncsmith.toml`. For each target file, it stores the version of the source at the last successful synchronization (commit hash for git sources, content hash for folder sources). This is used to detect edits made to the target and to generate patches in Patch mode.
