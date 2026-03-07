#!/usr/bin/env bash
set -euo pipefail

run_tests=1

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-tests) run_tests=0; shift ;;
    -h|--help)
      cat <<'EOF'
Usage: scripts/bootstrap-policy-stack.sh [--skip-tests]

Steps:
1. install git hooks
2. run policy stack health check
3. run validator regression tests (unless --skip-tests)
4. run policy drift audit
EOF
      exit 0
      ;;
    *)
      echo "Unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

echo "[bootstrap] installing hooks..."
bash scripts/install-git-hooks.sh

echo "[bootstrap] running policy stack health check..."
bash scripts/check-policy-stack.sh

if [[ "$run_tests" == "1" ]]; then
  if [[ -x tests/policy/run.sh ]]; then
    echo "[bootstrap] running policy validator tests..."
    bash tests/policy/run.sh
  else
    echo "[bootstrap] policy tests not found; skipping."
  fi
fi

echo "[bootstrap] running policy drift audit..."
bash scripts/policy-drift-audit.sh

echo "[bootstrap] policy stack bootstrap verification passed."
