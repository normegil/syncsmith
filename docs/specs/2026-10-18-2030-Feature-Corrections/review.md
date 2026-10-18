# Lead Review — 2026-10-18-2030-Feature-Corrections

Reviewer: Lead Reviewer (validation pass)
Scope: working-tree changes of the Executor and Test-Generator against `spec.md` and `plan.md`.
Repository state: branch `main`, uncommitted changes over commit `66637a7`.

Review history:
- Iteration 1: CHANGES_REQUIRED — one action item (F-1, spec 1.2 indicator missing on the
  empty-configuration confirm).
- Iteration 2 (this pass): F-1 verified resolved, gates re-run as evidence — APPROVED.

## 1. Executive Summary

The implementation is a high-fidelity realization of the spec. Every enumerated prompt
wording, default, and label change (spec 1.1-1.7, 2.1-2.2, 3.1-3.2, 4.1-4.5, 5.1-5.2) is in
place, the `Prompter` trait redesign matches the non-functional constraints, and the new
`Flow` struct cleanly enables the `add another entry` review action. All quality gates
pass, re-verified in iteration 2: `cargo fmt --check` clean,
`cargo clippy --all-targets -- -D warnings` with 0 warnings, and `cargo test` with 166
tests passing (96 unit + 70 integration), 0 failed. Config serialization is untouched
(`name()` stays lowercase for serde; `from_name` now also accepts capitalized labels),
and existing tests were updated rather than weakened.

The single iteration-1 action item is resolved: the `The configuration has no entries.
Write it anyway?` confirm (src/init/mod.rs:70-71) now passes `Some(false)`, so inquire
renders the `(y/N)` indicator with "do not write" preselected — consistent with the
empty-selection confirm and the conservative GOTCHA comment (declining aborts before any
write). The hook-overwrite prompt correctly remains default-less per spec 1.7
(src/init/hook.rs:50). No test scripts needed updating: both scripted prompters ignore
the confirm default argument and replay answers positionally, so the fix is invisible to
them — confirmed by the full suite passing unchanged.

Known observation (b) — `colored` resolving TTY against stdout while the summary prints
to stderr — remains accepted per plan Section 4 Risks; `NO_COLOR` is respected either way
and the failure mode is cosmetic (ANSI codes in redirected stderr only).

## 2. Specification & Plan Compliance

