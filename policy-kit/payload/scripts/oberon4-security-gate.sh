#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

: "${SECURITY_GATE_ROOT:=${ROOT_DIR}}"
: "${SECURITY_GATE_LOG_DIR:=${ROOT_DIR}/artifacts/security-gate}"
: "${RUN_SECRET_SCAN:=1}"
: "${RUN_SAST:=1}"
: "${RUN_DEPENDENCY_AUDIT:=1}"
: "${SEMGREP_CONFIG_PATH:=${ROOT_DIR}/security/semgrep.yml}"
: "${CARGO_DENY_CONFIG_PATH:=${ROOT_DIR}/deny.toml}"
: "${GITLEAKS_CONFIG_PATH:=${ROOT_DIR}/security/gitleaks.toml}"
: "${GITLEAKS_BASELINE_PATH:=${ROOT_DIR}/security/gitleaks-baseline.json}"

RUN_ID="$(date -u +%Y%m%dT%H%M%S)-$$"
RUN_DIR="${SECURITY_GATE_LOG_DIR%/}/${RUN_ID}"
SUMMARY_PATH="${RUN_DIR}/summary.json"
mkdir -p "${RUN_DIR}"

secret_scan_status="skipped"
sast_status="skipped"
dependency_audit_status="skipped"
dependency_policy_status="skipped"
[[ "${RUN_SECRET_SCAN}" == "1" ]] && secret_scan_status="pending"
[[ "${RUN_SAST}" == "1" ]] && sast_status="pending"
[[ "${RUN_DEPENDENCY_AUDIT}" == "1" ]] && dependency_audit_status="pending"
[[ "${RUN_DEPENDENCY_AUDIT}" == "1" ]] && dependency_policy_status="pending"

COMMAND_HISTORY=()

record_command() {
  local step_name="$1"
  shift
  COMMAND_HISTORY+=("${step_name}: $*")
}

write_commands_file() {
  local output_path="$1"
  printf '%s\n' "${COMMAND_HISTORY[@]}" >"${output_path}"
}

set_stage_status() {
  local step_name="$1"
  local status="$2"
  case "${step_name}" in
    secret_scan) secret_scan_status="${status}" ;;
    sast) sast_status="${status}" ;;
    dependency_audit) dependency_audit_status="${status}" ;;
    dependency_policy) dependency_policy_status="${status}" ;;
    *) echo "unknown stage ${step_name}" >&2; exit 1 ;;
  esac
}

write_summary() {
  SUMMARY_PATH="${SUMMARY_PATH}" \
  RUN_DIR="${RUN_DIR}" \
  RUN_ID="${RUN_ID}" \
  SECURITY_GATE_ROOT="${SECURITY_GATE_ROOT}" \
  SECRET_SCAN_STATUS="${secret_scan_status}" \
  SAST_STATUS="${sast_status}" \
  DEPENDENCY_AUDIT_STATUS="${dependency_audit_status}" \
  DEPENDENCY_POLICY_STATUS="${dependency_policy_status}" \
  node <<'NODE'
const fs = require("fs");
const path = require("path");

const stages = [
  {
    name: "secret_scan",
    status: process.env.SECRET_SCAN_STATUS,
    log_path: path.join(process.env.RUN_DIR, "secret_scan.log"),
    report_path: path.join(process.env.RUN_DIR, "gitleaks-report.json"),
  },
  {
    name: "sast",
    status: process.env.SAST_STATUS,
    log_path: path.join(process.env.RUN_DIR, "sast.log"),
    report_path: path.join(process.env.RUN_DIR, "semgrep-report.json"),
  },
  {
    name: "dependency_audit",
    status: process.env.DEPENDENCY_AUDIT_STATUS,
    log_path: path.join(process.env.RUN_DIR, "dependency_audit.log"),
    report_path: path.join(process.env.RUN_DIR, "cargo-audit-report.json"),
  },
  {
    name: "dependency_policy",
    status: process.env.DEPENDENCY_POLICY_STATUS,
    log_path: path.join(process.env.RUN_DIR, "dependency_policy.log"),
    report_path: path.join(process.env.RUN_DIR, "cargo-deny-report.json"),
  },
];

const statuses = stages.map((stage) => stage.status);
const overall =
  statuses.includes("fail")
    ? "fail"
    : statuses.every((status) => status === "pass" || status === "skipped")
      ? "pass"
      : "running";

const summary = {
  timestamp_utc: process.env.RUN_ID,
  root: process.env.SECURITY_GATE_ROOT,
  status: overall,
  stages,
};

fs.writeFileSync(process.env.SUMMARY_PATH, JSON.stringify(summary, null, 2));
NODE
}

