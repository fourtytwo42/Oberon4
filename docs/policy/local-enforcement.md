## Local Enforcement (No GitHub Actions)

This repo uses local git hooks for policy checks.

### Install hooks

```bash
bash scripts/install-git-hooks.sh
```

PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/install-git-hooks.ps1
```

Windows guarded agent launcher:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/run-agent.ps1 -Role orchestrator -TaskId T123 -Prompt "Create a plan for FR-101."
```

Preflight policy doctor:

```bash
bash scripts/policy-doctor.sh --msg .git/COMMIT_EDITMSG
```

Risk/type suggestion helper:

```bash
bash scripts/risk-classifier.sh --cached
```

Policy kit sync helper (BOS -> standalone `bos-policy-kit`):

```bash
bash scripts/sync-policy-kit.sh --push
```

Notes:
- Sync copies `policy-kit/` into `bos-policy-kit` without mutating BOS by default.
- Use `--build-source` only when you intentionally want to refresh payload from BOS source first.

### What is enforced locally

- `commit-msg` hook:
  - Requires issue linkage: `Issue: #123` or GitHub issue URL
  - Repo slug is resolved from `git remote origin` (or `GITHUB_REPOSITORY` / `HOOKS_REPO_SLUG`)
  - Requires `Type: feature|bugfix|refactor|docs|chore|security`
  - Requires `Risk: low|medium|high`
  - For code-impacting types (`feature|bugfix|refactor|security`), requires:
    - `Stack: node|python|go|java|dotnet|rust|php|ruby|polyglot|other`
    - `TestScope: unit,integration,contract,e2e,property,mutation,security,migration,smoke`
    - For `Risk: medium|high`, at least two `TestScope` categories
    - If stack-map enforcement is enabled:
      - `Stack` must be in active profiles from `docs/policy/stack-map.env` (unless `polyglot|other`)
      - if one stack is detected from staged files, declared `Stack` must match
      - if multiple stacks are detected, declared `Stack` must be `polyglot`
  - When enabled, requires verification commands to match declared stack tools
  - For non-trivial commits, requires `Traceability: FR-###, AC-###`
  - Requires verification evidence with at least:
    - one command line: `$ <command>`
    - one result line: `Output: ...` (or `Observed:` / `Expected:`)
  - Rejects placeholder verification text (`TBD`, `N/A`, `not run`, `skipped`)
  - For `Risk: medium|high`, requires:
    - `Assumptions: ...`
    - `Rollback: ...`
    - `ResearchInputs: ...`
    - `EvidenceScope: in=<...>; out=<...>`
    - `SelfEval: pass|rule-update-needed`
    - `PolicyDelta: none|updated:<paths>`
    - `WorkflowType: feature-development|maintenance|collaboration`
    - `WorkflowMode: simple|plan-build|incremental-gates`
    - `TaskState: planned|in-progress|blocked|verify|done`
    - `NextAction: ...` when `TaskState=blocked`
    - `NoNewFacts: yes`
    - `QualitySignals: ...` including one of `issue_density|churn|rework|mutation_delta`
    - `AgentRole: orchestrator|implementer|verifier|reviewer|solo`
    - `StackProfile: node|python|go|java|dotnet|rust|php|ruby|polyglot|other`
    - `AgentDepth: 0|1`
    - `Delegation: none|planner,implementer,verifier,reviewer`
    - For non-orchestrator roles: `Delegation: none` and `SpawnedAgents: 0`
    - For `AgentRole: implementer|verifier|reviewer`: `Invocation: run-agent.sh`
    - For `AgentRole: implementer|verifier|reviewer`, verification commands must not use direct `codex exec`
    - For orchestrator when delegating: `SpawnBudget: <n>` and `MaxSpawnDepth: 1`
    - `ReviewFocus: security|performance|maintainability`
    - `ReviewPasses: pass1-logic-security,pass2-maintainability`
    - `Observability: logs=updated|unchanged,metrics=updated|unchanged,alerts=updated|unchanged`
    - If `ReviewFocus: performance`:
      - `PerfBaseline: ...`
      - `PerfBudget: ...`
      - `PerfProfile: ...`
      - `PerfBench: ...`
    - If `ReviewFocus: maintainability`:
      - `DuplicationCheck: pass|fail|not-applicable - ...`
      - `SmellCheck: pass|fail|not-applicable - ...`
      - `MaintainabilityChecklist: docs/maintainability/maintainability-checklist-<topic>.md|not-applicable`
    - For security/high-risk data-sensitive changes:
      - `DataHandling: pii=none|redacted|encrypted|not-applicable,retention=<impact>,logs=redacted|not-applicable`
    - at least one FR/AC status line containing `met` or `not met`
    - if `SelfEval: rule-update-needed`, policy artifacts must be updated in same commit
  - For `Type: bugfix`, requires:
    - `MRE: ...`
    - `Before: ...`
    - `After: ...`
    - `IndependentCheck: ...`
    - `DebugBrief: ...`
  - For `Type: feature`, requires:
    - `Plan: docs/plans/...`
    - `TaskBrief: ...`
  - For `Type: refactor`, requires:
    - `Characterization: added|existing`
  - For `Risk: high` or `Override: large-change`, requires:
    - `PhaseCheckpoint: docs/handoffs/handoff-<phase>.md`
    - `ThreatModel: STRIDE|OWASP|both`
    - `RollbackRehearsal: pass|simulated|not-applicable - ...`
    - `MitigationPlan: rollback|disable-path|capacity - ...`
    - `ReviewPacket: assumptions,threat-model,test-plan,rollback-plan`
    - `SecondReview: yes`
    - `SecuritySignoff: human+tooling`
    - `SecurityScans: sast=...,dependency=...,sbom=...,secrets=...,dast=...`
    - `ReleaseStrategy: canary|staged|full|not-applicable - ...`
    - `Runbook: docs/runbooks/<name>.md|not-applicable` (must exist when path is used)
    - Performance evidence fields may also be required by policy:
      - `PerfBaseline`, `PerfBudget`, `PerfProfile`, `PerfBench`
  - For `Type: security`, requires:
    - `UntrustedInputBoundary: validated|not-applicable - ...`
    - `SecuritySignoff: human+tooling`
    - `SecurityChecklist: docs/security/security-checklist-<topic>.md` (must exist)
    - `SecurityScans: sast=...,dependency=...,sbom=...,secrets=...,dast=...`
  - If dependency manifest files are changed, requires:
    - `DependencyCheck: registry-verified,pinned,maintained - ...`
    - `DependencyLicenseCheck: pass|fail|not-applicable - ...`
  - If contract/interface files are changed, requires:
    - `Compatibility: backward-compatible|breaking-change - ...`
    - `Deprecation: none|date=YYYY-MM-DD|not-applicable`
  - For `Type: bugfix` commits touching `>=10` staged files, requires:
    - `ContextReduction: yes - <2-6 files per slice strategy>`
  - If `PolicyException: yes` is declared, requires:
    - `ExceptionOwner`, `ExceptionReason`, `ExceptionIssue`, `ExceptionExpires`, `ExceptionFollowup`
    - rejects expired `ExceptionExpires` dates when enabled
  - If `FlakyTest: ...` is declared, requires:
    - `FlakyOwner`, `FlakyIssue`, `FlakyBy`
    - rejects expired `FlakyBy` dates when enabled

