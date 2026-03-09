# Task Brief

## Goal
- Execute human acceptance for the Oberon4 v0.1 release candidate after automated build and runtime plans are complete.

## Requirement IDs
- FR-001: daemon runtime and local workflow are usable in human-operated flows
- FR-002: TUI command and chat flow remain usable under manual operation
- FR-007: OAuth-backed account flows remain usable
- FR-101: host-managed tool discovery and invocation remain understandable and safe
- FR-201: versioned workspace/git/github package is usable through the host-managed tool runtime

## In Scope
- daemon, CLI, and TUI manual smoke
- account and provider spot-checks
- tool-loop and approval-pause behavior
- production-mode plugin-runtime operator checks
- release candidate usability and safety notes

## Out of Scope
- feature redesign
- new implementation work except stop-ship fixes opened separately
- post-v0.1 roadmap prioritization

## Hard Constraints (`must`)
- Use this brief only after automated release-validation preconditions are green.
- Capture any observed defect before proceeding to release decision.
- Do not stage or commit runtime evidence directories such as `artifacts/`, `.live-manual/`, or repair/failure bundles.
- Any approval-required or push-capable test must use a safe test repository or explicitly non-destructive environment.

## Soft Constraints (`should`)
- Use the existing release and human-test runbooks rather than inventing ad hoc steps.
- Prefer daemon-backed flows so the CLI and TUI exercise the same runtime and provider loop shipped to users.
- Record short operator notes for usability friction even if they are not stop-ship issues.

## Acceptance Criteria
- AC-901: Given automated release gates are green, when the manual CLI smoke flow is executed, then daemon/config/chat/doctor flows complete without panic or secret leakage.
- AC-902: Given the TUI release candidate is exercised, when approval-required tool actions are encountered, then the TUI pauses, presents approval state, and resumes the same turn after decision.
- AC-903: Given a safe test workspace/repo is available, when the tool-loop scenario is exercised, then the assistant uses host-native tool discovery/planning/invocation rather than raw filesystem or provider-native shortcuts.
- AC-904: Given production mode is enabled for operator validation, when built-in package flows and ordinary-plugin admission are checked, then built-ins remain usable and non-allowlisted ordinary plugins are blocked with explicit policy diagnostics.
- AC-905: Given manual smoke is complete, when release decision preparation begins, then defects, residual risks, and skipped steps are explicitly recorded.

## Files Allowed To Change
- `docs/runbooks/oberon4-human-test-checklist.md`
- `docs/runbooks/oberon4-release-readiness.md`
- `docs/handoffs/`
- `docs/runbooks/`
- `docs/plans/`
