# Code Review Report - 2026-10-19-1430-Feature-Refactoring

Iteration: 1
Date: 2026-10-19

## 1. Executive Summary

The structural refactoring is faithful to `spec.md` and `plan.md` and is ready
to merge. `main.rs` is reduced to parse/log/resolve/dispatch, the three command
runners live in the new `src/commands/` package, `discovery` is a `config`
subpackage with `dir()`/`lock_path()` on `Resolution`, `ExitState` is gone, and
the `init` hook offer now runs after the final synchronization (the single
validated behavior change). All three gates pass in the working tree:

- `cargo fmt --check`: clean.
- `cargo clippy --all-targets -- -D warnings`: zero warnings.
- `cargo test`: 199 tests, 0 failed (106 lib, 19 check, 7 config_io, 9
  discovery, 10 hook, 17 init_flow, 6 shallow_clone, 22 sync_command,
  3 sync_merge).

The Executor's five deviations and the Test-Generator's relocation judgment
call were each examined and are all acceptable (details in section 2). No
MUST-level rule violation was found. The findings below are minor, non-blocking
observations and optional improvements.

## 2. Specification & Plan Compliance

- [PASS] Functional requirements coverage from spec.md.
  - Section 1 (structure): `src/commands/{mod.rs, sync.rs, check.rs}`,
    `src/commands/init/` (flow, hook, prompts, review, source_loader, target
    unchanged apart from the documented test fix), `src/sync/` and
    `src/check/` untouched as domain packages. `main.rs` now parses, inits
    logging, resolves the configuration once in `dispatch`, and delegates.
  - Section 2 (resolution before dispatch): `discovery` moved under
    `src/config/` keeping its public surface (`Resolution`,
    `resolve_config_path`, `find_config`); `Resolution::dir()` replaces both
    `config_dir_of` copies (grep confirms no leftovers);
    `Resolution::lock_path()` replaces `SyncContext::lock_path()`; `sync` and
    `check` fail with `ConfigNotFound` when `!resolution.exists`, both now
    covered by new tests.
  - Section 3 (init): `InitTarget { resolution, existing }` with the
    `existing.is_some() == resolution.exists` invariant documented on the
    type; `run`/`run_with` take `(InitTarget, json)` only; `ExitState`
    deleted (grep confirms no references); `run_with` decomposed into
    `initial_state`, `flow_entries`, `save_config`, `run_final_sync`,
    `offer_hook`; GOTCHA comments preserved and updated for the new ordering;
    hook offered only after a successful final sync, with the
    sync-failure path propagating (covered by the new
    `a_failed_final_sync_skips_the_hook_offer_and_propagates` test).
  - Section 4 (walk decomposition): `classify` + `ClassifiedEntry` extract the
    per-entry logic; BFS queue loop, `MAX_WALK_DEPTH = 16`, `.git`/symlink
    skipping, sort by relative path, and `SourceLoad` error reporting are
    byte-for-byte equivalent to the previous implementation.
  - Section 5 (out of scope): no CLI argument, config format, or sync/check
    semantic changes. Binary-spawn tests confirm unchanged exit codes.
- [PASS] Architectural fidelity to plan.md: every task in plan Section 2 is
  implemented, including `SyncInvocation`/`CheckInvocation` shapes,
  `SyncContext::resolve(&Resolution)`, `InitTarget::from_resolution`, and the
  documented known consideration (domain packages importing
  `crate::commands::init::target`, plan 2.7).
- [PASS] Test fidelity: integration-test updates are limited to import paths,
  entry construction, and scripted-answer queue position (hook answer moved
  after the sync); file/config assertions are unchanged, as required. The
  `ExitState::Completed` assertions disappear exactly as the spec sanctions.

### Executor deviations (all acceptable)

1. **`FinalSync` struct instead of the planned `run_final_sync(target, json)`
   signature** (`src/commands/init/mod.rs`). Encapsulating config, resolution,
   target_root, and json respects `Commons-FN-4`/`Rust-A-2`; behavior identical.
2. **In-module flow test fix: `Answer::Select("target")` → `"Target"`
   (`src/commands/init/flow.rs:941`).** This is a repair, not an assertion
   change: commit `dfd1de8` capitalized the field choices in `review.rs`
   (`"Target"/"Mode"/"Policy"`) but left the scripted answer lowercase, so
   `review_edit_changes_target_of_entry` could only fail at HEAD (the scripted
   prompter returns the raw value and `edit_entry` rejects `"target"` with
   `InvalidArguments`). Without this fix the gate `cargo test` could not pass.
3. **`write_report` made `pub(crate)` in `commands/sync.rs` and shared with
   `init`.** The plan requires "the same report writer" for the final sync;
   this is the minimal way to honor it. Payload is byte-identical.
4. **`init` reuses `sync::current_dir()` instead of re-mapping
   `std::env::current_dir` inline.** Consolidation of a duplicated error
   mapping; same error content.
5. **`InitTarget` carries the full `Resolution` rather than the spec sketch's
   bare `path: PathBuf`.** This is the plan's user-validated refinement; the
   spec sketch is superseded by the plan.

### Test-Generator judgment call (acceptable)

- `Resolution::dir()`/`lock_path()` tests placed in a `#[cfg(test)]` submodule
  of `src/config/discovery.rs` instead of `tests/discovery.rs`. The plan's
  Section 3.1 parenthetical suggested the latter, but `Rust-T-1` (MUST) says
  pure in-memory tests belong in-module; the MUST-level rule wins. The tests
  are pure (no temp dirs, no filesystem), well-named, and document the `""`
  parent edge case of a bare relative filename.

