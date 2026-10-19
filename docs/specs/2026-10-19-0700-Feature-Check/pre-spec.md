I'd like to implement the command that will be used to check if files are synchronized.

`syncsmith check`

or, for an individual target:

`syncsmith check <Target>`

The goal is to check if files are updated to the last files according to the configuration file content. Documentation for configuration file is in `docs/configuration.md`.

This command is not interactive, and should be able to run as part of the CI, eventually failing the build if files are detected as not syncrhonized.

## Options

You may propose options that seems like good fits for this command, or propose other names for described flag if you find a best fit.

All global flags should be able to be reused.

Here are some of the command-specific flags:

- `-a, --all`: Also check sync state of the `Manual` policy files.
- `--no-fail`: The command will not fail, even if any files are detected as not synchronized. Theses not-in-sync files will still be reported.

You can also specify the target, to only check sync state of a specific file, weither it has an Automatic or Manual sync policy. Multiple targets can be specified.

Mode should not have any effect.

## Output

On stdout, we want to list all the targets not in sync.

Json output should look like the `sync` json output, but list no-in-sync files.

Sync command documentation and analysis can be found in `docs/specs/2026-10-18-0800-Feature-Sync`.
