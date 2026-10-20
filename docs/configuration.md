# Configuration file

Synchronization is based on a toml file named `.syncsmith.toml` at the repository root (or the path given to `--config`). This page documents the exact schema the implementation reads and writes.

A configuration file is not required for a one-off synchronization: `syncsmith sync --override-from <Source> [--ref <ref>] [--source-path <path>] <Target>` synchronizes a single target directly from a source, loading no configuration and touching no lock file — the target is always overwritten with the latest source content, and it resolves against this configuration file's directory when the file exists (the repository root otherwise). See the `sync` section of the README for the full semantics.

The file holds one top-level `sync` array; each `[[sync]]` entry maps one source file or folder to one target path. Different targets may have different sources, but each target is mapped by exactly one entry.

For each entry:

- `source` — either:
  - a git repository (remote URL or local path to the repository), with:
    - `repository`: the repository URL or local path,
    - `ref` (optional): branch, tag, or commit; omitted means the repository's default branch,
    - `path`: the path of the file or folder inside the repository;
  - or a local folder, with `folder`: the path of the selected file or folder **itself** (not its parent). A whole folder maps to one entry whose tree is mirrored. Relative paths resolve against the directory containing the configuration file.
- `target` — the path of the target file or folder inside the target repository.
- `mode` — how the target is synchronized when a new version is available:
  - `patch`: the target is synchronized with the latest version of the source.
    - If the target has not been edited since the last synchronization, it is updated directly to the latest version of the source.
    - If the target has been edited since the last synchronization, a patch of the difference between the last synchronized version of the source and the latest available version of the source is generated and applied to the target.
      - If conflicts result from the operation, the synchronization fails for that entry and the conflict is reported — the user handles it.
  - `overwrite`: the target is synchronized using the latest source. Any modification done to the target by the user is discarded and ignored.

  `patch` is incompatible with folder sources (the last synchronized version of a folder cannot be reconstructed from a content hash, so an edited target could not be patched). A configuration combining a folder source with `patch` mode is rejected — use `overwrite` for folder sources.
- `policy` — when the entry is synchronized:
  - `automatic`: the entry is updated on every synchronization.
  - `manual`: the entry is synchronized only when specifically requested by the user.

Mode and policy names are lowercase and required; unknown fields in an entry are rejected.

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

## Validation

A configuration is rejected when loading or when written by `init` if:

- a `target` is empty, absolute, or lexically escapes the target repository (`..`),
- two entries normalize to the same target path (for example `a.yml` and `./a.yml`),
- an entry combines a folder source with `patch` mode (see above).

## Environment variables

`$VAR` and `${VAR}` references are expanded in all string fields (repository, ref, path, folder, target) **at load time, in memory only**. The `.syncsmith.toml` file itself keeps the raw `$VAR`/`${VAR}` strings, so a configuration written on one machine stays portable to machines where the variables resolve differently. A `$` that does not start a valid variable name stays literal; referencing an unset variable is an error.

## Path separators

All path fields of `.syncsmith.toml` and `.syncsmith.lock` (`path`, `folder`, `target`) are written with forward slashes, whatever the platform the file was generated on. Backslash separators in hand-written values are normalized at load time, so a configuration initialized on Windows keeps working; the portable form is written back the next time the file is saved.

## Synchronization semantics

The behaviors below describe the `sync` command.

- If a target file does not exist yet, the first synchronization creates it from the source.
- For a single-file entry, if the source file no longer exists at the pinned ref (or in the source folder), the synchronization fails for that entry: syncsmith reports an error, leaves the target untouched, and continues with the other entries.
- For a folder entry, the entire tree is mirrored: additions, content changes, and removals. A source-side deletion is mirrored only when the target file is unedited; in `patch` mode an edited target whose source file was deleted fails the entry instead of losing the edits (`overwrite` deletes regardless of local edits).
- In `patch` mode with an edited target, the last synchronized source version is recovered from the source repository using the commit hash stored in the lock. If it cannot be recovered (the server disallows fetching a commit by hash, or the commit was pruned), the entry fails with an actionable error; pinning a `ref` or using `overwrite` mode avoids the problem.
- Empty directories in folder sources are ignored (git cannot represent them).
- Entries are processed in configuration order, without parallelism. A per-entry failure does not stop the other entries; the run exits non-zero when any processed entry failed.

## Check semantics

The `check` command decides, per target file, whether it is synchronized — read-only, using the same lock records. The latest available version of a file is the source commit hash (git sources) or the content blob hash (folder sources).

- **Automatic** entries are in sync if and only if no patch needs to be applied: the lock version matches the latest available version, and the target is unedited (local edits are accepted in `patch` mode when no patch is pending, since a future synchronization only refreshes the lock record; in `overwrite` mode they are reported, as a synchronization would discard them).
- **Manual** entries ignore newer available source versions: they are in sync as long as the target is still at its last synchronized version. Local edits follow the mode rules: forbidden in `overwrite` mode, accepted in `patch` mode when the pending source patch would merge cleanly with them (a trial three-way merge, result discarded).
- A file without a lock record (never synchronized, or the lock file was not committed) is in sync only when its content is byte-identical to the latest source.
- A target file that no longer exists in the source (`missing` is the reverse) is reported `extra` — a synchronization would delete it.

Files that are not in sync are reported on stdout with one of the statuses `stale`, `edited`, `conflicted`, `missing`, `extra` (see the README); the command exits non-zero unless `--no-fail` is given. Evaluation failures (a source that cannot be loaded, an unrecoverable base version) always exit non-zero.

## State

Syncsmith records its state in a `.syncsmith.lock` file (TOML) in the same directory as the configuration file, meant to be committed next to `.syncsmith.toml`. It is keyed by normalized target path, with one record per synchronized target file (a folder entry yields one record per file of the mirrored tree). Each record holds:

- `target`: the normalized target path, relative to the target repository root;
- `version`: the source version at the last successful synchronization — the commit hash for git sources, the content blob hash for folder sources;
- `content`: the blob hash of the synchronized content, used to detect edits made to the target since then (a commit hash alone cannot detect them).

Records are updated only for entries that succeed, and the file is written atomically (temporary file + rename) once at the end of a run. A missing lock file loads as empty, which means existing targets are treated as unedited and updated directly on the next run.
