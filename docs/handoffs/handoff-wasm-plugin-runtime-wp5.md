# Handoff: WASM Plugin Runtime WP5

Phase: `WP5`
Status: complete
Owner: Oberon4

## Completed Scope
- added the first concrete versioned workspace package as built-in brokered tools:
  - `workspace.*`
  - `git.*`
  - `github.*`
- registered these tools into the daemon runtime search/describe/index path as Tier 2 brokered privileged commands
- implemented brokered `workspace.*` operations on top of the logical workspace broker:
  - read/list/stat
  - revision-aware atomic writes
  - revision-aware patch application
- implemented embedded host-managed `git.*` behavior without a system `git` dependency:
  - status
  - diff with artifact offload
  - branch creation
  - commit
  - push through brokered GitHub service execution
- implemented brokered `github.*` behavior on top of the service broker:
  - auth status
  - clone to logical workspace
  - repo metadata lookup
- normalized built-in dispatch so:
  - built-ins execute through the same host redaction and error-envelope discipline as ordinary plugins
  - `github.clone_to_workspace` does not incorrectly require a pre-existing workspace handle
  - stale workspace revisions return structured `CONFLICT` errors

## Verification Evidence
- `cargo fmt --check`
- `cargo test -p oberon4-core`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`
- `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
- `bash scripts/policy-drift-audit.sh`
- `bash scripts/exception-debt-report.sh`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' bash scripts/local-ci-gate.sh`

## Remaining Phases
- `WP6` Provider and Agent Integration
- `WP7` Mode and Release Hardening

## Risks / Notes
- the current `git.*` implementation is intentionally bounded to safe non-destructive operations; no force push or history rewrite exists
- the versioned workspace package is host-managed and install-and-go, but it is not yet exposed through provider-specific prompt shaping; that belongs to `WP6`
- on constrained hosts, low-debug Rust settings remain important for the heaviest Wasmtime-linked test and gate paths

## Next Action
- begin `WP6` by integrating the runtime and concrete package surface into provider and agent adapter flows while preserving the canonical search -> describe -> plan -> invoke workflow
