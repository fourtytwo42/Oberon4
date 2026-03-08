#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
source "${ROOT_DIR}/scripts/lib/runtime-env.sh"

: "${REPAIR_LOOP_LOG_DIR:=${ROOT_DIR}/artifacts/repair-loop}"
: "${RUN_LIVE_LOCAL_SMOKE:=0}"
: "${RUN_REPAIR_REMEDIATE:=0}"

RUN_ID="$(date -u +%Y%m%dT%H%M%S)-$$"
RUN_DIR="${REPAIR_LOOP_LOG_DIR}/${RUN_ID}"
SUMMARY_PATH="${RUN_DIR}/summary.json"
DOCTOR_PAYLOAD_PATH="${RUN_DIR}/doctor.json"
REMEDIATION_LOG_PATH="${RUN_DIR}/remediation.log"
mkdir -p "${RUN_DIR}"

doctor_status="pending"
gate_status="pending"
tui_status="pending"
live_status="skipped"
if [[ "${RUN_LIVE_LOCAL_SMOKE}" == "1" ]]; then
  live_status="pending"
fi
failure_bundle_path=""
remediation_attempted="false"
remediation_status="not_requested"

COMMAND_HISTORY=(
  "doctor_json: cargo run -p oberon4-cli -- doctor --json"
  "oberon4_gate: env GATE_MODE=strict bash scripts/oberon4-gate.sh"
  "tui_pty_smoke: bash scripts/tui-pty-smoke.sh"
  "live_local_smoke: bash scripts/live-local-smoke.sh"
  "remediation_build: cargo build -p oberon4-cli -p oberon4-daemon"
  "remediation_daemon_stop: cargo run -p oberon4-cli -- daemon stop"
  "remediation_daemon_start: cargo run -p oberon4-cli -- daemon start"
  "remediation_doctor_json: cargo run -p oberon4-cli -- doctor --json"
)

write_summary() {
  SUMMARY_PATH="${SUMMARY_PATH}" \
  TIMESTAMP_UTC="${RUN_ID}" \
  RUN_DIR="${RUN_DIR}" \
  RUN_LIVE_LOCAL_SMOKE="${RUN_LIVE_LOCAL_SMOKE}" \
  RUN_REPAIR_REMEDIATE="${RUN_REPAIR_REMEDIATE}" \
  DOCTOR_STATUS="${doctor_status}" \
  GATE_STATUS="${gate_status}" \
  TUI_STATUS="${tui_status}" \
  LIVE_STATUS="${live_status}" \
  FAILURE_BUNDLE_PATH="${failure_bundle_path}" \
  REMEDIATION_ATTEMPTED="${remediation_attempted}" \
  REMEDIATION_STATUS="${remediation_status}" \
  node <<'NODE'
const fs = require("fs");
const path = require("path");

const stages = [
  {
    name: "doctor_json",
    status: process.env.DOCTOR_STATUS,
    log_path: path.join(process.env.RUN_DIR, "doctor_json.log"),
  },
  {
    name: "oberon4_gate",
    status: process.env.GATE_STATUS,
    log_path: path.join(process.env.RUN_DIR, "oberon4_gate.log"),
  },
  {
    name: "tui_pty_smoke",
    status: process.env.TUI_STATUS,
    log_path: path.join(process.env.RUN_DIR, "tui_pty_smoke.log"),
  },
  {
    name: "live_local_smoke",
    status: process.env.LIVE_STATUS,
    log_path: path.join(process.env.RUN_DIR, "live_local_smoke.log"),
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
  timestamp_utc: process.env.TIMESTAMP_UTC,
  status: overall,
  run_live_local_smoke: process.env.RUN_LIVE_LOCAL_SMOKE === "1",
  run_repair_remediate: process.env.RUN_REPAIR_REMEDIATE === "1",
  remediation_attempted: process.env.REMEDIATION_ATTEMPTED === "true",
  remediation_status: process.env.REMEDIATION_STATUS,
  doctor_payload_path: path.join(process.env.RUN_DIR, "doctor.json"),
  failure_bundle_path: process.env.FAILURE_BUNDLE_PATH || null,
  stages,
};

fs.writeFileSync(process.env.SUMMARY_PATH, JSON.stringify(summary, null, 2));
NODE
}

set_stage_status() {
  local name="$1"
  local status="$2"
  case "${name}" in
    doctor_json) doctor_status="${status}" ;;
    oberon4_gate) gate_status="${status}" ;;
    tui_pty_smoke) tui_status="${status}" ;;
    live_local_smoke) live_status="${status}" ;;
    *) echo "unknown stage: ${name}" >&2; exit 1 ;;
  esac
}

extract_bundle_path() {
  local log_path="$1"
  if command -v rg >/dev/null 2>&1; then
    rg -o 'bundle=[^ ]+' "${log_path}" | tail -n 1 | sed 's/^bundle=//'
  else
    grep -oE 'bundle=[^ ]+' "${log_path}" | tail -n 1 | sed 's/^bundle=//'
  fi
}

