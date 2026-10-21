# Implementation Plan – 2026-10-21-2100-Feature-GithubAction

This plan outlines the steps required to set up GitHub Actions CI and tag-driven releases for the syncsmith repository.

## 1. Research and Analysis (Completed)

- Quality gates exist in `Taskfile.yml`; commands map 1:1 to cargo invocations.
- All dependencies are pure Rust (only `libc`, no `cc`/`openssl`), so the musl target builds on `ubuntu-latest` with `musl-tools` installed.
- `#[cfg(windows)]` paths exist in `src/config/io.rs`, `src/commands/init/hook.rs`, `src/commands/sync/lock.rs` — the Windows test matrix (FR-2) executes them.
- No `.github/` directory yet. No `rust-toolchain.toml`; CI pins stable via the toolchain action (P1 decision).
- Actions used: `actions/checkout@v5`, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `actions/upload-artifact@v4`, `actions/download-artifact@v4`, `softprops/action-gh-release@v2`.

## 2. Proposed Changes

Two new files, no application code touched.

### 2.1 — `.github/workflows/ci.yml` (`name: CI`)

- `on: push: branches: [main]` + `pull_request` (FR-1); `permissions: contents: read`.
- Concurrency: group `ci-${{ github.ref }}`, `cancel-in-progress: true` (P3).
- Job `verify`, matrix `os: [ubuntu-latest, windows-latest]`:
  - steps: checkout, `dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, then `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --all-features` (FR-2).
- Linux-only extra step (`if: matrix.os == 'ubuntu-latest'`): `rustup target add x86_64-unknown-linux-musl` then `cargo check --target x86_64-unknown-linux-musl` (FR-4).
- Job `audit` (`ubuntu-latest`, `if: github.event_name == 'push' && github.ref == 'refs/heads/main'`, parallel — P2): checkout, toolchain, `cargo install cargo-audit --locked`, `cargo audit` (FR-3).

### 2.2 — `.github/workflows/release.yml` (`name: Release`)

- `on: push: tags: ['v*']` (FR-5); `permissions: contents: write` (FR-12).
- Job `check-version` (`ubuntu-latest`): shell step comparing `${GITHUB_REF_NAME#v}` against the `version` field in `Cargo.toml` (grep), exiting non-zero with a clear message on mismatch (FR-6). Runs first, before any build resource is spent.
- Job `build` (`needs: check-version`), matrix includes:
  - `{os: ubuntu-latest, target: x86_64-unknown-linux-musl, ext: none}`
  - `{os: windows-latest, target: x86_64-pc-windows-msvc, ext: .exe}`
  - steps: checkout, `dtolnay/rust-toolchain@stable` (with `targets` for Linux), `Swatinem/rust-cache@v2`, `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-features` (FR-8).
  - Linux-only: `apt-get install musl-tools`; `cargo install cargo-audit --locked`; `cargo audit`; `cargo install cargo-cyclonedx --locked`; `cargo cyclonedx -f json`; rename output to `sbom.json` (FR-8, FR-11).
  - `cargo build --release --target <target>` (FR-7).
  - Packaging step in `bash` (present on both runners): build `syncsmith-<version>-<target>.tar.gz` / `.zip` with the binary at archive root (FR-9); `sha256sum <archive> > <archive>.sha256` (FR-10); `actions/upload-artifact@v4` with `dist/`.
- Job `release` (`needs: build`, `ubuntu-latest`): `actions/download-artifact@v4` (merged into `dist/`), then `softprops/action-gh-release@v2` with `draft: true`, `generate_release_notes: true`, `files: dist/**` (FR-12).

## 3. Verification Plan

This change is pure workflow YAML; there is no Rust unit surface. Coverage maps to executable local checks:

### 3.1 Unit Tests

Not applicable — no Rust code is modified. Equivalents are the executable local checks below.

### 3.2 Integration / Local Checks

- **AC-1, AC-2, AC-3** — structural review of `ci.yml` (triggers, matrix, step conditions) plus running the mirrored commands locally (`cargo check`, clippy, fmt, test pass per Rust gates). Full runtime proof occurs on the first push to GitHub; flagged as such.
- **AC-4** — structural review of `release.yml` job graph and asset paths; archive name/checksum logic exercised by a local dry-run of the packaging script against a locally built binary.
- **AC-5** — extract the version-check step's shell and execute it locally with a fake `GITHUB_REF_NAME` (matching and non-matching) against the real `Cargo.toml`; assert exit codes and error message (EC-2).
- **AC-6** — local dry-run: create the tar.gz, regenerate `.sha256`, run `sha256sum -c` (verifies the checksum contract); static-binary claim verified by building with `--target x86_64-unknown-linux-musl` locally if the target is installable, otherwise documented as deferred to the runner.
- **AC-7** — download `actionlint` (scratchpad, not repo) and run it against both workflow files; additionally parse both files as YAML.

No AC is left uncovered; AC-1..AC-4 runtime confirmation is deferred to the first real push/tag and will be stated explicitly.

## 4. Risks and Considerations

- `cargo install cargo-audit` / `cargo-cyclonedx` add ~1-2 min per relevant job; mitigated by `rust-cache` (does not cache installs — acceptable).
- Windows `bash` on hosted runners is Git Bash; `sha256sum` and `tar`/`zip` availability confirmed standard there; fallback documented if missing.
- `checkout@v5` / `upload-artifact@v4` majors assumed current; `actionlint` will flag unknown or invalid refs.
- Re-running the release workflow re-uploads assets idempotently (EC-3, softprops behavior).
- No backward-compatibility impact: workflows are additive; `Taskfile.yml` and `src/` untouched.
