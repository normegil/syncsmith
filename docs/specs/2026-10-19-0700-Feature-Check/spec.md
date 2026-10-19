# Feature: `check` command

Non-interactive verification that the target files are synchronized, driven
by `.syncsmith.toml`. Read-only: no target file, lock file, or repository
state is ever written. Designed for CI pipelines.

Parent input: `pre-spec.md` in this folder. Configuration schema and lock
file: `docs/configuration.md`. Synchronization semantics:
`docs/specs/2026-10-18-0800-Feature-Sync/spec.md`.

## CLI

```
syncsmith check [--all] [--no-fail] [<target>...]
```

- Global flags (`--config`, `-v`, `--json`) are reusable.
- No arguments: check all `automatic` entries only.
- `-a, --all`: also check `manual` entries. Mutually exclusive with
  target arguments.
- `<target>...`: check the matching entries only, whatever their policy.
  Unknown targets are a usage error raised before any entry is processed,
  listing the configured targets (same behavior as `sync`).
- `--no-fail`: still report not-in-sync files on stdout, but exit 0 even
  when some are found. Fatal errors (config not found, unreadable lock
  file, source load failure) still exit non-zero.
- Selection reuses the `sync` selection semantics: the same `Selection`
  model, configuration order, no parallelism.
- "Mode has no effect" is interpreted as: `check` never *applies* a mode
  (no merge, no write); `patch`/`overwrite` only influence the evaluation
  rules below.

## Evaluation rules (per target file)

The latest available version of a file is the source commit hash (git
sources) or the content blob hash (folder sources), obtained by
snapshotting the sources exactly like `sync` does (shallow clone with
bounded retry/timeout for remotes, direct read for folders and local
repositories).

### Automatic entries

In sync if and only if no patch needs to be applied:

1. No lock record (never synchronized, or lock not committed): in sync iff
   the target content is byte-identical to the latest source; otherwise
   not in sync (`missing` when the target does not exist).
2. Lock record present and `lock.version` equals the latest available
   version (no patch pending):
   - unedited target (content hash matches the lock): in sync;
   - edited target in `patch` mode: in sync — no patch is pending and a
     future sync only refreshes the lock record;
   - edited target in `overwrite` mode: not in sync (`edited`) — sync
     would discard the edits.
3. Lock record present and `lock.version` differs from the latest
   available version (patch pending): not in sync:
   - unedited target, or any target in `overwrite` mode: `stale`;
   - edited target in `patch` mode: trial three-way merge with the base
     recovered from the lock version (same recovery as `sync`): clean
     merge → `stale`, conflict → `conflicted`.
4. Folder entries mirroring a tree: a target file absent from the source
   snapshot is not in sync (`extra`) — sync would delete it.

### Manual entries

Newer available source versions are ignored: a manual entry is in sync iff
the target is still at its last synchronized version. Local edits are
allowed as long as they respect the `patch` mode rules.

1. No lock record: in sync iff the target content is byte-identical to
   the latest source (same fallback as automatic entries).
2. Unedited target: in sync, whatever the source has done since.
3. Edited target:
   - `overwrite` mode: not in sync (`edited`);
   - `patch` mode: trial merge of the pending source patch (base recovered
     from the lock version, theirs = latest source): clean → in sync
     (the local edits respect the patch rules), conflict → not in sync
     (`conflicted`).

### Evaluation failures

A per-entry evaluation failure (source load failure, unrecoverable base
version) is reported like a `sync` entry failure on stderr and makes the
run exit non-zero even with `--no-fail`; the other entries are still
evaluated. CI can thus distinguish "out of date" from "cannot verify".

## Output

- All diagnostics go to stderr (Rust-CLI-1); stdout carries the
  not-in-sync payload.
- Default (lines): one not-in-sync path per line, relative to the target
  repository root, in processing order — the same shape as the `sync`
  report, so tooling is reusable.
- `--json`: a single JSON document mirroring the `sync` report shape,
  with a status instead of an action:

  ```json
  [
    { "path": "lints/base.yml", "status": "stale" },
    { "path": "docs/aliases/zsh.txt", "status": "conflicted" }
  ]
  ```

Statuses:

| Status       | Meaning                                                     |
| ------------ | ----------------------------------------------------------- |
| `stale`      | a patch is pending (source moved since the last sync)        |
| `edited`     | local edits that a sync would not preserve                  |
| `conflicted` | the pending patch conflicts with local edits (`patch` mode)  |
| `missing`    | the target file does not exist but the source has it         |
| `extra`      | the target file exists but the source no longer has it       |

## Exit codes

- 0: every checked entry is in sync, or `--no-fail` was given and no
  fatal error occurred.
- Non-zero: any checked entry is not in sync (without `--no-fail`), or any
  evaluation, usage, or fatal error occurred (always, `--no-fail`
  notwithstanding).

## Reuse requirement

Reuse the existing `sync` machinery as much as possible, adapting it where
needed (including the modules that own the shared functions). Avoid
duplication of the `sync` command functions in particular:

- selection: `sync::selection` reused as is;
- source snapshots and version recovery: `sync::source` reused as is;
- lock loading: `sync::lock` reused as is;
- edit detection and decision logic: `sync::merge` extended or refactored
  to expose the file state evaluation without writing, so `sync` and
  `check` share one decision code path;
- three-way merge: `sync::merge::merge_edited` reused for trial merges,
  without persisting its result;
- report rendering: `sync::report` extended to carry the check payload
  (path + status) in the same lines/JSON formats;
- stdout contract and error types: shared with `sync` (`SyncsmithError`
  gains the check-specific variants it needs).

## Non-goals

- Writing anything (targets, lock file) — `check` never mutates the
  repository.
- Interactive resolution of conflicts: conflicts are only reported.
- Parallel entry processing.
