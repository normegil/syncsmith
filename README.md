# syncsmith

syncsmith is a command-line tool that keeps files synchronized between a source and a target git repository.

- **Source**: a git repository or a local folder. Each synchronized target file maps to exactly one source file; different targets may have different sources.
- **Target**: the git repository syncsmith is invoked from. Synchronized files are written inside it.

What is synchronized is described by a `.syncsmith.toml` file at the target repository root — see [docs/configuration.md](docs/configuration.md) for the exact schema, validation rules, and environment-variable handling.

## Install / build

Requires Rust and a `git` binary on the `PATH` (syncsmith shells out to `git`).

```sh
cargo install --path .   # install
cargo build              # or build in place
cargo test               # run the test suite
```

## Commands

| Command | Description |
|---|---|---|
| `syncsmith init` | Create or update `.syncsmith.toml` interactively (see below). |
| `syncsmith sync` | Synchronize files according to their policy (see below). Exits 0 when all processed entries succeeded, non-zero if any failed. |
| `syncsmith check` | Read-only verification that Automatic entries are up to date and Manual entries are still at their last synchronized version, for CI pipelines. Exits 0 when all checks pass, non-zero when files are not in sync; `--no-fail` reports them without failing, `-a`/`--all` also checks `manual` entries, `--json` renders the not-in-sync files as a JSON payload. |

### `syncsmith init`

Interactive setup of the configuration file:

