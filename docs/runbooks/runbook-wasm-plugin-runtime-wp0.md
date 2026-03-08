# Runbook: WASM Plugin Runtime WP0

## Purpose
- document the rollback and verification steps for the WP0 contract-only runtime foundation slice

## Verification
- run `cargo test -p oberon4-core`
- confirm schema fixtures pass in `crates/oberon4-core/tests/plugin_runtime_schema_validation.rs`
- confirm the WIT draft still exposes:
  - `describe-plugin`
  - `list-commands`
  - `plan`
  - `invoke`
  - `health-check`

## Rollback
1. revert the WP0 implementation commit
2. revert the governance/checkpoint commit that added the change request and handoff artifacts
3. rerun `cargo test -p oberon4-core`
4. confirm the repo no longer contains the WP0 schema/WIT/runtime contract artifacts

## Escalation
- if later phases fail due to contract shape mismatch, stop implementation and update the PRD or change request before resuming
