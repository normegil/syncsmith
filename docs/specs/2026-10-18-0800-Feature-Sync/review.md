# Code Review Report - 2026-10-18-0800-Feature-Sync

Iteration: 2
Date: 2026-10-18

## 1. Executive Summary

Iteration 1 blocked on two issues: unit tests that spawned `git` or touched
the filesystem from `#[cfg(test)]` submodules (Rust-T-1 / Commons-T-1), and a
swallowed stdout write failure in `main.rs` that could exit 0 with a missing
payload. Both are resolved and verified, and the minor notes from iteration 1
(tracing field name, empty lock file on no-op runs, missing GOTCHA comments)
were also addressed. The Test-Generator closed the remaining plan-3.2 gap with
three binary-level tests that spawn the built executable and pin the
exit-code/stdout contract, including the conflict and unknown-target paths.
All tooling gates pass: `cargo fmt --check` clean,
`cargo clippy --all-targets -- -D warnings` clean, and `cargo test` green
(158 tests, 0 failures). The one point raised for adjudication — whether a
failed (conflicted) entry may be excluded from the stdout payload — is
adjudicated in section 5 as conformant and preferable; my iteration-1 remark
that described the opposite was a misreading of the code, now reconciled.
No new blocking issues. Ready to merge.

## 2. Specification & Plan Compliance

- [PASS] Functional requirements coverage from spec.md. Unchanged from
  iteration 1 (which found full coverage): CLI shape and flags, selection
  semantics, per-entry overwrite/patch behavior, source resolution, lock file
  design, stdout contract in both formats, pre-commit hook script, and the
  folder/patch incompatibility across validation, `init` prompts, and docs.
  The binary-level tests added this iteration additionally pin the spec's
  exit-code requirements at the process level: exit 0 with the changed-paths
  payload on success (`the_binary_synchronizes_and_exits_zero_with_the_stdout_payload`,
  including the no-op second run with an empty payload), exit 1 on a per-entry
  conflict with the other entries' changes still on stdout and conflict
  markers in the file, and exit 1 on an unknown target before any processing
  with no lock file and no target written (`tests/sync_command.rs:787-874`).
- [PASS] Architectural fidelity to plan.md. Unchanged from iteration 1: the
  module layout, error set, and documentation updates follow the plan, with
  only justified additions (`SyncContext.target_root`, four extra error
  variants). Plan section 3's verification matrix is now fully covered,
  including the previously untested exit-code glue in `main.rs`.

## 3. Code Quality & Architecture

- Iteration-1 blocking item 1 (test placement) — RESOLVED. The three
  `merge_edited` tests now live in `tests/sync_merge.rs` with a module doc
  explaining the Rust-T-1 rationale; the missing-lock-file test moved to
  `tests/sync_command.rs:225` (`a_missing_lock_file_loads_as_empty`). The
  `src/sync` unit modules now contain only pure, hermetic tests (`decide`,
  `blob_hash`, lock round-trip/upsert/remove, selection, report, cli); a grep
  confirms no `TempDir`, `tempdir`, or `Command::new` remains under `src/`
  test modules.
- Iteration-1 blocking item 2 (stdout write failure) — RESOLVED.
  `main.rs::write_report` now returns `Result` and `sync_command` propagates
  the failure to a non-zero exit, with a comment explaining the hook
  rationale; behavior now matches `init`'s final-sync path exactly.
- Minor notes — RESOLVED: the `tracing::error!` field is renamed
  `target` → `entry` (`src/sync/mod.rs:110`); `run` no longer creates an empty
  `.syncsmith.lock` when the lock is absent and no records exist
  (`src/sync/mod.rs:117-120`, via the new `LockFile::is_empty`, with the
  right rationale that an existing file is still rewritten so record removals
  persist); the `sync_entry` partial-writes GOTCHA (`src/sync/mod.rs:191-196`)
  and the `merge-file` ≥255-hunks GOTCHA (`src/sync/merge.rs:141-148`) are
  documented.
- New binary-level tests are well built: they spawn
  `env!("CARGO_BIN_EXE_syncsmith")` with `current_dir` per child (no process
  cwd mutation, so no cross-test serialization needed) and the same hermetic
  git environment as the fixtures (`GIT_CONFIG_GLOBAL=/dev/null`,
  `GIT_CONFIG_NOSYSTEM=1`).
