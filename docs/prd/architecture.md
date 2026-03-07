# Oberon4 Architecture

## 1. System Shape
Oberon4 is a daemon-centered Rust workspace with separated modules.

Proposed crate boundaries:
- `crates/oberon-core`: domain types, config, chat/session state, error model
- `crates/oberon-openai`: OpenAI-compatible API client and transport policy
- `crates/oberon-tui`: terminal UI state, rendering, command handling
- `crates/oberon-tray`: tray contracts + platform adapters
- `crates/oberon-daemon`: app composition, runtime lifecycle, orchestration

## 2. Runtime Flow
1. daemon boots config
2. daemon initializes endpoint client and UI/tray services
3. TUI input emits chat request
4. endpoint client executes request to configured API
5. response normalized into core domain model
6. TUI renders response and state updates

## 3. Contracts
- `CORE-C-001`: all user-visible errors mapped to typed domain errors
- `CORE-C-002`: chat request/response model independent from transport format
- `OPENAI-C-001`: endpoint client accepts base URL/model/key via config
- `TUI-C-001`: renderer consumes core message events only
- `TRAY-C-001`: tray adapters implement common action contract

## 4. Extensibility Rules
- new features must land in leaf crates first
- `oberon-daemon` composes crates, it should not own domain logic
- platform-specific code stays in adapter modules

## 5. Security Boundaries
- secrets are sourced from env/config providers, never hardcoded
- logs must redact API keys and auth headers
- network failures must be sanitized before UI rendering
