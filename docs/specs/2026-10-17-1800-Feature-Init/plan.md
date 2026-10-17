# Implementation Plan - 2026-10-17-1800-Feature-Init

This plan outlines the steps required to implement the interactive `syncsmith init` command per `docs/specs/2026-10-17-1800-Feature-Init/spec.md`.

## 1. Research and Analysis (Completed)

- Greenfield Rust 2024 binary, no dependencies. All modules are new.
- Config schema from `docs/configuration.md`: `[[sync]]` entries with `source` (git repository + ref + path, or folder), `target`, `mode` (patch|overwrite), `policy` (automatic|manual); env expansion; paths resolved relative to the config file's directory.
- Design decisions:
  - **Git interaction by shelling out to the `git` CLI** (shallow clone `--depth 1 --branch <ref>`, repo-root detection) rather than `git2`/`gix`: keeps the dependency tree small (`Commons-DEP-1`); the target is a git repo anyway so git is present. Remote clones get bounded retries with exponential backoff and a timeout (`Commons-EXT-1`).
  - **Interactive flow as a testable state machine**: all prompting goes through a `Prompter` trait; `inquire` is only the production implementation. Unit tests drive the flow with a scripted `Prompter`.
  - **Sync stub**: `sync::run(&SyncContext)` returns a `SyncsmithError::SyncNotImplemented`; `init` maps it to a stderr warning and exits 0. No `sync` subcommand is registered (non-goal).
  - Crates: `clap` (derive), `inquire`, `serde`, `toml`, `thiserror`, `tracing`, `tracing-subscriber` (fmt layer to stderr with `EnvFilter`).

## 2. Proposed Changes

Module layout:

```
src/
├── main.rs          # parse CLI, init tracing, dispatch, format errors, ExitCode
├── lib.rs           # module declarations
├── cli.rs           # clap derive: Cli, GlobalArgs, Command::Init
├── logging.rs       # verbosity → EnvFilter levels
├── error.rs         # SyncsmithError (thiserror)
├── config/
│   ├── mod.rs       # Config, SyncEntry, Source, Mode, Policy; validation
│   ├── model.rs     # serde types matching the documented schema
│   ├── env.rs       # $VAR / ${VAR} expansion for all string fields
│   └── io.rs        # load(path), atomic_write(path, &Config) (temp + rename)
├── discovery.rs     # find_repo_root(cwd), find_config(start) → upward search
├── git.rs           # repo root, shallow_clone(url, ref, dest) w/ backoff+timeout
├── sync.rs          # sync::run() stub → SyncNotImplemented
└── init/
    ├── mod.rs       # InitFlow orchestration: state machine, Abort semantics
    ├── flow.rs      # pure flow logic driven by Prompter trait + InitState
    ├── prompts.rs   # InquirePrompter (inquire-backed)
    ├── source_loader.rs # enumerate local tree; remote via shallow clone temp dir
    ├── target.rs    # default target computation + validation (coherence,
    │                #   duplicates, containment, collision warnings)
    ├── review.rs    # summary table data + per-entry edit/remove actions
    └── hook.rs      # precommit hook install (core.hooksPath-aware, confirm overwrite)
```

### 2.1 Scaffold and dependencies

- **`Cargo.toml`**: add `clap = { version = "4", features = ["derive"] }`, `inquire`, `serde` (derive), `toml`, `thiserror`, `tracing`, `tracing-subscriber`. Add `tempfile` and `assert_fs`/`predicates` as dev-dependencies for `tests/`.
- **`src/lib.rs`**: `pub mod` for all modules; `pub use error::SyncsmithError`.
- **`src/error.rs`**: `SyncsmithError` with variants: `ConfigNotFound { searched_from }`, `ConfigParse { path, source }`, `ConfigWrite { path, source }`, `InvalidTarget { target, reason }`, `DuplicateTarget { target }`, `GitCommand { args, source }`, `CloneFailed { url, ref, source }`, `SourceLoad { source, reason }`, `HookInstall { path, reason }`, `Aborted`, `SyncNotImplemented`, `InvalidArguments { reason }`.

