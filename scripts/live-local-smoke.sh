#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
source "${ROOT_DIR}/scripts/lib/failure-bundle.sh"

: "${OBERON4_LIVE_BASE_URL:=http://192.168.50.239:51824/v1}"
: "${OBERON4_LIVE_OPENAI_MODEL:=gpt-5.1-codex-mini}"
: "${OBERON4_LIVE_API_KEY_ENV:=OPENAI_API_KEY}"
: "${LIVE_LOCAL_LOG_DIR:=${ROOT_DIR}/artifacts/live-local-smoke}"

export OBERON4_LIVE_BASE_URL
export OBERON4_LIVE_OPENAI_MODEL
export OBERON4_LIVE_API_KEY_ENV

RUN_ID="$(date -u +%Y%m%dT%H%M%S)-$$"
RUN_DIR="${LIVE_LOCAL_LOG_DIR}/${RUN_ID}"
SUMMARY_PATH="${RUN_DIR}/summary.json"
DOCTOR_DATA_DIR="${RUN_DIR}/doctor-data"
DOCTOR_SOCKET_PATH="${DOCTOR_DATA_DIR}/daemon.sock"
DOCTOR_OUTPUT_PATH="${RUN_DIR}/doctor-output.json"
DOCTOR_BASE_URL="${OBERON4_LIVE_BASE_URL%/}"
DOCTOR_BASE_URL="${DOCTOR_BASE_URL%/v1}"
mkdir -p "${RUN_DIR}" "${DOCTOR_DATA_DIR}"

hotspot_status="pending"
daemon_status="pending"
doctor_status="pending"
COMMAND_HISTORY=(
  "live_hotspot_smoke: bash scripts/live-hotspot-smoke.sh"
  "live_daemon_smoke: bash scripts/live-daemon-smoke.sh"
  "doctor_json: cargo run -p oberon4-cli -- doctor --json"
)

write_summary() {
  SUMMARY_PATH="${SUMMARY_PATH}" \
  TIMESTAMP_UTC="${RUN_ID}" \
  BASE_URL="${OBERON4_LIVE_BASE_URL}" \
  MODEL="${OBERON4_LIVE_OPENAI_MODEL}" \
  API_KEY_ENV="${OBERON4_LIVE_API_KEY_ENV}" \
  RUN_DIR="${RUN_DIR}" \
  HOTSPOT_STATUS="${hotspot_status}" \
  DAEMON_STATUS="${daemon_status}" \
  DOCTOR_STATUS="${doctor_status}" \
  node <<'NODE'
const fs = require("fs");
const path = require("path");

const statuses = [
  {
    name: "live_hotspot_smoke",
    status: process.env.HOTSPOT_STATUS,
    log_path: path.join(process.env.RUN_DIR, "live_hotspot_smoke.log"),
  },
  {
    name: "live_daemon_smoke",
    status: process.env.DAEMON_STATUS,
    log_path: path.join(process.env.RUN_DIR, "live_daemon_smoke.log"),
  },
  {
    name: "doctor_json",
    status: process.env.DOCTOR_STATUS,
    log_path: path.join(process.env.RUN_DIR, "doctor_json.log"),
  },
];

const overall = statuses.some((stage) => stage.status === "fail")
  ? "fail"
  : statuses.every((stage) => stage.status === "pass")
    ? "pass"
    : "running";

const summary = {
  timestamp_utc: process.env.TIMESTAMP_UTC,
  base_url: process.env.BASE_URL,
  model: process.env.MODEL,
  api_key_env: process.env.API_KEY_ENV,
  status: overall,
  stages: statuses,
};

fs.writeFileSync(process.env.SUMMARY_PATH, JSON.stringify(summary, null, 2));
NODE
}

set_stage_status() {
  local name="$1"
  local status="$2"
  case "$name" in
    live_hotspot_smoke) hotspot_status="$status" ;;
    live_daemon_smoke) daemon_status="$status" ;;
    doctor_json) doctor_status="$status" ;;
    *) echo "unknown stage: ${name}" >&2; exit 1 ;;
  esac
}

