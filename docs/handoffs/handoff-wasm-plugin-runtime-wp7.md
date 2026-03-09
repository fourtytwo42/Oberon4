# Handoff: WASM Plugin Runtime WP7

Phase: `WP7`
Status: complete
Owner: Oberon4

## Completed Scope
- threaded `operating_mode` and `allowed_plugin_checksums` through the shared runtime config
- extended policy/runtime contracts with:
  - `plugin_admission_status`
  - `allowlist_status`
  - `conformance_status`
  - `production_eligible`
- added registry-time conformance evaluation for ordinary plugins and stored per-plugin conformance metadata
- implemented mode-aware admission behavior:
  - `dev` allows valid ordinary plugins without allowlist enforcement
  - `production` blocks ordinary plugins that are not allowlisted or not conformance-passed
  - built-in host/brokered packages remain host-trusted and available in production
- tightened production defaults:
  - lower memory/time/storage/output limits
  - approval required for `plugin-private-write`
  - `destructive` blocked by default
- made `tool_search`, `tool_describe`, `tool_plan`, and `tool_invoke` fail closed for plugins blocked by production admission
- made `tool_policy_inspect` report operating mode, allowlist state, conformance state, production eligibility, and machine-readable block reasons
- added explicit regression coverage for the release-hardening threat areas called out in the runtime threat model

## Verification Evidence
- `cargo fmt --check`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-core`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`
- `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
- `bash scripts/policy-drift-audit.sh`
- `bash scripts/exception-debt-report.sh`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' bash scripts/local-ci-gate.sh`

## Remaining Phases
- none

## Risks / Notes
- `dev` remains permissive by default to preserve local iteration, so production safety depends on explicit operator configuration
- production allowlisting keys off manifest provenance checksum, not plugin id alone
- conformance is currently evaluated from host-visible contract/component checks; deeper ecosystem certification can build on this but is outside `WP7`
- built-in brokered packages remain exempt from ordinary-plugin allowlist blocking by design

## Next Action
- leave the `WP*` runtime implementation track and move into release validation, human acceptance, and backlog planning for post-v0.1 work
