# Change Request: Zip Plugin Archive Install

Status: approved-for-implementation
Owner: Oberon4

## Current State
- ordinary plugins load from unpacked `plugin.json + plugin.wasm` directories
- first-party packages load from unpacked `package.json + tools/*.plugin.json` directories
- package enablement and plugin discovery are split across root-level runtime config and directory presence

Evidence:
- runtime loader behavior in `crates/oberon4-daemon/src/plugin_runtime.rs`
- runtime defaults in `crates/oberon4-core/src/config.rs`

## Proposed Delta
- add archive-root discovery for `.obplg.zip` and `.obpkg.zip`
- make zip archives the primary installed artifact
- generate `plugin-config/<id>.toml` for every discovered archive
- preserve operator-edited settings while keeping install metadata host-managed
- make archive-backed entries authoritative over compatibility directory roots when both exist

## Non-Goals
- no remote installer or marketplace
- no live hot reload
- no change to the agent-facing host tool workflow
- no movement of first-party package execution out of host-brokered code

## Compatibility and Contract Impact
- backward-compatible
- compatibility directory roots remain supported during migration
- archive roots become the primary install surface
- host tools and command names stay stable

## Migration Plan
1. add archive roots and generated config root to runtime config
2. support archive-backed discovery in the registry
3. preserve compatibility directory loading only when no archive-managed id exists
4. document the new install flow and generated config behavior

## Rollback Plan
Trigger:
- archive discovery blocks daemon startup
- duplicate registration reappears
- generated config loading prevents known-good plugins/packages from activating

Steps:
1. revert the archive install commits
2. rerun `cargo test -p oberon4-core -p oberon4-daemon`
3. rerun `bash scripts/oberon4-security-gate.sh`
4. confirm directory-based loading is restored

## Verification Plan
- `cargo test -p oberon4-core -p oberon4-daemon`
- `cargo clippy -p oberon4-core -p oberon4-daemon --all-targets -- -D warnings`
- `bash scripts/oberon4-security-gate.sh`
- archive discovery and generated-config regression tests in `crates/oberon4-daemon/src/plugin_runtime.rs`
