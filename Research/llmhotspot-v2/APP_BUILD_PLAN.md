# LLM Hotspot v2 App Build Plan

## 1. Goal

Build a single-binary cross-platform app (Windows/Linux/macOS) that provides:

- A documented CLI for automation and agent invocation
- A simple, robust, one-page mobile-friendly Web UI
- Multi-account OpenAI OAuth connection management with priority/failover/usage tracking
- Local + remote endpoint exposure with key management (multiple keys, revoke, expiration)
- Remote interface parity (same UI over subdomain URL)
- System tray integration:
  - Left-click opens console Web UI
  - Right-click menu: Open Console, Restart App, Exit

KISS constraints:

- Keep architecture minimal and explicit
- Avoid optional complexity unless required for core functionality
- Keep all commands and user actions clearly documented

Locked transport rule:

- OpenAI ChatGPT OAuth transport is Codex-specific and must remain pinned to the Codex backend contract, not normal OpenAI public API endpoint semantics.
- Reference: `docs/OAUTH_CODEX_ENDPOINT_RULE.md`

---

## 2. Product Requirements (Locked)

### 2.1 CLI

- CLI must support:
  - Starting/stopping/status of app
  - Hotspot session start/stop/status
  - OAuth account connect/list/activate/reprioritize/disconnect
  - API key create/list/revoke/expire
  - Remote connection status and URL output
  - Endpoint test commands (local + remote)
- All CLI commands must include:
  - `--help` docs
  - examples
  - machine-readable output option (`--json`)

### 2.2 Web UI

- Single-page UI
- Mobile-friendly and desktop-friendly
- Must display:
  - Local endpoint URL
  - Remote endpoint URL (copyable)
  - Remote console URL (copyable)
  - OAuth account list (status, usage, priority, failover state)
  - API key table (name, created, expires, status, copy/revoke)
  - Key generation + expiration datetime input
- All keys/URLs are click-to-copy
- Same UI content for local and remote console access

### 2.3 Auth and Routing

- Multiple OpenAI OAuth accounts connected simultaneously
- Priority order editable by user
- Automatic failover on upstream/auth/rate errors
- Per-account usage tracking shown in UI
- Key validation and expiration enforcement before request forward

### 2.4 Remote Access

- App receives remote URL from control-plane/subdomain registration
- Main UI shows remote URL
- Remote UI mirrors local UI actions and state

### 2.5 Tray Behavior

- Left-click: open console URL in default browser
- Right-click menu:
  - Open Console
  - Restart App
  - Exit

---

## 3. System Architecture

## 3.1 Runtime Components

1. `llm-control-api`
- Hosts HTTP API and serves embedded Web UI assets
- Handles local and remote console API routes

2. `llm-auth`
- OAuth flow orchestration, token refresh, account selection/failover

3. `llm-storage`
- SQLite for durable state
- Keyring for secrets

4. `llm-gateway`
- OpenAI-compatible request handling
- failover routing across connected OAuth accounts

5. `llm-governance` (minimal)
- auth/rate/guardrails/telemetry

6. `llm-capabilities`
- command/tool execution interfaces for agent/script use (scope-controlled)

7. tray runtime module (new crate to add)
- Cross-platform notification icon and menu integration

## 3.2 Storage Model (SQLite)

Add tables:

- `oauth_accounts`
  - `account_id` PK
  - `display_name`
  - `email`
  - `priority`
  - `enabled`
  - `last_used_unix`
  - `last_error`
- `oauth_usage`
  - `account_id`
  - `window_type` (`5h`, `weekly`)
  - `used`
  - `limit`
  - `reset_unix`
- `api_keys`
  - `key_id` PK
  - `label`
  - `key_hash`
  - `created_unix`
  - `expires_unix` nullable
  - `revoked_unix` nullable
  - `last_used_unix`
- `app_state`
  - key/value for remote URL, connector state, console settings

Secrets in keyring:

- OAuth refresh/access token bundles (keyed by workspace + connection id)

---

## 4. CLI Design

Top-level command: `llmhotspot`

Subcommands:

1. `serve`
- start daemon + API + UI + tray (unless `--no-tray`)

2. `status`
- show local URL, remote URL, connector status, account summary

3. `oauth`
- `oauth start`
- `oauth list`
- `oauth activate --account <id>`
- `oauth priority set --account <id> --index <n>`
- `oauth disable --account <id>`
- `oauth enable --account <id>`
- `oauth disconnect --account <id>`

