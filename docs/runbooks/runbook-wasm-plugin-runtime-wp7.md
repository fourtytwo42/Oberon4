# Runbook: WASM Plugin Runtime WP7

## Purpose
- verify operating-mode hardening, production admission behavior, and conformance/threat-model regressions for the plugin runtime

## Verification
- run `cargo fmt --check`
- run `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-core`
- run `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- run repo-level gates:
  - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
  - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`
  - `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
  - `bash scripts/policy-drift-audit.sh`
  - `bash scripts/exception-debt-report.sh`
  - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' bash scripts/local-ci-gate.sh`

Expected outcomes:
- `dev` mode permits valid ordinary plugins without allowlist enforcement
- `production` mode blocks ordinary plugins missing allowlist or conformance pass
- installed and enabled host-trusted first-party `workspace`, `git`, and `github` packages remain operational in production
- `tool_policy_inspect` reports admission, allowlist, conformance, and reason codes
- threat-model regressions for secrets, workspace containment, and confused-deputy broker use remain covered by tests

## Rollback
1. revert the `WP7` implementation commit
2. rerun:
   - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-core`
   - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
3. confirm:
   - mode/allowlist fields are absent from runtime config
   - admission and conformance diagnostics are absent from `tool_policy_inspect`
   - production-specific plugin blocking is no longer enforced

## Operational Note
- keep the constrained Rust profile and external target dir exported for the final push so the pre-push hook reuses `/home/hendo420/.cache/oberon4-target` instead of rebuilding into the repo filesystem