### 2.2 CLI and logging

- **`src/cli.rs`**:
  - `GlobalArgs { config: Option<PathBuf>, verbose: u8 }` (flatten, `global = true`).
  - `Command::Init` (unit variant only — `--config`/`-v` are global).
- **`src/logging.rs`**: `init_tracing(verbosity: u8)` — levels: `0 = warn`, `1 = info`, `2 = debug`, `3+ = trace`; writer to stderr, no ANSI when not a TTY.
- **`src/main.rs`**: parse, init tracing, dispatch `Init`, catch `SyncsmithError`, print actionable message to stderr, `ExitCode::FAILURE`. `Aborted` prints "aborted, nothing written" and exits 0 (user aborts without modification are not errors; hard errors are non-zero).

### 2.3 Configuration model and I/O

- **`src/config/model.rs`**:
  - `Mode { Patch, Overwrite }`, `Policy { Automatic, Manual }` (serde rename_all lowercase).
  - `Source` enum adapted to the documented illustrative schema: `Repository { repository, ref: Option<String>, path }`, `Folder { folder }`.
  - `SyncEntry { source: Source, target: PathBuf, mode: Mode, policy: Policy }`; `Config { sync: Vec<SyncEntry> }`.
  - `impl Config::validate()`: non-empty targets, no duplicate target paths (normalized), mode/policy present.
- **`src/config/env.rs`**: `expand(input: &str) -> Result<String, SyncsmithError>` handling `$VAR` and `${VAR}`; applied to every string field on load. Unset variable = error with actionable message.
- **`src/config/io.rs`**: `load(path) -> Result<Config>`; `save_atomic(path, &Config)` — serialize, write `.<name>.tmp` in the same directory, `fs::rename` over destination.

### 2.4 Config discovery and git helpers

- **`src/discovery.rs`**: `resolve_config_path(args: &GlobalArgs) -> Result<Resolution>` returning `Resolution { path: PathBuf, exists: bool }`.
  - Explicit `--config`: use it, report whether it exists.
  - Otherwise walk up from cwd; stop at the first directory containing `.syncsmith.toml`, or at the git repository root (inclusive), whichever comes first. If a repo root exists but no config: `init` creates at repo root; other commands error `ConfigNotFound`.
  - Not in a repository and no config found: `init` creates in cwd; other commands error.
- **`src/git.rs`**:
  - `repo_root(start: &Path) -> Option<PathBuf>` (walk up looking for `.git`).
  - `shallow_clone(CloneRequest) -> Result<TempDir>`: `git clone --depth 1 --branch <ref> <url> <dest>` (branch omitted when ref not given); bounded retries (3) with exponential backoff, overall timeout (60s default); errors carry stderr excerpt.
  - `hooks_path(root) -> Result<PathBuf>`: reads `core.hooksPath` via `git config`, falls back to `.git/hooks`.

### 2.5 Init flow core (testable state machine)

- **`src/init/flow.rs`**:
  - `trait Prompter` — all prompt operations (`select`, `confirm`, `text`, multi-select), returning typed values; `Err` on Ctrl-C/EOF → `Aborted`.
  - `struct InitState { existing: Vec<SyncEntry>, added: Vec<SyncEntry> }`.
  - `run_flow(state, prompter, fs) -> Result<Vec<SyncEntry>>` — pure orchestration:
    1. source loop (`confirm("Add a source?")`) → source type select → path/ref text prompts;
    2. selection loop per source (file/folder multi-select from loaded tree, loop until done);
    3. target mapping: default = source-relative path mirrored at target root; editable per entry; `target.rs` validation;
    4. per-source default mode + policy questions;
    5. review loop: select entry → edit target/mode/policy or remove → until confirmed;
    6. returns final entry list.
- **`src/init/target.rs`** (pure):
  - `default_target(selection) -> PathBuf`.
  - `validate_target(ValidationInput) -> Result<(), SyncsmithError>`: file→file / folder→folder coherence, duplicate check against all entries, containment within target repo root (lexical normalization, reject `..` escape), collision warning flag when the target exists on disk but is not a configured target.
