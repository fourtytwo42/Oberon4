Summary: <short imperative summary>
# Priority execution order reference: AGENTS.md `## 0.2) Priority Core Rules (Compressed)`

Issue: #123
Type: feature
Risk: low
Stack: node
TestScope: unit,integration
Traceability: FR-101, AC-201
Verification:
$ <command>
Output: <result>

# Required when Risk is medium/high:
# In HOOKS_POLICY_MODE=minimal, some non-safety fields may be optional.
Assumptions: <key assumptions>
Rollback: <how to revert safely>
ResearchInputs: <docs/files/commands consulted before build>
EvidenceScope: in=<what evidence included>; out=<what intentionally excluded>
SelfEval: pass|rule-update-needed
PolicyDelta: none|updated:<paths>
WorkflowType: feature-development|maintenance|collaboration
WorkflowMode: simple|plan-build|incremental-gates
TaskState: planned|in-progress|blocked|verify|done
NextAction: <required when TaskState=blocked>
NoNewFacts: yes
FR-101: met, AC-201: met
AgentRole: orchestrator|implementer|verifier|reviewer|solo
StackProfile: node|python|go|java|dotnet|rust|php|ruby|polyglot|other
AgentDepth: 0|1
Delegation: none|planner,implementer,verifier,reviewer
SpawnedAgents: 0
Invocation: run-agent.sh      # required for implementer|verifier|reviewer
SpawnBudget: <n>                # required for orchestrator when Delegation != none
MaxSpawnDepth: 1                # required for orchestrator when Delegation != none
QualitySignals: <issue_density|churn|rework|mutation_delta>: <note>
ReviewFocus: security|performance|maintainability
ReviewPasses: pass1-logic-security,pass2-maintainability
Observability: logs=updated|unchanged,metrics=updated|unchanged,alerts=updated|unchanged
PerfBaseline: p50=<ms>,p95=<ms>,cpu=<%>,mem=<mb>,queries=<n>,bundle=<kb>|not-applicable
PerfBudget: p95<___ms,mem<___mb,queries<=___,bundle<___kb|not-applicable
PerfProfile: <tool/report path or summary>|not-applicable
PerfBench: micro=<before->after>;macro=<before->after>|not-applicable
DuplicationCheck: pass|fail|not-applicable - <delta or note>
SmellCheck: pass|fail|not-applicable - <tool/result>
MaintainabilityChecklist: docs/maintainability/maintainability-checklist-<topic>.md|not-applicable
DataHandling: pii=none|redacted|encrypted|not-applicable,retention=<impact>,logs=redacted|not-applicable

# Required when Type is feature:
Plan: docs/plans/plan-<topic>.md
TaskBrief: docs/task-briefs/<topic>.md

# Required when Type is bugfix:
MRE: <minimal repro command or steps>
Before: <failing evidence on repro>
After: <passing evidence on same repro>
IndependentCheck: <counterexample|property|differential|regression>
DebugBrief: docs/debug-brief.md

# Required when Type is refactor:
Characterization: added|existing

# Required when Risk is high or Override is large-change:
PhaseCheckpoint: docs/handoffs/handoff-<phase>.md
ThreatModel: STRIDE|OWASP|both
RollbackRehearsal: pass|simulated - <command/steps and observed outcome>
MitigationPlan: rollback|disable-path|capacity - <trigger>
ReleaseStrategy: canary|staged|full|not-applicable - <scope/trigger>
Runbook: docs/runbooks/<name>.md|not-applicable
ReviewPacket: assumptions,threat-model,test-plan,rollback-plan
SecondReview: yes
SecuritySignoff: human+tooling
SecurityScans: sast=pass,dependency=pass,sbom=pass,secrets=pass,dast=not-applicable

# Stack examples:
# Stack: node|python|go|java|dotnet|rust|php|ruby|polyglot|other
# TestScope: unit,integration,contract,e2e,property,mutation,security,migration,smoke

# Required when Type is security (or when untrusted input/tool boundary changes):
UntrustedInputBoundary: validated|not-applicable - <what is allowlisted/blocked>
SecurityChecklist: docs/security/security-checklist-<topic>.md
SecurityScans: sast=pass,dependency=pass,sbom=pass,secrets=pass,dast=pass|not-applicable

# Required when bugfix effort exceeded timebox:
EscalationAfterMins: 30
EscalationPath: rollback|restart|second-review|mre-narrow

# Optional exception protocol (required fields if PolicyException: yes):
PolicyException: yes
ExceptionOwner: <name/role>
ExceptionReason: <why exception is needed>
ExceptionIssue: #123
ExceptionExpires: YYYY-MM-DD
ExceptionFollowup: <what will remove exception>

# Optional flaky test quarantine fields (required together when used):
FlakyTest: <test id/name>
FlakyOwner: <name/role>
FlakyIssue: #123
FlakyBy: YYYY-MM-DD

# Required when dependency manifests are changed:
DependencyCheck: registry-verified,pinned,maintained - <evidence note>
DependencyLicenseCheck: pass|fail|not-applicable - <license/compliance note>

# Required for bugfix commits touching 10+ files:
ContextReduction: yes - <2-6 files per slice strategy>

# Required when contract/interface files are changed:
Compatibility: backward-compatible|breaking-change - <impact>
Deprecation: none|date=YYYY-MM-DD|not-applicable
