# Implementation Plan - 2026-10-19-0700-Feature-Check

This plan outlines the steps required to implement the `check` command: a
read-only verification that target files are synchronized, designed for CI.

## 1. Research and Analysis (completed)

- `src/cli.rs`: clap derive; `Command::Sync { targets, all }` pattern to
  mirror; `--json` is a global flag.
- `src/sync/`: `SyncContext::resolve` (config + lock path + target root),
  `selection::select` (policy/target matching, usage errors),
  `source::snapshot` (shallow clone / `git show` / folder walk; `version`
  = commit hash for git sources, `None` for folder sources), `lock::LockFile`,
  `merge::decide`/`merge_edited` (pure decision + temp-file three-way
  merge), `report::render/write` (lines/JSON), private helpers
  `read_target`, `list_target_files`, `current_dir`, `entry_failure`.
- Per-file version: git entries → `snapshot.version` (commit hash); folder
  entries → `blob_hash(latest content)` — matches what
  `FileSpec::lock_record` stores in the lock.
- Tests: unit tests in module `tests` submodules (Rust-T-1); integration
  tests in `tests/*.rs` with local git repositories plus a spawned-binary
  section (`tests/sync_command.rs` conventions); gates `cargo fmt
  --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  (Rust-GATE-1..3).

Key design choice: a new top-level `src/check/` module that reuses the
`sync` internals (exposed `pub(crate)`) instead of duplicating them. One
shared decision code path is not forced: `merge::decide` models *actions
to apply*, while check models *states to report* — so check gets its own
pure `evaluate`, reusing `is_edited` and `merge_edited` for trial merges.

## 2. Proposed Changes

### 2.1 `src/sync/mod.rs` — expose shared internals

- Make `read_target`, `list_target_files`, `current_dir`, `entry_failure`
  `pub(crate)` (signatures unchanged).
- Extract the base-recovery request construction from `sync_edited` into
  `pub(crate) fn recovery_request<'a>(entry: &'a SyncEntry, source_path:
  &Path) -> Result<RecoveryRequest<'a>, SyncsmithError>` (folder-source
  guard included); `sync_edited` uses it.

### 2.2 `src/sync/merge.rs` — expose edit detection

- `pub(crate) fn content_edited(record: &LockRecord, target_content:
  &[u8]) -> bool` (narrow form of the private `is_edited`), reused by
  `evaluate` without coupling to `FileState`.

### 2.3 `src/sync/report.rs` — generalize rendering (backward compatible)

- Extract `pub fn render_lines(paths: impl Iterator<Item = &Path>) ->
  String` and `pub fn render_json<T: Serialize>(items: &[T]) -> String`;
  `render`/`write` keep their current public signatures and delegate, so
  `main.rs` and `tests/sync_command.rs` are unaffected.

### 2.4 New `src/check/evaluate.rs` — pure evaluation

- `pub enum CheckStatus { Stale, Edited, Conflicted, Missing, Extra }`
  (serde lowercase) and `pub enum SyncState { InSync, OutOfSync(CheckStatus) }`.
- `pub struct CheckFileState<'a> { policy: Policy, mode: Mode, target:
  &Path (normalized key), latest: Option<&[u8]>, latest_version:
  Option<String> (commit hash, or blob hash for folder sources),
  target_content: Option<&[u8]>, record: Option<&LockRecord> }`.
- `pub fn evaluate(state: &CheckFileState) -> SyncState` implementing the
  specification rules; pure, no I/O. Trial merges are *not* performed here
  (see 2.5): `evaluate` returns the candidate outcome and the orchestrator
  refines `Conflicted` candidates.

### 2.5 New `src/check/mod.rs` — orchestration

- `pub struct OutOfSyncFile { path: PathBuf, status: CheckStatus }`
  (Serialize/Deserialize; mirrors `ChangedFile`).
- `pub struct CheckOutcome { out_of_sync: Vec<OutOfSyncFile>, failures:
  Vec<EntryFailure> }` (`EntryFailure` reused from `sync`).
- `pub fn run(ctx: &SyncContext, selection: &Selection) ->
  Result<CheckOutcome, SyncsmithError>`: load the lock read-only (never
  saved), iterate the selected entries in configuration order, snapshot
  via `source::snapshot`, and per file: read the target
  (`sync::read_target`), compute `latest_version` (commit hash or
  `blob_hash`), call `evaluate`; for `Conflicted` candidates (edited +
  patch + pending version), recover the base via `recovery_request` +
  `snapshot.recover`, run `merge_edited` as a trial merge, and map
  `Merged` → `Stale` (automatic) / `InSync` (manual), `Conflicted` →
  `Conflicted`. Tree entries add `Extra` for target files absent from the
  snapshot. Evaluation errors (source load, unrecoverable base) become
  `EntryFailure`s and the run continues with the other entries, exactly
  like `sync`.
