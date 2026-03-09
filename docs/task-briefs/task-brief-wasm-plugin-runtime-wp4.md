# Task Brief: WASM Plugin Runtime WP4

## Goal
Implement `WP4` workspace and artifact brokers for the Oberon WASM/WASI plugin runtime so plugins can operate on host-managed logical workspaces and large payloads without raw host filesystem exposure.

## In Scope
- Extend `oberon4-core` with workspace/artifact broker contracts and result models.
- Implement daemon-owned workspace containment, revision tracking, and artifact retention/eviction.
- Integrate workspace and artifact capabilities into runtime planning and invocation.
- Update WP4 plan, change request, handoff, and runbook artifacts.

## Out of Scope
- Versioned workspace/git/github command package work from `WP5`.
- Provider adapter changes.
- Raw host path exposure or generic shell/file-descriptor APIs.

## Hard Constraints
- Keep handles opaque and host-issued.
- Keep all workspace paths logical, relative, and Oberon-native.
- Fail closed on traversal, absolute paths, ambiguous resolution, and broker misuse.
- Preserve the existing top-level agent tool workflow.

## Acceptance
- `EG-WP4` passes with workspace containment, shared-workspace conflict, artifact retention/eviction, and cross-platform path semantics covered by tests.
- `FR-112`: plugins can use host-brokered workspace and artifact capabilities without raw host path exposure.
- `FR-201`: shared logical workspaces remain revision-safe and conflict-aware.
- `AC-112.5`: workspace containment rejects absolute paths, traversal, and escape attempts.
- `AC-112.6`: artifact offload is host-created and validated.
- `AC-112.7`: policy/runtime surfaces reflect workspace and artifact capability state.
- `AC-201.2`: workspace writes require revision or equivalent precondition checks.
- `AC-201.4`: successful mutations return the new `workspaceRevision`.
- `AC-201.6`: multiple plugins can share a workspace only through host-issued handles and conflict checks.
- Given a plugin uses `workspace.write`, when it attempts a stale write with an old revision, then the broker returns a structured conflict and does not overwrite newer state.
