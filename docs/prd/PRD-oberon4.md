# PRD: Oberon4 Daemon (Rust-first)

Version: 0.1
Owner: Oberon4
Status: draft

## 1. Problem Statement
Build a local-first, modular agent host with:
- daemon-style runtime
- tray/notification-bar integration path
- terminal TUI chat surface
- OpenAI-compatible custom endpoint support
- strict test discipline for each subsystem

## 2. Goals
- `FR-001`: daemonized runtime with clean startup/shutdown control.
- `FR-002`: TUI chat experience for interactive agent conversations.
- `FR-003`: configurable OpenAI-compatible endpoint and API key handling.
- `FR-004`: modular crate boundaries so subsystems can evolve independently.
- `FR-005`: tray/notification-bar integration contract with platform adapters.
- `FR-006`: deterministic test coverage for every subsystem before merge.
- `FR-007`: provider OAuth account connect flows for OpenAI and Anthropic implemented in Rust, aligned with llmhotspot behavior.
- `FR-008`: CLI command surface for daemon control, configuration, diagnostics, and one-shot chat.
- `FR-009`: TUI command surface for interactive session control and operator actions.

## 3. Non-Goals (v0.1)
- full plugin marketplace
- multi-tenant remote hosting
- mobile clients
- autonomous background task orchestration

## 4. Functional Requirements
- `FR-001` Daemon runtime
  - `AC-001.1`: app supports foreground run mode and daemon lifecycle abstraction.
  - `AC-001.2`: graceful shutdown drains active chat loop without data corruption.
  - `AC-001.3`: runtime exposes health/state endpoint or equivalent status contract.

- `FR-002` TUI chat
  - `AC-002.1`: user can send prompt and receive assistant response in terminal UI.
  - `AC-002.2`: TUI supports minimum commands (`/help`, `/quit`, `/config`).
  - `AC-002.3`: TUI failures show sanitized errors with correlation/request IDs.

- `FR-003` OpenAI-compatible endpoint
  - `AC-003.1`: endpoint base URL is configurable at runtime.
  - `AC-003.2`: model and API key are configurable without code changes.
  - `AC-003.3`: request/response translation supports `/v1/chat/completions` contract.
  - `AC-003.4`: timeout/retry policy is explicit and test-covered.

- `FR-004` Modularity
  - `AC-004.1`: separate crates for core domain, endpoint client, TUI, tray, daemon app.
  - `AC-004.2`: each crate has unit tests for core logic paths.
  - `AC-004.3`: cross-crate contracts are versioned and documented.

- `FR-005` Tray path
  - `AC-005.1`: tray runtime contract exists with platform adapters.
  - `AC-005.2`: unsupported platforms fail cleanly with explicit status.
  - `AC-005.3`: tray action set includes open UI, show status, restart, quit.

- `FR-006` Testing discipline
  - `AC-006.1`: every merged subsystem change includes tests.
  - `AC-006.2`: CI test command and local test command are documented.
  - `AC-006.3`: regressions require an added failing-before/passing-after test.

- `FR-007` Provider OAuth parity (Rust transplant parity)
  - `AC-007.1`: supports browser-start OAuth with PKCE and state validation for OpenAI accounts.
  - `AC-007.2`: supports browser-start OAuth with PKCE and state validation for Anthropic accounts.
  - `AC-007.3`: supports local callback handling with sanitized `return_to` target policy for all OAuth providers.
  - `AC-007.4`: supports polling/status path for browser flow completion for all OAuth providers.
  - `AC-007.5`: stores refresh/access credentials through secure storage path (keyring/secret store), never logs raw secrets.
  - `AC-007.6`: exposes account lifecycle operations (`connect`, `list`, `activate`, `disable`, `disconnect`) through daemon CLI/API surface with provider selection.
  - `AC-007.7`: provider abstraction keeps OpenAI and Anthropic OAuth implementations modular so additional providers can be added without changing TUI core behavior.
  - `AC-007.8`: OAuth flow behavior is backed by Rust integration and contract tests for OpenAI and Anthropic before merge.

- `FR-008` CLI command surface
  - `AC-008.1`: CLI exposes `daemon start`, `daemon stop`, and `daemon status`.
  - `AC-008.2`: CLI exposes `tui` to launch the interactive terminal UI in inline scrollback-native mode.
  - `AC-008.2a`: CLI exposes `tui --fullscreen` for the structured full-screen workspace UI and keeps `tui --no-alt-screen` as a backward-compatible alias for inline mode.
  - `AC-008.3`: CLI exposes `config show` and `config set <key> <value>` for endpoint/model/runtime configuration.
  - `AC-008.4`: CLI exposes `endpoint test` to validate the configured OpenAI-compatible endpoint.
  - `AC-008.5`: CLI exposes one-shot `chat --message <text>` execution for scripting and smoke checks.
