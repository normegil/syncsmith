# `init` command UX corrections

## Scope

`syncsmith init` interactive flow only (`src/init/`). No change to `sync`, `check`, the
configuration file format, or the on-disk entry semantics: `mode`/`policy` stay lowercase
in `.syncsmith.toml`; only prompt labels and the review display change.

## 1. Prompt wording and formatting

1.1. Every prompt message starts with an uppercase letter; free-form text prompts end with
     `:` (e.g. `Folder path:`, `Target path for 'lints':`).
1.2. Yes/no prompts carry an explicit indicator: `(Y/n)` when the default is yes, `(y/N)`
     when the default is no.
1.3. `Add a source? (Y/n)` — default **yes** (currently a plain confirm).
1.4. Source type options become `Git repository` and `Local folder` (label parsing updated
     accordingly).
1.5. `Repository URL or local path:` (colon added).
1.6. `Ref` becomes `Repository reference` — full message:
     `Repository reference (branch, tag, or commit; empty for the default branch):`.
1.7. `Install a pre-commit hook that runs 'syncsmith sync'? (Y/n)` — default **yes**. The
     existing-hook overwrite prompt stays a plain confirm.

## 2. File selection

2.1. `Select files or folders to synchronize` is a **single** multi-select pass — no
     empty-input loop, and the `(empty selection to finish)` mention is removed.
     Folder-covers-descendants pruning applies within that single pass.
2.2. If nothing is selected: confirm prompt
     `No file was selected; the source will be ignored. Continue? (y/N)` — default **no**.
     On yes, the source is skipped; on no, the selection is re-asked.

## 3. Mode and policy prompts

3.1. `Default mode for entries from this source:` — options in order `Overwrite`
     (default, cursor preselected), then `Patch`; folder sources offer `Overwrite` only.
     Each option carries a short help description:
     - `Overwrite` — replace the target with the source content
     - `Patch` — apply source changes on top of local edits
3.2. `Default policy for entries from this source:` — options `Automatic` (default), then
     `Manual`, with help descriptions:
     - `Automatic` — synchronized on every `sync` run
     - `Manual` — synchronized only when explicitly requested

## 4. Resume (review screen)

4.1. An empty line is printed before the resume block.
4.2. Layout: title `Synchronization summary`, a separator line of dashes, the entry list,
     then another separator, then the validation question.
4.3. Each entry renders as:

     ```
     Source -> Target      (green)
       Mode: <mode>
       Policy: <policy>
     ```

4.4. Question becomes `Do you validate the current synchronization list?` with options
     `validate` (replaces `confirm`), `edit entry`, `remove entry`, and new
     `add another entry`.
4.5. `add another entry` runs the full add-source flow (source type, location/ref,
     selection, target mapping, mode, policy), then returns to the review loop which
     reprints the updated summary.

## 5. Colors

5.1. The `colored` crate (new dependency) renders the `Source -> Target` line green; it
     handles `NO_COLOR`/TTY detection itself.
5.2. Review output stays on stderr (stdout reserved for the sync report).

## Non-functional / constraints

- The `Prompter` trait gains default-value and option-description capabilities; the
  `InquirePrompter` maps them to inquire features (`with_default`, help text, starting
  cursor). Scripted tests are updated accordingly.
- All existing quality gates apply: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`.
