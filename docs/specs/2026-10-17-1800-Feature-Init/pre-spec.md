I'd like a command to initialize the configuration file for syncsmith.

`syncsmith init`

The goal is to init the configuration file and trigger a sync right after (`sync` command is not yet implemented).

Documentation for configuration file is in `docs/configuration.md`.

## Workflow

- Command is interactive (Use inquire).
- It should ask for each source to synchronize (folder or repository), until the user don't want to add other sources.
  - Load the current content of the source
  - Ask the user what he want to sync in the current source (can be full folder or just a file) until he doesn't want to synchronize anything else (or everything as been considered)
    - For each source, propose to create a target corresponding to the current paths of the file in the source directory. Let the user modify the target path if needed (Check the coherence: folder to folder, files to files).
    - For each source, ask for which sync mode he want to use.
    - For each source, ask if the sync should be automatic or manual.

(Review the current described process and evaluate if it correspond to the best UX for users - You may propose something better if it's not good enough)

## Options

You may propose options that seems like good fits for this command.

### Global

- `--config`: Allow to specify the path where the file should be loaded or created. 
  - If not specified, starting from the current working directory, go up a level until you find a configuration file, or the repository root.
    - If none were found and:
      - You're executing the `init` command, create the file based on the user interaction & setup. This is the classic behavior.
      - You're not executing `init`, throw an error.
    - If a file is found:
      - You're executing the `init` command, user may want to overwrite thr current config (ask before overwriting) - Load the config and follow the classic init process but use the content of the current config file as defaults.
      - You're not executing the `init` command: Load the file, and use the paths from the locations of the configuration file.

- `-v, --verbose`: (Can be used multiple times, Each time add more details) Enable logging to stderr.