- `AC-008.6`: CLI exposes `logs tail`, `doctor`, and `doctor --json` for diagnostics.
  - `AC-008.7`: all CLI commands return deterministic exit codes and sanitized errors.

- `FR-009` TUI command surface
  - `AC-009.1`: TUI supports `/help`, `/quit`, `/status`, `/sessions`, `/settings`, `/accounts`, `/diagnostics`, and `/more`.
  - `AC-009.2`: TUI supports `/config` to show effective runtime configuration.
  - `AC-009.3`: TUI supports `/model <name>` to switch active model for the session.
  - `AC-009.4`: TUI supports `/endpoint <url>` to override endpoint for the session.
  - `AC-009.5`: TUI supports `/system <text>` to set or replace the session system prompt.
  - `AC-009.6`: TUI supports `/clear` to clear the visible transcript buffer.
  - `AC-009.7`: TUI supports `/save <name>`, `/load <name>`, and `/fork <name>` for session persistence and branching.
  - `AC-009.8`: invalid or unsupported TUI commands fail with explicit help text and no state corruption.
  - `AC-009.9`: TUI uses a chat-first layout with transcript + composer as the default surface, a session browser that opens from `/sessions`, and full-pane menu screens for settings, status, help, accounts, diagnostics, and reserved future tools without requiring daemon contract changes.
  - `AC-009.10`: TUI supports multiline drafting (`Ctrl+J`), slash-command autocomplete (`Tab`), ordered `Esc` back/clear/quit handling, automatic session titling on first exchange, and session selection that returns focus to chat input.

## 5. Non-Functional Requirements
- `NFR-001` Reliability: no panics on malformed endpoint responses.
- `NFR-002` Security: no plaintext secret logging.
- `NFR-003` Observability: structured logs include request IDs.
- `NFR-004` Maintainability: crate-level boundaries enforced by API contracts.
- `NFR-005` Performance: interactive TUI latency target <250ms local render path.
- `NFR-006` Auth security: OAuth callback/state handling must fail closed on mismatch/expiry.
- `NFR-007` Code minimization: feature delivery must prefer reuse of shared daemon/core logic over duplicated surface-specific implementations.
- `NFR-008` Portability: platform-specific code must be isolated behind adapter boundaries so core chat/auth/config logic remains platform-neutral.
- `NFR-009` Testability: provider, storage, and IPC boundaries must be mockable without requiring UI or network execution.

## 6. Risks and Mitigations
- endpoint variance risk -> adapter tests + strict schema validation
- tray cross-platform risk -> adapter trait + progressive platform rollout
- coupling risk -> crate boundaries + contract tests

## 6.1 Build Strategy
- daemon-first architecture:
  - daemon is the source of truth for config, accounts, sessions, provider routing, and chat execution
  - CLI, TUI, and tray are thin clients over daemon APIs and must not duplicate business logic
- reuse-first implementation:
  - transplant proven Rust OAuth/account-flow components from `llmhotspot` where compatible instead of redesigning them
  - reuse shared domain types, request envelopes, error contracts, and test fixtures across all surfaces
- modular workspace layout:
  - `oberon4-core`: domain models, config schema, error types, service traits
  - `oberon4-provider-openai`: OpenAI-compatible endpoint client and request translation
  - `oberon4-auth`: provider OAuth flows and secure token storage abstractions
  - `oberon4-daemon`: orchestration layer and local IPC server
  - `oberon4-cli`: command parsing and daemon client only
  - `oberon4-tui`: terminal rendering/input and daemon client only
  - `oberon4-tray`: platform adapter shell over daemon client only
- abstraction rules:
  - provider differences must be isolated behind traits such as `ChatProvider`, `OAuthProvider`, `SecretStore`, and `ConfigStore`
  - platform differences must be isolated behind tray and IPC adapter traits
  - network retry/timeout logic belongs in provider modules, not in TUI or CLI layers
- minimal-code delivery rules:
  - if logic is needed by both CLI and TUI, it must live in shared core or daemon code
  - app surfaces may format output differently, but must call the same service operations
  - prefer a single local IPC contract and shared command/event model over separate surface-specific APIs

