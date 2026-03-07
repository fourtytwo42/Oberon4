#!/usr/bin/env bash
set -euo pipefail

horizon_days="${1:-${HOOKS_EXCEPTION_DEBT_HORIZON_DAYS:-14}}"
strict="${2:-${HOOKS_EXCEPTION_DEBT_STRICT:-0}}"

if ! [[ "$horizon_days" =~ ^[0-9]+$ ]]; then
  echo "Invalid horizon_days: ${horizon_days}" >&2
  exit 2
fi

today="$(date -u +%F 2>/dev/null || date +%F)"
warn_cutoff="$(date -u -d "+${horizon_days} days" +%F 2>/dev/null || date -d "+${horizon_days} days" +%F)"

issues=0
warnings=0

scan_field() {
  local field="$1"
  local label="$2"
  local dates
  dates="$(git log --format=%B -n 300 | grep -Eio "^${field}:[[:space:]]*[0-9]{4}-[0-9]{2}-[0-9]{2}[[:space:]]*$" | grep -Eo '[0-9]{4}-[0-9]{2}-[0-9]{2}' | sort -u || true)"
  [[ -z "${dates//[[:space:]]/}" ]] && return 0

  while IFS= read -r d; do
    [[ -z "$d" ]] && continue
    if [[ "$d" < "$today" ]]; then
      echo "Exception debt: ${label} expired on ${d}." >&2
      issues=$((issues + 1))
    elif [[ "$d" < "$warn_cutoff" || "$d" == "$warn_cutoff" ]]; then
      echo "Exception debt: ${label} expires soon (${d})." >&2
      warnings=$((warnings + 1))
    fi
  done <<<"$dates"
}

scan_field "ExceptionExpires" "policy exception"
scan_field "FlakyBy" "flaky quarantine"

if (( issues > 0 )); then
  if [[ "$strict" == "1" ]]; then
    exit 1
  fi
fi

if (( warnings > 0 )); then
  if [[ "$strict" == "1" ]]; then
    exit 1
  fi
fi

echo "Exception debt report: issues=${issues}, warnings=${warnings}, horizon_days=${horizon_days}"
