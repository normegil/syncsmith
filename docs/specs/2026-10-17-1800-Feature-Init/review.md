# Code Review Report - 2026-10-17-1800-Feature-Init

Iteration: 2
Date: 2026-10-17T17:12:00Z

## 1. Executive Summary

Iteration 2 addressed every CHANGES_REQUIRED item from iteration 1. All claims were independently
re-verified: the fixes are present in the code, each is covered by the promised test, and all three
gates re-ran green:

- `cargo fmt --check`: pass (exit 0, no diff)
- `cargo clippy --all-targets -- -D warnings`: pass (0 warnings)
- `cargo test`: pass — 54 unit tests + 44 integration tests (config_io 7, discovery 9, hook 9,
  init_flow 13, shallow_clone 6), 0 failed

All MUST-level rules from `.ai/rules/commons.md` and `.ai/rules/rust.md` now pass. No `unwrap`/
`expect` outside test modules (grep re-scanned), no unused dependencies, lib/bin split intact,
typed errors, stderr discipline, parameter counts within limits, public APIs documented. The
implementation is spec-compliant and genuinely ready. One optional observation (F-8 rationale not
persisted in the repo) does not block approval.

## 2. Specification & Plan Compliance

### Resolution of iteration-1 findings

| ID | Finding | Status | Verification |
|---|---|---|---|
| F-5 (MUST) | Unused dev-dependencies `assert_fs`/`predicates` | Resolved | `[dev-dependencies]` section removed from `Cargo.toml`; grep of `Cargo.lock` finds neither crate. Downloads/lock bloat eliminated (Commons-DEP-1). |
| F-2 | Hook-prompt interrupt falsely reported "nothing written" | Resolved | `init/mod.rs::run_with` now maps `SyncsmithError::Aborted` from `hook::offer_install` to a warning ("skipping the pre-commit hook (the configuration is written)") and continues; returns `ExitState::Completed`. Since the post-write hook offer is the only prompt after `save_atomic`, every `Aborted` reaching `main.rs` now genuinely precedes any write — the message is accurate again. Test `interrupt_at_the_hook_prompt_completes_with_the_config_on_disk` asserts Completed + config on disk + no hook installed; pre-write abort test unchanged and passing. |
| F-1 | Restart did not show existing entries "as defaults where relevant" | Resolved | New `SourceDefaults` derived from the most recent discarded entry (`defaults_from`) pre-fills the location and ref prompts via `text_with_default` (`ask_source_ref`). Unit tests cover the derivation; integration test `run_with_restart_prefills_the_source_prompts_from_the_discarded_git_entry` verifies the prefilled repository and `stable` ref through the recorded `text_defaults()` harness. Reasonable interpretation of the spec wording. |
| F-7 | `Config::validate()` ignored containment on pre-existing entries | Resolved | `validate()` now routes every target through `target::validate_relative_target` (empty, absolute, `..`-escape) before the duplicate check. Called on the expanded existing config before any prompt in `run_with`. Test `run_with_rejects_an_existing_config_with_an_escaping_target_before_prompting` uses an empty answer script (any prompt would panic) and asserts `InvalidTarget` with the original file untouched and no `.tmp` left behind. |
| F-4 | Empty config written without confirmation | Resolved | `run_with` asks "The configuration has no entries. Write it anyway?" before `save_atomic`; decline returns `Aborted` pre-write. Two tests: restart-then-decline keeps the original configuration byte-identical; fresh-then-decline writes no file at all. |
| F-6 | Undrained stderr pipe could mask git errors behind a timeout kill | Resolved | `git.rs::run_clone` now drains stderr on a spawned thread and reports `last_line` of the collected text after `try_wait` returns; the child is also killed on a `try_wait` error. Test `clone_failure_with_a_large_stderr_reports_gits_message_instead_of_a_timeout` uses `GIT_TRACE=1` plus a >64 KiB URL to force a large stderr stream, with a `TraceGuard` keeping the env mutation test-local, and asserts git's message is reported and "timed out" is not. |
| F-3 | Missing Windows rename-over-existing fallback promised by the plan | Resolved | `config::io::save_atomic` has a `#[cfg(windows)]` remove-and-retry-once fallback (with `cfg_attr` handling of the `mut` warning on other platforms); doc comment updated. Not exercised on Linux by nature of the `cfg` gate — acceptable. |
| F-8 (optional) | Deterministic clone failures retried in full | Accepted as-is | Rationale (classifying git stderr text as transient vs. deterministic is brittle and locale-dependent; the bounded retry costs ≤ ~1.5 s) is technically sound and was only ever an informational, optional item. Minor note: the rationale is not persisted in the repo (no comment in `git.rs`, no note in `plan.md` §4) — see §5. |

