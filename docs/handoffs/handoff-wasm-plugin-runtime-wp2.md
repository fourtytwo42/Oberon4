# Handoff: WASM Plugin Runtime WP2

Phase: `WP2`
Status: complete
Owner: Oberon4

## Completed Scope
- added Wasmtime Preview 2/component-model execution to the daemon runtime
- required `plugin.wasm` alongside `plugin.json` during plugin registration
- wired host execution to the canonical WIT world in `wit/oberon-plugin-runtime/plugin-runtime.wit`
- implemented fresh-store, fresh-instance execution for plugin `plan` and `invoke`
- applied host-authoritative resource limits to invocation context before execution
- enforced structured limit failures for timeout, memory, output, and storage
- kept brokered handles fail-closed in `WP2` via `NOT_YET_SUPPORTED_IN_WP2`
- added committed fixture components plus daemon execution tests for valid invoke, broken component load, resource limits, and fresh-instance isolation
- updated the implementation plan to mark `WP2.1` through `WP2.5` complete

## Verification Evidence
- `cargo fmt --all`
- `cargo test -p oberon4-daemon`
- `cargo test -p oberon4-daemon --test plugin_runtime_execution`

## Remaining Scope
- `WP3` Secret and Service Brokers
- `WP4` Workspace and Artifact Brokers
- `WP5` Versioned Workspace Package
- `WP6` Provider and Agent Integration
- `WP7` Mode and Release Hardening

## Risks / Notes
- `WP2` only supports ordinary portable plugins plus private/session/cache storage mounts
- workspace, artifact, service, and secret handle brokers remain intentionally unavailable in this phase
- output and storage limit failures are enforced by the host after plugin execution returns; later phases may refine broker-level accounting further
- the repo still contains unrelated unstaged changes outside the WP2 commit set and they must remain unstaged

## Next Action
- start `WP3` by implementing host-custodied secret and service brokers, redaction, and secure brokered operations on top of the execution engine introduced in `WP2`