require_tool() {
  local tool_name="$1"
  local step_name="$2"
  if ! command -v "${tool_name}" >/dev/null 2>&1; then
    set_stage_status "${step_name}" "fail"
    write_summary
    echo "[security-gate] stage=${step_name} status=fail missing_tool=${tool_name} run_dir=${RUN_DIR}" >&2
    exit 1
  fi
}

run_step() {
  local step_name="$1"
  local log_path="$2"
  shift 2
  record_command "${step_name}" "$@"
  echo "[security-gate] stage=${step_name}"
  if "$@" >"${log_path}" 2>&1; then
    set_stage_status "${step_name}" "pass"
    write_summary
    echo "[security-gate] stage=${step_name} status=pass"
  else
    set_stage_status "${step_name}" "fail"
    write_summary
    local report_path=""
    case "${step_name}" in
      secret_scan) report_path="${RUN_DIR}/gitleaks-report.json" ;;
      sast) report_path="${RUN_DIR}/semgrep-report.json" ;;
      dependency_audit) report_path="${RUN_DIR}/cargo-audit-report.json" ;;
      dependency_policy) report_path="${RUN_DIR}/cargo-deny-report.json" ;;
    esac
    if [[ -n "${report_path}" ]]; then
      echo "[security-gate] stage=${step_name} status=fail run_dir=${RUN_DIR} log=${log_path} report=${report_path}" >&2
    else
      echo "[security-gate] stage=${step_name} status=fail run_dir=${RUN_DIR} log=${log_path}" >&2
    fi
    exit 1
  fi
}

write_summary

echo "[security-gate] root=${SECURITY_GATE_ROOT}"
echo "[security-gate] run_dir=${RUN_DIR}"

if [[ "${RUN_SECRET_SCAN}" == "1" ]]; then
  require_tool "gitleaks" "secret_scan"
  run_step \
    "secret_scan" \
    "${RUN_DIR}/secret_scan.log" \
    gitleaks detect \
      --source "${SECURITY_GATE_ROOT}" \
      --config "${GITLEAKS_CONFIG_PATH}" \
      --baseline-path "${GITLEAKS_BASELINE_PATH}" \
      --report-format json \
      --report-path "${RUN_DIR}/gitleaks-report.json" \
      --redact \
      --no-banner
fi

if [[ "${RUN_SAST}" == "1" ]]; then
  require_tool "semgrep" "sast"
  run_step \
    "sast" \
    "${RUN_DIR}/sast.log" \
    semgrep scan \
      --error \
      --config "${SEMGREP_CONFIG_PATH}" \
      --json \
      --output "${RUN_DIR}/semgrep-report.json" \
      "${SECURITY_GATE_ROOT}"
fi

if [[ "${RUN_DEPENDENCY_AUDIT}" == "1" ]]; then
  require_tool "cargo-audit" "dependency_audit"
  run_step \
    "dependency_audit" \
    "${RUN_DIR}/dependency_audit.log" \
    bash -lc "cargo-audit audit --json > '${RUN_DIR}/cargo-audit-report.json'"

  require_tool "cargo-deny" "dependency_policy"
  run_step \
    "dependency_policy" \
    "${RUN_DIR}/dependency_policy.log" \
    bash -lc "cargo-deny --manifest-path '${ROOT_DIR}/Cargo.toml' --format json check --config '${CARGO_DENY_CONFIG_PATH}' advisories licenses bans > '${RUN_DIR}/cargo-deny-report.json'"
fi

write_commands_file "${RUN_DIR}/commands.txt"
write_summary
echo "[security-gate] PASS: Oberon4 security verification satisfied. run_dir=${RUN_DIR}"
