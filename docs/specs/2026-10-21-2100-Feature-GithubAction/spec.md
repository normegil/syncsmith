# Specification – 2026-10-21-2100-Feature-GithubAction

## Context

The repository has no CI/CD. The `Taskfile.yml` defines the quality gates (`check`, `lint`, `test`, `audit`, `sbom`) and the release flow (`verify` + `sbom` then `cargo build --release`), but everything runs only locally. We add GitHub Actions workflows mirroring those tasks, plus tag-driven releases with attached binaries and checksums.

## Scope

In scope:
- CI workflow (quality gates) in `.github/workflows/`
- Release workflow (tag-driven) in `.github/workflows/`
- Linux x86_64 (`x86_64-unknown-linux-musl`) and Windows x86_64 (`x86_64-pc-windows-msvc`) release builds
- Per-artifact SHA-256 checksums, CycloneDX SBOM, GitHub draft release

Out of scope:
- Changes to application code (`src/`)
- Changes to `Taskfile.yml`
- Publishing to crates.io, winget, or other distribution channels
- Release notes authoring beyond GitHub auto-generated notes

## Functional Requirements

- **FR-1**: A CI workflow triggers on pushes to `main` and on all pull requests.
- **FR-2**: On every CI trigger, on both `ubuntu-latest` and `windows-latest`: `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --all-features`.
- **FR-3**: On pushes to `main` only, the Linux CI job additionally runs `cargo audit`; the Windows CI job does not.
- **FR-4**: The Linux CI job additionally runs `cargo check --target x86_64-unknown-linux-musl` (musl compile guard) so release-target breakage surfaces in CI.
- **FR-5**: A release workflow triggers on pushing a tag matching `v*`.
- **FR-6**: The release workflow fails if the tag (leading `v` stripped) does not match the `version` in `Cargo.toml`.
- **FR-7**: Release builds run natively per platform: `x86_64-unknown-linux-musl` on `ubuntu-latest` (with `musl-tools`), `x86_64-pc-windows-msvc` on `windows-latest`.
- **FR-8**: Each release platform runs the verification gate before building: `cargo check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-features`; `cargo audit` on the Linux job only.
- **FR-9**: Artifacts are archives named `syncsmith-<version>-x86_64-unknown-linux-musl.tar.gz` and `syncsmith-<version>-x86_64-pc-windows-msvc.zip`, containing the binary at the archive root (`syncsmith` / `syncsmith.exe`).
- **FR-10**: Each archive has a sibling `.sha256` checksum file (e.g. `...tar.gz.sha256`).
- **FR-11**: The Linux release job generates the CycloneDX SBOM via `cargo cyclonedx -f json` and attaches it as `sbom.json` (dependencies are OS-independent).
- **FR-12**: A GitHub release is created as a draft with GitHub auto-generated release notes, and all archives, checksum files, and the SBOM are attached, using `softprops/action-gh-release` with `GITHUB_TOKEN` (`permissions: contents: write`).

## Acceptance Criteria

- **AC-1**: A push to `main` or an opened PR runs the CI workflow on both OSes and all four verify steps pass on a clean tree.
- **AC-2**: A PR build does not run `cargo audit`; a `main` push does.
- **AC-3**: The Linux CI job checks the `x86_64-unknown-linux-musl` target compiles.
- **AC-4**: Pushing `v0.1.0` (with matching `Cargo.toml` version) triggers the release workflow and produces a draft release with 5 assets: 2 archives, 2 `.sha256` files, and `sbom.json` (plus auto-notes).
- **AC-5**: Pushing a tag whose version does not match `Cargo.toml` fails the release workflow with a clear error before any build.
- **AC-6**: The musl archive contains a static binary; `sha256sum -c <archive>.sha256` verifies each archive.
- **AC-7**: Both workflows pass `actionlint`-level syntax validation (YAML valid, action pins resolved).

## Edge Cases

- **EC-1**: Tag without leading `v` → release workflow does not trigger (pattern `v*`); no partial releases.
- **EC-2**: Tag/Cargo version mismatch → explicit failure at step start (FR-6), before build resources are spent.
- **EC-3**: Release re-run after partial failure → `softprops/action-gh-release` is idempotent; assets are re-uploaded.
- **EC-4**: No advisory DB issues → `cargo audit` passes; DB download happens only on `main` pushes and the Linux release job.
- **EC-5**: Tests requiring `git` on the runner → both hosted runners ship git preinstalled.

## Decision Log

| Decision | Rationale | Alternatives Rejected |
|---|---|---|
| CI on push main + PRs | Fast feedback on PRs, guard on main | PRs only; all pushes |
| Audit on main pushes only | Avoids advisory-DB latency on every PR | Always audit; never audit |
| Linux + Windows test matrix | Exercises `#[cfg(windows)]` paths currently only compile-checked | Linux only |
| Native runners per platform | Simple, no containers/cross complexity | cross-rs cross-compilation |
| musl for Linux release | Static binary, no glibc coupling | gnu target |
| Full target triple in artifact names | Unambiguous, matches rust triples | Short os-arch names |
| musl check in CI | Release-target breakage surfaces before release day | Release-only musl build |
| Tag version must match Cargo.toml | Prevents publishing mislabeled binaries | Tag-only or Cargo-only |
| Per-artifact `.sha256` | Sibling file per archive, simple to host next to each asset | Single SHA256SUMS |
| Draft release | Review notes and assets before publishing | Publish directly |
| softprops/action-gh-release | Idempotent, widely used, one-step attach | gh release create |
| SBOM attached (single, Linux job) | Mirrors Taskfile release deps; deps OS-independent | Per-platform SBOMs; none |

## References

- `Taskfile.yml` (quality gates and release task)
- `docs/specs/2026-10-21-2100-Feature-GithubAction/pre-spec.md`
