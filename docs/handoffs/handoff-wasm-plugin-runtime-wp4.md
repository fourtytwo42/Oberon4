# Handoff: WASM Plugin Runtime WP4

Phase: `WP4`
Status: complete
Owner: Oberon4

## Completed Scope
- added shared runtime contract types and traits for:
  - workspace metadata and mutation outcomes
  - artifact metadata and retention/offload metadata
  - `WorkspaceBroker`
  - `ArtifactBroker`
- added daemon-owned logical workspace broker with:
  - host-managed workspace roots
  - normalized logical path enforcement
  - absolute/traversal rejection
  - symlink escape prevention
  - revision-aware write and patch handling
- added daemon-owned artifact broker with:
  - immutable artifact creation
  - retention metadata
  - expiry-first and size-pressure eviction
- upgraded runtime planning, invocation, and policy inspection so:
  - workspace-capable commands can run when granted a workspace handle
  - artifact-capable commands can offload oversized results into host-created artifacts
  - policy inspection surfaces workspace/artifact capability declarations
- added broker/runtime coverage for:
  - workspace containment
  - stale revision conflicts
  - cross-platform logical path normalization
  - artifact retention/eviction
  - runtime artifact offload

## Verification Evidence
- `cargo test -p oberon4-core`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`

## Remaining Phases
- `WP5` Versioned Workspace Package
- `WP6` Provider and Agent Integration
- `WP7` Mode and Release Hardening

## Risks / Notes
- workspace access is host-brokered and mounted into the WASI runtime only when a workspace handle grant exists; the fixture plugin used in tests does not yet exercise direct guest-side file editing semantics
- artifact offload is host-authoritative and currently targets JSON payload offload for commands that explicitly support artifacts
- unrelated unstaged user changes outside this patch must remain unstaged

## Next Action
- begin `WP5` by implementing the first concrete versioned workspace package on top of the now-real workspace and artifact brokers:
  - `workspace.*`
  - `git.*`
  - `github.*`
