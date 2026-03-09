# Change Request: WASM Plugin Runtime WP2

Version: 0.1
Status: approved-for-implementation
Owner: Oberon4

## Current State
- `WP1` introduced the host-owned plugin registry, manifest validation, policy resolution, and host-native tool workflow.
- Registered plugins were discoverable and plannable, but `tool_invoke` still failed closed because no component runtime existed yet.
- The daemon did not load `plugin.wasm`, instantiate components, or enforce runtime execution limits.

## Proposed Delta
- add a Wasmtime Preview 2/component-model execution engine to `oberon4-daemon`
- require `plugin.wasm` adjacent to `plugin.json` during registry loading and retain executable metadata in registered plugins
- bind the daemon to the canonical WIT world in `wit/oberon-plugin-runtime/plugin-runtime.wit`
- execute plugin `plan` and `invoke` with fresh store and fresh component instance per invocation
- inject host-authoritative invocation context including resolved limits and capability grants
- mount only plugin-private/session/cache storage in `WP2`
- enforce structured resource failures for:
  - timeout
  - memory
  - output
  - storage
- add committed Preview 2 fixture components and daemon execution tests for valid invokes, broken components, unsupported capabilities, and resource-limit behavior
- update the implementation plan to mark `WP2` complete after verification

## Non-Goals
- no workspace broker implementation
- no artifact broker implementation beyond `WP2` fail-closed behavior
- no service broker or secret broker implementation
- no provider adapter changes
- no plugin pooling or instance reuse

## Compatibility Impact
- backward-compatible
- plugin execution is now available for ordinary WP2-safe components
- commands requiring unsupported brokered handles still fail closed with structured `NOT_YET_SUPPORTED_IN_WP2`

## Migration
- plugins intended for `WP2` execution must ship `plugin.wasm` next to `plugin.json`
- later phases should keep using the same WIT world and daemon execution engine rather than introducing a parallel runtime

## Rollback
1. revert the `WP2` implementation commit
2. revert the `WP2` governance/checkpoint commit if split
3. rerun:
   - `cargo test -p oberon4-daemon`
4. confirm `tool_invoke` is no longer executing components and registry loading no longer requires `plugin.wasm`

## Verification Plan
- `cargo fmt --all`
- `cargo test -p oberon4-daemon`
- `cargo test -p oberon4-daemon --test plugin_runtime_execution`
- repo-level gates before push:
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
