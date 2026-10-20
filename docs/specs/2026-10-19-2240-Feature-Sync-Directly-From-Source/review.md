# Review — Iteration 2 — 2026-10-19-2240-Feature-Sync-Directly-From-Source

Iteration 2 re-review. Iteration 1 (status CHANGES_REQUIRED, one blocking
must-fix E1 and one should-fix E2) is superseded by this review. Scope of this
pass: verify the Executor's fixes, re-run the gates, and re-verify the
stateless guarantees on the direct path. Diff vs iteration 1: README.md and
AGENTS.md documentation (E1), an empty/blank-source guard in
`src/commands/sync/direct.rs` (E2), and three new tests in
`tests/override_command.rs` (E2 ×2, T1 ×1). No other tracked file changed
(`src/cli.rs`, `context.rs`, `mod.rs`, `main.rs`, `tests/sync_command.rs`
diffs are byte-identical to iteration 1).

## Executive Summary

Both action items are resolved and verified. E1: README gained a dedicated
"One-off sync from a source: `--override-from`" subsection stating the
stateless semantics — including the overwrite-without-warning behavior, plainly
worded, satisfying plan risk #1 — and `--config` silently ignored; AGENTS.md's
`sync` bullet carries the one-off form and its semantics. E2: empty and blank
`--override-from` values are rejected at the top of `resolve_source` — pure,
before any filesystem effect — with an `InvalidArguments` error whose message
names the flag, backed by two unit tests, a library-level no-effect test
(both `""` and `"   "`, asserting nothing is written), and a binary test
(non-zero exit, empty stdout, stderr naming the flag). T1: the
subdirectory-cwd binary test was added and proves the target resolves against
the current directory, not the repository root. The Executor's scoping choice
(E2 guards only `--override-from`) is evaluated below and accepted: empty
`--ref` fails fast and safely via git (`rev-parse ""` → exit 128, wrapped in
`SourceLoad`, no side effects — verified empirically), and an empty
`--source-path` resolves to the repository root tree (`HEAD:`), a well-defined
git object, not the cwd-mirroring foot-gun that justified guarding the source
flag. All three gates pass; the stateless guarantees still hold. No must-fix
defect remains: APPROVED.

## Specification & Plan Compliance — PASS

Iteration-1 findings, resolved:

