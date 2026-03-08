# Handoff: WASM Plugin Runtime WP0

Phase: `WP0`
Status: complete
Owner: Oberon4

## Completed Scope
- added canonical plugin/runtime contract types to `oberon4-core`
- added machine-readable runtime and first-package JSON Schemas plus fixtures
- added fixture-driven schema validation tests
- added initial `wit/oberon-plugin-runtime/plugin-runtime.wit`
- updated the runtime implementation plan to mark `WP0.1` through `WP0.4` complete

## Verification Evidence
- `cargo test -p oberon4-core`
- `rg -n "describe-plugin|list-commands|plan: func|invoke: func|health-check|workspace-handle|service-handle|secret-handle" wit/oberon-plugin-runtime/plugin-runtime.wit`
- `rg -n "shell|bash|/tmp|C:\\\\\\\\|SSH_AUTH_SOCK|secret read|raw filesystem" wit/oberon-plugin-runtime/plugin-runtime.wit`

## Remaining Scope
- `WP1` Host Runtime Core
- `WP2` WASM/WASI Execution Layer
- `WP3` Secret and Service Brokers
- `WP4` Workspace and Artifact Brokers
- `WP5` Versioned Workspace Package
- `WP6` Provider and Agent Integration
- `WP7` Mode and Release Hardening

## Risks / Notes
- the repo still contains unrelated unstaged changes outside the WP0 commit set
- runtime behavior is still unimplemented; this phase only materialized contracts and validation artifacts

## Next Action
- start `WP1` by implementing the manifest registry, policy engine scaffolding, and host-native tool surfaces against the newly added contract types
