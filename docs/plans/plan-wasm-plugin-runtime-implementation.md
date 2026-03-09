# Plan: Oberon4 WASM/WASI Plugin Runtime Implementation

Version: 0.1
Status: completed
Owner: Oberon4
WorkflowType: feature-development
WorkflowMode: incremental-gates
TaskState: done
Stack: rust
Risk: high-risk

## 1. Goal
Implement the Oberon4 WASM/WASI plugin runtime as a host-managed, cross-platform, policy-driven tool system, then ship the first concrete package for versioned workspaces plus git and GitHub integration.

## 2. Canonical Inputs
- `AGENTS.md`
- `docs/prd/PRD-wasm-tool-runtime.md`
- `docs/prd/architecture-wasm-tool-runtime.md`
- `docs/prd/tool-interaction-contract.md`
- `docs/prd/provider-tool-compatibility.md`
- `docs/prd/plugin-runtime-contract.md`
- `docs/prd/plugin-manifest-schema.md`
- `docs/prd/plugin-runtime-interface-schema.md`
- `docs/prd/approval-and-policy-schemas.md`
- `docs/prd/PRD-versioned-workspace-git.md`
- `docs/prd/workspace-git-github-command-schemas.md`
- `docs/security/threat-model-wasm-plugin-runtime.md`
- `Research/openclaw`
- `Research/llmhotspot-v2`

## 3. Scope
### In Scope
- host-side plugin registry and manifest loader
- host-side policy engine for capabilities, approvals, budgets, and operating modes
- typed runtime contract mapped into Rust types and future WIT files
- WASM/WASI execution engine integration
- host-side secret broker, service broker, and workspace broker abstractions
- `tool_search`, `tool_describe`, `tool_plan`, `tool_invoke`, `tool_policy_inspect`
- approval payload generation
- artifact and storage budget enforcement
- first versioned workspace package:
  - `workspace.*`
  - `git.*`
  - `github.*`
- automated tests for containment, redaction, approval, budgets, and cross-platform semantics

### Out of Scope
- plugin marketplace
- generalized third-party plugin distribution UX
- raw network capability for ordinary plugins
- raw secret reads
- shell-execution plugin
- wallet plugin implementation
- browser automation implementation

## 4. Execution Rules
- complete phases in order unless a dependency-free preparatory task is explicitly isolated
- do not begin implementation of a later phase before the earlier phase gate is green
- if implementation reveals a new requirement, update the relevant PRD and this plan before continuing
- do not mark tasks complete without verification evidence
- keep provider-wire logic adapter-scoped; do not leak provider quirks into the runtime core

## 5. Progress Tracker

### Phase 0: Schema and Contract Materialization
- [x] `WP0.1` Create Rust-native types for:
  - plugin manifest
  - runtime interface envelopes
  - approval payload
  - policy inspect result
- [x] `WP0.2` Create machine-readable JSON Schema files derived from:
  - `plugin-manifest-schema.md`
  - `approval-and-policy-schemas.md`
  - `workspace-git-github-command-schemas.md`
- [x] `WP0.3` Create initial WIT or component-interface drafts mapped from:
  - `plugin-runtime-interface-schema.md`
- [x] `WP0.4` Add contract tests that validate sample manifests and envelopes against the schema definitions.

Verification gate `EG-WP0`:
- schema validation tests pass
- Rust type serialization or deserialization tests pass
- WIT draft reviewed against `plugin-runtime-interface-schema.md`
Evidence:
- `cargo test -p oberon4-core`
- `rg -n "describe-plugin|list-commands|plan: func|invoke: func|health-check|workspace-handle|service-handle|secret-handle" wit/oberon-plugin-runtime/plugin-runtime.wit`
- `rg -n "shell|bash|/tmp|C:\\\\\\\\|SSH_AUTH_SOCK|secret read|raw filesystem" wit/oberon-plugin-runtime/plugin-runtime.wit`

### Phase 1: Host Runtime Core
- [x] `WP1.1` Implement plugin registry and manifest index.
- [x] `WP1.2` Implement manifest validation and provenance validation scaffolding.
- [x] `WP1.3` Implement host policy engine for:
  - capabilities
  - resource budgets
  - approvals
  - operating mode (`dev` / `production`)
