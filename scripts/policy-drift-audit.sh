#!/usr/bin/env bash
set -euo pipefail

err=0

require_pattern() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  if [[ ! -f "$file" ]]; then
    echo "Policy drift: missing file ${file} (${label})." >&2
    err=1
    return
  fi
  if ! grep -Eq "$pattern" "$file"; then
    echo "Policy drift: ${label} not found in ${file}." >&2
    err=1
  fi
}

# Canonical policy sections must exist.
require_pattern "AGENTS.md" "Security-First Rules" "security policy section"
require_pattern "AGENTS.md" "Performance Verification Standard" "performance policy section"
require_pattern "AGENTS.md" "Maintainability Verification Standard" "maintainability policy section"
require_pattern "AGENTS.md" "Policy Drift and Exception Debt Audits" "drift audit policy section"
require_pattern "AGENTS.md" "Subagent anti-recursion controls" "anti-recursion policy section"
require_pattern "AGENTS.md" "Stack Isolation Rule" "stack isolation policy section"
require_pattern "AGENTS.md" "Priority Core Rules \(Compressed\)" "compressed priority core section"
require_pattern "AGENTS.md" "Consolidated Rule Map" "consolidated rule map section"
require_pattern "docs/policy/stack-map.env" "^STACK_ACTIVE_PROFILES=" "stack map profile declaration"

# Commit template must expose core metadata fields.
require_pattern "docs/templates/commit-message-template.md" "^ReviewFocus:" "ReviewFocus field"
require_pattern "docs/templates/commit-message-template.md" "^SecurityScans:" "SecurityScans field"
require_pattern "docs/templates/commit-message-template.md" "^PerfBaseline:" "PerfBaseline field"
require_pattern "docs/templates/commit-message-template.md" "^DuplicationCheck:" "DuplicationCheck field"
require_pattern "docs/templates/commit-message-template.md" "^Observability:" "Observability field"
require_pattern "docs/templates/commit-message-template.md" "^ReleaseStrategy:" "ReleaseStrategy field"
require_pattern "docs/templates/commit-message-template.md" "^DependencyLicenseCheck:" "DependencyLicenseCheck field"
require_pattern "docs/templates/commit-message-template.md" "^Compatibility:" "Compatibility field"
require_pattern "docs/templates/commit-message-template.md" "^SelfEval:" "SelfEval field"
require_pattern "docs/templates/commit-message-template.md" "^PolicyDelta:" "PolicyDelta field"
require_pattern "docs/templates/agent-incident.md" "^# Agent Incident" "agent incident template"

# Validator must enforce corresponding checks.
require_pattern "scripts/validate-commit-message.sh" "SecurityScans" "SecurityScans enforcement"
require_pattern "scripts/validate-commit-message.sh" "PerfBaseline" "PerfBaseline enforcement"
require_pattern "scripts/validate-commit-message.sh" "DuplicationCheck" "DuplicationCheck enforcement"
require_pattern "scripts/validate-commit-message.sh" "Observability" "Observability enforcement"
require_pattern "scripts/validate-commit-message.sh" "ReleaseStrategy" "ReleaseStrategy enforcement"
require_pattern "scripts/validate-commit-message.sh" "DependencyLicenseCheck" "DependencyLicenseCheck enforcement"
require_pattern "scripts/validate-commit-message.sh" "Compatibility" "Compatibility enforcement"
require_pattern "scripts/validate-commit-message.sh" "SelfEval" "self-evaluation enforcement"
require_pattern "scripts/validate-commit-message.sh" "PolicyDelta" "policy delta enforcement"
require_pattern "scripts/validate-commit-message.sh" "Multiple stacks detected in staged files" "stack map multi-stack enforcement"
require_pattern "scripts/run-agent.sh" "record_agent_incident" "runtime incident generation"
require_pattern "scripts/run-agent.ps1" "FailWithIncident" "windows runtime incident generation"
require_pattern "scripts/bootstrap-policy-stack.sh" "bootstrap verification passed" "bootstrap verifier script"
require_pattern "tests/policy/run.sh" "run_msg_test" "policy validator test runner"

if (( err != 0 )); then
  exit 1
fi

echo "Policy drift audit: OK"
