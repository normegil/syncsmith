# Implementation Plan - 2026-10-19-2240-Feature-Sync-Directly-From-Source

One-off, stateless synchronization: `syncsmith sync --override-from <Source>
[--ref <ref>] [--source-path <path>] <Target>`, bypassing the configuration
and the lock file entirely (see `spec.md`).

## 1. Research and Analysis (Completed)

- `Command::Sync` in `src/cli.rs` carries `targets: Vec<PathBuf>` and
  `all: bool`; dispatch in `src/main.rs` builds `SyncInvocation` and calls
  `sync::run_command`.
- `sync::run_command` resolves `SyncContext` (and fails with
  `ConfigNotFound` when the file is absent): the override path must branch
  **before** that, so no configuration is required or loaded.
- Reuse opportunity: `entry::walk` + `source::snapshot` + `merge::decide`
  + `target_io` provide everything needed. `FileState { mode: Overwrite,
  record: None }` never returns `MergeWithBase`, and `Decision::Delete`
  mirrors source-side deletions — exactly the full-mirror stateless
  semantics.
- `git::is_remote` (scheme `://` or `git@` prefix) distinguishes URLs;
  local repository reads use `git show <ref>:<path>`.
- Test conventions: unit tests live in module `#[cfg(test)]` submodules;
  integration tests in `tests/*.rs` build fixture git repos (no network)
  and spawn the built binary for exit-code coverage.

## 2. Proposed Changes

### 2.1 `src/cli.rs` — CLI surface

Extend `Command::Sync` with three optional flags (doc comments included):

- `override_from: Option<String>` — `#[arg(long = "override-from",
  value_name = "SOURCE", conflicts_with = "all")]`
- `git_ref: Option<String>` — `#[arg(long = "ref", value_name = "REF",
  requires = "override_from")]`
- `source_path: Option<PathBuf>` — `#[arg(long = "source-path",
  value_name = "PATH", requires = "override_from")]`

"Exactly one target" is enforced in code (clap cannot express a conditional
positional count).

### 2.2 `src/commands/sync/override.rs` — new module (the core)

- `pub struct OverrideArgs { source: String, git_ref: Option<String>,
  source_path: Option<PathBuf> }` — one struct, no 3+ argument functions.
- `pub fn resolve_source(args: &OverrideArgs, target: &Path) ->
  Result<Source, SyncsmithError>` — the disambiguation table (pure):
  - `git::is_remote(source)` → `Source::Repository`;
  - local path + (`--ref` or `--source-path`) → `Source::Repository`
    (local repository read at ref);
  - bare local path → `Source::Folder` (plain file/folder read).
  - Repository path default: `source_path` or the normalized target's
    **file name**.
- `pub fn run_command(args: OverrideArgs, target: PathBuf, json: bool) ->
  Result<(), SyncsmithError>`:
  1. `target::validate_relative_target(&target)` (same rules as
     configuration targets);
  2. cwd via `commands::sync::current_dir()`; build the stateless
     `SyncContext` (empty config, `config_dir = target_root = cwd`);
  3. synthetic `SyncEntry { source, target, mode: Overwrite, policy:
     Automatic }`;
  4. `entry::walk` → snapshot + file walk (reuses folder mirroring and
     git-at-ref reads);
  5. per file: `merge::decide(FileState { mode: Overwrite, record: None,
     .. })` → `Unchanged` skip / `Write` → `write_target` + report /
     `Delete` → `delete_target` + report; `Fail(e)` → propagate;
     `MergeWithBase` → unreachable, mapped to `InvalidArguments` with a
     documented safety comment (precedent in `source.rs`);
  6. report via the existing `write_report`; any failure aborts with `Err`
     (single entry — nothing to continue);
  7. **no lock file read or written, no configuration loaded.**

### 2.3 `src/commands/sync/context.rs` — stateless context constructor

Add `SyncContext::stateless(config_dir: PathBuf) -> Self` (empty config,
`config_dir == target_root`, lock path unused) with a doc comment carrying
the GOTCHA that the stateless run must never touch the lock. Avoids
constructing a dummy path at the call site.

### 2.4 `src/commands/sync/mod.rs` — wiring

- `SyncInvocation` gains `override_from: Option<OverrideArgs>`.
- `run_command` branches on `override_from` **before**
  `SyncContext::resolve` (no configuration required or loaded); takes
  `targets[0]`, rejecting `targets.len() != 1` with `InvalidArguments`
  (usage error).

### 2.5 `src/main.rs` — dispatch

Pass the three new fields into `SyncInvocation`.

### 2.6 `src/error.rs`

No new variants expected; reuse `InvalidArguments`, `SourceLoad`,
`GitCommand`. If review finds that `--ref` on a non-git local path needs a
clearer message than git's `fatal: not a git repository`, add a dedicated
variant then.

## 3. Verification Plan

### 3.1 Unit Tests

- `src/cli.rs` (module tests): `--override-from` parses; `--ref` /
  `--source-path` require it; `--override-from` + `--all` is a clap error;
  plain `sync` unaffected.
- `src/commands/sync/override.rs` (module tests, pure): the full
  disambiguation table (URL, URL with ref/path, plain local, local with
  `--ref`, local with `--source-path`, both); path default = target file
  name.

### 3.2 Integration Tests — new `tests/override_command.rs`

Fixture git repos, no network, conventions of `tests/sync_command.rs`:

- local file source creates the target with **no configuration file
  present** and **no lock file created**;
- folder source mirrors: additions, updates, deletions of target files
  absent from the source;
- local git repository + `--ref` + `--source-path` reads the committed
  version, not the dirty working tree;
- omitted `--source-path` defaults to the target's file name at the
  repository root;
- locally edited target is overwritten; an **existing** `.syncsmith.lock`
  is left byte-identical;
- works when a `.syncsmith.toml` exists and maps the same target
  (configuration ignored);
- missing source / unresolvable ref / `--ref` on a non-git path →
  non-zero exit, target untouched;
- two positional targets or zero → usage error; absolute / `..`-escaping
  target rejected;
- stdout report: lines and `--json` formats match `sync`'s;
- binary-spawn tests for exit codes (mirroring the last section of
  `tests/sync_command.rs`).

Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test` (Rust-GATE-1/2/3).

## 4. Risks and Considerations

- **Overwrite without warning** is the specified behavior; the report and
  the documentation must state it plainly.
- `SyncContext` reuse couples the stateless path to the lock-carrying
  struct; mitigated by the `stateless()` constructor and its GOTCHA doc.
- A local path + `--ref` on a non-git directory surfaces git's own error
  text; acceptable, revisit if review disagrees.
- No backward-compatibility risk: the existing `sync` surface is untouched;
  `--ref` / `--source-path` are new and `requires` prevents standalone use.
