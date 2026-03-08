# Change Request: WASM Plugin Runtime WP1

Version: 0.1
Status: approved-for-implementation
Owner: Oberon4

## Current State
- `WP0` materialized the shared runtime contracts, JSON Schemas, fixtures, and WIT draft.
- Oberon4 still had no host-owned plugin registry, manifest validation pipeline, policy resolver, or host-native tool workflow for plugin discovery and planning.
- No daemon-side runtime surface existed for `tool_search`, `tool_describe`, `tool_plan`, `tool_invoke`, or `tool_policy_inspect`.

## Proposed Delta
- add pure registry, validation, policy, and host-tool traits/models to `oberon4-core`
- add a daemon-owned manifest registry that scans plugin roots for `plugin.json` without executing plugin code
- add manifest validation for runtime API compatibility, command uniqueness, schema shape, declared capability coverage, and provenance scaffolding
- add a static host policy resolver for effective capabilities, resource budgets, approvals, and operating mode resolution
- add host-native runtime surfaces for:
  - `tool_search`
  - `tool_describe`
  - `tool_plan`
  - `tool_invoke`
  - `tool_policy_inspect`
- add structured approval payload generation and explicit fail-closed `NOT_YET_EXECUTABLE` behavior for `tool_invoke` until `WP2`
- add artifact/storage broker foundation structs so later phases have a concrete daemon-owned runtime base
- update the implementation plan to mark `WP1` complete after verification

## Non-Goals
- no WASM/WASI instance loading or execution
- no workspace broker implementation
- no artifact persistence implementation
- no secret broker or service broker implementation
- no provider-facing tool adapter changes

## Compatibility Impact
- backward-compatible
- adds new daemon and core runtime surfaces without changing the current chat/session/provider behavior
- `tool_invoke` intentionally fails closed for registered plugin commands until `WP2`

## Migration
- none required for current users
- later phases should build on the daemon runtime registry/policy host rather than creating parallel plugin loaders or policy engines

## Rollback
1. revert the `WP1` runtime implementation commit
2. revert the `WP1` governance/checkpoint commit if split
3. rerun:
   - `cargo test -p oberon4-core`
   - `cargo test -p oberon4-daemon`
4. confirm the repo no longer exports the daemon plugin runtime host surfaces

## Verification Plan
- `cargo fmt --all`
- `cargo test -p oberon4-core`
- `cargo test -p oberon4-daemon`
- optional repo-level push gates before final push:
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets`