- `pre-push` hook:
  - Validates linked issues are open when `gh` and repo slug are available
  - Validates changed task-brief files contain `FR-###`, `AC-###`, and a `Given/When/Then` acceptance criterion
  - Validates spec/task files for vague acceptance criteria and requires measurable wording
  - Validates agent lineage in commit metadata for medium/high-risk commits
  - Enforces ADR updates for architecture-impact path changes
  - Requires plan artifacts for commit messages marked `Risk: medium|high`
  - Requires change-request artifacts for behavior/contract-impact changes
  - If `AGENTS.md` changes, requires validator/hook/template updates in same push
  - If `docs/debug-brief.md` changes, requires validator/hook/template updates in same push
  - No Hidden Magic gate:
    - if automation/hook scripts change, requires docs update in same push (`README.md`, `docs/policy/local-enforcement.md`, or `policy-kit/README.md`)
  - For high-risk/large-change pushes, requires checkpoint artifact update:
    - `docs/handoffs/handoff-<phase>.md`
  - Runs policy drift audit (can be configured)
  - Runs exception debt report (can be configured as warn-only or strict)
  - Runs local CI gate before push:
    - smoke command
    - test command
    - build command
    - push is blocked if any required step fails
    - hook execution prepends `$HOME/.local/bin:$HOME/.cargo/bin` so bootstrap-installed security tools stay visible under git-hook PATHs
    - Oberon4 defaults:
      - smoke: `GATE_MODE=smoke bash scripts/oberon4-gate.sh`
      - test: `GATE_MODE=strict bash scripts/oberon4-gate.sh`
      - build: optional/off by default
  - If churn is high (>=10 files or >=300 lines), requires:
    - at least one commit marked `Risk: high`
    - at least 2 verification commands in commit messages
    - explicit rollback note in commit messages
  - Runs local security gate: secret scan, pinned SAST, dependency audit, dependency/license policy
  - Oberon4 canonical security gate is `bash scripts/oberon4-security-gate.sh`
    - required tools: `gitleaks`, `semgrep`, `cargo-audit`, `cargo-deny`
    - canonical bootstrap: `bash scripts/bootstrap-security-tools.sh`
    - local enforcement now expects the security profile to be present by default
    - existing approved findings must live in `security/gitleaks-baseline.json`
    - pinned Semgrep policy lives in `security/semgrep.yml`
    - dependency/license policy lives in `deny.toml`
    - troubleshoot and refresh baseline only through `docs/security/runbook-secret-triage.md`
    - explicit operator invocation:
      - `bash scripts/install-git-hooks.sh`
      - `bash scripts/check-policy-stack.sh`
      - `bash scripts/policy-drift-audit.sh`
      - `bash scripts/bootstrap-security-tools.sh`
      - `bash scripts/oberon4-security-gate.sh`
      - `RUN_SECURITY_GATE=1 GATE_MODE=smoke bash scripts/oberon4-gate.sh`
  - Blocks oversized pushes by default:
    - max changed files: `15`
    - max changed lines: `500`
  - Override by adding this line to a commit message:
    - `Override: large-change`

