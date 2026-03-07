# AGENTS.md

## Purpose
Repo-wide policy for human and AI-assisted work. Optimize for safety, correctness, traceability, and reviewability.

## 0) Priority Ladder (Read First)
When rules conflict, apply in this order:
1. Security and safety
2. Correctness and contract compatibility
3. Verification evidence and stop-ship gates
4. Rollback/release safety
5. Maintainability/performance
6. Process/documentation hygiene

## 0.1) Fresh Session Execution Order
For fresh/compressed context, read first:
1. `## 1) Canonical Authority`
2. `## 15) Constraint Priority Order`
3. `## 7) Security-First Rules`
4. `## 8) Contract and Schema Discipline`
5. `## 6) Verification Gates`
6. `## 11) Stop-Ship Triggers`
7. `## 34) Local CI Gate Requirement`
Then read planning/orchestration/memory sections as needed.

## 0.2) Priority Core Rules (Compressed)
Use this as the fast-path policy for orchestration and execution:
1. Canonical-first: no speculative behavior; keep code/contracts/docs synchronized.
2. Plan before build for non-trivial work; define acceptance IDs (`FR-*`, `AC-*`) and boundaries.
3. No-New-Facts: pause and update artifacts if new requirements appear.
4. Security/correctness before speed; stop on contract mismatch or missing evidence.
5. Verify before done: run required checks and attach command/output evidence.
6. High-risk work requires rollback strategy, checkpoint artifact, and second-review metadata.
7. Multi-agent work is orchestrator-controlled, bounded depth, and fail-closed on delegation violations.
8. Medium/high-risk commits must include self-evaluation and policy delta metadata.
9. Stack isolation is mandatory; only active stack rules apply (see `docs/policy/stack-map.env`).
10. Policy improvements are mirrored to `bos-policy-kit` in the same working session.

## 0.3) Consolidated Rule Map
Use this map to reduce context load while preserving full detail:
- Planning and scope: `## 2`, `## 3`, `## 3.1`, `## 3.2`, `## 16`, `## 16.1`
- Build discipline and evidence: `## 3.3`, `## 3.4`, `## 4`, `## 5`, `## 6`
- Security/contracts/dependencies: `## 7`, `## 8`, `## 8.1`, `## 9`, `## 9.1`, `## 9.2`, `## 30`, `## 30.1`
- Review/release safety: `## 11`, `## 12`, `## 12.1`, `## 12.2`, `## 12.3`, `## 13`, `## 25.1`, `## 26`
- Orchestration/task reliability: `## 23`, `## 23.1`, `## 23.2`, `## 23.3`
- Memory/recovery/ops: `## 17`, `## 17.1`, `## 17.2`, `## 17.3`, `## 24`, `## 24.1`, `## 24.2`, `## 27`
- Continuous improvement and governance: `## 14`, `## 14.1`, `## 14.2`, `## 22`, `## 31`, `## 32`, `## 33`, `## 34`

## 1) Canonical Authority
1. Canonical requirement docs (for example `docs/SOURCE_OF_TRUTH.md`) are authoritative.
2. Do not implement speculative behavior.
3. If behavior changes, update code + canonical docs in the same change.

## 2) Workflow Baseline
1. Use the lightest safe workflow.
2. `tiny`: minimal diff + targeted verification.
3. `medium`: plan, then incremental implementation.
4. `high-risk`: staged plan, explicit gates, rollback path, approval points.

## 3) Task Input Contract (Mandatory for Non-Trivial Work)
Non-trivial work must define:
- Goal
- In scope / out of scope
- Hard (`must`) and soft (`should`) constraints
- Testable acceptance criteria
- Expected touched files or allowed boundaries
Reject ambiguous tasks without verifiable acceptance criteria.

## 3.1) Project Planning Standard (PRD-Style)
Medium/high-risk project work uses stable IDs:
- `FR-###` functional
- `NFR-###` non-functional
- `AC-###` acceptance
Prefer measurable criteria (Given/When/Then where possible). Implementation must map to IDs.

