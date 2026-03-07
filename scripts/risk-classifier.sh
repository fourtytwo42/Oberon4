#!/usr/bin/env bash
set -euo pipefail

source scripts/lib/common.sh
load_hooks_env

usage() {
  cat <<'USAGE'
Usage: scripts/risk-classifier.sh [--cached|--range <git-range>] [--json]

Classifies current change and prints suggested commit metadata:
- Suggested Type
- Suggested Risk
- Suggested Stack/TestScope
- Required fields for the inferred envelope

Defaults to staged changes (--cached).
USAGE
}

mode="cached"
range=""
json=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cached) mode="cached"; shift ;;
    --range)
      mode="range"
      range="${2:-}"
      [[ -n "$range" ]] || { echo "--range requires value" >&2; exit 2; }
      shift 2
      ;;
    --json) json=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

if [[ "$mode" == "cached" ]]; then
  files="$(git diff --cached --name-only)"
  lines="$(git diff --cached --numstat | awk '{a+=$1; d+=$2} END {print a+d+0}')"
else
  files="$(git diff --name-only "$range")"
  lines="$(git diff --numstat "$range" | awk '{a+=$1; d+=$2} END {print a+d+0}')"
fi

changed_files="$(wc -l <<<"$files" | tr -d ' ')"
if [[ -z "${files//[[:space:]]/}" ]]; then
  changed_files=0
fi

policy_mode="${HOOKS_POLICY_MODE:-strict}"
churn_escalation_files="${CHURN_ESCALATION_FILES:-12}"
churn_escalation_lines="${CHURN_ESCALATION_LINES:-300}"

is_docs_only=1
is_code_touch=0
is_policy_touch=0
is_contract_touch=0
is_security_touch=0

if grep -Eiq '(^|/)(src|apps|packages|api|infrastructure)/|\.(ts|tsx|js|jsx|py|go|java|cs|rs|rb|php|kt|swift|c|cpp|h)$' <<<"$files"; then
  is_code_touch=1
fi

if grep -Eiq '(^|/)scripts/|^\.githooks/|^AGENTS\.md$|^docs/policy/|^docs/templates/' <<<"$files"; then
  is_policy_touch=1
fi

if grep -Eiq 'openapi|schema|contract|interface|^api/' <<<"$files"; then
  is_contract_touch=1
fi

if grep -Eiq 'security|auth|permission|secret|credential|token|crypto|encryption|injection|xss|csrf|sql' <<<"$files"; then
  is_security_touch=1
fi

if grep -Eiq '(^|/)(src|apps|packages|api|infrastructure|scripts|\.githooks)/|\.(ts|tsx|js|jsx|py|go|java|cs|rs|rb|php|kt|swift|c|cpp|h|sh|ps1)$' <<<"$files"; then
  is_docs_only=0
fi

change_type="chore"
if [[ "$is_security_touch" == "1" ]]; then
  change_type="security"
elif [[ "$is_docs_only" == "1" ]]; then
  change_type="docs"
elif [[ "$is_code_touch" == "1" || "$is_contract_touch" == "1" ]]; then
  change_type="feature"
fi

risk="low"
if [[ "$is_security_touch" == "1" ]] || (( changed_files >= churn_escalation_files )) || (( lines >= churn_escalation_lines )) || [[ "$is_contract_touch" == "1" ]]; then
  risk="high"
elif (( changed_files >= 4 )) || (( lines >= 120 )) || [[ "$is_policy_touch" == "1" ]] || [[ "$is_code_touch" == "1" ]]; then
  risk="medium"
fi

stack="other"
if grep -Eiq '(^|/)(package\.json|package-lock\.json|pnpm-lock\.yaml|yarn\.lock)$|\.(ts|tsx|js|jsx)$' <<<"$files"; then
  stack="node"
elif grep -Eiq '(^|/)(requirements(\.txt|-dev\.txt)?|pyproject\.toml|poetry\.lock)$|\.py$' <<<"$files"; then
  stack="python"
elif grep -Eiq '(^|/)(go\.mod|go\.sum)$|\.go$' <<<"$files"; then
  stack="go"
