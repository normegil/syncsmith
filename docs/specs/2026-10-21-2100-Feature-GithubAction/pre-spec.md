I want to setup Github Actions for this repository, inspired by the content of `Taskfile.toml`.

In addition, when detecting a new tag, I want to create a Github release, and attach the generated binary to it.

If possible, I want a build for Linux (x86_64) and one for Windows (x86_64). For each of them, we could probably use a checksum as well.
