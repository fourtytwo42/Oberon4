#!/usr/bin/env bash
set -euo pipefail

failures=0

ok() {
  echo "[OK] $*"
}

fail() {
  echo "[FAIL] $*" >&2
  failures=$((failures + 1))
}

require_cmd() {
  local cmd="$1"
  if command -v "$cmd" >/dev/null 2>&1; then
    ok "command available: $cmd"
  else
    fail "missing command: $cmd"
  fi
}

check_executable() {
  local path="$1"
  if [[ -x "$path" ]]; then
    ok "executable: $path"
  else
    fail "not executable: $path"
  fi
}

check_file() {
  local path="$1"
  if [[ -f "$path" ]]; then
    ok "file exists: $path"
  else
    fail "missing file: $path"
  fi
}

check_file "AGENTS.md"
check_file "scripts/hooks.env"
check_file "docs/policy/stack-map.env"
check_file "scripts/run-agent.ps1"
check_file ".githooks/commit-msg"
check_file ".githooks/pre-push"

if git rev-parse --git-dir >/dev/null 2>&1; then
  ok "inside git repository"
else
  fail "not inside git repository"
fi

hooks_path="$(git config --get core.hooksPath || true)"
if [[ "$hooks_path" == ".githooks" ]]; then
  ok "hooksPath configured: .githooks"
else
  fail "hooksPath not set to .githooks (current: ${hooks_path:-<unset>})"
fi

if bash -lc 'source scripts/hooks.env' >/dev/null 2>&1; then
  ok "hooks configuration loads"
else
  fail "unable to load scripts/hooks.env"
fi

require_cmd bash
require_cmd git

required_exec=(
  "scripts/install-git-hooks.sh"
  "scripts/check-policy-stack.sh"
  "scripts/bootstrap-policy-stack.sh"
  "scripts/policy-doctor.sh"
  "scripts/risk-classifier.sh"
  "scripts/sync-policy-kit.sh"
  "scripts/run-agent.sh"
  "scripts/install-drift-cron.sh"
  "scripts/run-drift-watch.sh"
  "scripts/validate-commit-message.sh"
  "scripts/validate-push.sh"
  "scripts/validate-agent-lineage.sh"
  "scripts/policy-drift-audit.sh"
)

for p in "${required_exec[@]}"; do
  check_executable "$p"
done

if bash -n scripts/validate-commit-message.sh >/dev/null 2>&1 \
  && bash -n scripts/validate-push.sh >/dev/null 2>&1 \
  && bash -n scripts/validate-agent-lineage.sh >/dev/null 2>&1 \
  && bash -n scripts/run-agent.sh >/dev/null 2>&1; then
  ok "core policy scripts pass shell syntax check"
else
  fail "one or more policy scripts fail shell syntax check"
fi

if (( failures > 0 )); then
  echo "Policy stack health check failed (${failures} issue(s))." >&2
  exit 1
fi

echo "Policy stack health check passed."