- Never writes anything: no target writes, no `lock.save_atomic`.

### 2.6 `src/error.rs` — new variant

- `CheckOutOfSync { count: usize }` — message: "N target file(s) not in
  sync; run `syncsmith sync` to update them". Existing variants
  (`ConfigNotFound`, `TargetSelection`, `SourceLoad`,
  `SourceVersionUnavailable`, `GitCommand`, `UnsupportedContent`,
  `TargetIo`, `InvalidArguments`) cover everything else.

### 2.7 `src/cli.rs` — `Check` subcommand

- `Check { targets: Vec<PathBuf>, #[arg(short = 'a', long, conflicts_with
  = "targets")] all: bool, #[arg(long)] no_fail: bool }` plus parse tests
  (`-a`, `--no-fail`, targets, conflicts), mirroring the `sync` tests.

### 2.8 `src/main.rs` — dispatch

- `Command::Check { .. } => check_command(..)`: `SyncContext::resolve` →
  `Selection::from_args` → `check::run` → write the stdout payload (lines
  = paths only, JSON = `OutOfSyncFile` array) via the generalized report
  helpers → if `!out_of_sync.is_empty() && !no_fail` → `Err(
  CheckOutOfSync { count })`; any `failures` → non-zero exit regardless of
  `no_fail`. Diagnostics on stderr (Rust-CLI-1).

### 2.9 `src/lib.rs`

- `pub mod check;`.

## 3. Verification Plan

### 3.1 Unit tests (module `tests` submodules)

- `check/evaluate.rs`: table-driven coverage of every rule branch —
  automatic/manual × patch/overwrite × no-record/edited/unedited ×
  pending/aligned versions × missing target; pre-computed expected
  statuses (Commons-TQ-2).
- `cli.rs`: `check` parsing (`-a`, `--no-fail`, targets, conflicts),
  mirrors of the `sync` tests.
- `report.rs`: the new helpers render identical output for the existing
  sync cases (regression).
- `error.rs`: the `CheckOutOfSync` message contains the count and the
  suggested command.

### 3.2 Integration tests (`tests/check_command.rs`, reusing the
`tests/sync_command.rs` fixture conventions)

- Fresh repository, never synchronized, automatic entry: reports
  `missing`, exit non-zero; the same run with `--no-fail` exits 0 with the
  same stdout payload.
- Fully synchronized repository: empty stdout, exit 0 (lines and JSON).
- Automatic patch entry, source commit moved, unedited target: `stale`.
- Manual entry with a moved source: the default run ignores it; `--all`
  checks it and reports in sync (still at its lock version).
- Manual patch entry, local edits, clean pending patch: in sync under
  `--all`.
- Manual patch entry, local edits, conflicting pending patch:
  `conflicted`.
- Manual/automatic overwrite entry, local edits: `edited`.
- No lock record, content identical to the source: in sync; different:
  `stale`.
- Tree entry: target file absent from the source (`extra`), source file
  absent from the target (`missing`).
- `check <target>` selects any policy; unknown target → exit 1,
  actionable stderr, nothing processed.
- Read-only guarantee: target contents and lock file byte-identical
  before and after a check run.
- JSON shape: `[{"path":"...","status":"stale"}]`.
- Binary-level: exit codes for in-sync, not-in-sync, `--no-fail`,
  `--json`, and evaluation failure (source load failure exits non-zero
  even with `--no-fail`).
- Gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test` all green (Rust-GATE-1..3).

## 4. Risks and Considerations

- Backward compatibility: the `sync/report.rs` refactor must keep the
  current `render`/`write` API (used by `main.rs` and
  `tests/sync_command.rs`).
- `merge_edited`'s documented 255+ hunks exit-status GOTCHA is inherited
  by trial merges (diagnostic precision only, no corruption).
- Trial merges run only for edited patch-mode files with a pending
  version — cost bounded; remote git sources require network exactly
  like `sync` (CI expectation).
- `Extra` files in patch mode with local edits: `sync` would fail the
  entry rather than delete; check still reports `extra` (not-in-sync
  either way) — documented in the module docs.
- No new dependencies (Commons-DEP-1).