| Area | Verdict | Evidence |
|---|---|---|
| 1.1 Uppercase starts, `:` on text prompts | PASS | `Folder path:`, `Repository URL or local path:`, `Target path for 'x':` (src/init/flow.rs:396-404, 458), `New target path:` (src/init/review.rs:188) |
| 1.2 Explicit `(Y/n)`/`(y/N)` indicators | PASS (resolved in iter. 2) | All confirms with a default render the indicator via `Confirm::with_default` (src/init/prompts.rs:24-30). The empty-config confirm now passes `Some(false)` (src/init/mod.rs:70-71); the hook-overwrite prompt stays plain per the spec 1.7 exception (src/init/hook.rs:50) |
| 1.3 `Add a source?` default yes | PASS | `confirm("Add a source?", Some(true))` (src/init/flow.rs:309) |
| 1.4 Source type labels | PASS | `Git repository` / `Local folder`; `from_name` updated (src/init/flow.rs:34-52) |
| 1.5 `Repository URL or local path:` | PASS | src/init/flow.rs:355, 363 |
| 1.6 `Repository reference (...)` | PASS | Full message verbatim (src/init/flow.rs:365-368); integration test asserts the prefill message contains `Repository reference` (tests/init_flow.rs:529) |
| 1.7 Hook prompt default yes; overwrite stays plain | PASS | src/init/hook.rs:40-42 (`Some(true)`), src/init/hook.rs:50 (`None` for overwrite) |
| 2.1 Single-pass selection, no empty-finish mention | PASS | One `multi_select("Select files or folders to synchronize")` (src/init/flow.rs:415-432); pruning moved into the same pass via `resolve_picks` (src/init/flow.rs:437-465) |
| 2.2 Empty-selection confirm, default no | PASS | Exact wording, `Some(false)` (src/init/flow.rs:423-425); yes skips the source, no re-asks |
| 3.1 Mode prompt: Overwrite default, then Patch, folder = Overwrite only, descriptions | PASS | src/init/flow.rs:519-545; descriptions via `Choice::described`; cursor preselected through `with_starting_cursor` (src/init/prompts.rs:45) |
| 3.2 Policy prompt: Automatic default, then Manual, descriptions | PASS | src/init/flow.rs:550-563 |
| 4.1 Empty line before resume | PASS | `eprintln!()` first (src/init/review.rs:74) |
| 4.2 Title / separator / list / separator / question | PASS | `print_summary` (src/init/review.rs:72-90); question asked right after |
| 4.3 Entry rendering, green headline | PASS (accepted deviation) | `headline.green()` with `  Mode:` / `  Policy:` (src/init/review.rs:76-87). Entries are numbered (`1. Source -> Target`), which the literal spec block does not show; the numbering is documented and user-validated in plan 2.6 — accepted, plan takes precedence |
| 4.4 `validate` replaces `confirm`; new `add another entry` | PASS | `REVIEW_ACTIONS` (src/init/review.rs:17-22), question `Do you validate the current synchronization list?` (src/init/review.rs:55-59) |
| 4.5 Add-another runs the full flow and reprints | PASS | `ReviewAction::Add => flow.add_source(state)` inside the review loop (src/init/review.rs:42); the loop reprints the summary each iteration |
| 5.1 `colored` renders `Source -> Target` green | PASS | `colored = "3"` (Cargo.toml:8), locked at 3.1.1; `Colorize` used only for the headline |
| 5.2 Review output on stderr | PASS | `print_summary` uses `eprintln!` exclusively; stdout still reserved for the sync report (src/init/mod.rs:118-123) |
| Non-functional: Prompter gains defaults + option descriptions | PASS | `confirm(msg, Option<bool>)`, `select(msg, &[Choice], Option<&str>)` (src/init/flow.rs:97-110); inquire mapping in src/init/prompts.rs:24-57; scripted prompters updated in src/init/flow.rs tests and tests/common/mod.rs |
| Config semantics unchanged | PASS | `Mode::name()`/`Policy::name()` untouched (serde lowercase); `label()` added (src/config/model.rs:22-42, 59-79); `from_name` accepts both cases |

## 3. Code Quality & Architecture

- **Rules compliance**: no `unwrap()`/`expect()` outside test code (grep across `src/init/`
  confirms hits are confined to `#[cfg(test)]`); typed `SyncsmithError` throughout —
  Rust-ERR-1/ERR-2 satisfied. `Choice`, `SelectPrompt`, `SelectionTarget`, `NewEntry`,
  `EditRequest` are structs — Rust-A-2 satisfied. All changed functions stay at or below
  3 parameters besides `&self` (`select` has 3 + self; `resolve_picks` 3 + self) —
  Commons-FN-4 satisfied. Review summary on stderr, sync report on stdout —
  Commons-O-1 satisfied.
- **Commons-DEP-1**: `colored` 3.1.1 brings no new transitive crates on Linux; its only
  dependency, `windows-sys 0.61.2`, is already in `Cargo.lock` via existing crates.
  Footprint justified for the spec-mandated feature.
- **Architecture**: the `Flow<'a>` struct (src/init/flow.rs:293-305) removes the previous
  `MappingContext` parameter-bundling workaround and lets `review.rs` trigger a full
  `add_source` without re-plumbing prompter/fs — a clear separation-of-concerns win.
  `resolve_picks` extracts the pruning logic; `print_summary` extracts rendering. Dead
  code (`owned`, `summary_rows`) fully removed with all call sites updated.
- **Documentation**: new public API (`Choice`, `Flow`, `label()` methods) carries doc
  comments; the `Prompter` trait documents the default/indicator semantics.
  `docs/configuration.md` needed no change (file format untouched).
- **Minor style note (non-blocking)**: several unit tests hold more than one assertion,
  which the Commons-UT-1 SHOULD rule discourages; this matches the pre-existing project
  test style, so it is not treated as a defect.

