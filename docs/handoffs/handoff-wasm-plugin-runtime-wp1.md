# Handoff: WASM Plugin Runtime WP1

Phase: `WP1`
Status: complete
Owner: Oberon4

## Completed Scope
- added host-facing runtime traits and policy/validation models to `oberon4-core`
- added daemon-owned plugin registry loading from `plugin.json` manifests only
- added manifest validation for runtime API version, duplicate commands, capability declarations, schema-object shape, and provenance scaffolding
- added static host policy resolution for capabilities, resource budgets, approvals, and operating mode
- added daemon host tool surfaces for `tool_search`, `tool_describe`, `tool_plan`, `tool_invoke`, and `tool_policy_inspect`
- added structured approval payload generation and fail-closed `NOT_YET_EXECUTABLE` behavior for `tool_invoke`
- added artifact/storage broker foundation structs to anchor later broker phases
- updated the runtime implementation plan to mark `WP1.1` through `WP1.6` complete

## Verification Evidence
- `cargo fmt --all`
- `cargo test -p oberon4-core`
- `cargo test -p oberon4-daemon`

## Remaining Scope
- `WP2` WASM/WASI Execution Layer
- `WP3` Secret and Service Brokers
- `WP4` Workspace and Artifact Brokers
- `WP5` Versioned Workspace Package
- `WP6` Provider and Agent Integration
- `WP7` Mode and Release Hardening

## Risks / Notes
- plugin execution is still intentionally unavailable; `tool_invoke` returns a structured non-execution error until `WP2`
- artifact and storage foundations are configuration scaffolding only in this phase; broker behavior begins in later phases
- the repo still contains unrelated unstaged changes outside the WP1 commit set and they must remain unstaged

## Next Action
- start `WP2` by integrating the selected WASM/WASI runtime and binding invocation context, limits, and fail-closed execution semantics to the host runtime introduced in `WP1`
