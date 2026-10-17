This project defines an executable that keeps files synchronized between a source and a target.

- Source: Can be a git repository, or a local folder. Each synchronized target file maps to exactly one source file; different target files may have different sources.
- Target: A git repository. Syncsmith operates on the repository it is invoked from (the target repository); synchronized files are written inside it.

## Configuration file

Synchronization will be based on a toml file named `.syncsmith.toml` at the repository root. 

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

## Functionality

Commands
- `init`: Initializes or reinitializes the `.syncsmith.toml` configuration file. Interactive command that allows the user to set up the sources and target files for synchronization.
- `sync`: Synchronizes files according to their policy. With no arguments, only Automatic entries are processed. `sync <target...>` synchronizes the specified targets, including Manual ones. `sync --all` synchronizes every entry. Exits 0 when all processed entries succeeded, non-zero if any failed. May run as a precommit hook: the `init` flow offers to install the hook that calls `syncsmith sync`.
- `check`: Checks that all Automatic entries are up to date, and that Manual entries are still at their last synchronized version (local edits are allowed as long as they respect the Patch mode rules). Exits 0 when all checks pass, non-zero otherwise. Usually used as part of a CI pipeline.

## Rules

Before any change to the code, read the rules in the `.ai/rules` folder to ensure compliance with the user defined  rules.
