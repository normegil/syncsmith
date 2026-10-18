I'd like to implement the command that will be used to synchronize files.

`syncsmith sync`

or, for an individual target:

`syncsmith sync <Target>`

The goal is to syncrhonize files according to the configuration file content. Documentation for configuration file is in `docs/configuration.md`.

This command is not interactive, and should be able to run as a pre-commit hook in git.

## Options

You may propose options that seems like good fits for this command, or propose other names for described flag if you find a best fit.

All global flags should be able to be reused.

Here are some of the command-specific flags:

- `-m, --include-manuals`: Also sync the `Manual` policy files.

You can also specify the target, to only synchronize a specific file, weither it has an Automatic or Manual sync policy.

## Mode & Policy

How mod & Policy should be applied is described in the configuration file documentation.
