# Change Request: WASM Plugin Runtime WP5

Version: 0.1
Status: approved-for-implementation
Owner: Oberon4

## Current State
- `WP4` made workspace and artifact brokering real, but there was still no first concrete package using those brokers.
- The runtime had no built-in `workspace.*`, `git.*`, or `github.*` command surface that an agent could discover and invoke directly.
- Versioned workspace behavior, diff/commit flows, and brokered GitHub remote operations remained PRD-only.

## Proposed Delta
- add a first-party versioned workspace package registered directly in the daemon runtime:
  - `workspace.*`
  - `git.*`
  - `github.*`
- implement host-brokered `workspace.*` commands on top of the `WP4` workspace broker:
  - `read_file`
  - `list_dir`
  - `stat_path`
  - `write_file_atomic`
  - `apply_patch`
- implement embedded host-managed `git.*` commands without a system `git` dependency:
  - `status`
  - `diff`
  - `branch.create`
  - `commit`
  - `push`
- implement brokered `github.*` commands on top of the `WP3` service broker:
  - `auth_status`
  - `clone_to_workspace`
  - `repo_info`
- integrate these commands into:
  - `tool_search`
  - `tool_describe`
  - `tool_plan`
  - `tool_invoke`
  - `tool_policy_inspect`

## Non-Goals
- no system `git` shelling or host-path git repository dependence
- no force push or destructive history operations
- no new provider adapter work
- no raw secret exposure
- no raw host filesystem exposure

## Compatibility Impact
- backward-compatible
- top-level agent workflow remains unchanged:
  - `tool_search`
  - `tool_describe`
  - `tool_plan`
  - `tool_invoke`
  - `tool_policy_inspect`
- runtime search/describe results now include the first-party `workspace`, `git`, and `github` tool surfaces

## Migration
- no manifest or schema migration required for existing ordinary plugins
- runtime now auto-registers built-in brokered tools:
  - `workspace`
  - `git`
  - `github`
- users still install Oberon only; no system `git` dependency is introduced for package use

## Rollback
1. revert the `WP5` implementation commit
2. revert the `WP5` governance/checkpoint commit if split
3. rerun:
   - `cargo test -p oberon4-core`
   - `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
4. confirm:
   - built-in `workspace`, `git`, and `github` tools are absent from registry search/describe
   - `plugin_runtime_versioned` tests no longer exist or fail as expected
   - runtime falls back to ordinary plugin behavior only

## Verification Plan
- `cargo fmt --check`
- `cargo test -p oberon4-core`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`
- `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`
- `bash scripts/policy-drift-audit.sh`
- `bash scripts/exception-debt-report.sh`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' bash scripts/local-ci-gate.sh`
