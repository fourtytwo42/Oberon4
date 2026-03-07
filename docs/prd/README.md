# Oberon4 PRD Set

This folder contains the canonical PRD set for Oberon4.

Documents:
- `PRD-oberon4.md`: product scope, requirements, acceptance criteria
- `architecture.md`: modular system design and crate boundaries
- `test-strategy.md`: verification strategy and required test scopes
- `roadmap.md`: phased delivery plan with release gates

Process rules:
- Requirement IDs are stable and traceable (`FR-*`, `NFR-*`, `AC-*`).
- Any behavior change must update the relevant PRD docs in the same change.
- Implementation and tests must map back to IDs in these docs.
