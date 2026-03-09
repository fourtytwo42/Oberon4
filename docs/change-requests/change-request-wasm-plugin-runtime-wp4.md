# Change Request: WASM Plugin Runtime WP4

Version: 0.1
Status: approved-for-implementation
Owner: Oberon4

## Current State
- `WP3` added host-custodied secret and service brokers plus redaction.
- Workspace and artifact handles existed in the contract, but the runtime still treated logical workspaces and artifact offload as reserved or placeholder-only behavior.
- Plugins could not yet rely on host-managed shared workspaces, revision-aware writes, or immutable artifact retention.

## Proposed Delta
- add shared runtime contract types and traits for:
  - `WorkspaceBroker`
  - `ArtifactBroker`
  - workspace metadata, read/list/stat results, mutation conflict results
  - artifact metadata, retention policy, and offload results
- add daemon-owned logical workspace broker with:
  - host-issued workspace directories
  - normalized logical relative paths
  - revision tracking
  - atomic write and patch support
  - conflict detection on stale revisions
- add daemon-owned artifact broker with:
  - immutable artifact storage
  - retention metadata
  - expiry-first and size-pressure eviction
- upgrade runtime planning and invocation so:
  - `workspace.read` / `workspace.write` are supported when a workspace handle is granted
  - large outputs can offload into host-created artifacts when the command supports artifacts
  - policy inspection reports workspace/artifact capability involvement

## Non-Goals
- no `workspace.*` end-user command package yet
- no git or GitHub package commands yet
- no raw host filesystem exposure
- no artifact path exposure to plugins or agents
- no provider-adapter changes

## Compatibility Impact
- backward-compatible
- top-level agent workflow stays:
  - `tool_search`
  - `tool_describe`
  - `tool_plan`
  - `tool_invoke`
  - `tool_policy_inspect`
- commands that previously failed only because `workspace.*` / `artifact.*` were reserved can now execute when the host grants the required handles and the manifest/runtime path supports them

## Migration
- manifests that already declare `workspace.read`, `workspace.write`, or `artifact.create` do not need schema changes
- runtime behavior now depends on:
  - a granted `workspace_handle` for workspace-capable commands
  - host artifact retention policy for artifact offload
- output offload remains host-authoritative; plugins still do not mint raw artifact files themselves

## Rollback
1. revert the `WP4` implementation commit
2. revert the `WP4` governance/checkpoint commit if split
3. rerun:
   - `cargo test -p oberon4-core`
   - `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
4. confirm:
   - workspace/artifact broker tests are gone
   - workspace capabilities fail closed again
   - artifact offload no longer occurs

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