- **`src/init/source_loader.rs`**: `load_source(SourceRef) -> SourceTree` — walk local folder (bounded depth, skip `.git`, symlinks not followed) or shallow-clone remote and walk the clone; returns files and folders with relative paths. Environment-expanded before use.
- **`src/init/review.rs`**: entry rows, edit/remove semantics; pre-existing entries editable/removable, never duplicated.
- **`src/init/prompts.rs`**: `InquirePrompter` implementing `Prompter` with `inquire` (Select, Confirm, Text, MultiSelect).
- **`src/init/hook.rs`**: `offer_install(prompter, repo_root)` — confirm, resolve hooks dir (2.4), write `pre-commit` shell script calling `syncsmith sync` from the repo root; confirm before overwriting an existing file; no-op (info log) when not a git repo.
- **`src/init/mod.rs`**: `pub fn run(global: &GlobalArgs) -> Result<ExitState>` wiring: resolution → Add/Restart/Abort when config exists → flow → `save_atomic` → hook offer → `sync::run()` stub → map `SyncNotImplemented` to stderr warning.

### 2.6 Sync stub

- **`src/sync.rs`**: `pub fn run(ctx: &SyncContext) -> Result<(), SyncsmithError>` returning `Err(SyncNotImplemented)`. Reserved for the future sync spec; signature may evolve.

## 3. Verification Plan

### 3.1 Unit tests (inline `#[cfg(test)]` submodules, hermetic)

- `config/model.rs`: TOML parse of the documented example; round-trip serialize/parse; unknown field rejection; mode/policy casing.
- `config/env.rs`: `$VAR`, `${VAR}`, unset var → error, literal `$`, adjacent vars, expansion inside paths. (Env vars via `std::env::set_var` in serial tests — no disk/network.)
- `init/target.rs`: default target mirroring; file→file and folder→folder coherence; duplicate rejection; `..` escape rejection; existing-file collision warning flag.
- `init/flow.rs`: scripted-`Prompter` tests — add two sources and stop; abort mid-flow returns no entries; Restart vs Add semantics on existing config; review edit/remove; duplicate target between new and pre-existing entry rejected.
- `cli.rs`/`logging.rs`: `-v` count parsing, level mapping (pure function).
- `error.rs`: variant Display messages contain path/context.

### 3.2 Integration tests (`tests/`)

- `config_io.rs` (uses `tempfile`/`assert_fs`): load valid/invalid files; atomic save does not leave `.tmp` on success and leaves original intact on write failure; save over existing config.
- `discovery.rs`: temp dir tree with nested dirs — finds config in parent; stops at fake repo root (`.git` file); not-found resolution.
- `hook.rs`: temp git repo (`git init`) — hook written with expected content; declined → nothing written; existing hook → confirmed overwrite only. Local git only, no network.
- `shallow_clone.rs`: local bare repo fixture (`git init --bare` + a commit) cloned via `shallow_clone` with file:// URL; ref selection; failure case (nonexistent ref → `CloneFailed`). No network access.
- `init_flow.rs`: end-to-end on a temp target repo with a local folder source, driven by a scripted prompter (flow functions accept the prompter, so this exercises orchestration including config write and the sync-stub warning path).

### 3.3 Tooling gates

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` all green (Rust-GATE-1/2/3).

## 4. Risks and Considerations

- **Shelling out to git** assumes git ≥ 2.x in PATH; error surface is stderr text, kept in the error context. Tests use local file:// repos to stay hermetic.
- **inquire in unit tests**: avoided entirely — the flow depends only on the `Prompter` trait. inquire is exercised manually/by review.
- **Atomic rename**: guaranteed on same-directory rename (POSIX); Windows rename-over-existing needs `remove_file` first — handled with a targeted check (target platform is Linux).
- **Env expansion at load vs write**: expanded values are used in memory; the written file preserves the user's raw strings so `$HOME`-style sources stay portable across machines.
- **Aborted semantics**: Ctrl-C/EOF at any prompt exits without writes, exit 0.
- **Edition 2024 + current crate versions**: pinned to latest stable minor versions at implementation time.
