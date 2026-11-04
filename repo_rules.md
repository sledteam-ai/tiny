# Repository rules

## Repository boundary

- This repository is standalone.
- Do not add Cargo/path dependencies on sibling Sledteam runtime repositories.
- Follow cross-repository integration mechanisms that are already explicitly established
  by the task or existing repository contracts.
- If completing a task unexpectedly requires data, behavior, or types owned by another
  Sledteam repository, and the integration mechanism is not already defined, stop and
  report the dependency rather than inventing a solution.
- Do not choose a new transport, duplicate another repository's domain logic, introduce
  a shared crate, or otherwise create a new cross-repository boundary without explicit
  direction.

## Documentation

- Follow the shared
  [documentation policy](../../docs/conventions/DOCUMENTATION_POLICY.md).
- When writing or modifying architectural documentation, follow the
  [architectural writing guidelines](../../docs/conventions/ARCHITECTURE_WRITING.md).

## Change descriptions

- Follow the shared
  [change description guidelines](../../docs/conventions/COMMIT_GUIDELINES.md).

## Module layout

- Keep `mod.rs` files small and declarative.
- Do not put substantive implementation logic in `mod.rs`.
- Put non-trivial behavior in focused source files/modules.
- Do not introduce a `file.rs` + `file/` layout for the same concept.
- If a concept needs multiple files, use a directory with its own small `mod.rs`.

## Source size

- Soft limit: 500 lines.
- Hard limit: 600 lines.
- At 500 lines, consider whether the file should be split by responsibility.
- Do not exceed 600 lines.
- Split by coherent responsibility, not mechanically by line count.
- If an existing file already exceeds these limits, do not make it larger without
  explicit justification; prefer reducing it when the task naturally touches that code.

## Test ownership

- Test meaningful behavior owned by this repository.
- Use the minimum fixture semantics needed to exercise that behavior; do not mirror a
  sibling repository's current canonical data unless that exact data is part of this
  repository's contract.
- Test consumed and emitted contracts at this repository's boundary, but do not use
  repository-local tests to prove current cross-repository interoperability.
- Before adding overlapping coverage, identify the distinct failure class that is not
  already protected by an existing test.
- Prefer observable behavioral or contract assertions over incidental implementation
  properties.
- Cross-repository compatibility belongs in Sledteam integration/acceptance testing.
