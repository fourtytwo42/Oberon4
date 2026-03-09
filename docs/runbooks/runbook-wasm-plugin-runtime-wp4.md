# Runbook: WASM Plugin Runtime WP4

## Purpose
- verify the workspace and artifact broker phase for the WASM/WASI plugin runtime

## Verification
- run `cargo test -p oberon4-core`
- run `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- run repo-level gates:
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
  - `bash scripts/policy-drift-audit.sh`
  - `bash scripts/exception-debt-report.sh`
  - `bash scripts/local-ci-gate.sh`

Expected outcomes:
- logical workspace paths stay relative and normalized
- absolute, traversal, and symlink-escape paths fail closed
- stale workspace revisions return structured conflicts instead of silent overwrite
- artifact retention evicts expired entries first and then oldest entries under size pressure
- commands that support artifacts can offload oversized JSON output into host-created artifact references

## Rollback
1. revert the `WP4` implementation commit
2. rerun:
   - `cargo test -p oberon4-core`
   - `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
3. confirm:
   - workspace capabilities no longer execute
   - artifact offload no longer occurs
   - workspace/artifact broker unit tests are absent or fail as expected

## Operational Note
- on constrained hosts, keep the low-debug Rust profile used by the canonical gate scripts when running the daemon suite; the Wasmtime-linked execution tests remain the heaviest part of this phase
