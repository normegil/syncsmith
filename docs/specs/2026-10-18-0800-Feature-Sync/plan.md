# Implementation Plan - 2026-10-18-0800-Feature-Sync

This plan outlines the steps required to implement the `syncsmith sync`
command: non-interactive synchronization of target files from their sources,
driven by `.syncsmith.toml`, safe to run as a git pre-commit hook.

## 1. Research and Analysis (Completed)

- Entry point reserved at `src/sync.rs::run(&SyncContext)` (currently returns
  `SyncsmithError::SyncNotImplemented`); `init` calls it after writing the
  configuration and special-cases the not-implemented error.
- Reusable components identified:
  - `discovery::resolve_config_path` — configuration path resolution.
  - `config::io::load`, `Config::expand_env().validate()` — loading and
    validating the configuration.
  - `git::shallow_clone` — bounded-retry shallow clone.
  - `init::target::normalize` — lexical target normalization.
  - `init::source_loader::walk` rules — `.git` and symlink skipping, bounded
    depth.
  - `config::io::save_atomic` pattern — atomic temp+rename writes.
- Module layout: `src/sync.rs` becomes `src/sync/mod.rs` with submodules
  `selection`, `lock`, `source`, `merge`, `report`.
- New dependencies (approved): `sha1` (git blob hashes; avoids one
  `git hash-object` spawn per file) and `serde_json` (`--json` output).
- No backward-compatibility surface exists: syncsmith has no released usage,
  so invalid configurations are simply rejected and the
  `SyncNotImplemented` special case in `init` is deleted.

## 2. Proposed Changes

### 2.1 CLI (`src/cli.rs`, `src/main.rs`)

- Add `Command::Sync { targets: Vec<PathBuf>, all: bool }` with clap
  `conflicts_with` between `--all` and `targets`.
- Add global `--json: bool` to `GlobalArgs`.
- `main.rs` dispatches `Command::Sync` to `sync::run` and prints the report
  via the report writer (line format by default, JSON with `--json`); exits
  non-zero when any entry failed.

### 2.2 Errors (`src/error.rs`)

New variants, all with actionable messages carrying the failing target or
source:

- `LockWrite { path, source }` — the lock file could not be written.
- `MergeConflict { target }` — per-entry conflict (collected, not fatal to
  the run).
- `SourceVersionUnavailable { origin, version, reason }` — the last-synced
  source version cannot be recovered (unknown SHA on the remote, pruned
  local repository).
- `UnsupportedContent { target, reason }` — binary/invalid-UTF-8 content
  with an edited target in patch mode.
- `TargetSelection { unknown, configured }` — unknown target argument,
  listing the configured targets.
- Remove `SyncNotImplemented` once `sync` is implemented.

### 2.3 Lock file (`src/sync/lock.rs`)

- TOML `.syncsmith.lock` in the same directory as the configuration file.
- One record per target file (a folder entry yields one record per file in
  the mirrored tree):
  - `target`: normalized target path,
  - `version`: source version at last successful sync (commit hash for git
    sources, blob hash for folder sources),
  - `content`: blob hash of the synced content, used for edit detection
    (a commit hash alone cannot detect target edits).
- Atomic write (temp file + rename, same pattern as `config::io`).
- Missing lock file loads as empty; records updated only for successful
  entries, written once at the end of the run.

### 2.4 Source snapshots (`src/sync/source.rs`)

- Latest content per entry:
  - folder source: walk + read (reuse the `.git`/symlink/depth rules).
  - local git repository: read at ref directly with `git show` (`-C <repo>`),
    no clone.
  - remote git repository: `git::shallow_clone`, then read at ref.
- Old version recovery for patch mode (git sources only):
  - local repository: `git -C <repo> show <sha>:<path>`.
  - remote: `git fetch --depth 1 origin <sha>` inside the clone, then
    `git show <sha>:<path>`.
  - unrecoverable version (server disallows fetching by SHA, pruned
    repository): the entry fails with `SourceVersionUnavailable`, suggesting
    a pinned ref.
- Folder versioning: git blob hash (`sha1("blob <len>\0" + content)`),
  computed in Rust with the `sha1` crate.
- Empty directories are ignored (git cannot represent them).

### 2.5 Three-way merge and mirroring (`src/sync/merge.rs`)

- Patch with edited target: `git merge-file` over three temp files with
  `-L ours -L base -L theirs` labels; base = last-synced source content,
  theirs = latest source content.
  - Clean merge: write the merged result.
  - Conflict: write the file with conflict markers; the entry fails with
    `MergeConflict`; the lock record is not updated.
  - Binary/invalid-UTF-8 content with an edited target: the entry fails with
    `UnsupportedContent` (no silent corruption).