- All other iteration-1 quality findings stand unchanged and pass:
  Rust-A-1, Rust-ERR-1 (all `unwrap/expect` confined to test modules),
  Commons-FN-4 (parameter bundles in structs), Commons-O-1/Rust-CLI-1
  (stderr diagnostics, stdout payload).

## 4. Linting & Static Analysis

- `cargo fmt --check`: clean.
- `cargo clippy --all-targets -- -D warnings`: clean, zero warnings
  (Commons-L-1, Rust-GATE-1/2).
- `cargo test`: 158 tests, all passing — 90 lib unit tests (down from 94:
  the four misplaced tests moved out), and 68 integration tests across
  `config_io` (7), `discovery` (9), `hook` (9), `init_flow` (13),
  `shallow_clone` (6), `sync_command` (21, up from 17: +1 lock test, +3
  binary tests), `sync_merge` (3) (Rust-GATE-3).

## 5. Identified Bugs & Inconsistencies

### Adjudication: failed entries are excluded from the stdout payload

Iteration-1 items 5 and 7 conflicted, and the Test-Generator asked for an
explicit ruling. The actual behavior: on a conflicted patch, the file is
written with conflict markers (`src/sync/mod.rs` `MergeResult::Conflicted`
arm), a `ChangedFile` is constructed, but `sync_edited` returns
`Err(MergeConflict)` — `sync_entry` converts that into an `EntryFailure`
whose `EntryOutcome` (the constructed `ChangedFile` included) is discarded by
`run`. So a failed entry contributes nothing to stdout, even though its
already-written files remain on disk. The new binary test asserts exactly
this (conflict run reports `ok.txt` only, exit 1).

**Verdict: conformant and acceptable; no change required.** Rationale:

- The spec assigns conflict reporting to `stderr` and requires the lock
  record not be updated; it never requires the conflicted path to appear on
  stdout. The stdout sentence ("the paths modified, created, or deleted by
  the run") is a payload summary, and "failed entries do not appear in the
  recorded outcome" is a defensible, coherent reading of it.
- The exclusion makes the payload honest: stdout mirrors the lock — a file
  reported as `updated` is finalized and recorded; a conflicted file is
  neither. Listing it as `{"action":"updated"}` would mislead `--json`
  consumers about a file that is actually unresolved.
- It is safe for the hook: `changed=$(syncsmith sync) || exit 1` aborts
  before staging on the non-zero exit, so the payload difference cannot cause
  staging divergence; the conflicted file is named in the per-entry
  `MergeConflict` diagnostic on stderr, so nothing is hidden from the user.
- It unifies with the fail-fast rule of iteration-1 item 7: one uniform
  contract — a failed entry contributes nothing to stdout or the lock, while
  its partial disk writes remain (now documented as the `sync_entry` GOTCHA).

Reconciliation of the record: iteration-1 item 5's statement that the
conflicted file is "reported as `updated` on stdout" was a misreading of the
code on the reviewer's part (it described the constructed `ChangedFile`
without tracing its discard); iteration-1 item 7 described the actual
behavior. The iteration-1 text is superseded by this adjudication, and the
new binary test documents the contract going forward.

Remaining iteration-1 observations, unchanged and still non-blocking: the
pathological ≥255-hunks misclassification (now documented in code), the
user-dependent conflict-marker layout (tests assert style-common boundaries),
`is_remote` heuristic edge (pre-existing), and the `InvalidArguments` variant
being reused for a stdout I/O failure in `main.rs`/`init` (cosmetic; both
paths use it consistently).

### New observations (non-blocking)

- README's wording "stdout carries exactly the paths modified, created, or
  deleted by the run" reads, strictly, as including the marker-written
  conflicted file, which the adjudicated behavior excludes. One clarifying
  clause ("...by the run's successful entries") would remove the ambiguity.
  Documentation precision only; the spec wording carries the same nuance and
  the adjudicated reading resolves it.

## 6. Action Items & Next Steps

### Executor

None blocking. Optional (documentation precision, can be folded into any
future change): qualify the README stdout sentence per the adjudication in
section 5.

### Test-Generator

None. The binary-level exit-code tests close the last plan-3.2 gap; the
moved tests preserve the full iteration-1 coverage (nothing was dropped:
17 + 4 relocated + 3 binary = 24 sync-related tests against 17 before).

## Status: APPROVED
