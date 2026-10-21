# Code Review Report - 2026-10-21-2100-Feature-GithubAction

Iteration: 2
Date: 2026-10-21

## 1. Executive Summary

The iteration-1 blockers (cyclonedx `--override-filename` producing `sbom.json.json`, which would have killed every release) and majors (PR-scoped cancellation, third-party action pinning) are fixed and independently verified. Iteration 2's own regression — a one-hex-digit typo in the `Swatinem/rust-cache` SHA pin introduced by the pinning fix — was caught by the correctness reviewer, fixed, and re-verified byte-exact against the upstream tag. All automated checks pass (actionlint, cargo fmt/clippy/test, musl compile guard, version-check dry-run, packaging/checksum dry-run, local cargo-cyclonedx SBOM generation). No Rust source was modified. Remaining findings are 6 accepted MINORs and 2 INFOs; runtime confirmation on GitHub-hosted runners is deferred to the first real push/tag per plan.md Section 3.

## 2. Panel Summary

- **correctness**: 0 blockers, 0 majors | Status: APPROVED
- **security**: 0 blockers, 0 majors | Status: APPROVED
- **quality**: 0 blockers, 0 majors | Status: APPROVED
- **performance**: 0 blockers, 0 majors | Status: APPROVED
- **architecture**: 0 blockers, 0 majors | Status: APPROVED
- **readability**: 0 blockers, 0 majors | Status: APPROVED
- **tooling** (automated checks): 0 blockers, 0 majors | Status: APPROVED

Note: the architecture, quality, performance, and readability iteration-2 reviews and this synthesis were conducted by the orchestrator directly (with per-domain findings files written separately) because the session reached its sub-agent child limit; all findings files live under `reviews/iteration-2/` in the standard format. The merge script was executed by the orchestrator for the same reason — a forced, documented deviation from the normal synthesizer delegation.

## 3. Consolidated Findings

| ID | Severity | File | Line | Category | Description | Action for | Suggested action |
|---|---|---|---|---|---|---|---|
| C1 | INFO | .github/workflows/ci.yml | 30, 63, release.yml:54 | logic error — resolved | Originally BLOCKER: iteration-2 SHA pinning introduced a one-hex-digit typo in the `Swatinem/rust-cache` pin (`...12a0` vs `...12e0`), making every CI/Release job fail at startup. FIXED and re-verified: all three use sites pin `d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3`, byte-exact against peeled `refs/tags/v2.9.3^{}`. actionlint cannot catch SHA typos; `git ls-remote` is the only automated guard. | none | – |
| P1 | INFO | release.yml | 70 | redundant-work | `cargo check` runs immediately before clippy, which subsumes its compilation. Spec FR-8 prescribes both; spec-driven cost, not a defect. | none | Optional: amend FR-8 to drop `cargo check` from the release gate. |
| R1 | MINOR | release.yml | 29-38 | redundancy | Matrix carries 4 keys where `binary` and `archive_ext` are determined by `target`. Accepted trade-off for explicitness. | none | Optional: trim matrix keys. |
| R2 | MINOR | release.yml | 60/63/66/71 | repetition | `if: matrix.target == 'x86_64-unknown-linux-musl'` repeated 5 times. Accepted. | none | Optional: `is_linux` matrix boolean. |
| R3 | MINOR | release.yml | 17/89 | duplication | Tag-stripped version derived in `check-version` and again in `set version`. Accepted across job boundaries. | none | Optional: job output from check-version. |
| R5 | MINOR | ci.yml, release.yml | 56/30 | inconsistency | musl target added inline in ci.yml but via toolchain action `targets:` in release.yml. Accepted divergence (avoids musl install on Windows leg). | none | Documented rationale. |
| R6 | MINOR | ci.yml, release.yml | both | duplication | Verify-gate sequence duplicated across workflows. Accepted for a two-workflow repo. | none | Revisit if a third workflow appears. |
| S1 | MINOR | ci.yml, release.yml | 17/30/57 etc. | supply-chain | First-party actions (`actions/checkout@v5`, `upload/download-artifact@v4`) remain on mutable major tags; third-party pins are SHA-pinned. Inconsistent policy, lower risk (actions org supply chain). | executor | Optional: pin first-party actions to SHAs too, or document accepted risk. |

## 4. AC Coverage Matrix

| Acceptance Criterion | Test(s) | Status |
|---|---|---|
| AC-1 | Structural review of ci.yml triggers/matrix (all reviewers) + local `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-features` (248 tests) | covered (local); runner runtime deferred to first push |
| AC-2 | Structural review of audit job `if:` gate (correctness, architecture) | covered (structural); runner runtime deferred |
| AC-3 | ci.yml musl step + local `cargo check --target x86_64-unknown-linux-musl` PASS | covered (executed locally) |
| AC-4 | release.yml job graph review + local cargo-cyclonedx 0.5.9 run producing valid `sbom.json` + packaging dry-run | covered (local dry-run); first tag deferred |
| AC-5 | Version-check dry-run: `GITHUB_REF_NAME=v0.1.0` exit 0, `v9.9.9` exit 1 with clear error | covered (executed locally) |
| AC-6 | Packaging dry-run: tar.gz with binary at root, `sha256sum -c` PASS; musl target compiles locally | covered (checksum contract executed); static-binary property on runner deferred |
| AC-7 | `actionlint` on both workflows exit 0 (re-run after every fix) | covered (executed locally) |

## 5. Stagnation Analysis

No stagnant findings detected from previous iteration (script output, iteration 2 vs iteration 1).

## 6. Unresolved MINORs

- R1: release.yml:29-38 — matrix key redundancy (accepted).
- R2: release.yml:60/63/66/71 — repeated musl `if:` conditions (accepted).
- R3: release.yml:17/89 — version derived in two jobs (accepted).
- R5: ci.yml/release.yml — musl target-add style divergence (accepted, documented rationale).
- R6: ci.yml/release.yml — verify-gate sequence duplication (accepted).
- S1: ci.yml/release.yml — first-party actions on mutable tags (optional executor task).

## 7. Action Items & Next Steps

### Executor Tasks
- [S1 - MINOR, optional] Pin first-party actions (`actions/checkout@v5`, `upload/download-artifact@v4`) to commit SHAs, or document the accepted supply-chain risk.

### Test-Generator Tasks
- No pending action items.

## Status: APPROVED
