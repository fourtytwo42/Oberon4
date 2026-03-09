# Change Request: WASM Plugin Runtime WP3

Version: 0.1
Status: approved-for-implementation
Owner: Oberon4

## Current State
- `WP2` introduced the host-owned registry, policy engine, and Wasmtime-backed execution layer for ordinary plugins.
- Commands requiring service-backed capabilities still failed closed and there was no host-owned broker path for secret-backed operations.
- The runtime did not yet redact brokered outputs or surface broker-aware approval and policy metadata.

## Proposed Delta
- add host-owned secret broker contracts and runtime metadata for:
  - `secret_handle`
  - `secret_use_handle`
  - `service_handle`
- add daemon-owned secret backend abstractions for:
  - macOS
  - Windows
  - Linux
  - encrypted local fallback
- add host-brokered service execution for:
  - `github.request`
  - `oauth.request`
  - placeholder deny-by-default signing scaffolding for SSH and wallet paths
- add host-native redaction for:
  - tool results
  - error payloads
  - broker diagnostics
- upgrade `tool_plan`, `tool_invoke`, and `tool_policy_inspect` to understand brokered service capabilities and secret-class metadata
- update runtime schemas and fixtures to include broker-aware policy inspection metadata

## Non-Goals
- no workspace broker implementation
- no artifact broker expansion beyond existing `WP2` behavior
- no raw secret-read surface for agents or ordinary plugins
- no generic raw network capability
- no provider adapter workflow changes

## Compatibility Impact
- backward-compatible
- agent-facing tool names and workflow remain unchanged
- ordinary plugin execution remains unchanged for non-brokered commands
- broker-backed commands now route through host-owned services instead of failing with the old `WP2` unsupported-capability path

## Migration
- existing manifests that declare `service.github.request` or equivalent brokered service capabilities can now execute through the host broker when a matching `serviceHandle` is provided
- policy inspection responses now include optional broker metadata:
  - `serviceHandlesUsed`
  - `secretClassesInvolved`
  - `securityLevels`

## Rollback
1. revert the `WP3` implementation commit
2. revert the `WP3` governance/checkpoint commit if split
3. rerun:
   - `cargo test -p oberon4-core`
   - `cargo test -p oberon4-daemon`
4. confirm brokered service commands fail closed again and no broker/redaction paths remain active

## Verification Plan
- `cargo test -p oberon4-core`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- repo-level gates before push:
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
  - `bash scripts/policy-drift-audit.sh`
  - `bash scripts/exception-debt-report.sh`
  - `bash scripts/local-ci-gate.sh`