- [x] `WP1.4` Implement core host tools:
  - `tool_search`
  - `tool_describe`
  - `tool_plan`
  - `tool_invoke`
  - `tool_policy_inspect`
- [x] `WP1.5` Implement artifact broker and storage broker foundations.
- [x] `WP1.6` Implement structured error and approval payload generation.

Dependencies:
- depends on `EG-WP0`

Verification gate `EG-WP1`:
- registry and manifest validation tests pass
- core tool contract tests pass
- policy inspection responses match schema
- approval payload tests pass
Evidence:
- `cargo test -p oberon4-core`
- `cargo test -p oberon4-daemon`
- `cargo fmt --all`

### Phase 2: WASM/WASI Execution Layer
- [x] `WP2.1` Integrate the selected WASM/WASI runtime.
- [x] `WP2.2` Implement per-invocation instance lifecycle.
- [x] `WP2.3` Implement invocation context injection:
  - request ids
  - handles
  - resolved limits
  - workspace revision where applicable
- [x] `WP2.4` Enforce:
  - memory limits
  - wall-clock timeouts
  - output limits
  - storage growth limits
- [x] `WP2.5` Implement structured resource-limit failures.

Dependencies:
- depends on `EG-WP1`

Verification gate `EG-WP2`:
- timeout tests pass
- memory limit tests pass
- output limit tests pass
- structured limit error tests pass
Evidence:
- `cargo test -p oberon4-daemon`
- `cargo test -p oberon4-daemon --test plugin_runtime_execution`
- `cargo fmt --all`

WP2 stabilization follow-up:
- [x] `WP2-S1` Upgrade Wasmtime and Wasmtime-WASI to the patched `36.0.6` line and raise the workspace Rust baseline to `1.86`
- [x] `WP2-S2` Pin the repo-local Rust toolchain to `1.86.0`
- [x] `WP2-S3` Align the canonical gate with the constrained Rust test profile used by local human preflight
- [x] `WP2-S4` Re-run workspace, security, and local CI gates on the remediated stack
Evidence:
- `cargo test --workspace`
- `GATE_MODE=strict RUN_SECURITY_GATE=1 bash scripts/oberon4-gate.sh`
- `bash scripts/local-ci-gate.sh`
- `PATH="$HOME/.cargo/bin:$PATH" bash scripts/oberon4-security-gate.sh`

### Phase 3: Secret and Service Brokers
- [x] `WP3.1` Implement host secret broker contract with:
  - `secret_handle`
  - `secret_use_handle`
  - `service_handle`
- [x] `WP3.2` Implement platform adapter abstraction for:
  - macOS secure storage
  - Windows secure storage
  - Linux secure storage
  - encrypted local fallback
- [x] `WP3.3` Implement brokered service execution for:
  - GitHub request path
  - OAuth-backed request path
  - future SSH or wallet signing scaffolding
- [x] `WP3.4` Implement redaction at:
  - logs
  - traces
  - stdout/stderr capture
  - tool results
  - error payloads

Dependencies:
- depends on `EG-WP2`

Verification gate `EG-WP3`:
- secret non-read tests pass
- redaction tests pass
- secret-class policy tests pass
- cross-platform adapter abstraction tests pass
Evidence:
- `cargo test -p oberon4-core`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`

### Phase 4: Workspace and Artifact Brokers
- [x] `WP4.1` Implement logical workspace broker with:
  - `workspace_handle`
  - normalized relative path enforcement
  - revision tracking
- [x] `WP4.2` Implement workspace containment protections:
  - absolute path rejection
  - traversal rejection
  - symlink or reparse-point escape prevention
- [x] `WP4.3` Implement artifact broker with retention and eviction policy.
- [x] `WP4.4` Implement shared-workspace conflict detection and revision preconditions.

Dependencies:
- depends on `EG-WP2`
- depends on `EG-WP3`

Verification gate `EG-WP4`:
- workspace containment tests pass
- shared-workspace conflict tests pass
- artifact retention and eviction tests pass
- cross-platform path semantics tests pass
Evidence:
- `cargo test -p oberon4-core`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`

### Phase 5: Versioned Workspace Package
- [x] `WP5.1` Implement `workspace.*` commands:
  - `read_file`
  - `list_dir`
  - `stat_path`
  - `write_file_atomic`
  - `apply_patch`
