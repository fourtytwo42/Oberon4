#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
PRUNE_SCRIPT="${ROOT_DIR}/scripts/prune-artifacts.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

mkdir -p \
  "${TMP_DIR}/artifacts/failures" \
  "${TMP_DIR}/artifacts/repair-loop" \
  "${TMP_DIR}/artifacts/live-local-smoke" \
  "${TMP_DIR}/artifacts/keep-me"

make_entry() {
  local path="$1"
  local days_ago="$2"
  mkdir -p "${path}"
  touch -d "${days_ago} days ago" "${path}"
}

make_entry "${TMP_DIR}/artifacts/failures/new-a" 0
make_entry "${TMP_DIR}/artifacts/failures/new-b" 1
make_entry "${TMP_DIR}/artifacts/failures/old-a" 30
make_entry "${TMP_DIR}/artifacts/repair-loop/new-a" 0
make_entry "${TMP_DIR}/artifacts/repair-loop/old-a" 40
make_entry "${TMP_DIR}/artifacts/live-local-smoke/new-a" 0
make_entry "${TMP_DIR}/artifacts/live-local-smoke/old-a" 25
make_entry "${TMP_DIR}/artifacts/keep-me/unrelated" 50

DRY_OUTPUT="${TMP_DIR}/dry.out"
(
  cd "${TMP_DIR}" && \
    ARTIFACT_PRUNE_ROOT="${TMP_DIR}" \
    ARTIFACT_PRUNE_MAX_AGE_DAYS=14 \
    ARTIFACT_PRUNE_KEEP_NEWEST=1 \
    bash "${PRUNE_SCRIPT}" --dry-run
) >"${DRY_OUTPUT}"

rg -n --fixed-strings "PRUNE ${TMP_DIR}/artifacts/failures/old-a" "${DRY_OUTPUT}" >/dev/null
rg -n --fixed-strings "KEEP ${TMP_DIR}/artifacts/failures/new-a" "${DRY_OUTPUT}" >/dev/null
[[ -d "${TMP_DIR}/artifacts/failures/old-a" ]] || { echo "dry-run should not remove old-a" >&2; exit 1; }

RUN_OUTPUT="${TMP_DIR}/run.out"
(
  cd "${TMP_DIR}" && \
    ARTIFACT_PRUNE_ROOT="${TMP_DIR}" \
    ARTIFACT_PRUNE_MAX_AGE_DAYS=14 \
    ARTIFACT_PRUNE_KEEP_NEWEST=1 \
    bash "${PRUNE_SCRIPT}"
) >"${RUN_OUTPUT}"

[[ -d "${TMP_DIR}/artifacts/failures/new-a" ]] || { echo "newest failure artifact should be kept" >&2; exit 1; }
[[ ! -d "${TMP_DIR}/artifacts/failures/old-a" ]] || { echo "old failure artifact should be pruned" >&2; exit 1; }
[[ ! -d "${TMP_DIR}/artifacts/repair-loop/old-a" ]] || { echo "old repair-loop artifact should be pruned" >&2; exit 1; }
[[ ! -d "${TMP_DIR}/artifacts/live-local-smoke/old-a" ]] || { echo "old live-local artifact should be pruned" >&2; exit 1; }
[[ -d "${TMP_DIR}/artifacts/keep-me/unrelated" ]] || { echo "unrelated directory should remain untouched" >&2; exit 1; }

echo "test-prune-artifacts.sh: PASS"