### Optional gate configuration

- Centralized config file:
  - `scripts/hooks.env`
  - `HOOKS_POLICY_MODE=strict|minimal` (`strict` default; `minimal` reduces non-safety metadata requirements for smaller-model workflows)
  - `HOOKS_AUTO_SYNC_POLICY_KIT_ON_PUSH=0|1` (when `1`, pre-push auto-syncs policy changes to `bos-policy-kit`)
  - `POLICY_KIT_REPO_DIR` (default: `$HOME/bos-policy-kit`)
  - `POLICY_KIT_TARGET_BRANCH` (default: `main`)
  - `HOOKS_ENFORCE_STACK_MAP=0|1` and `STACK_MAP_FILE` (`docs/policy/stack-map.env`)

- Override repo resolution and strictness:

```bash
export HOOKS_REPO_SLUG='owner/repo'
export HOOKS_POLICY_MODE='strict'   # or 'minimal'
export HOOKS_AUTO_SYNC_POLICY_KIT_ON_PUSH=0
export POLICY_KIT_REPO_DIR="$HOME/bos-policy-kit"
export POLICY_KIT_TARGET_BRANCH=main
export HOOKS_ENFORCE_STACK_MAP=1
export STACK_MAP_FILE="docs/policy/stack-map.env"
export HOOKS_REQUIRE_SAME_REPO_URL=1
export HOOKS_REQUIRE_GH_ISSUE_VALIDATION=0
export HOOKS_ENFORCE_TEST_CONTRACT=1
export HOOKS_ENFORCE_STACK_COMMAND_HINTS=1
export HOOKS_REQUIRE_HIGH_RISK_THREAT_MODEL=1
export HOOKS_REQUIRE_HIGH_RISK_ROLLBACK_REHEARSAL=1
export HOOKS_REQUIRE_MEDIUM_HIGH_QUALITY_SIGNALS=1
export HOOKS_REQUIRE_SECURITY_TRUST_BOUNDARY=1
export HOOKS_ENFORCE_POLICY_EXCEPTION_FIELDS=1
export HOOKS_REQUIRE_REVIEW_PASSES=1
export HOOKS_REQUIRE_HIGH_RISK_REVIEW_PACKET=1
export HOOKS_REQUIRE_DEPENDENCY_CHECK=1
export BUGFIX_HIGH_CONTEXT_FILE_THRESHOLD=10
export HOOKS_REQUIRE_HIGH_RISK_SECOND_REVIEW=1
export HOOKS_REQUIRE_SECURITY_SIGNOFF=1
export HOOKS_REJECT_EXPIRED_POLICY_DATES=1
export HOOKS_REQUIRE_REVIEW_FOCUS=1
export HOOKS_REQUIRE_SECURITY_CHECKLIST=1
export HOOKS_REQUIRE_SECURITY_SCANS_DECLARATION=1
export HOOKS_REQUIRE_PERFORMANCE_EVIDENCE=1
export HOOKS_REQUIRE_HIGH_RISK_PERFORMANCE_EVIDENCE=1
export HOOKS_REQUIRE_MAINTAINABILITY_EVIDENCE=1
export HOOKS_REQUIRE_OBSERVABILITY_EVIDENCE=1
export HOOKS_REQUIRE_AGENT_ROLE=1
export HOOKS_REQUIRE_STACK_PROFILE=1
export HOOKS_REQUIRE_SELF_EVAL=1
export HOOKS_REQUIRE_DATA_HANDLING_DECLARATION=1
export HOOKS_REQUIRE_RELEASE_SAFETY_FIELDS=1
export HOOKS_REQUIRE_COMPATIBILITY_FIELDS=1
export HOOKS_REQUIRE_DEPENDENCY_LICENSE_CHECK=1
export HOOKS_RUN_POLICY_DRIFT_AUDIT=1
export HOOKS_RUN_EXCEPTION_DEBT_REPORT=1
export HOOKS_EXCEPTION_DEBT_STRICT=0
export HOOKS_EXCEPTION_DEBT_HORIZON_DAYS=14
export HOOKS_RUN_LOCAL_CI_ON_PUSH=1
export HOOKS_LOCAL_CI_STRICT=1
export HOOKS_LOCAL_CI_REQUIRE_SMOKE=1
export HOOKS_LOCAL_CI_REQUIRE_TEST=1
export HOOKS_LOCAL_CI_REQUIRE_BUILD=0
export HOOKS_LOCAL_SMOKE_CMD="GATE_MODE=smoke bash scripts/oberon4-gate.sh"
export HOOKS_LOCAL_TEST_CMD="GATE_MODE=strict bash scripts/oberon4-gate.sh"
export HOOKS_LOCAL_BUILD_CMD=""
```