## 3.2) Change Planning Standard (Delta-First)
Behavior changes require a change artifact with:
- Current state + evidence
- Proposed delta
- Explicit non-goals
- Compatibility/contract impact
- Migration plan (if needed)
- Rollback triggers + steps
- Verification plan
No behavior-impacting implementation without this artifact.

## 3.3) No-New-Facts Build Rule
Implementation mode cannot invent new requirements.
If new requirements are discovered:
1. Stop implementation for that part.
2. Update approved planning artifact.
3. Resume after approval.

## 3.4) Research Intake Before Build
For `Risk: medium|high`, capture research inputs before implementation:
- `ResearchInputs`: canonical docs/files/commands consulted before planning
- `EvidenceScope`: what evidence was used, and what was intentionally excluded
Rules:
1. Research intake is required before build mode.
2. If a requirement is unsupported by listed inputs, treat it as unapproved and stop.
3. Keep inputs concise and reproducible (paths, commands, IDs).

## 4) Output and Handoff Contracts
Implementation output must include:
- Plan summary
- Assumptions
- Files touched
- Diff/patch
- Verification commands + outcomes
- Requirement checklist (`met` / `not met`)
Handoffs must include:
- Done, remaining, risks/blockers, evidence links, exact next step
Use fixed structure; avoid narrative-only handoffs.

## 5) Pre-Flight and In-Flight Discipline
Before edits: state objective, acceptance checks, touched files.
During edits:
- Keep diffs small/reviewable
- No opportunistic refactors unless requested or required
- If scope drifts, stop and split follow-up work

## 5.1) No Hidden Magic
Automation/hook behavior must be documented.
If changing hooks/validators/bootstrap/install/config:
- Update user-facing docs in the same change
- Document trigger, enforced checks, safe override path
- Do not ship implicit behavior changes

## 6) Verification Gates
Before claiming done:
- Run relevant tests
- Run lint/type checks when configured
- Validate acceptance criteria
- Avoid unrelated churn
Refactors require characterization evidence first.

## 6.1) Mandatory Test Scope Contract
For `feature|bugfix|refactor|security`, commit metadata and evidence must include:
- `Stack:` active runtime/tooling stack
- `TestScope:` executed categories
Allowed `TestScope`: `unit`, `integration`, `contract`, `e2e`, `property`, `mutation`, `security`, `migration`, `smoke`.
Rules:
- At least one category for all code-impacting changes
- `Risk: medium|high` should include multiple categories
- Bugfixes require `Before` + `After` + one `IndependentCheck`

## 6.2) Stack-Specific Test Baseline
Use stack-native commands (no placeholders):
- `node`: lint/type + tests (+e2e/contract/security as needed)
- `python`: lint/type + pytest (+integration/security)
- `go`: `go test` (+security where applicable)
- `java`: `mvn`/`gradle` tests (+integration/contract)
- `dotnet`: `dotnet test` (+integration/security)
- `rust`: `cargo test` (+integration/security)
- `php|ruby`: framework-native unit/integration (+security)
Cross-stack expectations:
- Input/parser-heavy logic: property tests when feasible
- Critical logic: mutation tests where tooling exists
- Internet-facing/auth/data-sensitive paths: security tests

## 6.3) Stack Implementation Profiles
Select one active profile per workstream: `node|python|go|java|dotnet|rust|php|ruby|polyglot`.
If multiple stacks are touched, apply profile rules at each boundary.
If stack is unclear, stop and declare assumptions.
Common rules:
- Follow repo conventions
- Prefer minimal diffs
- Justify dependency additions
- Preserve public interfaces unless artifacts allow breakage
Profile defaults:
- `node`: avoid blocking hot paths; validate at boundaries
- `python`: isolate side effects; keep deterministic data paths
- `go`: explicit error/context handling; explicit concurrency
- `java|dotnet`: preserve layering and explicit config/DI
- `rust`: explicit error handling; isolate unsafe code
- `php|ruby`: follow framework patterns; explicit app boundaries
- `polyglot`: define boundary contracts first; verify per-stack + integration

