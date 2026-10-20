# Sync Directly From Source

## Motivation

Some situations make a `.syncsmith.toml` file in the target repo undesirable:
a single one-off import, or a repository that must stay config-free. The
alternative is a direct, one-off synchronization from a source to a target
that bypasses the configuration entirely.

## Command

```text
syncsmith sync --override-from <Source> [--ref <ref>] [--source-path <path>] <Target>
```

- `--override-from <Source>` — a remote repository URL, a local git
  repository path, or a local file/folder path.
- `--ref <ref>` — optional branch, tag, or commit; repository sources only.
- `--source-path <path>` — optional path of the file or folder inside a
  repository source.
- `<Target>` — exactly one relative target path, resolved against the
  directory containing the configuration file when one exists (its
  location only — nothing is loaded), otherwise the git repository root
  containing the current directory, or the current directory outside any
  repository. Same validation rules as configuration targets: non-empty,
  relative, no `..` escape.
- Usage errors (non-zero exit, no side effects):
  - `--override-from` combined with `--all` or with more than one positional
    target;
  - `--ref` or `--source-path` given for a source that resolves to a plain
    local path;
  - the required `<Target>` positional missing.
- The configuration is never loaded: an existing `.syncsmith.toml` —
  including one given by `--config` — only determines the directory the
  target resolves against; its entries are ignored.

## Semantics

- **Stateless.** No `.syncsmith.toml` is loaded (the command works when no
  configuration file exists and ignores any existing entries), and no
  `.syncsmith.lock` is read or written. The target is always overwritten
  with the latest source content; local edits are discarded without
  warning.
- **Folder sources fully mirror the tree** — additions, updates, and
  deletions — like configuration-driven `overwrite` folder entries. A
  single-file source writes a single target file. Parent directories of
  the target are created as needed.
- **Exit code, reporting, and error handling are identical to `sync`:**
  changed target paths are reported on stdout (`--json` supported, same
  report format as `sync`), diagnostics go to stderr, exit 0 on success,
  non-zero on failure.

## Source resolution

| Input | Resolution |
|---|---|
| Remote URL (scheme or SCP-like form) | Git repository, always |
| Local path, no `--ref`/`--source-path` | Plain file/folder read, relative to the current directory |
| Local path, `--ref` and/or `--source-path` | Git repository, read at the given ref |

Defaults (the "good defaults" of the pre-spec):

- `--ref` omitted → the repository default branch (`HEAD`).
- `--source-path` omitted → the target's file/folder name, searched at the
  repository root.

Failure cases, each reported with an actionable error and the target left
untouched:

- the source path is missing, or not a file or folder in the repository;
- `--ref`/`--source-path` given for a non-git local path;
- the ref cannot be resolved (unknown branch, tag, or commit).

## Out of scope

- Any `check` counterpart: a stateless sync leaves no record, so `check`
  keeps reporting such targets by its existing rules.
- Adding the one-off mapping to the configuration file (`init` remains the
  way to persist a mapping).