write_commands_file() {
  local output_path="$1"
  printf '%s\n' "${COMMAND_HISTORY[@]}" >"${output_path}"
}

write_live_env_file() {
  local output_path="$1"
  {
    printf 'OBERON4_LIVE_BASE_URL=%s\n' "${OBERON4_LIVE_BASE_URL}"
    printf 'OBERON4_LIVE_OPENAI_MODEL=%s\n' "${OBERON4_LIVE_OPENAI_MODEL}"
    printf 'OBERON4_LIVE_API_KEY_ENV=%s\n' "${OBERON4_LIVE_API_KEY_ENV}"
    if [[ -n "${OBERON4_LIVE_API_KEY_ENV}" ]]; then
      if [[ -n "${!OBERON4_LIVE_API_KEY_ENV:-}" ]]; then
        printf 'OBERON4_LIVE_API_KEY_STATUS=set\n'
      else
        printf 'OBERON4_LIVE_API_KEY_STATUS=unset\n'
      fi
    fi
  } >"${output_path}"
}

run_doctor_json_command() {
  local cli_env=(
    env
    "OBERON4_DATA_DIR=${DOCTOR_DATA_DIR}"
    "OBERON4_SOCKET_PATH=${DOCTOR_SOCKET_PATH}"
  )
  local daemon_bin="${ROOT_DIR}/target/debug/oberon4-daemon"
  if [[ -x "${daemon_bin}" ]]; then
    cli_env+=("OBERON4_DAEMON_BIN=${daemon_bin}")
  fi

  local started=0
  cleanup() {
    if [[ "${started:-0}" == "1" ]]; then
      "${cli_env[@]}" cargo run -p oberon4-cli -- daemon stop >/dev/null 2>&1 || true
    fi
  }
  trap cleanup RETURN

  "${cli_env[@]}" cargo run -p oberon4-cli -- daemon start >/dev/null
  started=1
  "${cli_env[@]}" cargo run -p oberon4-cli -- config set endpoint.base_url "${DOCTOR_BASE_URL}" >/dev/null
  "${cli_env[@]}" cargo run -p oberon4-cli -- config set endpoint.model "${OBERON4_LIVE_OPENAI_MODEL}" >/dev/null
  if [[ -n "${OBERON4_LIVE_API_KEY_ENV}" ]]; then
    "${cli_env[@]}" cargo run -p oberon4-cli -- config set endpoint.api_key_env "${OBERON4_LIVE_API_KEY_ENV}" >/dev/null
  fi

  "${cli_env[@]}" cargo run -p oberon4-cli -- doctor --json
}

run_doctor_config_show_command() {
  local cli_env=(
    env
    "OBERON4_DATA_DIR=${DOCTOR_DATA_DIR}"
    "OBERON4_SOCKET_PATH=${DOCTOR_SOCKET_PATH}"
  )
  local daemon_bin="${ROOT_DIR}/target/debug/oberon4-daemon"
  if [[ -x "${daemon_bin}" ]]; then
    cli_env+=("OBERON4_DAEMON_BIN=${daemon_bin}")
  fi

  local started=0
  cleanup() {
    if [[ "${started:-0}" == "1" ]]; then
      "${cli_env[@]}" cargo run -p oberon4-cli -- daemon stop >/dev/null 2>&1 || true
    fi
  }
  trap cleanup RETURN

  "${cli_env[@]}" cargo run -p oberon4-cli -- daemon start >/dev/null
  started=1
  "${cli_env[@]}" cargo run -p oberon4-cli -- config set endpoint.base_url "${DOCTOR_BASE_URL}" >/dev/null
  "${cli_env[@]}" cargo run -p oberon4-cli -- config set endpoint.model "${OBERON4_LIVE_OPENAI_MODEL}" >/dev/null
  if [[ -n "${OBERON4_LIVE_API_KEY_ENV}" ]]; then
    "${cli_env[@]}" cargo run -p oberon4-cli -- config set endpoint.api_key_env "${OBERON4_LIVE_API_KEY_ENV}" >/dev/null
  fi

  "${cli_env[@]}" cargo run -p oberon4-cli -- config show
}

