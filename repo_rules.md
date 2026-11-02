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

Documentation:
- For Sledteam-specific architecture, `public/docs/` is the authoritative home; see the [documentation guide](../../../docs/README.md) and [authoring guidelines](../../../conventions/README_GUIDELINES.md). Describe current responsibilities, ownership boundaries, relationships, communication and execution pathways, lifecycles and significant behaviour. Source code remains authoritative for implementation mechanics.
- When working on Tiny's Sledteam integration, consult relevant architectural documents to understand its responsibilities and relationships. Changes affecting the architectural mental model require review of relevant documents against the resulting implementation.
- A document found accurate without edits may be recorded as `reviewed`; a document modified to reflect the architecture must then be verified before recording it as `updated`. Reading or consulting a document alone is not verification.
- Internal refactoring that preserves the architectural model does not automatically require documentation changes. Do not require review of unrelated architectural documents.
- Sledteam architectural documents use stable `doc-id` ULIDs in first-line Markdown HTML comments (`<!-- doc-id: <ULID> -->`). Copy existing identifiers for attestations; do not invent identifiers or generate new IDs for existing documents. Refer to the central documentation guide for identity conventions and progressive enrollment.
- Sledteam architectural explanations in local READMEs are being consolidated gradually into `public/docs/`. Preserve useful architectural content there before removing a redundant README. Repository-root READMEs may remain as entry points. Do not initiate migration or removal merely to align these rules.

Change descriptions:
- When writing or revising JJ change descriptions, read and follow the [change description guidelines](../../../conventions/COMMIT_GUIDELINES.md), including `Docs:` attestations for Sledteam architectural documents. Record only documents actually reviewed or updated and verified; do not attest while known architectural discrepancies remain unresolved.

Source size:
- Soft limit: 500 lines.
- Hard limit: 600 lines.
- At 500 lines, consider whether the file should be split by responsibility.
- Do not exceed 600 lines.
- Split by coherent responsibility, not mechanically by line count.
- If an existing file already exceeds these limits, do not make it larger without explicit justification; prefer reducing it when the task naturally touches that code.
