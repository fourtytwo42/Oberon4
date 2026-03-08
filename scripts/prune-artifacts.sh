#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
: "${ARTIFACT_PRUNE_ROOT:=${ROOT_DIR}}"

: "${ARTIFACT_PRUNE_MAX_AGE_DAYS:=14}"
: "${ARTIFACT_PRUNE_KEEP_NEWEST:=10}"

DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

roots=(
  "${ARTIFACT_PRUNE_ROOT}/artifacts/failures"
  "${ARTIFACT_PRUNE_ROOT}/artifacts/repair-loop"
  "${ARTIFACT_PRUNE_ROOT}/artifacts/live-local-smoke"
)

now_epoch="$(date +%s)"
threshold_seconds="$(( ARTIFACT_PRUNE_MAX_AGE_DAYS * 86400 ))"

for root in "${roots[@]}"; do
  echo "ARTIFACT_ROOT ${root}"
  if [[ ! -d "${root}" ]]; then
    echo "ARTIFACT_ROOT_STATUS missing"
    continue
  fi

  mapfile -t entries < <(find "${root}" -mindepth 1 -maxdepth 1 -type d -printf '%T@ %p\n' | sort -rn)
  if [[ "${#entries[@]}" -eq 0 ]]; then
    echo "ARTIFACT_ROOT_STATUS empty"
    continue
  fi

  index=0
  for entry in "${entries[@]}"; do
    index=$((index + 1))
    mtime="${entry%% *}"
    path="${entry#* }"
    age_seconds="$(awk -v now="${now_epoch}" -v mtime="${mtime}" 'BEGIN { printf "%.0f", now - mtime }')"

    keep_reason=""
    if (( index <= ARTIFACT_PRUNE_KEEP_NEWEST )); then
      keep_reason="keep_newest"
    elif (( age_seconds <= threshold_seconds )); then
      keep_reason="keep_recent"
    fi

    if [[ -n "${keep_reason}" ]]; then
      echo "KEEP ${path} reason=${keep_reason} age_seconds=${age_seconds}"
      continue
    fi

    if [[ "${DRY_RUN}" == "1" ]]; then
      echo "PRUNE ${path} age_seconds=${age_seconds} dry_run=1"
    else
      rm -rf "${path}"
      echo "PRUNE ${path} age_seconds=${age_seconds} dry_run=0"
    fi
  done
done
