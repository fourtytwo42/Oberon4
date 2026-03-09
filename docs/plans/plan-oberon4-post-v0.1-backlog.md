# Plan: Oberon4 Post-v0.1 Backlog Framing

Version: 0.1
Status: active
Owner: Oberon4
WorkflowType: maintenance
WorkflowMode: plan-build
TaskState: planned
Stack: rust
Risk: medium

## 1. Goal
Turn release-validation findings and deferred ideas into a bounded post-v0.1 backlog, prioritized by security, correctness, release follow-up, and user-facing value.

## 2. Canonical Inputs
- `AGENTS.md`
- `docs/plans/plan-oberon4-v0.1-release-validation.md`
- `docs/runbooks/oberon4-release-readiness.md`
- `docs/runbooks/oberon4-human-test-checklist.md`
- `docs/prd/PRD-oberon4.md`
- `docs/prd/PRD-wasm-tool-runtime.md`
- `docs/prd/PRD-versioned-workspace-git.md`
- release decision and human-acceptance notes once produced

## 3. Scope
### In Scope
- classify follow-up work discovered during release validation
- separate stop-ship fixes from post-release improvements
- group backlog items by security, reliability, UX, provider/runtime expansion, and packaging/distribution
- identify candidates for the next canonical plan after v0.1 release decision

### Out of Scope
- implementing backlog items
- rewriting the current PRDs without a separate approved change artifact
- new `WP*` execution phases for the completed runtime plan

## 4. Backlog Buckets

### B0: Release Follow-Up
- items found during release validation that are not stop-ship but should land immediately after v0.1

### B1: Runtime Hardening and Operations
- deeper conformance automation
- production-policy ergonomics
- plugin admission UX
- operator observability and diagnostics improvements

### B2: Tool and Package Expansion
- additional built-in packages
- safer ordinary-plugin authoring flow
- richer workspace/git/github capabilities that were deferred from v0.1

### B3: Provider and UX Expansion
- provider compatibility beyond `OpenAI-compatible` and `Anthropic`
- richer approval UX
- session durability for paused approval flows

### B4: Distribution and Governance
- plugin packaging/distribution UX
- release process refinements
- policy automation or documentation debt found during v0.1 release work

## 5. Backlog Planning Rules
- anything that blocks v0.1 stays out of this backlog and returns to the release-validation path as a stop-ship fix
- each backlog item should identify:
  - reason discovered
  - affected requirement or subsystem
  - severity/priority
  - recommended artifact type for follow-up (`plan`, `change-request`, `task-brief`, `runbook`)
- post-v0.1 work should start from a fresh canonical plan, not by extending the completed `P*` or `WP*` plans

## 6. Exit Criteria
- release-validation outputs are triaged into explicit buckets
- stop-ship items are separated from non-blocking backlog items
- the next post-v0.1 planning artifact can be created from a bounded, prioritized backlog rather than chat-only memory
