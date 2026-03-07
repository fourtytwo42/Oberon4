#!/usr/bin/env bash
set -euo pipefail

out="${1:-docs/debug-brief.md}"
mkdir -p "$(dirname "$out")"

branch="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
commit="$(git rev-parse --short HEAD 2>/dev/null || echo none)"
status="$(git status --short || true)"

{
  echo "# Debug Brief"
  echo
  echo "## Symptom"
  echo "- Fill symptom here"
  echo
  echo "## Repro Steps"
  echo "1. Fill repro step"
  echo
  echo "## Current State Snapshot"
  echo "- Branch: ${branch}"
  echo "- Commit: ${commit}"
  echo "- Dirty files:"
  if [[ -n "${status//[[:space:]]/}" ]]; then
    while IFS= read -r line; do
      [[ -n "$line" ]] && echo "  - ${line}"
    done <<<"$status"
  else
    echo "  - none"
  fi
  echo
  echo "## Key Evidence"
  echo "- Logs:"
  echo "- Failing tests:"
  echo "- Error output:"
  echo
  echo "## Hypotheses Tried"
  echo "- H1:"
  echo "- H2:"
  echo
  echo "## Outcomes"
  echo "- "
  echo
  echo "## Next Smallest Step"
  echo "- "
} > "$out"

echo "Wrote debug brief snapshot: $out"
