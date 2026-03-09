# Runbook: WASM Plugin Runtime WP2

## Purpose
- document the verification and rollback steps for the WASM/WASI execution-layer slice of the plugin runtime

## Verification
- run `cargo fmt --all`
- run `cargo test -p oberon4-daemon`
- run `cargo test -p oberon4-daemon --test plugin_runtime_execution`
- confirm the daemon runtime now supports:
  - valid Preview 2 component load and invoke
  - real plugin `plan`
  - host-authoritative limit enforcement
  - structured failures for timeout, memory, output, and storage
  - fail-closed rejection of unsupported handle-dependent capabilities in `WP2`

## Rollback
1. revert the `WP2` implementation commit
2. revert the `WP2` governance/checkpoint commit if split
3. rerun:
   - `cargo test -p oberon4-daemon`
4. confirm the daemon no longer loads `plugin.wasm` or executes component-backed `tool_invoke`

## Escalation
- if component execution diverges from the WIT world, or if unsupported handles become reachable in `WP2`, stop and update the PRD/change request before continuing with `WP3`
