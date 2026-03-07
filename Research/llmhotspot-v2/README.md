# llmhotspot-v2

Modular Rust workspace for the next-generation LLM Hotspot stack.

## Deployment model

This workspace is designed for a single-binary runtime on Windows/Linux/macOS:

- no Redis dependency
- durable embedded state in SQLite
- OS-native secret storage via keychain (`keyring`)
- in-memory caches for hot runtime paths

## Crates

- `llm-foundation`: contracts + runtime primitives
- `llm-storage`: state/secret/token stores
- `llm-auth`: OAuth/session auth services
- `llm-gateway`: OpenAI-compatible request gateway + relay abstractions
- `llm-governance`: policy, audit, telemetry primitives
- `llm-capabilities`: tools/channels capability services
- `llm-control-api`: HTTP control-plane adapters

## Quality gates

All changes must pass:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## Locked transport rule

OpenAI ChatGPT OAuth traffic is pinned to a Codex-specific backend contract (not standard OpenAI API endpoint semantics).  
See `docs/OAUTH_CODEX_ENDPOINT_RULE.md`.

## Quick start

```bash
make ci
```

## Remote endpoint assignment sources

`llmhotspot` resolves remote endpoint/console URLs in this precedence order:

1. Website endpoint manager (`POST /api/endpoints/details`) when all are set:
   - `LLMHOTSPOT_WEBSITE_BASE_URL`
   - `LLMHOTSPOT_LICENSE_KEY`
   - `LLMHOTSPOT_DEVICE_ID`
   - optional `LLMHOTSPOT_ENDPOINT_SLUG` (used for auto-register when no endpoint exists)
2. Control-plane assignment (`GET /assignment`) when set:
   - `LLMHOTSPOT_CONTROL_PLANE_URL`
   - optional `LLMHOTSPOT_CONTROL_PLANE_TOKEN`
3. Static fallback values:
   - `LLMHOTSPOT_REMOTE_BASE_URL`
   - `LLMHOTSPOT_REMOTE_CONSOLE_URL`

For website mode, the runtime maps:

- `remote_endpoint_url = endpoint.publicBaseUrl`
- `remote_console_url = endpoint.remoteConsoleUrl` when provided by website (otherwise unset)

If `/api/endpoints/details` returns `endpoint: null`, runtime attempts auto-register using
`/api/endpoints/register` and then re-fetches details.

Example:

```bash
LLMHOTSPOT_WEBSITE_BASE_URL=https://llmhotspot.com \
LLMHOTSPOT_LICENSE_KEY=LLMH-XXXX-XXXX-XXXX-XXXX \
LLMHOTSPOT_DEVICE_ID=my-machine-id \
./target/debug/llmhotspot serve --listen 127.0.0.1:11434 --no-tray
```
