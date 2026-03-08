#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

: "${SECURITY_GATE_ROOT:=${ROOT_DIR}}"
: "${SECURITY_GATE_LOG_DIR:=${ROOT_DIR}/artifacts/security-gate}"
: "${RUN_SECRET_SCAN:=1}"
: "${RUN_SAST:=1}"
: "${RUN_DEPENDENCY_AUDIT:=1}"

mkdir -p "${SECURITY_GATE_LOG_DIR}"

COMMAND_HISTORY=()

record_command() {
  local step_name="$1"
  shift
  COMMAND_HISTORY+=("${step_name}: $*")
}

require_tool() {
  local tool_name="$1"
  local step_name="$2"
  if ! command -v "${tool_name}" >/dev/null 2>&1; then
    echo "[security-gate] step=${step_name} status=fail missing_tool=${tool_name}" >&2
    exit 1
  fi
}

run_step() {
  local step_name="$1"
  shift
  local log_path="${SECURITY_GATE_LOG_DIR}/${step_name}.log"
  record_command "${step_name}" "$@"
  echo "[security-gate] step=${step_name}"
  if "$@" >"${log_path}" 2>&1; then
    echo "[security-gate] step=${step_name} status=pass"
  else
    echo "[security-gate] step=${step_name} status=fail log=${log_path}" >&2
    exit 1
  fi
}

echo "[security-gate] root=${SECURITY_GATE_ROOT}"

if [[ "${RUN_SECRET_SCAN}" == "1" ]]; then
  require_tool "gitleaks" "secret_scan"
  run_step secret_scan gitleaks detect --source "${SECURITY_GATE_ROOT}" --redact --no-banner
fi

if [[ "${RUN_SAST}" == "1" ]]; then
  require_tool "semgrep" "sast"
  run_step sast semgrep --error --config auto "${SECURITY_GATE_ROOT}"
fi

if [[ "${RUN_DEPENDENCY_AUDIT}" == "1" ]]; then
  require_tool "cargo-audit" "dependency_audit"
  run_step dependency_audit cargo-audit audit
fi

echo "[security-gate] PASS: Oberon4 security verification satisfied."