### Spec re-check (unchanged areas)

All items verified green in iteration 1 remain green: CLI shape, discovery walk-up with `.git`
file support, Add/Restart/Abort semantics, source loop, shallow clone, selection loop, target
validation with collision warnings, per-source mode/policy, review table with edit/remove,
atomic write, hooksPath-aware hook install with overwrite confirmation, sync-stub warning path
exiting 0, stderr-only diagnostics, and exit codes. The non-goals are respected. The plan §4
"handled with a targeted check" wording about Windows rename now matches the code.

## 3. Code Quality & Architecture

- **Rust-A-1 (MUST)**: still met — `main.rs` is parsing/dispatch/exit-code only.
- **Rust-ERR-1 (NEVER)**: re-scanned — every `unwrap()`/`expect()` is inside a `#[cfg(test)]`
  module; `drain.join().unwrap_or_default()` in `git.rs` is an `unwrap_or` variant, safe.
- **Rust-ERR-2/3 (MUST)**: met; the new hook-interrupt handling is a local, typed-error mapping.
- **Commons-FN-4 (MUST)**: met. `ask_source_ref` grew to 3 parameters (prompter, kind, defaults) —
  at the limit, not over it; new types (`SourceDefaults`) encapsulate related data.
- **Commons-D-1 (MUST)**: met — `SourceDefaults` and the new branches are documented.
- **Commons-EXT-1 (MUST)**: met — unchanged bounded retry/deadline, now with proper stderr
  draining.
- **Commons-T (MUST)**: new tests remain hermetic (file:// fixtures, `TraceGuard` for env
  mutation, `CWD_LOCK` for cwd changes, empty-script panic trick for pre-prompt rejection).
- **Commons-TQ/T**: test names state behavior and condition; the empty-script design in the F-7
  test is a strong negative-verification technique.

Architecture unchanged and sound: `Prompter`/`FlowFs`/`TargetFs` seams, `inquire` confined to
`prompts.rs`, git confined to `git.rs`, no cross-layer leaks introduced by the fixes.

## 4. Linting & Static Analysis

- `cargo fmt --check`: clean (Rust-GATE-1).
- `cargo clippy --all-targets -- -D warnings`: clean (Rust-GATE-2 / Commons-L-1), including the
  new `#[cfg(windows)]` block with its `cfg_attr(not(windows), allow(unused_mut))` guard.
- `cargo test`: 98 tests total, all green (Rust-GATE-3). Counts match the agents' claims
  (54 unit + 44 integration).
- No `todo!`/`FIXME`/commented-out code; no dead code introduced by the changes.

## 5. Identified Bugs & Inconsistencies

No new bugs found. Residual observations, none blocking:

- **O-1 (informational)** — The F-8 rationale (brittle stderr classification) exists only in the
  agents' reports, not in the repo. A one-line comment in `git.rs::shallow_clone` or a note in
  `plan.md` §4 would preserve it for future maintainers. Optional.
- **O-2 (informational)** — The Windows rename fallback (`#[cfg(windows)]`) is untested on this
  Linux environment by construction; it is compile-checked only on Windows CI, if one ever
  exists. The plan already declares Linux as the target platform.
- **O-3 (informational)** — The `Interrupt` answer at the hook prompt now silently skips the hook
  with a `tracing::warn!`. This is the correct trade-off (config is durable, hook is optional),
  and the warning is visible at verbosity 0.

## 6. Action Items & Next Steps

None required for approval. Optional, non-blocking, for a future iteration:

1. Persist the F-8 retry rationale as a short comment in `src/git.rs::shallow_clone` (O-1).
2. Consider documenting in `README`/`docs/` that `init` may write an empty config only after an
   explicit confirmation, once user-facing documentation is extracted.

Both are cosmetic; no functional or compliance work remains.

## Status: APPROVED
