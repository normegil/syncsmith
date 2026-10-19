# Feature: Structural Refactoring

- **SpecName**: `2026-10-19-1430-Feature-Refactoring`
- **Status**: Validated by the user
- **Date**: 2026-10-19

## Objective

Pure structural refactoring of the `syncsmith` codebase: no behavior change,
same CLI surface, same stdout payloads, same exit codes — except the `init`
hook-offer timing described in section 3. Existing integration tests keep
passing and are updated only where module paths and function signatures
change; their assertions must not change.

The refactoring applies the project coding rules (`Rust-A-1`, `Rust-A-2`,
`Commons-FN-1`, `Commons-FN-4`, `Commons-C-2`): `main.rs` reduced to
parse/log/dispatch, commands isolated in a dedicated package, long functions
decomposed, and shared helpers consolidated on the types that own them.

## 1. Project structure

The `init`, `sync` and `check` command entry points live in a new
`src/commands/` package (and subfolders):

```
src/commands/mod.rs        ← command dispatch surface
src/commands/init/        ← moved from src/init/ (flow, hook, prompts,
                             review, source_loader, target unchanged)
src/commands/sync.rs      ← sync_command, write_report (moved from main.rs)
src/commands/check.rs     ← CheckInvocation, check_command,
                             write_check_payload (moved from main.rs)
```

`src/sync/` and `src/check/` remain the domain packages holding the
non-CLI logic (merge, source snapshots, selection, report, lock,
evaluation).

After the move, `src/main.rs` only parses the CLI arguments, initializes
logging, resolves the configuration path, and delegates to a command
runner.

## 2. Configuration resolution before dispatch

`discovery.rs` is a subpackage of `config`: it becomes
`src/config/discovery.rs` (or `src/config/discovery/mod.rs`), keeping its
public surface (`Resolution`, `resolve_config_path`, `find_config`).

`main.rs` resolves the configuration file path once, before invoking any
command-specific code, and passes the resolved result as a parameter to the
commands.

On `Resolution` (the resolved config):

- `resolution.dir()`: directory containing the configuration file,
  replacing the two private `config_dir_of` copies in `init` and `sync`.
- `resolution.lock_path()`: `dir().join(LOCK_FILE_NAME)`. The
  `lock_path()` method on `SyncContext` is removed; the lock path is
  derived from the resolution instead.

The `sync` and `check` commands require an existing configuration: when
`Resolution.exists` is false they fail with `SyncsmithError::ConfigNotFound`
as today.

## 3. `init` package

Located at `src/commands/init/`.

### Entry type

`init::run` receives the resolved configuration as a type that reflects
the file might not exist yet, using `Some`/`None`:

```rust
pub struct InitTarget {
    /// Where the configuration is written (always present, existing or not).
    pub path: PathBuf,
    /// The loaded configuration when the file exists; `None` when it must
    /// be created.
    pub existing: Option<Config>,
}
```

- `Some(config)` drives the existing Add/Restart/Abort flow.
- `None` starts from a default state.

### Signature

`run` (and `run_with`) no longer receive the verbose count or the whole
`GlobalArgs`: only what is needed is passed (`json: bool`, plus the
`InitTarget` and the prompter for `run_with`).

### `ExitState` removed

`ExitState` is deleted: `Completed` was its only variant and every other
outcome is already conveyed by errors. `run` and `run_with` return
`Result<(), SyncsmithError>`.

### `run_with` decomposition

`run_with` is split into small, readable subfunctions, one per phase:

1. State setup: load/validate the existing configuration or start fresh.
2. Interactive flow and the empty-configuration confirmation.
3. Atomic write of the configuration.
4. Final synchronization of the automatic entries and report writing.
5. Pre-commit hook offer.

Existing GOTCHA comments about the write-before-hook-prompt and empty-config
confirmation are preserved and updated to match the new ordering.

### Hook timing

The pre-commit hook is offered at the end of the process, after the final
synchronization has been completed, per the validated decision:

- Hook offer happens only after a successful final sync.
- When the final sync has per-entry failures, the hook offer is skipped and
  the error propagates: exit non-zero, configuration already written.

## 4. `source_loader.rs`

In `src/commands/init/source_loader.rs`, the `walk` function is decomposed
into subfunctions (for instance: directory-entry classification versus
queue management) without changing walk semantics:

- Bounded depth (`MAX_WALK_DEPTH` = 16).
- `.git` entries and symlinks skipped.
- Items sorted by relative path.
- Same error reporting (`SyncsmithError::SourceLoad`).

## 5. Out of scope

- CLI arguments and flags.
- Configuration file format and validation rules.
- Sync/check semantics, merge and lock file formats.
- New features of any kind.

## 6. Testing requirements

- All existing integration tests keep passing; they are updated only for
  the new module paths and function signatures (`discovery` moves under
  `config`, `run_with` takes the new entry types, `ExitState` usages
  disappear).
- Test assertions must not change: the refactoring is behavior-preserving
  apart from the hook-offer ordering in `init`.
- Standard gates apply: `cargo fmt --check`, `cargo clippy --all-targets
  -- -D warnings`, `cargo test`.
