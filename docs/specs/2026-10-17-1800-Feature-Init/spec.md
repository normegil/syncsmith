# Specification: `syncsmith init` command

## Overview

Add an interactive `init` command to syncsmith that creates or updates the `.syncsmith.toml` configuration file, then triggers a sync. The `sync` command is not implemented yet: `init` calls the sync entry point, and the resulting "not implemented" error is reported as a non-fatal warning (exit 0). The command is implemented in Rust, interactive via the `inquire` crate, with `clap` for argument parsing.

## Command

`syncsmith init [--config <path>] [-v...-vvv]`

- Exit 0 when the config is written successfully (even if the post-init sync call fails as "not implemented").
- Non-zero exit if the config could not be discovered, written, or the user aborts.

## Global options (shared by all future commands)

### `--config <path>`

- Explicit path to a `.syncsmith.toml` file, loaded or created at that location.
- When omitted: starting from the current working directory, walk up one level at a time until a `.syncsmith.toml` is found or the git repository root is reached.
  - Config file found:
    - Under `init`: offer the user Add / Restart / Abort (see Existing config behavior).
    - Under any other command: load it; all relative paths (sources, targets) resolve relative to the directory containing the configuration file.
  - No config found:
    - Under `init`: create the file at the repository root (or current directory if not inside a repository) through the interactive flow.
    - Under any other command: error, non-zero exit, actionable message stating no configuration was found.

### `-v, --verbose` (repeatable, up to 3)

- No flag: errors and warnings only.
- `-v`: info; `-vv`: debug; `-vvv`: trace. All diagnostics go to stderr; stdout stays clean.

## Interactive workflow (`init`)

1. **Config discovery**: resolve the config path per `--config` rules. If a config exists, present Add / Restart / Abort:
   - **Add**: keep existing entries; the interactive flow continues and appends new entries.
   - **Restart**: discard current content and run a fresh flow (existing entries shown as defaults where relevant).
   - **Abort**: exit without modifying anything.
2. **Source loop**: "Add a source?" (yes/no). For each source:
   - Source type: git repository or local folder.
   - Git repository: URL or local path, plus optional ref (defaults to the default branch).
   - Load the source content:
     - Local folder or local git path: read directly.
     - Remote URL: shallow clone (depth 1, pinned ref) into a temporary directory for browsing; the config records the URL and ref.
3. **Selection loop** within the current source: pick files or whole folders to sync (repeat until the user is done or everything has been considered).
4. **Target mapping**: for each selection, propose the default target = the selection's path relative to the source root, mirrored at the target repository root (e.g. `lints/base.yml` → `lints/base.yml`). The user may edit each target path. Validation:
   - File selections map to file targets; folder selections map to folder targets.
   - No duplicate target paths across entries (including entries already in the config).
   - Targets must stay inside the target repository.
   - Warn if a target would overwrite an existing, non-synchronized file or folder.
5. **Mode & policy defaults**: ask once per source for a default mode (`patch` or `overwrite`) and default policy (`automatic` or `manual`); applied to all entries from that source.
6. **Review summary**: table of all entries (source, target, mode, policy, including pre-existing entries when adding). Per-entry edits allowed here: change target, mode, or policy; remove an entry. Pre-existing entries may only be edited or removed, not duplicated.
7. **Write**: on confirmation, write `.syncsmith.toml` in the documented schema (source/repository/ref/path or source/folder, target, mode, policy).
8. **Precommit hook**: offer to install a precommit hook in the target repository (`.git/hooks/pre-commit` or the repository's configured hooks directory) that calls `syncsmith sync`. Skipped without error when the target is not a git repository or the user declines. `init` never overwrites an existing hook without confirmation.
9. **Initial sync**: call the sync entry point for all entries. Until `sync` is implemented, the resulting error is printed as a warning to stderr and `init` exits 0.

## Error handling

- All user-facing errors go to stderr with actionable context (which path/URL failed, why).
- Interruption (Ctrl-C/EOF) aborts without writing anything.
- The config file is written atomically (temp file + rename) so an aborted write cannot corrupt an existing config.

## Non-goals

- Implementing the `sync` or `check` commands.
- Patch/lock-file logic beyond defining that `sync` will own `.syncsmith.lock`.
- Environment variable editing in the interactive flow (env-var expansion applies at load time, as documented).
