#!/usr/bin/env bash
set -euo pipefail

interval_secs="${1:-900}"
if [[ ! "$interval_secs" =~ ^[0-9]+$ || "$interval_secs" -lt 30 ]]; then
  echo "Usage: $0 [interval_seconds>=30]" >&2
  exit 2
fi

echo "Starting drift watch (interval=${interval_secs}s). Ctrl+C to stop."
while true; do
  ts="$(date -u +%FT%TZ)"
  echo "[${ts}] running policy drift audit..."
  if bash scripts/policy-drift-audit.sh; then
    echo "[${ts}] drift audit OK"
  else
    echo "[${ts}] drift audit FAILED" >&2
  fi
  sleep "$interval_secs"
done
