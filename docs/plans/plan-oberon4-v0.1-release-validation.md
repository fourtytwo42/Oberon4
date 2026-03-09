# Plan: Oberon4 v0.1 Release Validation

Version: 0.1
Status: active
Owner: Oberon4
WorkflowType: feature-development
WorkflowMode: incremental-gates
TaskState: planned
Stack: rust
Risk: high-risk

## 1. Goal
Validate Oberon4 v0.1 as a release candidate after the completed `P*` and `WP*` build sequences, then produce a human-readable release decision with explicit evidence, residual risks, and rollback posture.

## 2. Canonical Inputs
- `AGENTS.md`
- `docs/plans/plan-oberon4-v0.1-build.md`
- `docs/plans/plan-wasm-plugin-runtime-implementation.md`
- `docs/runbooks/oberon4-release-readiness.md`
- `docs/runbooks/oberon4-human-test-checklist.md`
- `docs/security/threat-model-wasm-plugin-runtime.md`
- `docs/prd/PRD-oberon4.md`
- `docs/prd/PRD-wasm-tool-runtime.md`
- `docs/prd/PRD-versioned-workspace-git.md`

## 3. Scope
### In Scope
- full release-candidate verification against completed build and runtime plans
- operator validation of daemon, CLI, TUI, OAuth, provider, and tool-runtime paths
- production-mode validation for the plugin runtime
- release evidence capture, residual-risk review, and go/no-go recommendation

### Out of Scope
- new feature development
- PRD expansion
- post-v0.1 implementation work
- plugin marketplace or distribution UX

## 4. Execution Rules
- no new behavior changes unless a stop-ship issue is found
- if a stop-ship defect is discovered, open a new change artifact and return to implementation mode
- release validation evidence must be reproducible and linked to specific commands, runs, or artifacts
- keep `artifacts/` as operational output only; do not stage it for release commits

## 5. Release Validation Phases

### Phase RV0: Evidence Freeze
- [ ] `RV0.1` Confirm `EG-P0` through `EG-P8` and `EG-WP0` through `EG-WP7` evidence is current and reproducible.
- [ ] `RV0.2` Re-run the current release and security gates from the active release candidate state.
- [ ] `RV0.3` Confirm the canonical build and runtime plans are marked complete and point to this release-validation plan.

Verification gate `EG-RV0`:
- `docs/runbooks/oberon4-release-readiness.md` required-gate section can be checked from current evidence
- no stale or contradictory plan status remains

### Phase RV1: Automated Release Validation
- [ ] `RV1.1` Run the strict release gate, security gate, repair loop, PTY smoke, and local live smoke paths as applicable to the release environment.
- [ ] `RV1.2` Validate daemon, CLI, TUI, provider, OAuth, workspace/git/github, and provider-tool-loop behavior using the current release candidate.
- [ ] `RV1.3` Confirm production-mode plugin admission, allowlist enforcement, conformance gating, and built-in package availability.

Verification gate `EG-RV1`:
- all automated release-validation commands in `docs/runbooks/oberon4-release-readiness.md` pass or are explicitly marked not-applicable with reason
- no stop-ship defect remains open

### Phase RV2: Human Acceptance
- [ ] `RV2.1` Execute `docs/task-briefs/task-brief-oberon4-v0.1-human-acceptance.md`.
- [ ] `RV2.2` Complete the applicable sections of `docs/runbooks/oberon4-human-test-checklist.md`.
- [ ] `RV2.3` Record any human-observed defects, surprises, or usability risks before release decision.

Verification gate `EG-RV2`:
- human acceptance notes exist
- manual smoke is complete or each skipped item is justified

### Phase RV3: Release Decision
- [ ] `RV3.1` Produce a release decision artifact with:
  - go/no-go
  - residual risks
  - rollback triggers
  - operator notes
- [ ] `RV3.2` Confirm no open stop-ship defects remain for the release candidate.
- [ ] `RV3.3` Freeze the v0.1 release evidence set for handoff.

Verification gate `EG-RV3`:
- release decision is documented with evidence references
- rollback triggers and mitigation path are explicit

## 6. Verification Commands
- `bash scripts/install-git-hooks.sh`
- `bash scripts/check-policy-stack.sh`
- `bash scripts/policy-drift-audit.sh`
- `bash scripts/repair-loop.sh`
- `GATE_MODE=strict bash scripts/oberon4-gate.sh`
- `bash scripts/tui-pty-smoke.sh`
- `bash scripts/oberon4-security-gate.sh`
- `bash scripts/live-local-smoke.sh`
- `cargo run -p oberon4-cli -- doctor --json`

## 7. Exit Criteria
- release-readiness checklist is complete
- human-acceptance task brief is complete
- release decision artifact exists
- any remaining work is moved into post-v0.1 backlog planning rather than left implicit