- [x] `WP5.2` Implement `git.*` commands against the brokered workspace.
- [x] `WP5.3` Implement GitHub-backed brokered commands:
  - `auth_status`
  - `clone_to_workspace`
  - `repo_info`
- [x] `WP5.4` Ensure the initial package can run in install-and-go mode with prebuilt artifacts and no local compile step for users.

Dependencies:
- depends on `EG-WP4`

Verification gate `EG-WP5`:
- workspace command contract tests pass
- git command contract tests pass
- GitHub broker command contract tests pass
- workspace edit -> diff -> commit integration test passes
- commit -> push integration test passes with brokered auth
Evidence:
- `cargo fmt --check`
- `cargo test -p oberon4-core`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
- `env CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`

### Phase 6: Provider and Agent Integration
- [x] `WP6.1` Integrate tool runtime with provider adapters using canonical tool contracts.
- [x] `WP6.2` Ensure provider adapters preserve:
  - tool call ids
  - normalized finish states
  - search/describe/plan/invoke workflow
- [x] `WP6.3` Add policy-inspection and approval-aware agent integration tests.

Dependencies:
- depends on `EG-WP1`
- depends on `EG-WP5`

Verification gate `EG-WP6`:
- provider tool compatibility tests pass
- agent workflow tests pass for search -> describe -> plan -> invoke
- approval-required flows pass end-to-end
Evidence:
- `cargo fmt --check`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-core`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-provider-openai`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-provider-anthropic`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-cli`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-tui`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`

### Phase 7: Mode and Release Hardening
- [x] `WP7.1` Implement `dev` and `production` operating mode policy resolution.
- [x] `WP7.2` Implement stricter production gating:
  - validated or allowlisted plugins
  - stricter budgets
  - stricter approvals
- [x] `WP7.3` Add conformance and threat-model-driven regression tests.

Dependencies:
- depends on `EG-WP6`

Verification gate `EG-WP7`:
- dev/production mode tests pass
- conformance suite passes
- threat-model coverage review completed against `docs/security/threat-model-wasm-plugin-runtime.md`
Evidence:
- `cargo fmt --check`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-core`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test -p oberon4-daemon`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo clippy --workspace --all-targets -- -D warnings`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' cargo test --workspace`
- `PATH=\"$HOME/.cargo/bin:$PATH\" bash scripts/oberon4-security-gate.sh`
- `bash scripts/policy-drift-audit.sh`
- `bash scripts/exception-debt-report.sh`
- `env CARGO_TARGET_DIR=/home/hendo420/.cache/oberon4-target CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 RUSTFLAGS='-C debuginfo=0' bash scripts/local-ci-gate.sh`

## 6. Requirement Mapping
- `FR-101` to `FR-110`: `WP0` to `WP3`, `WP6`, `WP7`
- `FR-111` to `FR-114`: `WP1` to `WP4`, `WP7`
- `FR-201` to `FR-205`: `WP4`, `WP5`, `WP6`
- schema docs:
  - `plugin-manifest-schema.md` -> `WP0.1`, `WP0.2`
  - `plugin-runtime-interface-schema.md` -> `WP0.1`, `WP0.3`
  - `approval-and-policy-schemas.md` -> `WP0.1`, `WP1.6`
  - `workspace-git-github-command-schemas.md` -> `WP0.2`, `WP5.*`

## 7. Verification Strategy
Required test scopes across phases:
- `contract`
- `unit`
- `integration`
- `security`
- `smoke`

Expected evidence categories:
- schema validation results
- redaction test results
- containment test results
- budget-limit test results
- provider compatibility results
- shared-workspace conflict results
- install-and-go package validation results

## 8. Completion Transition
The `WP0` through `WP7` runtime implementation sequence is complete.

Execution now moves to:
- `docs/plans/plan-oberon4-v0.1-release-validation.md`
- `docs/task-briefs/task-brief-oberon4-v0.1-human-acceptance.md`
- `docs/plans/plan-oberon4-post-v0.1-backlog.md`

This plan remains the canonical build-history and evidence artifact for the WASM/WASI plugin runtime, but it is no longer the active execution plan.
