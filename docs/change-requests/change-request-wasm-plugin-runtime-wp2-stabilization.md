# Change Request: WASM Plugin Runtime WP2 Stabilization

Phase: `WP2-S1` / `WP2-S2`
Status: complete
Owner: Oberon4

## Current State
- `WP2` introduced Wasmtime-backed plugin execution using `wasmtime 34.0.2` / `wasmtime-wasi 34.0.2`
- security verification later identified Wasmtime advisories against that dependency line
- strict local verification also proved fragile on lower-memory machines because the canonical gate did not use the same constrained Rust build profile as the human preflight

## Proposed Delta
- raise the workspace Rust baseline from `1.85` to `1.86`
- pin the repo-local toolchain to `1.86.0`
- upgrade `wasmtime` and `wasmtime-wasi` in `oberon4-daemon` to `36.0.6`
- apply the necessary Wasmtime 36 execution-layer compatibility fixes in the daemon runtime
- align the canonical Rust gate scripts with a constrained local build profile:
  - `TMPDIR=/home/hendo420/Oberon4/.tmp`
  - `CARGO_TARGET_DIR=/home/hendo420/Oberon4/target`
  - `CARGO_INCREMENTAL=0`
  - `CARGO_PROFILE_DEV_DEBUG=0`
  - `CARGO_PROFILE_TEST_DEBUG=0`
  - `CARGO_BUILD_JOBS=1`
  - `RUSTFLAGS='-C debuginfo=0'`

## Non-Goals
- no new `WP3` secret or service broker behavior
- no plugin contract redesign
- no workspace or artifact broker changes

## Compatibility Impact
- backward-compatible at the agent and plugin contract layer
- build/toolchain requirement increases to Rust `1.86`
- canonical gate behavior changes only at the build-profile level; runtime behavior is unchanged

## Verification Plan
- `cargo test -p oberon4-daemon`
- `cargo test -p oberon4-daemon --test plugin_runtime_execution`
- `cargo test -p oberon4-core`
- `cargo test --workspace`
- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
- `bash scripts/policy-drift-audit.sh`
- `bash scripts/exception-debt-report.sh`
- `bash scripts/local-ci-gate.sh`

## Rollback Triggers
- Wasmtime 36 breaks `tool_plan` / `tool_invoke` execution semantics
- security gate still fails for Wasmtime advisories
- strict gate still fails reproducibly under the constrained local profile

## Rollback Steps
1. revert the stabilization patch commit
2. restore the prior workspace Rust baseline and daemon Wasmtime versions
3. rerun:
   - `cargo test -p oberon4-daemon`
   - `cargo test -p oberon4-daemon --test plugin_runtime_execution`
4. reassess dependency remediation separately before resuming `WP3`
