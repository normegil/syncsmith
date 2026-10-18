# Implementation Plan - 2026-10-18-2030-Feature-Corrections

This plan outlines the steps required to implement the `init` command UX corrections
defined in `spec.md`.

## 1. Research and Analysis (Completed)

- Affected modules: `src/init/flow.rs` (prompts, selection loop, orchestration),
  `src/init/review.rs` (resume display, actions), `src/init/prompts.rs` (inquire
  mapping), `src/init/hook.rs` (confirm default), `src/init/mod.rs` (orchestration,
  unchanged), `src/config/model.rs` (display labels only).
- inquire 0.7.5 capabilities confirmed: `Confirm::with_default(bool)` renders
  `(Y/n)` / `(y/N)` automatically; `Select::with_starting_cursor(usize)` preselects an
  option; there is **no per-option help text**, so option descriptions are embedded in
  the displayed option label (a `Choice` struct implementing `Display`).
- New dependency: `colored` (v3) for the green `Source -> Target` line; it handles
  `NO_COLOR` itself.
- Config-file semantics untouched: `name()` stays lowercase for serde; new `label()`
  methods provide the capitalized prompt form.

## 2. Proposed Changes

### 2.1. `src/config/model.rs` — display labels
- `Mode::label() -> &'static str` returning `"Overwrite"` / `"Patch"`;
  `Policy::label()` returning `"Automatic"` / `"Manual"`.
- `from_name` accepts the capitalized labels (keeps accepting lowercase for
  robustness). `name()` (serde) unchanged.

### 2.2. `src/init/flow.rs` — `Prompter` trait redesign
- New struct `Choice { label: String, description: Option<String> }` implementing
  `Display` as `label` when no description, `label — description` otherwise.
- `fn confirm(&self, message: &str, default: Option<bool>)` — `None` renders a plain
  y/n.
- `fn select(&self, message: &str, options: &[Choice], default: Option<&str>) ->
  Result<String>` — returns the chosen **label**; `default` preselects by label.
- `text`, `text_with_default`, `multi_select` unchanged.

### 2.3. `src/init/flow.rs` — prompt wording
- `SourceKind::name()` → `"Git repository"` / `"Local folder"`; `from_name` updated.
- `EXISTING_CHOICES` → `["Add", "Restart", "Abort"]`; match arms updated.
- Messages: `"Add a source?"` with default `Some(true)`; `"Source type:"`;
  `"Repository URL or local path:"`; `"Folder path:"`; `"Repository reference (branch,
  tag, or commit; empty for the default branch):"`; `"Target path for `<path>`:"`.
- `ask_mode`: choices `Overwrite` ("Replace the target with the source content") then
  `Patch` ("Apply the source changes on top of local edits"), default `Overwrite`;
  folder sources offer `Overwrite` only. Message `"Default mode for entries from this
  source:"`.
- `ask_policy`: `Automatic` ("Synchronized on every sync run") then `Manual`
  ("Synchronized only when explicitly requested"), default `Automatic`. Message
  `"Default policy for entries from this source:"`.

### 2.4. `src/init/flow.rs` — single-pass selection
- `select_items`: one `multi_select("Select files or folders to synchronize")` pass;
  covered descendants are pruned within that pass (existing `covers`).
- Empty pick → `confirm("No file was selected; the source will be ignored. Continue?",
  Some(false))`; on `true` return empty (source skipped), on `false` re-ask the
  multi-select (loop).

### 2.5. `src/init/flow.rs` — flow context
- Introduce `pub struct Flow<'a> { prompter: &'a dyn Prompter, fs: &'a dyn FlowFs }`
  with methods `run(&self, state)`, `add_source(&self, state)`, `ask_source`,
  `ask_source_ref`, `ask_mode`, `ask_policy`, `select_items`, `map_targets` — so
  `review` can trigger a full `add_source`. Replaces the free functions of the same
  names (complies with the ≤3-parameters rule).

### 2.6. `src/init/review.rs` — resume screen
- `review(flow: &Flow, state: &mut InitState)`, called from `Flow::run`.
- Print block to stderr: empty line, `"Synchronization summary"`, separator
  `-`.repeat(60), then per entry:

  ```
  1. source -> target        (green via `colored`)
       Mode: Overwrite
       Policy: Automatic
  ```

  then the separator again, then the prompt.
- `REVIEW_ACTIONS` → `["Validate", "Edit entry", "Remove entry", "Add another
  entry"]`; question `"Do you validate the current synchronization list?"`. `Validate`
  ends the loop; `Add another entry` calls `flow.add_source(state)` and the loop
  reprints the updated summary.
- `entry_label` stays single-line for the edit/remove pick lists (targets use `->`,
  mode/policy shown capitalized via `label()`).
- Edit-entry field prompt unchanged (`"target"`, `"mode"`, `"policy"` options);
  `"New target path:"` gets its colon.

### 2.7. `src/init/prompts.rs` — inquire mapping
- `confirm`: `Confirm::with_default` when `Some`; plain otherwise.
- `select`: map `Choice` to inquire options (a display struct),
  `with_starting_cursor(index of the default)`, return the label of the chosen option.

### 2.8. `src/init/hook.rs`
- `"Install a pre-commit hook that runs 'syncsmith sync'?"` with default `Some(true)`;
  the existing-hook overwrite prompt stays default-less.

### 2.9. `Cargo.toml`
- Add `colored = "3"`.

### 2.10. Test updates
- `src/init/flow.rs` unit-test `ScriptedPrompter`: new trait signatures; records the
  offered `Choice` labels and defaults (replaces `select_options`); scripts updated to
  the new labels (`"Local folder"`, `"Overwrite"`, `"Automatic"`, `"Validate"`),
  empty-`Multi` answers removed from selection scripts.
- `tests/common/mod.rs`: same trait update for the integration `ScriptedPrompter`.
- `tests/init_flow.rs`, `tests/hook.rs`: all scripts updated.

## 3. Verification Plan

### 3.1. Unit tests (`src/init/flow.rs` `tests` module)
- `empty_selection_confirmed_skips_the_source` — confirm(true) after an empty Multi →
  no entries.
- `empty_selection_declined_reasks_the_selection` — confirm(false), then a pick →
  entry created.
- `mode_prompt_defaults_to_overwrite` — recorded default is `Overwrite`, order
  `[Overwrite, Patch]` for git; `[Overwrite]` for folder.
- `policy_prompt_defaults_to_automatic`.
- `review_add_another_entry_appends_entries` — review script: `Add another entry` →
  full source flow → `Validate`.
- `review_validate_ends_the_loop`.
- Existing tests are updated, not deleted.

### 3.2. Integration tests (`tests/init_flow.rs`)
- `run_with_writes_mirrored_folder_targets_and_exits_success` and friends: updated
  scripts (single-pass selection, new labels).
- New: `run_with_empty_selection_continue_skips_the_source` (config written without
  the skipped source; the empty-config confirm path may apply).
- New: `run_with_review_add_another_entry_writes_both_sources`.

### 3.3. Gates
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` — all
  green.

## 4. Risks and Considerations

- **Label parsing**: `from_name` accepts the new capitalized labels everywhere;
  config serialization is untouched (serde keeps lowercase).
- **colored TTY detection** targets stdout while the summary goes to stderr — with
  redirected output the line may be colored when stdout is a TTY and stderr is not.
  Acceptable; `NO_COLOR` is respected either way.
- **Review actions capitalized** (`Validate`, `Edit entry`, ...) — the pre-spec wrote
  them lowercase; the "first letter uppercased" rule is applied uniformly (validated
  with the user).
- Separator width fixed at 60 dashes (validated with the user).
