# Migration Plan (v1 -> v2)

This repo starts as a modular scaffold with tested interfaces.

## Source mapping

- `external/llm-hotspot/crates/common` -> `llm-foundation`
- `external/llm-hotspot/crates/daemon/src/state_store` + `secrets` -> `llm-storage`
- `external/llm-hotspot/crates/daemon/src/auth` -> `llm-auth`
- `external/llm-hotspot/crates/daemon/src/api/openai_compat` + hotspot/relay logic -> `llm-gateway`
- policy/safety/compliance/telemetry/audit/provenance -> `llm-governance`
- tools/terminal/channels -> `llm-capabilities`
- HTTP route adapters -> `llm-control-api`

## Parity status

- foundation: scaffolded
- storage: scaffolded (in-memory + SQLite embedded backend)
- auth: scaffolded (OAuth core flow)
- gateway: scaffolded
- governance: scaffolded
- capabilities: scaffolded
- control-api: scaffolded