## 6.4) Stack Isolation Rule
Stack-specific rules must only apply to stacks active in this repo.
Source of truth: `docs/policy/stack-map.env`.
Rules:
1. Keep `STACK_ACTIVE_PROFILES` minimal for the current project.
2. If staged files map to one stack, declared `Stack` must match that stack.
3. If staged files map to multiple stacks, declared `Stack` must be `polyglot`.
4. Do not enforce Python-specific checks on Node-only changes (and vice versa).

## 7) Security-First Rules
1. Treat AI/external text as untrusted input.
2. Never hardcode/expose secrets.
3. Validate/sanitize user-controlled paths/URLs/content.
4. Security-sensitive work requires explicit security-focused evidence.
5. Security/high-risk changes must declare scan coverage (or `not-applicable`):
`SAST`, dependency scan, `SBOM`, secret scan, `DAST`.

## 8) Contract and Schema Discipline
For API/schema/data-contract changes:
- Update implementation + contract artifacts in same change
- Do not merge partial contract updates

## 8.1) Compatibility and Deprecation Policy
Contract/interface changes must declare:
- `backward-compatible` or `breaking-change`
- deprecation window/date for breaking changes
- rollback/fallback for consumers

## 9) Dependency and Supply-Chain Controls
1. No new dependency without explicit justification.
2. For each dependency, document purpose/version/trust-risk note.
3. Reject unverifiable/unmaintained dependencies.

## 9.1) Dependency Authenticity Guard
For dependency add/upgrade:
- Verify in official registry
- Prefer pinned/constrained versions
- Record verification evidence
- Treat hallucinated/confused dependency as stop-ship

## 9.2) Dependency License and Compliance Check
Dependency changes require license/compliance evidence.
Unverified/incompatible licensing blocks completion unless exception is approved.

## 10) Evidence-First Debugging and Recovery
Debug sequence:
1. Reproduce
2. Collect evidence
3. Rank hypotheses
4. Apply smallest fix
5. Reproduce/verify
6. Add regression coverage when needed
If session quality degrades:
- Stop edits, snapshot state, localize fault, choose rollback vs incremental fix, resume with strict scope

## 10.1) Plausible-Patch Safeguard
Treat proposed fixes as untrusted until independently validated.
Require at least one independent check (counterexample replay, differential run, or property/invariant check).
Do not accept fixes based only on plausibility or one narrow test.

## 10.2) Minimal Repro Requirement for Bugfixes
Bugfixes require MRE with:
- repro command/steps
- failing evidence
- post-fix evidence on same path
No speculative bugfixing without reproducible evidence.

## 11) Stop-Ship Triggers
Stop and resolve on:
- Scope drift beyond objective
- Contract mismatch between code/docs
- Unverifiable dependency addition
- Security-sensitive change without evidence
- Repeated speculative fixes without reproducible failure

## 12) Review and DoD Standard
Done requires:
- Relevant checks green
- Security/dependency review for scope
- AI-assisted review
- Human review
- Summary with risks + rollback

## 12.1) Layered Review Passes
Significant changes require:
- Pass 1: logic/correctness/error/security
- Pass 2: maintainability/readability/style
Do not merge on pass-2 quality if pass-1 risks remain.

## 12.2) Review Packet Requirement (High Risk)
High-risk changes must include review packet:
- assumptions
- threat model
- test plan
- rollback plan
AI review is supplemental; human accountability is required.

## 12.3) Reviewer Independence Rule
AI review cannot be sole approval for security/high-risk changes.
Require:
- human signoff
- tooling-backed security checks
- second-opinion review (human or independent model), with human final decision

## 13) High-Risk Review Mode
For auth/security/payments/migrations/CI/CD/infrastructure:
- second-opinion review
- explicit rollback plan
- concrete verification evidence before merge

## 14) Continuous Improvement Loop
If a review issue repeats 3+ times, add/refine a rule, template, or automated gate.