4. `keys`
- `keys create --label <name> [--expires <RFC3339>]`
- `keys list`
- `keys revoke --key-id <id>`
- `keys copy --key-id <id>`

5. `endpoint`
- `endpoint local`
- `endpoint remote`
- `endpoint test local`
- `endpoint test remote`

6. `app`
- `app open-console`
- `app restart`
- `app stop`

CLI docs requirement:

- each subcommand includes examples in help text
- docs page generated to `docs/CLI_REFERENCE.md`

---

## 5. Web UI Design (Single Page)

## 5.1 Sections

1. Header
- app status chip
- local/remote reachability indicator

2. Endpoints card
- local endpoint URL + copy
- remote endpoint URL + copy
- remote console URL + copy

3. OAuth accounts card
- account rows with:
  - provider/email
  - enabled
  - priority
  - usage bars
  - last error
  - actions (activate, move up/down, disable/enable, disconnect)

4. API keys card
- create key form:
  - label
  - expiration datetime (optional)
- key list table:
  - masked key / copy button
  - expires
  - status
  - revoke button

5. Diagnostics card
- request counters
- failover events
- recent errors

## 5.2 UX constraints

- one page only
- mobile first (responsive stack on small screens)
- all copy buttons explicit and keyboard accessible
- no hidden critical actions

---

## 6. Remote Interface and Subdomain

1. On startup, app obtains/refreshes remote assignment from control-plane
2. App stores and exposes:
- `remote_base_url`
- `remote_console_url`
3. UI and CLI display these URLs
4. Remote console routes map to same API handlers as local UI
5. Security:
- remote key/session auth required for privileged console actions
- all key management endpoints protected

---

## 7. Tray Integration Plan

Add crate: `llm-tray`

Library choice:
- `tray-icon` crate (cross-platform)

Behavior:
- left-click event -> open `console_url`
- context menu:
  - `Open Console`
  - `Restart App`
  - `Exit`

Platform notes:
- Linux requires StatusNotifier/AppIndicator support fallback handling
- macOS app lifecycle integration with menu loop
- Windows native tray icon + shell notify

---

## 8. Implementation Phases and Test Gates

Each phase must pass:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

### Phase A: CLI foundation + docs generator

Build:
- introduce CLI crate/app binary command tree
- wire JSON/text output modes

Tests:
- parse tests for all subcommands
- smoke tests for help output

### Phase B: SQLite schema + key management

Build:
- finalize `api_keys` table + CRUD
- add expiration + revoke enforcement

Tests:
- key creation/revoke/expiry behavior tests
- hash verification tests

### Phase C: Multi-account OAuth manager

Build:
- connect multiple accounts
- priority/failover algorithm
- usage polling/tracking

Tests:
- failover chain tests
- priority routing tests
- token refresh fallback tests

### Phase D: Gateway integration with failover

Build:
- route requests across active accounts
- maintain OpenAI-compatible responses

Tests:
- request succeeds after first-account failure
- `401/429/503` mapping and retries

### Phase E: Web UI single-page implementation

Build:
- embed static assets in binary
- one-page UI with all required sections
- click-to-copy utilities

Tests:
- endpoint/API integration tests
- UI route serves and action endpoints respond correctly

### Phase F: Remote interface parity

Build:
- remote console URL support
- same handlers as local

Tests:
- local and remote produce same payloads for core endpoints
- auth checks enforced remotely

### Phase G: Tray integration

Build:
- tray icon + menu events
- open/restart/exit actions

Tests:
- unit tests for command dispatch
- platform smoke tests (manual checklist)

### Phase H: E2E verification

Tests:
- OAuth connect test (real/manual + automated mock)
- local endpoint test via CLI and UI
- remote endpoint test via CLI and UI
- key expiration test
- failover live test (simulate account outage)

---

## 9. Acceptance Criteria

1. Single binary runs on Windows/Linux/macOS
2. CLI commands fully documented and callable from scripts/agents
3. One-page responsive UI with copyable URLs/keys
4. Multiple OAuth accounts with failover/priority/usage visible and functional
5. Multiple API keys with revoke and expiration enforcement
6. Remote URL displayed and remote console parity works
7. Tray behavior matches spec
8. Full quality gates pass

---

## 10. Immediate Next Build Actions

1. Add CLI crate and command skeleton
2. Implement persistent API key manager with SQLite + hash validation
3. Add account priority/failover manager in `llm-auth` + `llm-gateway`
4. Add basic one-page embedded UI shell served by `llm-control-api`
5. Add remote URL display and copy endpoints
6. Add tray module and wire left/right click behavior
7. Run full test matrix after each milestone
