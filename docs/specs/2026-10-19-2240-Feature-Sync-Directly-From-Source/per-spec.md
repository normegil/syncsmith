For some situation where creating configuration file in a repo might not be desirable, we could offer an alternive. Something like this:

`syncsmith sync --override-from <Source> <Target>`

where:
- `Source` can be a path to a git repository (including ref & path to the file inside the repo - Provide good default !), or a local path.
- `Target` is a single relative target path inside the current repo.
