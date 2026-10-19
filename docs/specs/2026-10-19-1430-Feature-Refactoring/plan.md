# Implementation Plan - 2026-10-19-1430-Feature-Refactoring

This plan outlines the steps required to implement the structural
refactoring specified in `docs/specs/2026-10-19-1430-Feature-Refactoring/spec.md`.

## 1. Research and Analysis (Completed)

- Commands today: `src/main.rs` holds `sync_command`/`write_report` and
  `check_command`/`CheckInvocation`/`write_check_payload`; `src/init/` is
  already a package; `src/sync/` and `src/check/` are the domain packages.
- `src/discovery.rs` is used by `init/mod.rs`, `sync/mod.rs`
  (`SyncContext::resolve`), and `tests/discovery.rs`.
- `config_dir_of` is duplicated in `init/mod.rs` and `sync/mod.rs`;
  `SyncContext::lock_path()` derives from it.
- `sync/mod.rs` and `check/mod.rs` import `crate::init::target`
  (`normalize`, `TargetFs`) — after the move this becomes
  `crate::commands::init::target`.
- Integration tests: `tests/hook.rs` and `tests/init_flow.rs` call
  `init::run_with(prompter, &GlobalArgs)` and assert on `ExitState`;
  `tests/sync_command.rs` and `tests/check_command.rs` construct
  `SyncContext` via struct literal (a new `lock_path` field requires
  updates) and also spawn the built binary for exit-code coverage.
- Confirmed design choices (validated by the user):
  - Command entry points live in a new `src/commands/` package.
  - `init` receives an `Option`-shaped entry type (`InitTarget`).
  - `lock_path` is a method on `Resolution`.
  - Hook offer is skipped when the final sync fails; error propagates
    (exit non-zero, configuration already written).

## 2. Proposed Changes

### 2.1 `discovery` becomes a `config` subpackage; `Resolution` gains `dir()` and `lock_path()`

- `git mv src/discovery.rs src/config/discovery.rs`; `config/mod.rs`
  adds `pub mod discovery;`; `src/lib.rs` drops `pub mod discovery;`.
- New methods on `Resolution`:
  - `dir() -> PathBuf`: parent of `path`, or `.` when it has none
    (replaces both private `config_dir_of` copies).
  - `lock_path() -> PathBuf`: `dir().join(LOCK_FILE_NAME)`.
- Delete `config_dir_of` in `init/mod.rs` and `sync/mod.rs`; call sites
  use `resolution.dir()`.
- Remove the `SyncContext::lock_path()` method; `SyncContext` gains a
  `pub lock_path: PathBuf` field populated from
  `resolution.lock_path()`; `sync::run` and `check::run` read the field.
- Update imports: `main.rs`, `init/mod.rs`, `sync/mod.rs`,
  `tests/discovery.rs`.

### 2.2 New `src/commands/` package; `main.rs` reduced to parse/log/resolve/dispatch

- `git mv src/init src/commands/init`; new `src/commands/mod.rs`:
  `pub mod check; pub mod init; pub mod sync;`; `src/lib.rs` exposes
  `pub mod commands;` and drops `pub mod init;`.
- `src/commands/sync.rs` (moved from `main.rs`):
  - `pub struct SyncInvocation { pub targets: Vec<PathBuf>, pub all: bool, pub json: bool }`
  - `pub fn run(resolution: &Resolution, invocation: SyncInvocation) -> Result<(), SyncsmithError>`
    containing the current `sync_command` + `write_report` logic
    (`SyncContext::resolve(&resolution)` internally).
- `src/commands/check.rs` (moved from `main.rs`):
  - `CheckInvocation` gains a `pub json: bool` field.
  - `pub fn run(resolution: &Resolution, invocation: CheckInvocation) -> Result<(), SyncsmithError>`
    containing the current `check_command` + `write_check_payload` logic.
- `SyncContext::resolve(&Resolution)` replaces `(&GlobalArgs)`: raises
  `SyncsmithError::ConfigNotFound` when `!resolution.exists` (same
  message/behavior); derives `config_dir` from `resolution.dir()` and
  `lock_path` from `resolution.lock_path()`.
- `src/main.rs` after the change: parse CLI (`Cli::parse`), initialize
  logging, `config::discovery::resolve_config_path(&cli.global)`,
  dispatch to `commands::{init, sync, check}::run` with the resolution,
  keep the `Aborted` → "aborted, nothing written" exit 0 mapping and the
  generic error → exit 1 mapping (`Rust-ERR-3`).

### 2.3 `init` entry types; `ExitState` removed

