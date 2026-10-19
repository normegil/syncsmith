# Commons coding guidelines

These are generic guidelines for all languages, which must be read and followed. 

## Implementation Best Practices

### 1 — Before Coding

- **Commons-BP-1 (MUST)** Ask the user clarifying questions when specifications or requirements are ambiguous or incomplete.
- **Commons-BP-2 (SHOULD)** Draft and confirm an approach before implementing complex work.
- **Commons-BP-3 (SHOULD)** When multiple approaches exist, list clear trade-offs and pros/cons.

---

### 2 — While Coding

- **Commons-C-1 (MUST)** Name functions and data models using existing domain vocabulary for consistency.
- **Commons-C-2 (MUST)** Prefer small, composable, and testable functions.
- **Commons-C-3 (SHOULD)** Delete dead code and stale comments rather than commenting them out.
- **Commons-C-4 (SHOULD)** Functions should be pure as much as possible.
- **Commons-C-5 (SHOULD)** Structs should be immutable as much as possible.
- **Commons-C-6 (SHOULD)** Variables should be defined close to their usage.

---

### 3 — Testing

- **Commons-T-1 (MUST)** Separate pure-logic unit tests from external service/network integration tests.
- **Commons-T-2 (MUST)** Keep unit tests hermetic by avoiding network calls, external processes, or disk state.
- **Commons-T-3 (SHOULD)** Prioritize fast unit tests for rapid feedback loops.
- **Commons-T-4 (SHOULD)** Prefer integration tests over extensive internal state mocking.
- **Commons-T-5 (SHOULD)** Unit-test algorithmic logic, state transitions, and edge cases thoroughly.
- **Commons-T-6 (SHOULD)** Parameterize test cases instead of duplicating test logic.
- **Commons-T-7 (NEVER)** Add tests with trivial or self-fulfilling assertions (e.g., asserting hardcoded equality without exercising code).

#### 3.1 — Test Quality

- **Commons-TQ-1 (SHOULD)** State the specific behavior and condition being validated in the test name.
- **Commons-TQ-2 (SHOULD)** Verify outputs against pre-computed expectations or domain invariants, not by re-evaluating the system under test.
- **Commons-TQ-3 (SHOULD)** Cover boundary values, empty collections, whitespace variations, and malformed inputs.
- **Commons-TQ-4 (NEVER)** Write tests that only verify conditions guaranteed statically by the compiler or type system.

#### 3.2 Unit Tests

- **Commons-UT-1 (SHOULD)** Limit to one assertion per test maximum in unit tests.

---

### 4 — Linting and Formatting

- **Commons-L-1 (MUST)** Ensure code adheres to project formatting and linter checks with zero warnings.

---

### 5 — Documentation

- **Commons-D-1 (MUST)** Document public APIs, exported types, and modules with clear descriptions.
- **Commons-D-2 (MUST)** Keep existing documentation and usage examples synchronized with code changes.
- **Commons-D-3 (SHOULD)** Document parameters, return values, failure conditions, and errors.
- **Commons-D-4 (SHOULD)** Include runnable or illustrative examples in API docs.
- **Commons-D-5 (SHOULD)** Create Architecture Decision Records (ADRs) under `docs/` for significant design decisions.
- **Commons-D-6 (SHOULD)** Add deprecation notices.

---

### 6 — Code Review

- **Commons-R-1 (MUST)** Verify correctness: business logic, edge conditions, null/empty checks, and error paths.
- **Commons-R-2 (MUST)** Verify security: validate CLI inputs, sanitize paths, and safeguard credentials.
- **Commons-R-3 (MUST)** Verify performance: avoid unnecessary allocations, redundant roundtrips, or unbounded resource consumption.
- **Commons-R-4 (MUST)** Verify test coverage: ensure bug fixes contain regression tests and features have functional coverage.
- **Commons-R-5 (MUST)** Verify observability: ensure that the code is covered by logs, metrics & traces adequatly.

---

### 7 — Observability

- **Commons-O-1 (MUST)** Direct data output to `stdout` and diagnostic logs/error messages to `stderr`.
- **Commons-O-2 (SHOULD)** Support configurable logging/verbosity levels (`--quiet`, `-v`, `-vv`).
- **Commons-O-3 (SHOULD)** Ensure errors provide actionable diagnostic context (e.g., path that failed, connection target).

#### 7.1 — Webapps and backends

When editing a web application, a REST backend, ...
- **Commons-O-4 (SHOULD)** Ensure metrics are emitted for critical operations.
- **Commons-O-5 (SHOULD)** Include distributed tracing at important points of an application.
- **Commons-O-6 (SHOULD)** Propagate correlation IDs for distributed tracing.

---

### 8 — Architecture and Dependencies

- **Commons-A-1 (SHOULD)** Prefer simple, modular designs; avoid premature abstraction.
- **Commons-A-2 (SHOULD)** Separate interface/presentation logic (CLI commands) from domain and backend interactions.
- **Commons-A-3 (SHOULD)** Use dedicated or delegated types, or newtypes, rather than raw unbounded `String`s and other types.
- **Commons-DEP-1 (MUST)** Avoid unnecessary dependencies; evaluate footprint and transitive dependencies before adding crates or packages.

#### 8.1 — Function design

- **Commons-FN-1 (SHOULD)** Keep cyclomatic complexity low; break down deeply nested conditionals into guard clauses or dedicated helper functions.
- **Commons-FN-2 (SHOULD)** Eliminate unused function arguments and dead parameters.
- **Commons-FN-3 (SHOULD)** Make dependencies explicit via function parameters rather than relying on global state.
- **Commons-FN-4 (MUST)** Keep the number of function arguments to under 3 (at most 3 parameters). When a function requires more parameters, encapsulate related arguments into dedicated structs.

---

### 9 — External Systems

- **Commons-EXT-1 (MUST)** Always use an exponential backoff algorithm and a bounded timeout/retry limit for connections to systems external to the program.

---

### 10 — Database

---

### 11 — Code Organization

---

### 12 — Tooling Gates

---
