## Project structure

`init`, `check` & `sync` should be in a command subpackage (and subfolder).

## File content

### In `main.rs`

`write_report` and `sync_command` should be defined in the `sync` package
`CheckInvocation`, `check_command` and `write_check_payload` should be defined in the `check` package

Resolution of the config file path should be done before invoking any command-specific code as it will alsways be needed. The resolved config file is then passed as parameter to the commands.
- In the case of `init` command, the configuration file might not yet exist. This should be reflect in the type passed to init::run.

### In `init/`

#### `mod.rs`

ExitState can be removed if the `Completed` is the only one available (and any other states are conveyed by errors).

`run` function don't need the verbose parameter - Only pass what's needed (json).

`run_with` is quite long and should be defined in multiple subfunctions, to make the code simpler and easier to read.

Offer to install the hook at the end of the current process, after the sync has been completed.

`config_dir_of` should be a function of Resolution. Something like `resolution.dir()`

#### `source_load.rs`

`walk` is a bit long a would benefit of some division in subfunctions.

### In `discovery.rs`

Discovery package exist only to discover the config file and should be defined as a subpackage of `config`.

### In `sync/`

#### `mod.rs`

`lock_path` in `SyncContext` should be defined directly on the Resolved config instead