- E1 (was MUST-FIX, Commons-D-2 / plan risk #1) — FIXED. README.md:78-91: the
  new subsection documents the command line, the stateless guarantees (no
  `.syncsmith.toml` loaded or required, no `.syncsmith.lock` read or written,
  `--config` silently ignored, existing configuration or lock untouched),
  the overwrite-without-warning semantics stated plainly ("the target is
  always overwritten with the latest source content — local edits are
  discarded"), folder mirroring with deletions, cwd-based target resolution
  with the same validation rules as configuration targets, the `--all`
  conflict, the source-disambiguation rules and defaults, and the exit/report
  parity with `sync` including `--json`. Consistent with the clap help text
  (`src/cli.rs:69-85`). AGENTS.md:14 extends the `sync` bullet with the
  one-off form and its stateless/overwrite/cwd semantics.
- E2 (was SHOULD-FIX) — FIXED for `--override-from`.
  `src/commands/sync/direct.rs:52-58` rejects an empty or blank source with
  `InvalidArguments` before any resolution; the doc comment
  (`src/commands/sync/direct.rs:47-50`) states the rationale truthfully.
  Coverage: `empty_source_is_rejected` and `blank_source_is_rejected` (pure
  unit tests),
  `an_empty_override_source_is_a_usage_error_before_any_effect`
  (library-level, both inputs, asserts nothing is written),
  `an_empty_override_source_fails_from_the_binary` (exit non-zero, stdout
  empty, stderr names `--override-from`, target not created).
- T1 (was optional) — DONE. `the_target_resolves_against_the_current_directory_not_the_repository_root`
  spawns the binary from `nested/` inside the target repository and asserts the
  target lands at `nested/app.yml` and NOT at the repository root — the spec's
  cwd-resolution clause is now proven for the target side, not just the
  source side.

Scoping choice evaluation (E2 guards only the source flag) — ACCEPTED:

- Empty `--ref ""`: the repository path builds `rev-parse ""`, which git
  rejects (verified empirically: exit 128, "ambiguous argument"). The failure
  surfaces as `SourceLoad` before any target write — non-zero exit, no side
  effects — exactly the plan-section-4 approach already accepted for ref
  misuse. Not a defect.
- Empty `--source-path ""`: `git cat-file -t "HEAD:"` resolves to the root
  tree (verified empirically); the run mirrors the repository root into the
  target folder. Degenerate input, but well-defined, repo-contained, and
  distinct in kind from the iteration-1 foot-gun (an empty *folder* source
  mirroring the caller's current directory, including files outside any
  repository). Not a defect.
- Optional hardening remains noted below (H1) for symmetry only.

Spec and plan compliance otherwise unchanged from iteration 1 — command
surface, usage errors, stateless semantics, source-resolution table, defaults,
and report/exit-code parity all verified there and re-confirmed unchanged in
this pass.

## Code Quality & Architecture

- The E2 guard is placed correctly: first statement of the pure
  `resolve_source`, executed before target validation effects, context
  construction, and any snapshot — the library-level test proves
  write-freedom. Error message is actionable and names the flag
  (Commons-O-3).
- `resolve_source` stays pure (Commons-C-4) and arity-2 (Commons-FN-4); the
  guard adds no state, no I/O, no new dependencies.
- The stateless guarantees still hold, re-verified on the current working
  tree: `direct::run_command` (`src/commands/sync/direct.rs:81-107`) touches
  no lock; `entry::walk`/`source::snapshot`/`target_io` never read
  `ctx.lock_path`; `run.rs`'s lock access is not on the direct path; the
  override branch in `src/commands/sync/mod.rs:67-76` still fires before
  `SyncContext::resolve` (no configuration loaded or required); `--config`
  resolution remains path-only. The GOTCHA doc on `SyncContext::stateless`
  and the `MergeWithBase` guard comment remain truthful.
- Doc comments: the new guard is documented on the function it belongs to;
  no undocumented public items. No `unwrap`/`expect` outside `#[cfg(test)]`;
  no dead code; no new `#[allow]`.

## Linting & Static Analysis

All three gates, run on the reviewed working tree:

| Gate | Result |
|---|---|
| `cargo fmt --check` | exit 0, clean |
| `cargo clippy --all-targets -- -D warnings` | exit 0, zero warnings |
| `cargo test` | exit 0 — 130 unit (lib), 116 integration pass; `tests/override_command.rs` 18/18 (was 15, +3: two E2 tests, one T1 test), `src/commands/sync/direct.rs` module tests 11 (was 9, +2 E2) |

Test coverage: iteration-1 plan-section-3 coverage is unchanged and complete;
the new tests extend it on the exact points the iteration-1 review asked for.

## Identified Bugs & Inconsistencies

1. RESOLVED (was finding 1) — documentation synchronized; see E1 above.
2. RESOLVED (was finding 2) — empty/blank `--override-from` rejected; see E2
   above.
3. REMAINS, PRE-EXISTING, OUT OF SCOPE — `validate_relative_target` accepts
   `"."` and `"foo/.."` (normalizes to an empty path, `src/target.rs:33-68`).
   Shared with config-driven sync; the direct path now partly contains it
   (an empty normalized target cannot derive a repository source-path default
   and fails with `InvalidArguments`). Recommend a follow-up guard in
   `src/target.rs` for both flows; not blocking this spec.
4. REMAINS, NIT — the clones in `resolve_source`
   (`src/commands/sync/direct.rs:66-73`) could be moves since `run_command`
   owns the args (Rust-C-3, SHOULD). Cosmetic; the shared, pure signature is
   a valid trade-off.
5. NEW, OPTIONAL HARDENING (H1) — for symmetry, `--ref ""` and
   `--source-path ""` could be rejected with `InvalidArguments` alongside the
   source guard. Today both fail safely (see the scoping evaluation above),
   so this is polish, not a defect. If taken, reuse the E2 test pattern.

## Action Items

Executor:
- None blocking. Optional: H1 (symmetric empty-value guards for `--ref` and
  `--source-path`, with the E2-style regression tests) and the clone nit
  (finding 4), both at the author's discretion.

Test-Generator:
- None. If H1 lands, mirror the E2 test pattern for the two flags.

## Status

APPROVED

All iteration-1 must-fix and should-fix items are resolved and verified; the
gates are clean; the stateless guarantees hold. The remaining entries are
pre-existing (finding 3) or optional polish (findings 4, H1) and do not block.
