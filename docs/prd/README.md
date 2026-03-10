# Oberon4 PRD Set

This folder contains the canonical PRD set for Oberon4.

Documents:
- `PRD-oberon4.md`: product scope, requirements, acceptance criteria
- `architecture.md`: modular system design and crate boundaries
- `PRD-wasm-tool-runtime.md`: product requirements for the host-managed WASM/WASI tool plugin runtime
- `architecture-wasm-tool-runtime.md`: runtime architecture, capability model, discovery flow, and storage/security boundaries
- `tool-interaction-contract.md`: canonical agent workflow for tool discovery, planning, invocation, and retry behavior
- `provider-tool-compatibility.md`: provider-specific tool-call normalization and compatibility rules
- `plugin-runtime-contract.md`: typed host/plugin boundary, handle model, and runtime envelope contracts
- `plugin-manifest-schema.md`: concrete `plugin.json` schema for WASM/WASI plugins
- `plugin-archive-schema.md`: concrete `oberon-archive.json` and generated-config schema for installed plugin and package archives
- `plugin-runtime-interface-schema.md`: concrete host/plugin runtime interface shapes for future WIT mapping
- `approval-and-policy-schemas.md`: concrete schemas for approval payloads and `tool_policy_inspect`
- `PRD-versioned-workspace-git.md`: first concrete versioned workspace, git, and GitHub integration package for the WASM/WASI tool system
- `workspace-git-github-command-schemas.md`: initial concrete command schemas for workspace, git, and GitHub surfaces
- `test-strategy.md`: verification strategy and required test scopes
- `roadmap.md`: phased delivery plan with release gates

Related security artifact:
- `docs/security/threat-model-wasm-plugin-runtime.md`: threat model for the WASM/WASI plugin runtime, brokered services, workspaces, artifacts, and secret handling

Process rules:
- Requirement IDs are stable and traceable (`FR-*`, `NFR-*`, `AC-*`).
- Companion contract identifiers such as `TIC-*`, `PTC-*`, and `PRC-*` are stable within their respective documents.
- Any behavior change must update the relevant PRD docs in the same change.
- Implementation and tests must map back to IDs in these docs.
