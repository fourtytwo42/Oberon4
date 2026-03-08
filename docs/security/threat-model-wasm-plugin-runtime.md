# Threat Model: Oberon4 WASM/WASI Plugin Runtime

Version: 0.1
Owner: Oberon4
Status: draft

## 1. Scope
This document covers the main threat areas for the Oberon4 WASM/WASI plugin runtime, including:
- ordinary WASM plugins
- brokered privileged services
- shared workspaces
- artifacts
- secret-backed operations
- provider-facing tool use

## 2. Protected Assets
- host filesystem outside granted workspaces
- host secret stores and signing backends
- API credentials, OAuth tokens, SSH keys, wallet keys
- user workspaces and repository state
- artifact store contents
- audit logs and approval records
- provider and model interaction surfaces

## 3. Primary Threats

### 3.1 Prompt Injection Through Tool Results
Risk:
- malicious file contents, web content, repo contents, or plugin output attempt to steer the agent into unsafe tool use

Controls:
- treat all tool output as untrusted
- keep approval and policy decisions host-native
- require search/describe/plan discipline for risky commands
- keep structured side-effect metadata authoritative over tool prose

### 3.2 Secret Exfiltration
Risk:
- plugin, provider output, logs, or agent-visible errors reveal tokens, keys, or secret-bearing payloads

Controls:
- host-custodied secret model
- no raw secret reads for agents or ordinary plugins
- brokered secret-backed operations only
- redaction on logs, traces, stdout, stderr, tool results, and error envelopes

### 3.3 Workspace Escape
Risk:
- plugin escapes a granted workspace through absolute paths, traversal, symlinks, or platform-native path tricks

Controls:
- logical workspace handles only
- reject absolute and traversal paths
- prevent symlink or reparse-point escape
- no raw writable host mounts for ordinary plugins

### 3.4 Malicious or Compromised Plugin Package
Risk:
- plugin intentionally abuses granted capabilities or attempts contract evasion

Controls:
- provenance and conformance validation
- deny-by-default capabilities
- per-invocation isolation
- resource budgets
- allowlisted or validated plugin activation in stricter modes

### 3.5 Confused Deputy Through Brokered Services
Risk:
- plugin tricks the host into using credentials or capabilities more broadly than intended

Controls:
- narrow service and secret-use handles
- approval and policy checks at the host boundary
- secret-class policy matrix
- explicit allowed operations per handle

### 3.6 SSRF and Network Abuse
Risk:
- plugin or brokered operation reaches unintended endpoints, especially local-network or metadata endpoints

Controls:
- raw network denied by default
- brokered network or service operations only
- explicit allowlists and quotas for future network capabilities

### 3.7 Artifact Poisoning or Oversharing
Risk:
- oversized or malicious artifacts bloat prompts, hide exfiltration, or confuse downstream tools

Controls:
- artifact size limits
- retention policy
- structured artifact metadata
- agent guidance to inspect artifacts through host tools rather than trusting inline summaries alone

## 4. Residual Risk Areas
- malicious but signed plugins
- user-approved dangerous actions
- provider-side prompt or output ambiguity
- weaker security in encrypted-local fallback secret storage compared with hardware-backed or OS-protected backends

## 5. Security Focus Areas For Implementation
- secret redaction correctness
- workspace containment correctness
- approval path integrity
- broker handle scoping
- resource limit enforcement
- cross-platform secret adapter behavior