Recommended for strict enforcement after configuring your stack-specific commands:

```bash
export HOOKS_LOCAL_CI_STRICT=1
export HOOKS_LOCAL_CI_REQUIRE_SMOKE=1
export HOOKS_LOCAL_CI_REQUIRE_TEST=1
export HOOKS_LOCAL_CI_REQUIRE_BUILD=1
```

Safe override path for a lighter local workflow:

```bash
export HOOKS_LOCAL_CI_STRICT=0
export HOOKS_LOCAL_SMOKE_CMD="GATE_MODE=smoke RUN_WORKSPACE_TESTS=1 bash scripts/oberon4-gate.sh"
```

- Customize architecture-impact path detection for ADR enforcement:

```bash
export ADR_REQUIRED_PATH_REGEX='^src/|^apps/|^crates/|^infrastructure/|^api/|^docs/architecture/'
```

- Customize behavior/contract-impact path detection for change-request enforcement:

```bash
export CHANGE_REQUEST_REQUIRED_PATH_REGEX='^src/|^apps/|^crates/|^api/|^infrastructure/migrations/|openapi|schema'
```

- Customize churn escalation thresholds:

```bash
export CHURN_ESCALATION_FILES=10
export CHURN_ESCALATION_LINES=300
```

### Local utility workflows

