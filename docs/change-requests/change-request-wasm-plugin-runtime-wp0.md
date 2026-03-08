# Change Request: WASM Plugin Runtime WP0

Version: 0.1
Status: approved-for-implementation
Owner: Oberon4

## Current State
- Oberon4 had no canonical shared contract module for the planned WASM/WASI plugin runtime.
- Runtime schemas and the host/plugin WIT boundary existed only as markdown PRD material.
- No machine-validated fixtures existed for plugin manifests, tool envelopes, approval payloads, or the first workspace/git/GitHub command package.

## Proposed Delta
- add a shared `oberon4-core` contract module for plugin manifests, runtime envelopes, approval/policy payloads, and handle/resource metadata
- add machine-readable JSON Schemas for the runtime contract and first-package command families
- add fixture-driven contract tests validating the schemas and Rust types together
- add an initial WIT draft for the host/plugin boundary
- update the implementation plan to mark `WP0` complete after verification

## Non-Goals
- no plugin registry or runtime execution
- no WASM/WASI instance management
- no workspace, artifact, secret, or service broker implementation
- no provider adapter integration

## Compatibility Impact
- backward-compatible
- adds new contract artifacts without changing current daemon/provider runtime behavior

## Migration
- none required for current users
- later phases should consume the new `oberon4-core` contract types and schemas instead of inventing parallel wire shapes

## Rollback
- revert the `WP0` contract commit and the follow-up documentation checkpoint commit
- remove the added `jsonschema` dev-dependency if contract validation is rolled back

## Verification Plan
- `cargo test -p oberon4-core`
- schema fixture validation from `crates/oberon4-core/tests/plugin_runtime_schema_validation.rs`
- WIT sanity checks against `docs/prd/plugin-runtime-interface-schema.md`