## 14.1) Policy Kit Mirror Rule
This repo is the live policy authoring surface. Policy stack improvements must be mirrored to standalone `bos-policy-kit`.
When changing policy stack files (rules/templates/validators/hooks/tests/payload tooling):
1. Sync to policy kit: `bash scripts/sync-policy-kit.sh` (or `--push`).
2. Keep mirror updates in the same working session.
3. If auto-sync is enabled (`HOOKS_AUTO_SYNC_POLICY_KIT_ON_PUSH=1`), pre-push performs the sync for matching policy changes.

## 14.2) Mandatory Self-Evaluation Before Final Push
Before final commit/push, perform a self-evaluation on process quality and rule fit.
For `Risk: medium|high`, commit metadata must include:
- `SelfEval: pass|rule-update-needed`
- `PolicyDelta: none|updated:<paths>`
Rules:
1. If `SelfEval: rule-update-needed`, update affected policy artifacts in the same change.
2. Policy updates must be explicit and reviewable (rules/templates/validators/docs).
3. Do not defer required rule corrections to later sessions.

## 15) Constraint Priority Order
When constraints conflict:
1. Security
2. Correctness
3. Contract compatibility
4. Performance
5. Style/format
If lower priority conflicts with higher, satisfy higher and document tradeoff.

## 16) Mandatory Difficulty Gate
Classify each task:
- `tiny`: single-file, low risk
- `medium`: multi-file or moderate risk
- `high-risk`: security/auth/payments/data/migration/CI/infrastructure or broad architecture impact
Workflow must match classification.

## 16.1) Type-to-Artifact Matrix
- `feature`: plan (`docs/plans/plan-*.md` or `Plan:`) + task brief (`docs/task-briefs/*.md` or `TaskBrief:`)
- `bugfix`: `MRE`, `Before`, `After`, `IndependentCheck`, debug brief/checkpoint
- `refactor`: `Characterization` + behavior-preservation evidence
Missing required artifact => stop and add artifact first.

## 17) Context Budget Rule
Non-trivial tasks must create a compact context pack:
- goal/non-goals
- constraints
- relevant contracts/interfaces
- relevant file map
Do not include unrelated files or long dumps.

## 17.1) Context Saturation and Reset Threshold
Defaults:
- Target: <= 8,000 estimated tokens
- Warning: >= 70% (>= 5,600)
- Hard reset: >= 100% (>= 8,000)
At warning:
- update handoff summary
- trim to active files/contracts
At hard threshold:
- stop adding context
- produce structured handoff
- restart fresh session with minimal artifacts

## 17.2) Phase Checkpoint Cadence
Long tasks require checkpoints at phase boundaries (`design -> implement -> verify`).
High-risk/large-change work needs at least one checkpoint per phase.
Checkpoint artifact must include:
- phase
- completed scope
- remaining scope
- risks
- next action

## 17.3) High-Context Bug Strategy
If debugging spans >=10 files:
- switch to context-reduction mode
- slice into 2-6 files per step
- require per-slice verification evidence
- avoid broad speculative edits

## 18) Structured Output Contracts by Task Type
- Planning: numbered tasks, acceptance IDs, risks, verification plan
- Implementation: diff/patch, touched files, verification commands/outcomes
- Debugging: repro evidence, ranked hypotheses, minimal fix, post-fix verification

## 19) Review Focus Modes
Every significant review pass must declare one primary focus:
- `security`
- `performance`
- `maintainability`
Multiple passes allowed; one focus per pass.

## 19.1) Security Checklist Artifact
Security changes require checklist artifact with:
- threat model scope
- abuse cases
- controls verified
- verification evidence
- residual risk note

## 19.2) Performance Verification Standard
Performance-impacting changes require:
- baseline metrics (`p50/p95/p99`, CPU, memory, query count, bundle/frame where relevant)
- explicit budget/target
- profiler evidence
- post-change benchmark evidence
Rules:
- profile before optimize
- no performance claims without profiler/benchmark evidence
- hot-path regressions block completion unless approved exception

