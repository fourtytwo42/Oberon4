#!/usr/bin/env bash
set -euo pipefail

interval_mins="${1:-60}"
if [[ ! "$interval_mins" =~ ^[0-9]+$ || "$interval_mins" -lt 1 ]]; then
  echo "Usage: $0 [interval_minutes>=1]" >&2
  exit 2
fi

if ! command -v crontab >/dev/null 2>&1; then
  echo "crontab is not available on this system." >&2
  exit 1
fi

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "$repo_root/.agent-runtime"

marker="# BOS_POLICY_DRIFT_CHECK"
cron_cmd="cd $repo_root && bash scripts/policy-drift-audit.sh >> .agent-runtime/drift-audit.log 2>&1"
cron_line="*/${interval_mins} * * * * ${cron_cmd} ${marker}"

existing="$(crontab -l 2>/dev/null || true)"
filtered="$(printf '%s\n' "$existing" | sed "/${marker//\//\\/}/d")"
{
  printf '%s\n' "$filtered"
  printf '%s\n' "$cron_line"
} | crontab -

echo "Installed periodic drift audit cron job (every ${interval_mins} minute(s))."
echo "Log file: ${repo_root}/.agent-runtime/drift-audit.log"