## 4. Linting & Static Analysis

Re-run in iteration 2 after the F-1 fix; all green.

| Command | Result (iteration 2) |
|---|---|
| `cargo fmt --check` | PASS — exit 0, no diff |
| `cargo clippy --all-targets -- -D warnings` | PASS — 0 warnings, 0 errors |
| `cargo test` (full suite) | PASS — 96 unit + 7 config_io + 9 discovery + 9 hook + 15 init_flow + 6 shallow_clone + 21 sync_command + 3 sync_merge integration tests; 166 passed, 0 failed |

Test coverage matches plan Section 3: unit tests
`empty_selection_confirmed_skips_the_source`,
`empty_selection_declined_reasks_the_selection`,
`mode_prompt_defaults_to_overwrite` (asserts `[Overwrite, Patch]` order and default for
git, `[Overwrite]` for folder),
`policy_prompt_defaults_to_automatic`, `review_add_another_entry_appends_entries`,
`review_validate_ends_the_loop`, plus a valuable extra
(`simultaneous_folder_and_descendant_picks_prune_the_descendant`, exercising the
single-pass pruning edge). Integration tests
`run_with_empty_selection_continue_skips_the_source` and
`run_with_review_add_another_entry_writes_both_sources` assert the written config and
synchronized files — non-trivial, behavior-verifying, consistent with Commons-T-4/TQ-2.

## 5. Identified Bugs & Inconsistencies

- **F-1 (RESOLVED in iteration 2)** — src/init/mod.rs:70-71: the
  `The configuration has no entries. Write it anyway?` confirm previously passed
  `default: None` and rendered no indicator. It now passes `Some(false)`: inquire renders
  `(y/N)`, Enter declines and aborts before any write, matching the conservative GOTCHA
  comment and the empty-selection confirm's default. Verified in code and by the full
  test suite (scripted prompters ignore the confirm default, so no script changes were
  needed — the suite passing unchanged confirms this). Spec 1.2 now satisfied on every
  yes/no prompt except the one deliberate spec 1.7 exception.
- **F-2 (informational, accepted)** — `colored` resolves TTY against stdout while the
  summary goes to stderr (src/init/review.rs:85). Pre-accepted in plan Section 4 Risks;
  acceptance stands. Worst case is ANSI escapes in redirected stderr while stdout is a
  TTY; `NO_COLOR` is honored either way. No action.
- **F-3 (informational, accepted)** — Review entries are rendered as `1. Source ->
  Target` (numbered) rather than the bare `Source -> Target` of the spec 4.3 block;
  the numbering is specified and user-validated in plan 2.6. No action.
- **F-4 (informational, no action)** — `InquirePrompter::select` maps an unrecognized
  rendered option back through `SyncsmithError::InvalidArguments`
  (src/init/prompts.rs:48-56). This branch is unreachable with the current call sites
  (labels are echoed back by inquire) and mirrors the defensive pattern used elsewhere;
  acceptable.
- **Edge cases verified sound**: interruption at any prompt maps to `Aborted` before any
  write (src/init/prompts.rs:80-84, run loop propagates `?`); empty selection
  confirmed-skip returns to `Add a source?`, declined re-asks with the same options
  (src/init/flow.rs:415-432); a non-empty selection can never prune to emptiness
  (`covers` only removes descendants of picked folders, and the folder itself survives);
  target collision `configured` is rebuilt from `state.all()` on every `add_source`
  call, so entries added via the review `Add another entry` path are correctly
  protected against target collisions; restart defaults still prefill location/ref
  (unit and integration tests confirm), unchanged from prior behavior.

## 6. Action Items & Next Steps

1. ~~Executor — src/init/mod.rs: give the empty-configuration confirm an explicit
   default~~ — DONE in iteration 2 (`Some(false)`), verified in code and by re-running
   all three gates (Section 4).
2. ~~Test-Generator — update scripted tests attached to action 1~~ — Not needed and
   correctly not done: scripted prompters ignore the confirm default argument and
   replay answers positionally; the full suite passing unchanged confirms this.
3. No remaining action items. The branch is ready to commit.

## Status: APPROVED
