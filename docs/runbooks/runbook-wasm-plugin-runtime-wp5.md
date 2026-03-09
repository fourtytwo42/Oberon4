# Runbook: WASM Plugin Runtime WP5

## Purpose
- verify the first concrete versioned workspace package on top of the WASM/WASI plugin runtime

## Verification
- run `cargo fmt --check`
- run `cargo test -p oberon4-core`
- run `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- run repo-level gates:
  - `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
  - `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`
  - `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
  - `bash scripts/policy-drift-audit.sh`
  - `bash scripts/exception-debt-report.sh`
  - `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' bash scripts/local-ci-gate.sh`

Expected outcomes:
- `workspace`, `git`, and `github` built-ins appear in runtime search/describe surfaces
- workspace reads, writes, and patches are revision-safe and brokered
- `git.diff` produces artifact-backed diff metadata
- `git.commit` and `git.push` work without a system `git` binary
- GitHub broker operations do not expose raw secrets
- stale workspace revisions produce structured conflict responses

## Rollback
1. revert the `WP5` implementation commit
2. rerun:
   - `cargo test -p oberon4-core`
   - `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
3. confirm:
   - built-in versioned-package search results are absent
   - `plugin_runtime_versioned` tests no longer pass because the package is gone
   - ordinary plugin runtime behavior remains intact

## Operational Note
- on hosts with tight disk or memory ceilings, clear stale `target/` outputs before the full workspace gate and keep the constrained Rust profile from the canonical scripts
