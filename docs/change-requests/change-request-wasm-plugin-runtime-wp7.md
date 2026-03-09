# Change Request: WASM Plugin Runtime WP7

Version: 0.1
Status: approved-for-implementation
Owner: Oberon4

## Current State
- `WP6` completed the provider-integrated host tool loop, but operating mode remained mostly doc-only.
- The daemon runtime did not read `dev` vs `production` mode from shared config.
- Ordinary plugins were not subject to production allowlisting or conformance-based admission rules.
- `tool_policy_inspect` did not expose production eligibility, allowlist state, or conformance state.
- Threat-model coverage existed as prose in [threat-model-wasm-plugin-runtime.md](/home/hendo420/Oberon4/docs/security/threat-model-wasm-plugin-runtime.md), but not as explicit release-hardening regressions.

## Proposed Delta
- extend shared runtime config with:
  - `operating_mode`
  - `allowed_plugin_checksums`
- extend the shared policy contract with:
  - `plugin_admission_status`
  - `allowlist_status`
  - `conformance_status`
  - `production_eligible`
- add runtime-managed conformance evaluation for ordinary plugins during registration
- treat built-in brokered packages as host-trusted and exempt from ordinary-plugin allowlist blocking
- enforce production-only activation rules for ordinary plugins:
  - provenance checksum allowlisted
  - conformance passed
  - contract/runtime validation passed
- tighten production budgets and approval defaults
- fail `tool_search`, `tool_describe`, `tool_plan`, and `tool_invoke` closed for plugins blocked by production admission policy
- expose mode/admission diagnostics through `tool_policy_inspect`
- add threat-model-driven regression coverage for mode blocking, conformance blocking, and continued secret/workspace protections

## Non-Goals
- no new provider features
- no new plugin capabilities
- no raw filesystem, raw secret, or raw network interfaces
- no redesign of the plugin manifest format beyond policy/reporting fields already described in PRD
- no new runtime-selectable providers

## Compatibility Impact
- backward-compatible for existing configs because:
  - `operating_mode` defaults to `dev`
  - `allowed_plugin_checksums` defaults to empty
- production mode is intentionally stricter and can block ordinary plugins that were previously invokable
- built-in `workspace`, `git`, and `github` packages remain available in both modes

## Migration
- existing configs continue to parse and fall back to `dev`
- operators may opt into production hardening by setting:
  - `runtime.operating_mode = "production"`
  - `runtime.allowed_plugin_checksums = ["sha256:..."]`
- no manifest migration is required for already-valid plugins

## Rollback
1. revert the `WP7` implementation commit
2. revert the `WP7` governance/checklist commit if split
3. rerun:
   - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-core`
   - `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
4. confirm:
   - runtime config no longer exposes mode/allowlist
   - `tool_policy_inspect` no longer reports admission/conformance status
   - production-only allowlist and conformance blocking are absent

## Verification Plan
- `cargo fmt --check`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-core`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`
- `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
- `bash scripts/policy-drift-audit.sh`
- `bash scripts/exception-debt-report.sh`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' bash scripts/local-ci-gate.sh`