capture_live_failure_bundle() {
  local failed_stage="$1"
  local primary_log_path="$2"
  local bundle_dir
  bundle_dir="$(failure_bundle_create_dir "${ROOT_DIR}" "live-local-smoke")"
  mkdir -p "${bundle_dir}/logs"
  if [[ -f "${SUMMARY_PATH}" ]]; then
    cp "${SUMMARY_PATH}" "${bundle_dir}/summary.json"
  fi
  failure_bundle_write_metadata \
    "${bundle_dir}/bundle-meta.json" \
    "live-local-smoke" \
    "${failed_stage}" \
    "${primary_log_path}"
  write_commands_file "${bundle_dir}/commands.txt"
  write_live_env_file "${bundle_dir}/env.txt"
  find "${RUN_DIR}" -maxdepth 1 -type f -name '*.log' -print0 | while IFS= read -r -d '' log_file; do
    failure_bundle_copy_file "${log_file}" "${bundle_dir}/logs"
  done
  if [[ -f "${DOCTOR_OUTPUT_PATH}" ]]; then
    cp "${DOCTOR_OUTPUT_PATH}" "${bundle_dir}/doctor.json"
    printf 'exit_code=0\nsource=wrapper-stage\n' >"${bundle_dir}/doctor-status.txt"
  else
    failure_bundle_capture_doctor "${bundle_dir}" run_doctor_json_command
  fi
  failure_bundle_capture_effective_config "${bundle_dir}" run_doctor_config_show_command
  printf '%s\n' "${bundle_dir}"
}

run_stage() {
  local name="$1"
  local log_path="$2"
  shift 2

  echo "[live-local] stage=${name}"
  if "$@" >"${log_path}" 2>&1; then
    set_stage_status "${name}" "pass"
    write_summary
    echo "[live-local] stage=${name} status=pass"
  else
    set_stage_status "${name}" "fail"
    write_summary
    local bundle_dir
    bundle_dir="$(capture_live_failure_bundle "${name}" "${log_path}")"
    echo "[live-local] stage=${name} status=fail bundle=${bundle_dir} log=${log_path}" >&2
    exit 1
  fi
}

run_doctor_json_stage() {
  local doctor_output
  doctor_output="$(run_doctor_json_command)"
  printf '%s\n' "${doctor_output}" >"${DOCTOR_OUTPUT_PATH}"
  printf '%s\n' "${doctor_output}"
  printf '%s' "${doctor_output}" | node -e '
let input = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  input += chunk;
});
process.stdin.on("end", () => {
  let decoded;
  try {
    decoded = JSON.parse(input);
  } catch (err) {
    console.error(`doctor output was not valid JSON: ${err.message}`);
    process.exit(1);
  }

  if (
    decoded.running !== true ||
    decoded.ready !== true ||
    decoded.endpoint_check !== "pass"
  ) {
    console.error(`doctor JSON did not report healthy state: ${input}`);
    process.exit(1);
  }
});
'
}

write_summary

run_stage \
  "live_hotspot_smoke" \
  "${RUN_DIR}/live_hotspot_smoke.log" \
  bash scripts/live-hotspot-smoke.sh

run_stage \
  "live_daemon_smoke" \
  "${RUN_DIR}/live_daemon_smoke.log" \
  bash scripts/live-daemon-smoke.sh

run_stage \
  "doctor_json" \
  "${RUN_DIR}/doctor_json.log" \
  run_doctor_json_stage

echo "[live-local] PASS run_dir=${RUN_DIR}"
