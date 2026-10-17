# Rust coding guidelines

These are Rust-specific guidelines, extending and taking precedence over common rules.

## Implementation Best Practices

### 1 — Code Structure and Architecture

- **Rust-A-1 (MUST)** Separate library logic (`src/lib.rs` and modules) from CLI binary entry point (`src/main.rs`). `main.rs` should only parse CLI arguments, initialize logging, and delegate to library functions.
- **Rust-A-2 (MUST)** Use structs instead of tuples for return types, arguments & parameters.

---

### 2 — Error Handling

- **Rust-ERR-1 (NEVER)** Use `.unwrap()` or `.expect()` in non-test code unless provably unreachable (and documented with a safety comment).
- **Rust-ERR-2 (MUST)** Return typed domain errors from internal modules (using `thiserror`).
- **Rust-ERR-3 (MUST)** Bubble errors to `main.rs` and format human- and agent-readable error messages to `stderr` with an appropriate non-zero exit code (`std::process::ExitCode`).

---

### 3 — CLI & Agent Interaction

- **Rust-CLI-1 (MUST)** Emit all tracing, status messages, and diagnostics to `stderr` to avoid corrupting `stdout` payloads.
- **Rust-CLI-2 (SHOULD)** Use `clap` with derive macros for declarative, consistent command-line flag and argument parsing.

---

### 4 — Idiomatic Rust and Tooling

- **Rust-C-1 (MUST)** Pass `cargo clippy --all-targets -- -D warnings` without warnings.
- **Rust-C-2 (MUST)** Adhere to standard formatting validated by `cargo fmt --check`.
- **Rust-C-3 (SHOULD)** Avoid unnecessary heap allocations and cloning; prefer borrowed slices (`&str`, `&[u8]`) where ownership is not required.
- **Rust-C-4 (SHOULD)** Use idiomatic pattern matching and combinators (`map`, `and_then`, `unwrap_or_else`) instead of manual conditional checks on `Option` and `Result`.

---

### 5 — Testing

- **Rust-T-1 (MUST)** Unit tests (tests that are purely in memory, and don't need external databases, testcontainers, filesystem or network) should be stored as much as possible in a "test" submodule of the current tested module. Tests that rely on any of the previously described systems should be placed in the tests/ subfolder.
- **Rust-T-2 (SHOULD)** Use `assert_eq!`, `assert_matches!`, or custom assertion helpers to test node content, metadata, and error variants clearly.

---

### 6 - Tool Gating

- **Rust-GATE-1 (MUST)** `cargo fmt --check` to ensure that the code is following the standard format.
- **Rust-GATE-2 (MUST)** `cargo clippy --all-targets -- -D warnings` to ensure that no linter errors are triggered. This check the minimal quality of the code.
- **Rust-GATE-3 (MUST)** `cargo test` to ensure that functionalities keep working as intended.