- New `InitTarget` in `src/commands/init/mod.rs`:
  ```rust
  pub struct InitTarget {
      /// Resolved configuration location (path, existence, dir, lock path).
      pub resolution: Resolution,
      /// The loaded configuration when the file exists; `None` when it
      /// must be created. Invariant: `existing.is_some() == resolution.exists`.
      pub existing: Option<Config>,
  }
  ```
  with `InitTarget::from_resolution(&Resolution) -> Result<InitTarget, SyncsmithError>`
  (loads, expands, validates when the file exists — same handling as
  today's `run_with` prologue).
- `init::run(target: InitTarget, json: bool) -> Result<(), SyncsmithError>`
  and `run_with(prompter: &dyn Prompter, target: InitTarget, json: bool)
  -> Result<(), SyncsmithError>`; `ExitState` is deleted; `verbose` is no
  longer passed anywhere in `init`.
- The `--config`-explicit-but-missing case works unchanged: `Some`/`None`
  on `existing` drives the fresh-config flow at that location.

### 2.4 `run_with` decomposition + hook timing

Split `run_with` into one subfunction per phase (all in
`src/commands/init/mod.rs` or a small `runner` submodule):

1. `initial_state(prompter, &Option<Config>) -> Result<InitState>`:
   existing-config Add/Restart/Abort flow or fresh default.
2. `flow_entries(prompter, fs, state) -> Result<Vec<SyncEntry>>`:
   interactive `Flow::run` + the empty-configuration confirmation
   (GOTCHA preserved: declining aborts before any write).
3. `save_config(path, entries) -> Result<Config>`: build, validate,
   `save_atomic`, trace.
4. `run_final_sync(target, json) -> Result<(), SyncsmithError>`: expand,
   validate, build `SyncContext` (with `lock_path` from
   `target.resolution.lock_path()`), sync automatic entries, write the
   stdout report, return `SyncFailures` when entries failed.
5. `offer_hook(prompter, repo_root)` **last**: runs only after a
   successful final sync; on sync failure the error propagates without
   the hook offer. Interrupt at the hook prompt still logs a warning and
   exits 0.

`repo_root`/`target_root` keep their current computation (cwd-based),
inside the `init` command.

### 2.5 `source_loader.rs` `walk` decomposition

In `src/commands/init/source_loader.rs`, split `walk` so that:

- `walk(root, origin)` keeps the BFS queue loop and final sort.
- A dedicated helper classifies one `fs::DirEntry`: skips `.git` and
  symlinks, builds the `TreeItem`, and returns the child `WalkNode` to
  queue (depth-bounded).

Semantics identical: `MAX_WALK_DEPTH` = 16, items sorted by relative
path, `SyncsmithError::SourceLoad` error reporting, non-folder roots
rejected.

### 2.6 Test updates (signatures/paths only, assertions unchanged)

- `tests/discovery.rs`: import `syncsmith::config::discovery::…`; add
  small tests for `Resolution::dir()` and `Resolution::lock_path()`.
- `tests/hook.rs`, `tests/init_flow.rs`: build `InitTarget` via a helper
  (`resolve_config_path` with an explicit `--config`, then
  `InitTarget::from_resolution`); `run_with(...).expect("init completes")`
  replaces `ExitState` assertions; scripted answer queues reordered —
  the hook confirm now comes after the final sync (assertions on the
  hook file and config content stay identical).
- `tests/sync_command.rs`, `tests/check_command.rs`: add the `lock_path`
  field to struct-literal `SyncContext` constructions
  (`config_dir.join(LOCK_FILE_NAME)`); binary-spawn sections untouched.

### 2.7 Known consideration

Domain packages (`sync`, `check`) will import
`crate::commands::init::target` for `normalize`/`TargetFs`. Minimal-diff
choice: import path update only, since the pre-spec says init's files
stay unchanged. A follow-up could relocate `target.rs` to a shared
module; out of scope here.

## 3. Verification Plan

### 3.1 Unit Tests

- In-module tests for `Resolution::dir()` and `Resolution::lock_path()`
  (pure path logic; add to `tests/discovery.rs` which already exercises
  the resolution behavior with temp dirs).
- Existing `src/cli.rs` in-module tests unchanged and passing.

### 3.2 Integration Tests

- All existing suites pass with updated signatures:
  `tests/discovery.rs`, `tests/hook.rs`, `tests/init_flow.rs`,
  `tests/sync_command.rs`, `tests/check_command.rs`,
  `tests/config_io.rs`, `tests/shallow_clone.rs`.
- Hook/init scripted queues adjusted for the new prompt order only;
  file/config assertions unchanged.
- Binary-spawn tests confirm unchanged exit codes for `sync`, `check`
  (`--no-fail`, `--json`, usage errors) and `init`.

### 3.3 Tooling Gates

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

## 4. Risks and Considerations

- **Scripted prompt order** in `hook.rs`/`init_flow.rs` is the most
  fragile point: the hook prompt moves after the sync report; only the
  queue order changes, never the assertions.
- **`InitTarget` invariant** (`existing.is_some() == resolution.exists`)
  holds only through `from_resolution`; documented on the type.
- **History preservation**: use `git mv` for all file moves.
- No external consumers: the crate's public API is only used by this
  binary and its tests, so the module reshuffle has no downstream
  migration.
- Behavior delta is limited to the hook-offer position in `init`
  (validated decision); everything else must be behavior-preserving.