## 19.3) Maintainability Verification Standard
Maintainability-focused work requires:
- duplication impact check
- smell/complexity check
- maintainability checklist artifact + rationale
Rules:
- avoid needless abstraction/scope creep
- stay consistent with project conventions
- no maintainability claims without evidence

## 19.4) Observability Verification Standard
Behavior/reliability/performance-sensitive changes require observability evidence:
- logs/traces updated or explicitly unchanged
- metrics coverage on critical paths
- alerting impact reviewed
No reliability/performance claims without observability evidence.

## 19.5) Execution Persona Contract
Persona is an execution aid, not a correctness substitute.

Rules:
- Use persona only when task risk/scope benefits from domain focus.
- Persona selection must map to a concrete checklist and evidence fields.
- Do not use generic roleplay language without required checks.
- If persona and policy conflict, policy wins.

Allowed personas and required checklists:
- `security-engineer`:
  - checklist: threat model, input-boundary validation, authz/authn checks, secret handling, scan coverage
  - evidence: `ThreatModel`, `SecurityScans`, `UntrustedInputBoundary`, `SecurityChecklist`
- `performance-engineer`:
  - checklist: baseline, budget, hotspot profile, post-change benchmark, regression guard
  - evidence: `PerfBaseline`, `PerfBudget`, `PerfProfile`, `PerfBench`
- `maintainability-engineer`:
  - checklist: duplication delta, complexity/smell check, convention alignment, abstraction control
  - evidence: `DuplicationCheck`, `SmellCheck`, `MaintainabilityChecklist`
- `reviewer`:
  - checklist: correctness edge cases, failure modes, test adequacy, rollback clarity, scope discipline
  - evidence: `ReviewPasses`, `ReviewFocus`, requirement status lines (`FR-*`, `AC-*`)

Anti-pattern:
- Persona theater (style-only roleplay with no checklist/evidence) is invalid for non-trivial work.

## 20) Operational Metrics Baseline
Track repo-level trends:
- throughput
- lead time
- defect rate
- review cycle time
Use trend data, not subjective speed impressions.

## 21) Agent Loop Numeric Defaults
Default unless overridden:
- max steps/task: 12
- max retries/tool-action: 2
- fail-closed threshold: 3 consecutive failures on same objective
On threshold breach, stop and escalate to user with evidence.

## 22) Rule-Change Enforcement
When a repeated issue triggers a rule:
- update related template/checklist/validator in same change when feasible
- if not feasible, document why + follow-up action

## 23) Parallel Orchestration Standard
Use parallel agents only for cleanly partitioned, independently verifiable work.
Required roles:
- Planner: scope/boundaries/acceptance IDs
- Implementer: scoped edits
- Verifier: checks/security/regression validation
Rules:
- isolated branches/worktrees per stream
- explicit file boundaries
- no overlapping edits unless coordinated
- each stream must produce handoff artifact
- merge only after full combined verification
Stream handoff must include: scope done, files changed, evidence, risks/blockers, merge order.
Stop parallel mode if contract/architecture conflicts or ambiguous verification appear.

## 23.1) Role-Specific Agent Contracts
All agents inherit this policy; role rules only add constraints.
`orchestrator`:
- define objective/scope/acceptance IDs
- assign file boundaries + stack profile
- prevent overlapping edits
- aggregate evidence and decide merge order
- fail closed on incomplete/conflicting evidence
`implementer`:
- edit assigned scope only
- follow stack profile + gates
- provide concise evidence (commands/output/files)
- escalate on scope drift/contract uncertainty
`verifier`:
- run required checks independently
- validate security/performance/maintainability claims with evidence
- reject placebo/ambiguous verification
`reviewer`:
- focus pass (`security|performance|maintainability`)
- report concrete file-level findings
- do not approve outside assigned focus
Commit metadata for medium/high risk must declare `AgentRole` and `StackProfile`.

## 23.2) Multi-Agent Execution Workflow (Recommended Default)
For non-trivial work, default to a multi-agent flow even when one person operates all roles.

Execution order:
1. Planner pass
2. Implementer pass
3. Verifier pass
4. Optional reviewer pass (`security|performance|maintainability`)
5. Orchestrator merge decision

