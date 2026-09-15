# Repository rules

Repository boundary:
- This repository is standalone.
- Do not add Cargo/path dependencies on sibling Sledteam runtime repositories.
- Use installed serialized contracts for static/configuration metadata.
- Use Intercom/IRC for dynamic internal Sledteam communication.
- Use MCP for agent/tool-facing capabilities where appropriate.
- Keep Rust types local to the repository at serialized boundaries.
- If a task appears to require a sibling runtime dependency, stop and report it rather than adding one.

Module layout:
- Keep `mod.rs` files small and declarative.
- Do not put substantive implementation logic in `mod.rs`.
- Put non-trivial behavior in focused source files/modules.
- Do not introduce a `file.rs` + `file/` layout for the same concept.
- If a concept needs multiple files, use a directory with its own small `mod.rs`.

Source size:
- Soft limit: 500 lines.
- Hard limit: 600 lines.
- At 500 lines, consider whether the file should be split by responsibility.
- Do not exceed 600 lines.
- Split by coherent responsibility, not mechanically by line count.
- If an existing file already exceeds these limits, do not make it larger without explicit justification; prefer reducing it when the task naturally touches that code.
