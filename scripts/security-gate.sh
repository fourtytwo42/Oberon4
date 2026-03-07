#!/usr/bin/env bash
set -euo pipefail

root="${1:-.}"

run_cmd() {
  echo "[security-gate] $*"
  "$@"
}

# Secret scanning (optional toolchain)
if command -v gitleaks >/dev/null 2>&1; then
  run_cmd gitleaks detect --source "$root" --redact --no-banner
elif command -v trufflehog >/dev/null 2>&1; then
  run_cmd trufflehog filesystem "$root" --fail --no-update
else
  echo "[security-gate] skipping secret scan (gitleaks/trufflehog not installed)"
fi

# SAST (optional)
if command -v semgrep >/dev/null 2>&1; then
  run_cmd semgrep --error --config auto "$root"
else
  echo "[security-gate] skipping SAST (semgrep not installed)"
fi

# Dependency vulnerability scan for Node repos (if applicable)
if [[ -f package.json ]] && command -v npm >/dev/null 2>&1; then
  # Audit can be noisy; gate on high/critical only.
  run_cmd npm audit --audit-level=high || {
    echo "[security-gate] npm audit found high/critical vulnerabilities" >&2
    exit 1
  }
else
  echo "[security-gate] skipping npm audit (package.json/npm not available)"
fi

exit 0