Planner pass must produce:
- objective
- acceptance IDs (`FR-*`, `AC-*`)
- file boundary assignment per role
- stack profile per stream
- required verification commands and expected outcomes

Implementer pass rules:
- edit only assigned files
- no speculative scope expansion
- produce evidence and artifact updates per this policy

Verifier pass rules:
- run checks independently from implementer assumptions
- validate required gates (tests/security/contract/perf/maintainability as applicable)
- fail closed on missing or ambiguous evidence

Orchestrator merge rules:
- merge only when all assigned checks pass
- resolve conflicts using `## 0) Priority Ladder` and `## 1) Canonical Authority`
- require handoff checkpoint updates for high-risk/large-change work

Tooling note (Codex CLI compatible):
- Agents may be run via separate non-interactive sessions (for example, one session per role) with isolated scope.
- Keep each role in a separate branch/worktree or explicit file allowlist.
- Persist role outputs to canonical artifacts (`docs/context-pack.md`, `docs/handoffs/`, `docs/debug-brief.md`) before merge.
- Do not treat tool output as truth without verifier evidence.
- Use guarded launcher (`scripts/run-agent.sh`) for role/depth/budget enforcement.
- Keep orchestrator lock discipline with timeout/force-unlock controls to avoid concurrent orchestration collisions.
- Persist per-run JSON manifests under `.agent-runtime/<task-id>/runs/` for lineage/auditability.
- Windows parity: use `scripts/run-agent.ps1` with equivalent role/depth/lock controls.

## 23.3) Workflow Selection and Task State Reliability
Choose workflow explicitly before implementation (book-aligned workflow taxonomy):
- `WorkflowType`: `feature-development|maintenance|collaboration`
- `WorkflowMode`: `simple|plan-build|incremental-gates`

Track each task with explicit lifecycle state:
- `TaskState`: `planned|in-progress|blocked|verify|done`
- `NextAction`: required when `TaskState=blocked`

Rules:
1. Medium/high-risk work must declare workflow type + mode + task state in commit metadata.
2. For complex/high-risk changes, use `incremental-gates` and commit checkpoints per phase.
3. State transitions must be monotonic (`planned -> in-progress -> verify -> done`; `blocked` may interrupt and must include `NextAction`).
4. Do not mark `done` without verification evidence and requirement status lines.

Subagent anti-recursion controls (mandatory):
- Only `orchestrator` may spawn subagents.
- `implementer`, `verifier`, and `reviewer` must never spawn additional agents.
- Maximum spawn depth is `1` (`orchestrator -> subagent` only).
- Subagent sessions run fail-closed with delegation disabled.
- If a subagent attempts delegation/spawn, stop execution and escalate to orchestrator.
- Subagents must not invoke direct `codex exec`; they must use the guarded launcher `scripts/run-agent.sh`.
- Delegation/loop violations must produce an incident artifact from `docs/templates/agent-incident.md`.

Required lineage metadata for medium/high-risk commits:
- `AgentDepth: 0|1`
- `Delegation: none|planner,implementer,verifier,reviewer`
- If `AgentRole != orchestrator`, `Delegation` must be `none`.
- If `AgentRole != orchestrator`, include `SpawnedAgents: 0`.
- If `AgentRole == orchestrator` and `Delegation != none`, include:
  - `SpawnBudget: <n>`
  - `MaxSpawnDepth: 1`

## 24) Memory Architecture Standard
Layered memory model:
- Canonical: `AGENTS.md`
- Tool overlays (optional): `CLAUDE.md`, `.cursor/rules`, etc.
- Session artifacts: context pack, handoffs, debug briefs, plans, change requests
Rules:
- Canonical-first; overlays must not conflict
- Keep memory files small/stable
- Persist key decisions in durable docs, not chat only
- If overlay changes behavior, update `AGENTS.md` or document exception
Refresh at task boundaries and before saturation.
If sources conflict, canonical artifacts win; reconcile before coding.

