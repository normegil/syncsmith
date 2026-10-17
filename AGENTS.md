This project defines an executable that keeps files synchronized between a source and a target.

- Source: Can be a git repository, or a local folder. Each synchronized target file maps to exactly one source file; different target files may have different sources.
- Target: A git repository. Syncsmith operates on the repository it is invoked from (the target repository); synchronized files are written inside it.

## Configuration file

The configuration file documentation lives in [docs/configuration.md](docs/configuration.md).

## Functionality

Commands
- `init`: Initializes or reinitializes the `.syncsmith.toml` configuration file. Interactive command that allows the user to set up the sources and target files for synchronization.
- `sync`: Synchronizes files according to their policy. With no arguments, only Automatic entries are processed. `sync <target...>` synchronizes the specified targets, including Manual ones. `sync --all` synchronizes every entry. Exits 0 when all processed entries succeeded, non-zero if any failed. May run as a precommit hook: the `init` flow offers to install the hook that calls `syncsmith sync`.
- `check`: Checks that all Automatic entries are up to date, and that Manual entries are still at their last synchronized version (local edits are allowed as long as they respect the Patch mode rules). Exits 0 when all checks pass, non-zero otherwise. Usually used as part of a CI pipeline.

## Rules

Before any change to the code, read the rules in the `.ai/rules` folder to ensure compliance with the user defined  rules.