- Direct update (unedited or missing target): write the latest content.
- Folder mirroring (both modes):
  - additions and content changes mirrored;
  - source-side deletions: mirrored when the target file is unedited; an
    edited target with a deleted source fails the entry (patch mode) —
    overwrite mode deletes regardless of local edits.

### 2.6 Selection (`src/sync/selection.rs`, pure)

- No arguments: `automatic` entries.
- `--all`: every entry.
- Explicit targets: entries whose normalized `target` matches, any policy;
  unknown target → `TargetSelection` error before any entry is processed,
  listing configured targets.

### 2.7 Orchestrator (`src/sync/mod.rs`)

- `SyncContext` keeps `config` + `config_dir`; add the lock path resolution
  and `SyncOutcome { changed: Vec<ChangedFile>, failures: Vec<EntryFailure> }`.
- Per entry, in configuration order: snapshot latest → compare with lock →
  apply by mode → collect `ChangedFile { path, action }`
  (`created` / `updated` / `deleted`).
- Per-entry failures are logged to `stderr` and do not stop the other
  entries; the lock is updated only for successful entries and written once
  at the end.

### 2.8 stdout report (`src/sync/report.rs`)

- Default: changed paths, one per line, relative to the target repository
  root.
- `--json`: single JSON document — an array of
  `{ "path": ..., "action": "created" | "updated" | "deleted" }`.
- The same writer is used by the `sync` command and by `init`'s final sync
  step; diagnostics always go to `stderr`.

### 2.9 Configuration changes

- `Config::validate` (`src/config/model.rs`): reject `Source::Folder`
  combined with `Mode::Patch` with a dedicated error variant.
- `init` flow (`src/init/flow.rs`): when the source is a local folder,
  `patch` is not offered (mode prompt offers `overwrite` only, info log).
- `init` (`src/init/mod.rs`): drop the `SyncNotImplemented` special case;
  the final sync uses the shared report writer and propagates failures.

### 2.10 Pre-commit hook (`src/init/hook.rs`)

New script content:

```sh
#!/bin/sh
# Installed by syncsmith: run syncsmith sync before each commit.
changed=$(syncsmith sync) || exit 1
if [ -n "$changed" ]; then
  printf '%s\n' "$changed" | git add --pathspec-from-file=-
fi
```

### 2.11 Documentation

- `docs/configuration.md`: document the folder/patch incompatibility and the
  lock file record fields.
- `README.md`: `sync` usage, selection table, stdout contract, git 2.25+
  requirement for the hook.

## 3. Verification Plan

### 3.1 Unit Tests (in `#[cfg(test)]` submodules, hermetic)

- `src/sync/selection.rs`: policy filtering; `--all`; explicit target match
  after normalization; unknown-target error listing configured targets.
- `src/sync/lock.rs`: round-trip through TOML; missing file loads empty;
  record update keeps other records.
- `src/sync/source.rs`: blob hash against known precomputed values.
- `src/sync/merge.rs`: merge decisions over prepared temp files (clean,
  conflict, deletion cases).
- `src/sync/report.rs`: line and JSON rendering of the same report.
- `src/cli.rs`: `--all` conflicts with targets; `--json` parses globally.
- `src/config/model.rs`: folder+patch rejected by `validate`.

### 3.2 Integration Tests (`tests/`, local git fixtures, no network)

- `tests/sync_command.rs`:
  - first synchronization creates target files and lock records;
  - overwrite folder entry mirrors additions, updates, and removals;
  - patch entry with unedited target updates directly;
  - patch entry with edited target applies cleanly (three-way);
  - conflict leaves conflict markers, entry fails, exit non-zero, other
    entries still synchronized;
  - missing source file fails only that entry;
  - `sync --all` and explicit target selection (including manual entries);
  - stdout line format and `--json` format;
  - lock updated only for successful entries.
- `tests/hook.rs`: installed script content matches the new hook.
- `tests/init_flow.rs`: folder sources no longer offer `patch`.

### 3.3 Tooling gates

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

## 4. Risks and Considerations

- **Fetching by SHA**: `git fetch origin <sha>` requires
  `uploadpack.allowAnySHA1InWant` (enabled on GitHub and GitLab, possibly
  not on self-hosted servers). Mitigation: actionable
  `SourceVersionUnavailable` error suggesting a pinned ref.
- **New dependencies**: `sha1` (RustCrypto, minimal) and `serde_json`;
  both approved during planning.
- **Pathological paths**: filenames containing newlines are not handled by
  the hook's line-based staging; acceptable, documented.
- **Determinism**: entries processed in configuration order, no
  parallelism.