## 24.1) Canonical Artifact Placement Map
Use these paths consistently:
- `docs/context-pack.md`
- `docs/debug-brief.md`
- `docs/plans/`
- `docs/change-requests/`
- `docs/handoffs/`
- `docs/adr/`
- `docs/security/`
- `docs/maintainability/`
- `docs/runbooks/`
- `docs/incidents/`
Rules:
- Keep artifacts in canonical directories unless explicitly justified
- Use predictable names: `<type>-<topic>.md`
- Keep one active artifact per type/workstream; superseded docs must point to replacement

## 24.2) Fresh Context Recovery Order
Load in order:
1. `AGENTS.md`
2. active plan/change artifact
3. `docs/context-pack.md`
4. latest relevant handoff
5. relevant domain checklists/runbooks
This order is stack/system agnostic because it is repo-artifact based.

## 25) Incident and Recovery Policy
For production/high-risk failures:
- mitigate first (rollback/disable path/capacity)
- record mitigation path + trigger conditions
- capture post-incident learning in durable artifact after stabilization

## 25.1) Release Safety and Rollout Policy
For high-risk or user-facing behavior changes:
- declare release strategy: `canary|staged|full|not-applicable`
- define rollback triggers
- define immediate mitigation action
No explicit release safety strategy => invalid high-risk change.

## 26) Rollback Rehearsal Requirement
High-risk changes require rollback rehearsal evidence:
- rehearsal mode (`pass|simulated`)
- steps/commands
- observed outcome
If not applicable, document why with explicit scope limits.

## 27) Escalation Timebox Rule
For debugging/incidents:
- if no measurable progress in 30 minutes, escalate
- allowed escalation: rollback, restart/reset, second reviewer, or narrower MRE
- do not continue speculative retries past timebox without new evidence

## 28) Verification Debt Signals
Medium/high-risk work must include at least one quality signal beyond pass/fail:
- issue density
- churn spike
- rework indicator
- mutation trend/delta
Use signals to tune gate strictness over time.

## 29) Flaky Test Quarantine Policy
Flaky tests cannot be silent pass criteria.
Quarantine requires:
- owner
- linked issue
- expiry/fix-by date
- temporary mitigation note
Quarantine must be time-bound and removed after remediation.

## 30) Threat Modeling and Untrusted Input Boundary
Security/high-risk changes must declare:
- threat model framing (`STRIDE|OWASP|both`)
- untrusted-input boundary (validated/sanitized/blocked)
Untrusted text must never directly drive command execution without allowlisted controls.

## 30.1) Privacy and Data Handling Policy
Security/compliance/data-sensitive changes must declare:
- PII handling mode
- retention impact
- log redaction approach
No explicit privacy metadata => invalid.

## 31) Policy Exception Protocol
Exceptions only when explicitly declared with:
- owner
- reason
- linked issue
- expiry date
- follow-up action
Expired exceptions are invalid until renewed/removed.

## 32) Exception and Quarantine Expiry Discipline
Exception/flaky records must be time-bound.
Rules:
- ISO date `YYYY-MM-DD`
- expired records are policy violations
- renewal must update reason, owner, follow-up

## 33) Policy Drift and Exception Debt Audits
Run local audits for:
- `AGENTS.md` <-> templates <-> validators consistency
- exception/flaky expiry horizon
Audit failures must surface before push unless explicitly configured as warning.

## 34) Local CI Gate Requirement
Before push, run local smoke/test/build gates.
Rules:
- required scopes must pass
- command set is repo-configurable via hook env
- required gate failures must block push

## 35) Multi-Tier Agent Governance (CEO/Department/Verifier/Reconciliator)
This repository supports hierarchical agent orchestration for parallel execution.

Role contracts:
1. CEO orchestrator:
  - sets global priorities and wave goals
  - approves or blocks wave promotion
2. Department orchestrator:
  - owns domain queue and dispatches bounded jobs
  - must issue explicit job packets (scope/tests/end gate/context files)
3. Worker:
  - executes assigned job only
  - must stop at job end gate
4. Verifier:
  - independent validation of scope/tests/contracts
  - emits pass/fail with reason codes