elif grep -Eiq '(^|/)(pom\.xml|build\.gradle(\.kts)?|gradle\.lockfile)$|\.java$' <<<"$files"; then
  stack="java"
elif grep -Eiq '\.(cs|csproj)$' <<<"$files"; then
  stack="dotnet"
elif grep -Eiq '(^|/)(Cargo\.toml|Cargo\.lock)$|\.rs$' <<<"$files"; then
  stack="rust"
elif grep -Eiq '(^|/)(composer\.json|composer\.lock)$|\.php$' <<<"$files"; then
  stack="php"
elif grep -Eiq '(^|/)(Gemfile|Gemfile\.lock)$|\.rb$' <<<"$files"; then
  stack="ruby"
fi

test_scope=""
if [[ "$change_type" =~ ^(feature|bugfix|refactor|security)$ ]]; then
  if [[ "$risk" == "high" ]]; then
    test_scope="unit,integration,security"
  elif [[ "$risk" == "medium" ]]; then
    test_scope="unit,integration"
  else
    test_scope="unit"
  fi
  if [[ "$is_contract_touch" == "1" ]]; then
    test_scope="${test_scope},contract"
  fi
fi

required=("Issue" "Type" "Risk" "Verification")
if [[ "$change_type" =~ ^(feature|bugfix|refactor|security)$ ]]; then
  required+=("Stack" "TestScope")
fi
if [[ "$risk" == "medium" || "$risk" == "high" ]]; then
  required+=("Assumptions" "Rollback" "ResearchInputs" "EvidenceScope" "NoNewFacts" "FR/AC status")
  required+=("AgentRole" "AgentDepth" "Delegation")
  if [[ "$policy_mode" == "strict" ]]; then
    required+=("QualitySignals" "ReviewPasses" "ReviewFocus" "Observability" "StackProfile")
  fi
fi
if [[ "$change_type" == "feature" ]]; then
  required+=("Plan" "TaskBrief")
fi
if [[ "$change_type" == "bugfix" ]]; then
  required+=("MRE" "Before" "After" "IndependentCheck" "DebugBrief")
fi
if [[ "$change_type" == "refactor" ]]; then
  required+=("Characterization")
fi
if [[ "$change_type" == "security" ]]; then
  required+=("UntrustedInputBoundary" "SecuritySignoff" "SecurityScans")
fi
if [[ "$risk" == "high" ]]; then
  required+=("PhaseCheckpoint" "ThreatModel" "RollbackRehearsal" "MitigationPlan" "ReleaseStrategy" "Runbook" "SecondReview")
fi

if [[ "$json" == "1" ]]; then
  printf '{\n'
  printf '  "policy_mode": "%s",\n' "$policy_mode"
  printf '  "changed_files": %s,\n' "$changed_files"
  printf '  "changed_lines": %s,\n' "$lines"
  printf '  "suggested_type": "%s",\n' "$change_type"
  printf '  "suggested_risk": "%s",\n' "$risk"
  printf '  "suggested_stack": "%s",\n' "$stack"
  printf '  "suggested_test_scope": "%s",\n' "$test_scope"
  printf '  "required_fields": ['
  for i in "${!required[@]}"; do
    printf '"%s"' "${required[$i]}"
    if (( i + 1 < ${#required[@]} )); then
      printf ', '
    fi
  done
  printf ']\n}\n'
  exit 0
fi

echo "Policy mode: $policy_mode"
echo "Changed files: $changed_files"
echo "Changed lines: $lines"
echo "Suggested Type: $change_type"
echo "Suggested Risk: $risk"
if [[ -n "$test_scope" ]]; then
  echo "Suggested Stack: $stack"
  echo "Suggested TestScope: $test_scope"
fi
echo
printf 'Required fields (suggested):\n'
for f in "${required[@]}"; do
  echo "- $f"
done

if [[ "$risk" != "low" ]]; then
  echo
  echo "Tip: run scripts/policy-doctor.sh --msg <commit-message-file> before committing."
fi