- Hooks-ready machine bootstrap:

```bash
bash scripts/install-git-hooks.sh
bash scripts/check-policy-stack.sh
bash scripts/policy-drift-audit.sh
```

- Build focused context pack from git range:

```bash
bash scripts/build-context-pack.sh HEAD~1 HEAD
```

- Context budget controls for context-pack generation:

```bash
export CONTEXT_TOKEN_BUDGET=8000
export CONTEXT_TOKEN_WARN_PCT=70
export CONTEXT_BUDGET_STRICT=0   # set to 1 to fail when over budget
```

- Snapshot a debug brief for recovery/handoff:

```bash
bash scripts/snapshot-debug-brief.sh docs/debug-brief.md
```

- Run policy stack health check:

```bash
bash scripts/check-policy-stack.sh
```

- Run full bootstrap verifier (hooks + health + tests + drift):

```bash
bash scripts/bootstrap-policy-stack.sh
```

- Run a guarded single-role agent session (prevents nested subagent spawning):

```bash
bash scripts/run-agent.sh --role orchestrator --task-id T123 --prompt "Create a plan for FR-101 and list file boundaries."
```

- Force clear stale orchestrator lock when needed (explicit operator action):

```bash
bash scripts/run-agent.sh --role orchestrator --task-id T123 --force-unlock --prompt "Resume orchestration from latest handoff."
```

- Agent runtime lineage manifests are written to:
  - `.agent-runtime/<task-id>/runs/*.json`
- Delegation/loop violations create incident artifacts in:
  - `docs/incidents/agent-incident-<task-id>-<timestamp>.md`
  - template: `docs/templates/agent-incident.md`

### Policy Validator Tests

Run local validator regression tests:

```bash
bash tests/policy/run.sh
```

### Periodic Drift Checks (Not Only Pre-Push)

One-shot audit:

```bash
bash scripts/policy-drift-audit.sh
```

Local watch loop:

```bash
bash scripts/run-drift-watch.sh 900
```

Install cron-based periodic audit:

```bash
bash scripts/install-drift-cron.sh 60
```

### Golden Commit Message Examples

- `docs/examples/commit-messages/`

### Fresh Context Recovery

For a fresh session, recover context in this order:
1. `AGENTS.md`
2. active plan/change artifact in `docs/plans/` or `docs/change-requests/`
3. `docs/context-pack.md`
4. latest handoff in `docs/handoffs/`
5. domain artifacts as needed (`docs/security/`, `docs/maintainability/`, `docs/runbooks/`)

Canonical placement map:
- `docs/memory/README.md`

- Preview or apply targeted runtime state reset:

```bash
bash scripts/reset-state.sh --scope safe
bash scripts/reset-state.sh --scope safe --apply
```

### Templates

- Commit message template: `docs/templates/commit-message-template.md`
- Task brief template: `docs/templates/task-brief.md`
- Handoff template: `docs/templates/handoff.md`
- Debug brief template: `docs/templates/debug-brief.md`
- Plan template: `docs/templates/plan.md`
- Change request template: `docs/templates/change-request.md`
- Security checklist template: `docs/templates/security-checklist.md`
- Performance checklist template: `docs/templates/performance-checklist.md`
- Maintainability checklist template: `docs/templates/maintainability-checklist.md`
- Runbook template: `docs/templates/runbook.md`
- ADR template: `docs/adr/0000-template.md`
- Handoff checkpoint docs: `docs/handoffs/README.md`

## Reuse In New Repos

Use `policy-kit/` to install this stack in another repository.

```bash
bash policy-kit/bootstrap.sh --target /path/to/repo --repo-slug owner/repo
```

Update an existing repo from the kit payload:

```bash
bash policy-kit/update.sh --target /path/to/repo
```