5. Reconciliator:
  - merges verified work into integration branches
  - fail-closed on unresolved conflicts or failed gates

Hard rules:
1. No worker starts without a dispatch packet.
2. No merge to integration branch without verifier pass.
3. Reconciliator cannot override verifier fail without explicit CEO decision.
4. Department orchestrator cannot promote incomplete dependency chains.
5. All role decisions must be artifacted in planning/handoff docs.
6. Every dispatched job must include explicit `endGateId` (`EG-Axx` or `EG-Mxx`).
7. Every department orchestrator must satisfy and record a department end gate (`DH-EG-*`) before stopping.
8. Verifier must reject jobs missing end gate ids or end gate evidence.
9. Dispatch packets must be micro-scoped (single bounded objective with clear acceptance and stop condition); never assign large multi-component builds to one worker.
10. Department orchestrators must keep active concern triage and resolve or block every open concern before phase promotion.

Reference execution docs:
- `docs/plans/plan-agent-work-packages.md`
- `docs/plans/plan-agent-micro-jobs.md`
- `docs/plans/plan-agent-kickoff-prompts.md`

## 35.1) Scripted Control-Plane Methodology (Locked Default)
Primary orchestration path is script-driven state management, not ad-hoc terminal coordination.

Control plane:
- `scripts/agent-control.js` is the source of truth for job state, phase state, concerns, and advancement.
- Workers are launched via guarded runner only: `scripts/run-agent.sh` (or `scripts/run-agent.ps1`).
- Manual edits to `.agent-orchestration/state.json` are prohibited.

Locked phase order per batch:
1. `build`
2. `reconcile`
3. `verify`
4. `fix` (only for failed verify jobs)
5. `advance` to next batch

Required lifecycle command surface:
- `start`, `complete`, `reconcile-done`, `verify`, `fix-done`, `advance`
- concern loop: `raise`, `concerns`, `guide`
- visibility: `monitor`, `ready`

Progress semantics:
- A final-progress tick (`X/total`) occurs only when a micro-job passes all required stages and reaches done for the active workflow.

## 35.2) Parallelism and Token-Budget Discipline
Default concurrency:
- `maxParallelWorkers=5` unless explicitly raised by orchestrator after a stable batch.
- If failure rate rises or concern volume spikes, reduce parallelism before retrying.

Context minimization rules for workers:
- Provide path references + exact lookup commands (`rg`, `sed -n`, `tail`) rather than pasting full files into prompts.
- Keep outputs concise; prefer tail-based error capture with a short failure summary.
- Worker prompts must include only task-local scope, end gate, and required validation commands.

## 35.3) Checkpoint and Git Hygiene for Long Runs
Checkpoint cadence:
- At minimum, checkpoint at each completed batch phase and before provider/account/token switches.
- Use `scripts/checkpoint.sh --phase <id> --summary "<text>"` for consistent handoff artifacts.

Git hygiene:
- Runtime orchestration churn (`.agent-runtime/`, `.agent-orchestration/`, `.tmp/`, `tmp/`) is non-product output and must not be committed.
- Push to remote at safe milestones (phase/batch completion with verifier green), not during unstable in-progress states.
- Agentic builds must run scripted git sync (`scripts/agent-git-sync.sh`) with frequent cadence so commits/pushes happen continuously and rollback points remain current.

## Templates in This Repo
- Task brief: `docs/templates/task-brief.md`
- Plan: `docs/templates/plan.md`
- Change request: `docs/templates/change-request.md`
- Handoff: `docs/templates/handoff.md`
- Debug brief: `docs/templates/debug-brief.md`
- Project brief: `docs/templates/project-brief.md`
- Security checklist: `docs/templates/security-checklist.md`
- Performance checklist: `docs/templates/performance-checklist.md`
- Maintainability checklist: `docs/templates/maintainability-checklist.md`
- Runbook template: `docs/templates/runbook.md`
- PR template: `.github/pull_request_template.md`
- PR gate script: `scripts/validate-pr-body.sh`