## 3. Code Quality & Architecture

- **Layering (Commons-A-2, Rust-A-1)**: clean. `main.rs` is 36 lines of
  parse/log/dispatch plus the exit-code mapping; command runners own stdout
  payloads and exit semantics; `sync`/`check` domain packages hold no CLI
  knowledge. `commands::init` reusing `commands::sync::write_report` keeps the
  two report writers unified.
- **Function design (Commons-FN-4, Commons-FN-1)**: all new functions have at
  most 3 parameters; `run_with` is now a linear five-phase pipeline that reads
  top-to-bottom. `walk` + `classify` removes the nested-continuation noise of
  the previous loop.
- **Error handling (Rust-ERR-1/2/3)**: no `unwrap`/`expect` in non-test code;
  typed `SyncsmithError` throughout; `main` keeps the `Aborted` → "aborted,
  nothing written" exit 0 mapping and the generic → exit 1 mapping. The
  "aborted, nothing written" message stays accurate because the only
  post-write prompt (the hook offer) converts its own `Aborted` into
  `Ok(())` inside `offer_hook`.
- **Documentation (Commons-D-1/D-3)**: every new public item (`SyncInvocation`,
  `CheckInvocation`, `InitTarget`, `Resolution::dir`/`lock_path`, `run`
  functions) carries doc comments describing parameters, failure conditions,
  and the invariants; GOTCHA comments on write ordering and the empty-config
  confirmation are preserved and updated.
- **Observability (Commons-O-1, Rust-CLI-1)**: unchanged; diagnostics to
  stderr, payloads to stdout.

Minor observations (non-blocking):

1. **`config::discovery` now imports `crate::sync::lock::LOCK_FILE_NAME`**
   (`src/config/discovery.rs:8`), a config→domain dependency that did not
   exist before. It is exactly what the spec mandates (`resolution.lock_path()
   = dir().join(LOCK_FILE_NAME)`), but a follow-up could move the lock file
   name constant to a shared location to keep `config` independent of `sync`.
2. **`dispatch` clones `targets` and `no_fail`** because it borrows `cli`
   (`src/main.rs:39-60`). Matching on an owned `cli.command` (or destructuring)
   would avoid the allocations (`Rust-C-3`). Cosmetic.
3. **`init_target` helper duplicated** in `tests/hook.rs` and
   `tests/init_flow.rs`. Hosting it in `tests/common/mod.rs` would remove the
   duplication (`Commons-C-2`). Cosmetic.
4. **`InitTarget` has public fields**, so the documented invariant
   (`existing.is_some() == resolution.exists`) is only enforced through
   `from_resolution`. With no external consumers this is acceptable; a
   private-fields + constructor design would make it airtight.

## 4. Linting & Static Analysis

- `cargo fmt --check`: PASS (zero diffs).
- `cargo clippy --all-targets -- -D warnings`: PASS (zero warnings), satisfying
  `Commons-L-1`, `Rust-C-1`, `Rust-GATE-1/2`.
- No dead code: `ExitState`, `config_dir_of`, `SyncContext::lock_path`, and
   all `crate::init::`/`crate::discovery` paths are fully gone (verified by
   grep). No commented-out code introduced.

## 5. Identified Bugs & Inconsistencies

None blocking. Edge cases checked explicitly:

- **Abort paths**: interrupt at the hook prompt still logs a warning and exits
  0 with the configuration on disk (`offer_hook`); the pre-write aborts
  ("Abort" choice, empty-config decline, prompt interrupts) still exit 0 via
  the `Aborted` mapping with nothing written. The new failed-final-sync test
  confirms `SyncFailures` propagates, the hook file is absent, and the
  configuration is already on disk.
- **Missing config**: `sync`/`check` with a non-existent resolved configuration
  fail with `ConfigNotFound` before any processing (no lock file created) —
  asserted by the two new command tests; `init` with `--config` pointing at a
  missing file starts the fresh flow at that location via
  `existing == None`.
- **Empty config**: unchanged; the confirmation prompt still gates the write.
- **Lock path consistency**: `SyncContext.lock_path` is now populated only from
  `resolution.lock_path()` in the two construction sites (`resolve` and the
  init final sync), both test `SyncContext` literals use
  `config_dir.join(LOCK_FILE_NAME)` — consistent with `dir().join(...)`
  semantics; the `dir()` fallbacks (`.` for parentless paths, `""` for bare
  relative names) match the previous `config_dir_of` behavior exactly.
- **Pre-existing breakage repaired**: `review_edit_changes_target_of_entry`
  was red at HEAD (`dfd1de8` capitalized the choice labels without updating
  the scripted answer); the working tree restores it.

## 6. Action Items & Next Steps

No blocking items. Optional, for a follow-up (not this iteration):

- **Executor** (optional): avoid the `targets`/`no_fail` clones in
  `main::dispatch` by matching on the owned `cli.command`; consider moving
  `LOCK_FILE_NAME` out of `sync::lock` if the config→sync dependency should
  disappear.
- **Test-Generator** (optional): hoist the duplicated `init_target` helper into
  `tests/common/mod.rs`.
- **Whoever commits** (hygiene, not code): the `git mv` renames are staged but
  `src/commands/{mod,sync,check}.rs` are untracked and the content
  modifications are unstaged; stage everything in one commit so history
  preservation (`-M` rename detection) holds.

## Status: APPROVED
