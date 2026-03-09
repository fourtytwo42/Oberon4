# Runbook: WASM Plugin Runtime WP3

## Purpose
- verify host-custodied secret brokerage, brokered service execution, and runtime redaction for the `WP3` plugin-runtime phase

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
- ordinary plugins still cannot read raw secrets
- brokered GitHub/OAuth requests execute without exposing credentials
- runtime outputs redact exact secret values and common token-bearing shapes
- policy inspection reports broker-aware metadata without leaking secret material
- encrypted-local fallback preserves the same plugin-facing contract as the other secret backends

## Rollback
1. revert the `WP3` implementation commit
2. rerun:
   - `cargo test -p oberon4-core`
   - `cargo test -p oberon4-daemon`
3. confirm brokered service commands no longer execute and policy inspection no longer reports broker metadata

## Operational Note
- on constrained hosts, use the same low-debug Rust profile from the canonical gate scripts when rerunning the daemon suite to avoid linker churn from Wasmtime-heavy test binaries
