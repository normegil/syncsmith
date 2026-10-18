# Feature: `sync` command

Non-interactive synchronization of target files from their sources, driven by
`.syncsmith.toml`. Safe to run as a git pre-commit hook.

Parent input: `pre-spec.md` in this folder. Configuration schema:
`docs/configuration.md`.

## CLI

```
syncsmith sync [--all] [<target>...]
```

- Global flags (`--config`, `-v`, and the new `--json`) are reusable on every
  command.
- No arguments: synchronize all `automatic` entries only. This is what the
  pre-commit hook runs.
- `<target>...`: one or more target paths matched against entry `target`
  values after normalization (same normalization as configuration
  validation). Selected entries are synchronized regardless of policy
  (`automatic` or `manual`).
- `--all`: synchronize every entry, including `manual` ones. Mutually
  exclusive with target arguments.
- An unknown target is a usage error: exit non-zero before any entry is
  processed, listing the configured targets for reference.
- All diagnostics go to `stderr` (Rust-CLI-1). `stdout` carries the changed
  paths payload described below.
- Exit 0 when all processed entries succeeded; non-zero if any failed.
  A per-entry failure does not stop the other entries; they are reported
  individually with actionable context.

## Selection semantics

| Invocation                    | Entries processed                     |
| ----------------------------- | ------------------------------------- |
| `syncsmith sync`              | all `automatic` entries               |
| `syncsmith sync --all`        | every entry                           |
| `syncsmith sync <target>...`  | the matching entries, any policy      |

Entries are processed in configuration order (deterministic output, no
parallelism).

## Per-entry synchronization

Sources resolve as documented in `docs/configuration.md`:

- **git source**: shallow clone of `repository` at `ref` (default branch when
  omitted), reusing the bounded retry/timeout behavior of `src/git.rs`. The
  entry's `path` is read from the clone.
- **folder source**: the file or tree is read directly; the version of each
  file is its git blob hash.
- A single-file entry whose source path no longer exists (at the pinned ref
  or in the folder): the entry fails, the target is left untouched, and the
  remaining entries continue.

Modes:

- **overwrite**: mirror the latest source; local modifications are discarded.
  A folder source tree is fully mirrored: additions, removals, and content
  changes.
- **patch**:
  - Target missing: create it from the source (first synchronization).
  - Target unedited since the last synchronization (target content hash
    matches the lock): update directly to the latest source version.
  - Target edited: generate the patch of (last-synced source version →
    latest source version) and apply it onto the target as a three-way
    merge. On conflict, the file is written with standard conflict markers,
    the entry is reported as failed on `stderr`, and the lock record is not
    updated.
  - The last-synced source version is recovered from the source repository
    using the commit hash stored in the lock. Git sources only — see
    "Configuration changes".
- Empty directories in folder sources are ignored (git cannot represent
  them).

## Lock file

`.syncsmith.lock` (TOML), written in the same directory as the configuration
file (default: repository root), meant to be committed next to
`.syncsmith.toml`.

- Keyed by normalized target path, one record per target file (a folder entry
  yields one record per file in the mirrored tree).
- Each record stores the source version at the last successful
  synchronization: commit hash for git sources, content hash for folder
  sources.
- Records are updated only for entries that succeed; the file is written
  atomically at the end of the run.

## stdout contract

- Default: the paths modified, created, or deleted by the run, one per line,
  relative to the target repository root.
- Global `--json` flag (all commands; only `sync` produces structured output
  for now): stdout is a single JSON document, an array with one object per
  changed file:

  ```json
  [
    { "path": "lints/base.yml", "action": "updated" },
    { "path": "docs/aliases/zsh.txt", "action": "created" },
    { "path": "docs/aliases/old.sh", "action": "deleted" }
  ]
  ```

  `action` is one of `created`, `updated`, `deleted`.
- Diagnostics remain on `stderr` in both modes.

## Pre-commit hook

`syncsmith sync` reports exactly the files it changed; the hook stages them.
No targets are embedded in the hook script, so it never goes stale when the
configuration is edited.

Hook script installed by `init`:

```sh
#!/bin/sh
# Installed by syncsmith: run syncsmith sync before each commit.
changed=$(syncsmith sync) || exit 1
if [ -n "$changed" ]; then
  printf '%s\n' "$changed" | git add --pathspec-from-file=-
fi
```

- `git add --pathspec-from-file=-` handles paths containing spaces and
  stages deletions. Requires git 2.25+ (documented requirement).
- Existing hook behavior is unchanged otherwise: installation is offered by
  `init`, an existing hook is overwritten only after confirmation.

## Configuration changes

Folder sources are incompatible with `patch` mode (the last-synced source
version of a folder cannot be reconstructed from a content hash, so patching
an edited target is impossible):

- `Config::validate` rejects `Source::Folder` combined with `Mode::Patch`
  with an actionable error message.
- The `init` flow no longer offers `patch` when the selected source is a
  folder.
- `docs/configuration.md` is updated to document the incompatibility.

## Non-goals

- The `check` command (CI usage) — separate spec.
- Interactive conflict resolution: conflicts are reported, the user resolves
  them manually.
- Parallel entry processing.