## 6.2 Robustness and Portability Strategy
- use typed request/response models for provider and IPC boundaries
- validate all user-controlled inputs at daemon/service boundaries
- separate config storage from secret storage
- keep platform-specific code out of core crates
- fail cleanly on unsupported tray/platform capabilities
- prefer Linux-first delivery, but preserve adapter boundaries for future macOS/Windows support

## 6.3 Test and Reuse Strategy
- most behavioral tests must target `oberon4-core`, `oberon4-auth`, and `oberon4-daemon`
- CLI tests should focus on argument parsing, exit codes, and daemon-client integration
- TUI tests should focus on command parsing and state transitions rather than terminal snapshot volume
- provider integrations must support deterministic mock/fake implementations for unit and contract tests
- end-to-end tests should cover:
  - configure endpoint
  - connect OAuth account
  - send chat request
  - save and load session
- shared fixtures, fake providers, and reusable integration harnesses must be preferred over per-surface duplicated test scaffolding

## 7. Release Gate (v0.1)
- all `AC-001.*` through `AC-009.*` are `met`
- required test suite green
- docs + implementation + tests synchronized

## 8. Command Reference (v0.1 target)

### 8.1 CLI Commands
- `oberon4 daemon start`
  - starts the local daemon in foreground mode for v0.1
- `oberon4 daemon stop`
  - requests graceful daemon shutdown through local control channel
- `oberon4 daemon status`
  - prints daemon status, pid when available, uptime, and endpoint health summary
- `oberon4 tui`
  - launches the interactive terminal UI in inline scrollback-native mode and connects to the local daemon
- `oberon4 tui --fullscreen`
  - launches the structured full-screen workspace UI explicitly
- `oberon4 config show`
  - prints effective runtime configuration with secrets redacted
- `oberon4 config set <key> <value>`
  - updates a persisted configuration key
- `oberon4 endpoint test`
  - performs a minimal connectivity/authentication test against the configured OpenAI-compatible endpoint
- `oberon4 chat --message <text>`
  - runs a one-shot non-interactive chat request through the daemon
- `oberon4 logs tail`
  - streams recent daemon logs with correlation/request identifiers when available
- `oberon4 doctor`
  - runs local environment and configuration diagnostics and exits non-zero on failed checks
- `oberon4 doctor --json`
  - returns the same diagnostic result as machine-readable JSON for automation

### 8.2 Account and OAuth CLI Commands
- `oberon4 account connect --provider <openai|anthropic>`
  - starts browser-based OAuth connect flow for the selected provider
- `oberon4 account list`
  - lists known connected accounts and active/inactive state
- `oberon4 account activate --provider <openai|anthropic> --account <id>`
  - marks a specific provider account as the active account
- `oberon4 account disable --provider <openai|anthropic> --account <id>`
  - disables a connected account without deleting local metadata
- `oberon4 account disconnect --provider <openai|anthropic> --account <id>`
  - removes the selected connected account and associated local credentials

### 8.3 TUI Commands
- `/help`
  - lists supported commands and key behaviors
- `/quit`
  - exits the TUI cleanly
- `/status`
  - opens the full-pane status workspace
- `/sessions`
  - opens the chat session browser in the active terminal flow and loads the selected session on `Enter`
- `/settings`
  - opens the settings menu in the active terminal flow
- `/accounts`
  - opens the account-management menu in the active terminal flow
- `/diagnostics`
  - opens the diagnostics menu in the active terminal flow
- `/more`
  - opens the reserved tools menu in the active terminal flow
- `/config`
  - redirects to the settings workspace while preserving redacted config views
- `/model <name>`
  - switches the active model for the current session
- `/endpoint <url>`
  - overrides endpoint base URL for the current session
- `/system <text>`
  - sets or replaces the current session system prompt
- `/clear`
  - clears the visible transcript buffer without deleting persisted session state
- `/save <name>`
  - saves the current session transcript and local session metadata
- `/load <name>`
  - loads a previously saved session transcript and metadata
- `Ctrl+J`
  - inserts a newline in the composer without sending
- `Tab`
  - cycles slash-command autocomplete when the current draft begins with `/`
- `Esc`
  - backs out of the current menu, form, or session browser before returning to chat, then clears draft, then quits on an empty composer

### 8.4 Command Contract Rules
- CLI commands must return deterministic exit codes and sanitized errors.
- TUI commands must fail with explicit help text when arguments are invalid or unsupported.
- Provider-specific account commands must reject unsupported providers.
- Secrets and tokens must never be printed by CLI or TUI output.
