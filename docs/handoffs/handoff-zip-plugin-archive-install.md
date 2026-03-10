# Handoff: Zip Plugin Archive Install

Done:
- added archive-based install model for ordinary plugins and first-party packages
- added generated per-plugin config files under `plugin_config_root`
- added archive-aware policy inspection fields and diagnostics
- added regression coverage for ordinary archive discovery, disable or missing states, config persistence, and first-party package archives

Files changed:
- `crates/oberon4-core/src/config.rs`
- `crates/oberon4-core/src/ipc.rs`
- `crates/oberon4-core/src/plugin_runtime.rs`
- `crates/oberon4-daemon/Cargo.toml`
- `crates/oberon4-daemon/src/lib.rs`
- `crates/oberon4-daemon/src/plugin_runtime.rs`
- `crates/oberon4-daemon/src/plugin_runtime_exec.rs`
- `crates/oberon4-daemon/src/plugin_runtime_versioned.rs`
- docs under `docs/prd/`, `docs/plans/`, `docs/runbooks/`, `docs/task-briefs/`, `docs/handoffs/`

Verification:
- targeted daemon archive tests pass
- full `oberon4-core` and `oberon4-daemon` test suites pass
- archive-backed ordinary plugin and first-party package paths are searchable and policy-visible

Remaining:
- optional operator-facing CLI diagnostics for installed or enabled archive state
- optional signature verification on top of `sha256`
