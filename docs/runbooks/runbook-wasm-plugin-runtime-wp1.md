# Runbook: WASM Plugin Runtime WP1

## Purpose
- document the verification and rollback steps for the host-runtime-core slice of the WASM/WASI plugin runtime

## Verification
- run `cargo fmt --all`
- run `cargo test -p oberon4-core`
- run `cargo test -p oberon4-daemon`
- confirm the daemon runtime provides:
  - manifest-only plugin discovery
  - `tool_search`
  - `tool_describe`
  - `tool_plan`
  - `tool_invoke` fail-closed behavior before `WP2`
  - `tool_policy_inspect`

## Rollback
1. revert the `WP1` implementation commit
2. revert the `WP1` governance/checkpoint commit if split
3. rerun:
   - `cargo test -p oberon4-core`
   - `cargo test -p oberon4-daemon`
4. confirm the repo no longer exports the daemon plugin runtime host surfaces or the runtime foundation structs

## Escalation
- if plugin discovery, manifest validation, or policy resolution diverges from the runtime schemas or PRD, stop and update the PRD/change request before continuing with `WP2`
