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

| Command | Status | Description |
|---|---|---|
| `syncsmith init` | Available | Create or update `.syncsmith.toml` interactively (see below). |
| `syncsmith sync` | Not implemented yet | Synchronize files according to their policy: no arguments processes Automatic entries only; `sync <target...>` also processes the named Manual entries; `sync --all` processes every entry. Exits 0 when all processed entries succeeded. |
| `syncsmith check` | Not implemented yet | Verify that Automatic entries are up to date and Manual entries are still at their last synchronized version. Exits 0 when all checks pass. Intended for CI pipelines. |

### `syncsmith init`

Interactive setup of the configuration file:

1. Resolve the configuration path (see `--config` below). If a configuration already exists, choose **add** (keep existing entries and append new ones), **restart** (discard the entries and start fresh — the most recent discarded entry prefills the source-location and ref prompts), or **abort**.
2. For each source (git repository or local folder): browse its content — remote repositories are shallow-cloned (depth 1, pinned ref) into a temporary directory — and select the files or folders to synchronize.
3. Map each selection to a target path inside the target repository (default: the selection's path relative to the source root), validated for containment, duplicates, and file/folder coherence.
4. Pick a default mode (`patch` or `overwrite`) and policy (`automatic` or `manual`) applied to all entries from that source.
5. Review the entry table; edit (target, mode, policy) or remove entries.
6. On confirmation, write `.syncsmith.toml` atomically.
7. Optionally install a pre-commit hook that runs `syncsmith sync` (honoring `core.hooksPath`; an existing hook is never overwritten without confirmation).

Notes:

- An empty configuration (no entries) is only written after an explicit confirmation.
- Interrupting (Ctrl-C, Esc, EOF) before the write aborts with nothing persisted. Interrupting at the hook-install prompt skips the hook but the configuration is already written; a warning is logged and `init` still exits 0.
- `init` ends by calling the sync entry point. Until `sync` is implemented this logs a warning on stderr and `init` still exits 0: a successful `init` means "configuration written", not "files synchronized".

## Global options

Available on every command:

- `--config <path>`: use the given file instead of searching. Without it, syncsmith walks up from the current directory until a `.syncsmith.toml` is found or the git repository root is reached; `init` creates the file at the repository root (or current directory when outside a repository).
- `-v`, repeatable: verbosity — no flag shows errors and warnings only, `-v` info, `-vv` debug, `-vvv` trace. `RUST_LOG` overrides the derived level. All diagnostics go to stderr; stdout stays clean.

## Exit codes

- `0`: success, or an abort before any write ("aborted, nothing written").
- `1`: any error — configuration not found, unreadable or invalid, a write failure, or an aborted flow after a write (details on stderr).
