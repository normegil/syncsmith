Some corrections:

## `init` command

- All fields should have their first letter uppercased
- `Add a source` should have a `true` default value, as well as the text "(Y/n)" added.
- `Source type` options should use Upperscal letters like "Git repository" and "Local folder"
- `Repository URL or local path` should be followed by a ":"
- `Ref` should be more explicit, like "Repository reference".
- `Select files or folder to synchronize` should not wait for an empty input - It should stop after the first selection of all files
- `Default mode for entries` should be default selected as "Overwrite" (and options should be described in a few words)
- Same remark for policies: Default is `Automatic` and should be explained.
- `Install a pre-commit hook`: Same remarks as "Add a source"

### Resume
- Add at least an empty line before printing the resume.
- Add a title to present the resume, followed by a separator (-------- ?), the list of source & targets, another separator, and finaly ask the question "Review the entries above". Change the question to "Do you validate the current synchronization list ?". Option "confirm" changed to "validate"
- Each options should be something like:
  - Source -> Target in green color
  - Tab - Mode:
  - Tab - Policy:
- Add an option to add another entry when asking for validation.