1. Resolve the configuration path (see `--config` below). If a configuration already exists, choose **Add** (keep existing entries and append new ones), **Restart** (discard the entries and start fresh — the most recent discarded entry prefills the source-location and ref prompts), or **Abort**.
2. For each source (git repository or local folder): browse its content — remote repositories are shallow-cloned (depth 1, pinned ref) into a temporary directory — and select the files or folders to synchronize in a single pass (a picked folder subsumes its descendants). An empty selection asks for confirmation before the source is skipped; declining re-asks the selection.
3. Map each selection to a target path inside the target repository (default: the selection's path relative to the source root), validated for containment, duplicates, and file/folder coherence.
4. Pick a default mode (`overwrite` by default; `patch` only for git sources) and policy (`automatic` by default) applied to all entries from that source; each option carries a one-line description.
5. Review the synchronization summary (one green `source -> target` line per entry, with its mode and policy); validate it, edit an entry (target, mode, policy), remove an entry, or add another entry (which replays the full source flow and reprints the summary).
6. On confirmation, write `.syncsmith.toml` atomically.
7. Synchronize the `automatic` entries and report the changed paths on stdout, exactly like `syncsmith sync`.
8. Optionally install a pre-commit hook that runs `syncsmith sync` (honoring `core.hooksPath`; an existing hook is never overwritten without confirmation). The offer is skipped when the final synchronization failed.

Notes:

- An empty configuration (no entries) is only written after an explicit confirmation.
- Interrupting (Ctrl-C, Esc, EOF) before the write aborts with nothing persisted. Interrupting at the hook-install prompt skips the hook but the configuration is written and the final synchronization already ran; a warning is logged and `init` still exits 0.
- A synchronization failure makes `init` exit non-zero (the configuration is still written); the hook offer never happens in that case.
- Folder sources only offer `overwrite` mode in the prompts (see [docs/configuration.md](docs/configuration.md)).

### `syncsmith sync`

Non-interactive synchronization of the target files described by the configuration, safe to run as a git pre-commit hook.

```
syncsmith sync [--all] [<target>...]
```

| Invocation | Entries processed |
|---|---|
| `syncsmith sync` | all `automatic` entries (this is what the pre-commit hook runs) |
| `syncsmith sync --all` | every entry, `manual` ones included |
| `syncsmith sync <target>...` | the matching entries, whatever their policy |

- Target arguments are matched against entry `target` values after normalization; an unknown target is a usage error (non-zero exit) before any entry is processed, listing the configured targets for reference. `--all` and explicit targets are mutually exclusive.
- Entries are processed in configuration order; a per-entry failure is reported on stderr and does not stop the other entries. The run exits 0 only when every processed entry succeeded.
- Sources resolve as documented in [docs/configuration.md](docs/configuration.md): git sources are read at their pinned `ref` (remote repositories are shallow-cloned, local repositories read directly), folder sources are read from disk. The last synchronized state is tracked in `.syncsmith.lock` next to `.syncsmith.toml` (commit it).
- In `patch` mode, an edited target is merged with the source change (three-way merge). On conflict, the file is written with standard conflict markers, the entry fails, and the lock record is not updated — resolve the markers manually and re-run.

stdout carries exactly the paths modified, created, or deleted by the run **for entries that succeed**, one per line, relative to the target repository root; a failed entry contributes nothing to stdout (its diagnostics are on stderr, and conflicted files are written but not reported). All diagnostics go to stderr. With the global `--json` flag, stdout is a single JSON document instead:

```json
[
  { "path": "lints/base.yml", "action": "updated" },
  { "path": "docs/aliases/zsh.txt", "action": "created" },
  { "path": "docs/aliases/old.sh", "action": "deleted" }
]
```

#### Pre-commit hook

The hook installed by `init` runs `syncsmith sync` and stages exactly the files it reports — no target is embedded in the script, so it never goes stale when the configuration is edited:

```sh
#!/bin/sh
# Installed by syncsmith: run syncsmith sync before each commit.
changed=$(syncsmith sync) || exit 1
if [ -n "$changed" ]; then
  printf '%s\n' "$changed" | git add --pathspec-from-file=-
fi
```

`git add --pathspec-from-file=-` stages deletions and paths containing spaces, and requires **git 2.25+**. Filenames containing newlines are not handled by the line-based staging. `syncsmith` must be on the `PATH` for the hook to run.

### `syncsmith check`

Read-only verification that the target files are synchronized, for CI pipelines. It never writes anything — no target file, no lock file, no repository state — and applies the same selection semantics as `sync`:

```
syncsmith check [--all] [--no-fail] [<target>...]
```

| Invocation | Entries checked |
|---|---|
| `syncsmith check` | all `automatic` entries |
| `syncsmith check --all` | every entry, `manual` ones included |
| `syncsmith check <target>...` | the matching entries, whatever their policy |

- `--no-fail`: still report the not-in-sync files, but exit 0 even when some are found. Evaluation failures (a source that cannot be loaded, an unrecoverable base version) and usage errors still exit non-zero — CI can distinguish "out of date" from "cannot verify".
- An entry is evaluated per file against the lock records (see [docs/configuration.md](docs/configuration.md) for the exact in-sync rules): `automatic` entries must be at the latest available source version; `manual` entries must still be at their last synchronized version, where local edits are accepted in `patch` mode as long as the pending source patch would merge cleanly.
- In `patch` mode with local edits and a pending source version, a trial three-way merge (same merge machinery as `sync`, result discarded) decides between `stale`/in sync and `conflicted`.

stdout lists the not-in-sync paths, one per line, relative to the target repository root — the same contract as `sync`, so tooling is reusable. With `--json`, stdout is a single JSON document with the status of each file:

```json
[
  { "path": "lints/base.yml", "status": "stale" },
  { "path": "docs/aliases/zsh.txt", "status": "conflicted" }
]
```

| Status | Meaning |
|---|---|
| `stale` | a patch is pending (the source moved since the last synchronization) |
| `edited` | local edits that a synchronization would not preserve (`overwrite` mode) |
| `conflicted` | the pending source patch conflicts with local edits (`patch` mode) |
| `missing` | the target file does not exist but the source has it |
| `extra` | the target file exists but the source no longer has it |

The run exits 0 when every checked file is in sync (or with `--no-fail`), non-zero otherwise.

## Global options

Available on every command:

- `--config <path>`: use the given file instead of searching. Without it, syncsmith walks up from the current directory until a `.syncsmith.toml` is found or the git repository root is reached; `init` creates the file at the repository root (or current directory when outside a repository).
- `--json`: render the stdout payload as a single JSON document where supported (currently `sync` and `check`; `init`'s final synchronization report honors it too).
- `-v`, repeatable: verbosity — no flag shows errors and warnings only, `-v` info, `-vv` debug, `-vvv` trace. `RUST_LOG` overrides the derived level. All diagnostics go to stderr; stdout stays clean.

## Exit codes

- `0`: success, or an abort before any write ("aborted, nothing written").
- `1`: any error — configuration not found, unreadable or invalid, a write failure, or an aborted flow after a write (details on stderr).