run_remediation() {
  remediation_attempted="true"
  remediation_status="running"
  write_summary

  : >"${REMEDIATION_LOG_PATH}"

  local remediate_cmd

  remediate_cmd=(cargo build -p oberon4-cli -p oberon4-daemon)
  echo "[repair-loop] remediation step=build" | tee -a "${REMEDIATION_LOG_PATH}"
  if ! "${remediate_cmd[@]}" >>"${REMEDIATION_LOG_PATH}" 2>&1; then
    remediation_status="fail"
    write_summary
    echo "[repair-loop] remediation step=build status=fail log=${REMEDIATION_LOG_PATH}" >&2
    return 1
  fi

  remediate_cmd=(cargo run -p oberon4-cli -- daemon stop)
  echo "[repair-loop] remediation step=daemon_stop" | tee -a "${REMEDIATION_LOG_PATH}"
  if ! "${remediate_cmd[@]}" >>"${REMEDIATION_LOG_PATH}" 2>&1; then
    echo "[repair-loop] remediation step=daemon_stop status=best_effort_fail" | tee -a "${REMEDIATION_LOG_PATH}"
  else
    echo "[repair-loop] remediation step=daemon_stop status=pass" | tee -a "${REMEDIATION_LOG_PATH}"
  fi

  remediate_cmd=(cargo run -p oberon4-cli -- daemon start)
  echo "[repair-loop] remediation step=daemon_start" | tee -a "${REMEDIATION_LOG_PATH}"
  if ! "${remediate_cmd[@]}" >>"${REMEDIATION_LOG_PATH}" 2>&1; then
    remediation_status="fail"
    write_summary
    echo "[repair-loop] remediation step=daemon_start status=fail log=${REMEDIATION_LOG_PATH}" >&2
    return 1
  fi

  echo "[repair-loop] remediation step=doctor_json" | tee -a "${REMEDIATION_LOG_PATH}"
  if ! cargo run -p oberon4-cli -- doctor --json | tee "${DOCTOR_PAYLOAD_PATH}" >>"${REMEDIATION_LOG_PATH}" 2>&1; then
    remediation_status="fail"
    write_summary
    echo "[repair-loop] remediation step=doctor_json status=fail log=${REMEDIATION_LOG_PATH}" >&2
    return 1
  fi

  remediation_status="pass"
  write_summary
  echo "[repair-loop] remediation status=pass log=${REMEDIATION_LOG_PATH}"
}

run_stage() {
  local name="$1"
  local log_path="$2"
  shift 2

  echo "[repair-loop] stage=${name}"
  if "$@" >"${log_path}" 2>&1; then
    set_stage_status "${name}" "pass"
    write_summary
    echo "[repair-loop] stage=${name} status=pass"
  else
    set_stage_status "${name}" "fail"
    local bundle_path=""
    bundle_path="$(extract_bundle_path "${log_path}" || true)"
    if [[ -n "${bundle_path}" ]]; then
      failure_bundle_path="${bundle_path}"
    fi
    write_summary
    if [[ -n "${failure_bundle_path}" ]]; then
      echo "[repair-loop] stage=${name} status=fail bundle=${failure_bundle_path} log=${log_path}" >&2
    else
      echo "[repair-loop] stage=${name} status=fail log=${log_path}" >&2
    fi
    exit 1
  fi
}

run_doctor_stage() {
  local log_path="$1"
  local doctor_cmd=(bash -lc "set -o pipefail; cargo run -p oberon4-cli -- doctor --json | tee '${DOCTOR_PAYLOAD_PATH}'")

  echo "[repair-loop] stage=doctor_json"
  if "${doctor_cmd[@]}" >"${log_path}" 2>&1; then
    set_stage_status "doctor_json" "pass"
    write_summary
    echo "[repair-loop] stage=doctor_json status=pass"
    return
  fi

  if [[ "${RUN_REPAIR_REMEDIATE}" == "1" ]]; then
    echo "[repair-loop] stage=doctor_json status=fail remediation=attempting" >&2
    if run_remediation >>"${log_path}" 2>&1; then
      set_stage_status "doctor_json" "pass"
      write_summary
      echo "[repair-loop] stage=doctor_json status=pass remediation=pass"
      return
    fi
  fi

  set_stage_status "doctor_json" "fail"
  write_summary
  echo "[repair-loop] stage=doctor_json status=fail log=${log_path}" >&2
  exit 1
}

write_summary

if bash scripts/prune-artifacts.sh --dry-run >"${RUN_DIR}/prune_artifacts.log" 2>&1; then
  echo "[repair-loop] prune_artifacts_dry_run log=${RUN_DIR}/prune_artifacts.log"
else
  echo "[repair-loop] prune_artifacts_dry_run_failed log=${RUN_DIR}/prune_artifacts.log" >&2
fi

run_doctor_stage "${RUN_DIR}/doctor_json.log"

run_stage \
  "oberon4_gate" \
  "${RUN_DIR}/oberon4_gate.log" \
  env GATE_MODE=strict bash scripts/oberon4-gate.sh

run_stage \
  "tui_pty_smoke" \
  "${RUN_DIR}/tui_pty_smoke.log" \
  bash scripts/tui-pty-smoke.sh

if [[ "${RUN_LIVE_LOCAL_SMOKE}" == "1" ]]; then
  run_stage \
    "live_local_smoke" \
    "${RUN_DIR}/live_local_smoke.log" \
    bash scripts/live-local-smoke.sh
fi

write_summary
echo "[repair-loop] PASS run_dir=${RUN_DIR}"
