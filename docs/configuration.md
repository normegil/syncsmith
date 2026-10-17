# Configuration file

Synchronization is based on a toml file named `.syncsmith.toml` at the repository root (or the path given to `--config`). This page documents the exact schema the implementation reads and writes.

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
- two entries normalize to the same target path (for example `a.yml` and `./a.yml`).

## Environment variables

`$VAR` and `${VAR}` references are expanded in all string fields (repository, ref, path, folder, target) **at load time, in memory only**. The `.syncsmith.toml` file itself keeps the raw `$VAR`/`${VAR}` strings, so a configuration written on one machine stays portable to machines where the variables resolve differently. A `$` that does not start a valid variable name stays literal; referencing an unset variable is an error.

## Edge cases (synchronization semantics)

The behaviors below describe the `sync` command, which is not implemented yet; the configuration schema above is final.

- If a target file does not exist yet, the first synchronization creates it from the source.
- For a single-file entry, if the source file no longer exists at the pinned ref (or in the source folder), the synchronization fails for that entry: syncsmith reports an error, leaves the target untouched, and continues with the other entries.

## State

Syncsmith records its state in a `.syncsmith.lock` file at the repository root, meant to be committed next to `.syncsmith.toml`. For each target file, it stores the version of the source at the last successful synchronization (commit hash for git sources, content hash for folder sources). This is used to detect edits made to the target and to generate patches in `patch` mode. The lock file is owned by the future `sync` implementation.
