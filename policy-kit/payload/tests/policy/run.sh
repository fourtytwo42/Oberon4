#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
TMP_DIR="$(mktemp -d /tmp/policy-tests-XXXXXX)"
trap 'rm -rf "$TMP_DIR"' EXIT

pass_count=0
fail_count=0

pass() {
  echo "[PASS] $*"
  pass_count=$((pass_count + 1))
}

fail() {
  echo "[FAIL] $*" >&2
  fail_count=$((fail_count + 1))
}

run_msg_test() {
  local name="$1"
  local fixture="$2"
  local expected="$3" # pass|fail
  local setup="${4:-none}"
  local repo="$TMP_DIR/msg-${name}"

  mkdir -p "$repo/scripts/lib"
  cp "$ROOT_DIR/scripts/validate-commit-message.sh" "$repo/scripts/validate-commit-message.sh"
  cp "$ROOT_DIR/scripts/hooks.env" "$repo/scripts/hooks.env"
  cp "$ROOT_DIR/scripts/lib/common.sh" "$repo/scripts/lib/common.sh"
  chmod +x "$repo/scripts/validate-commit-message.sh"

  git -C "$repo" init >/dev/null 2>&1
  git -C "$repo" config user.name test
  git -C "$repo" config user.email test@example.com
  git -C "$repo" remote add origin https://github.com/fourtytwo42/BOS.git
  touch "$repo/README.md"
  git -C "$repo" add README.md
  git -C "$repo" commit -m "init" >/dev/null 2>&1

  case "$setup" in
    staged-dependency)
      cat > "$repo/package.json" <<'EOF'
{"name":"policy-test","version":"1.0.0"}
EOF
      git -C "$repo" add package.json
      ;;
    staged-contract)
      mkdir -p "$repo/api"
      cat > "$repo/api/openapi.yaml" <<'EOF'
openapi: "3.0.0"
info:
  title: test
  version: "1.0.0"
paths: {}
EOF
      git -C "$repo" add api/openapi.yaml
      ;;
    high-risk-artifacts)
      mkdir -p "$repo/docs/handoffs" "$repo/docs/runbooks"
      cat > "$repo/docs/handoffs/handoff-test.md" <<'EOF'
# Handoff
EOF
      cat > "$repo/docs/runbooks/runbook-test.md" <<'EOF'
# Runbook
EOF
      ;;
    none) ;;
    *)
      echo "Unknown setup scenario: $setup" >&2
      exit 2
      ;;
  esac

  cp "$ROOT_DIR/tests/policy/fixtures/$fixture" "$repo/msg.txt"

  if (cd "$repo" && scripts/validate-commit-message.sh msg.txt >/dev/null 2>&1); then
    if [[ "$expected" == "pass" ]]; then
      pass "commit-msg validator: $name"
    else
      fail "commit-msg validator: $name (expected fail, got pass)"
    fi
  else
    if [[ "$expected" == "fail" ]]; then
      pass "commit-msg validator: $name"
    else
      fail "commit-msg validator: $name (expected pass, got fail)"
    fi
  fi
}

run_lineage_test() {
  local name="$1"
  local role="$2"
  local depth="$3"
  local delegation="$4"
  local spawned="$5"
  local expected="$6" # pass|fail
  local repo="$TMP_DIR/lineage-${name}"

  mkdir -p "$repo/scripts"
  cp "$ROOT_DIR/scripts/validate-agent-lineage.sh" "$repo/scripts/validate-agent-lineage.sh"
  chmod +x "$repo/scripts/validate-agent-lineage.sh"

  git -C "$repo" init >/dev/null 2>&1
  git -C "$repo" config user.name test
  git -C "$repo" config user.email test@example.com
  touch "$repo/README.md"
  git -C "$repo" add README.md
  git -C "$repo" commit -m "init" >/dev/null 2>&1

  cat > "$repo/msg.txt" <<EOF
lineage test ${name}

Issue: #1
Type: chore
Risk: medium
Assumptions: test
Rollback: test
ResearchInputs: AGENTS.md#3.4, tests/policy/fixtures
EvidenceScope: in=lineage metadata checks; out=runtime execution
SelfEval: pass
PolicyDelta: none
WorkflowType: collaboration
WorkflowMode: plan-build
TaskState: verify
FR-101: met, AC-101: met
QualitySignals: churn: test
ReviewPasses: pass1-logic-security,pass2-maintainability
ReviewFocus: security
AgentRole: ${role}
StackProfile: polyglot
AgentDepth: ${depth}
Delegation: ${delegation}
SpawnedAgents: ${spawned}
Observability: logs=unchanged,metrics=unchanged,alerts=unchanged
NoNewFacts: yes
Traceability: FR-101, AC-101

Verification:
$ echo ok
Output: ok
EOF

  git -C "$repo" commit --allow-empty -F msg.txt >/dev/null 2>&1

  if (cd "$repo" && scripts/validate-agent-lineage.sh HEAD~1..HEAD >/dev/null 2>&1); then
    if [[ "$expected" == "pass" ]]; then
      pass "lineage validator: $name"
    else
      fail "lineage validator: $name (expected fail, got pass)"
    fi
  else
    if [[ "$expected" == "fail" ]]; then
      pass "lineage validator: $name"
    else
      fail "lineage validator: $name (expected pass, got fail)"
    fi
  fi
}

run_msg_test "low-risk-minimal" "valid-low-risk.txt" "pass"
run_msg_test "subagent-valid" "valid-medium-subagent.txt" "pass"
run_msg_test "medium-missing-self-eval-invalid" "invalid-medium-missing-self-eval.txt" "fail"
run_msg_test "medium-blocked-without-next-action-invalid" "invalid-medium-blocked-without-next-action.txt" "fail"
run_msg_test "subagent-direct-codex-invalid" "invalid-medium-subagent-direct-codex.txt" "fail"
run_msg_test "dependency-missing-license-invalid" "invalid-dependency-missing-license.txt" "fail" "staged-dependency"
run_msg_test "contract-missing-compat-invalid" "invalid-contract-missing-compat.txt" "fail" "staged-contract"
run_msg_test "high-risk-orchestrator-valid" "valid-high-risk-orchestrator.txt" "pass" "high-risk-artifacts"

run_lineage_test "implementer-valid" "implementer" "1" "none" "0" "pass"
run_lineage_test "implementer-invalid-delegation" "implementer" "1" "planner" "1" "fail"

echo
echo "Results: ${pass_count} passed, ${fail_count} failed"
if (( fail_count > 0 )); then
  exit 1
fi
