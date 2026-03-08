#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
WRAPPER_SCRIPT="${ROOT_DIR}/scripts/live-local-smoke.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

MOCK_BIN="${TMP_DIR}/bin"
ORDER_FILE="${TMP_DIR}/order.log"
LOG_ROOT="${TMP_DIR}/runs"
BUNDLE_ROOT="${TMP_DIR}/failure-bundles"
mkdir -p "${MOCK_BIN}" "${LOG_ROOT}" "${BUNDLE_ROOT}"

cat > "${MOCK_BIN}/bash" <<'SH'
#!/usr/bin/bash
set -euo pipefail
ORDER_FILE="${ORDER_FILE:?}"
if [[ "${1:-}" == "scripts/live-hotspot-smoke.sh" ]]; then
  printf 'live_hotspot_smoke\n' >>"${ORDER_FILE}"
  if [[ "${MOCK_LIVE_HOTSPOT_FAIL:-0}" == "1" ]]; then
    echo "live hotspot failed" >&2
    exit 1
  fi
  printf 'base=%s model=%s key_env=%s\n' \
    "${OBERON4_LIVE_BASE_URL:-}" \
    "${OBERON4_LIVE_OPENAI_MODEL:-}" \
    "${OBERON4_LIVE_API_KEY_ENV:-}"
  exit 0
fi
if [[ "${1:-}" == "scripts/live-daemon-smoke.sh" ]]; then
  printf 'live_daemon_smoke\n' >>"${ORDER_FILE}"
  if [[ "${MOCK_LIVE_DAEMON_FAIL:-0}" == "1" ]]; then
    echo "live daemon failed" >&2
    exit 1
  fi
  printf 'daemon smoke ok\n'
  exit 0
fi
exec /usr/bin/bash "$@"
SH
chmod +x "${MOCK_BIN}/bash"

cat > "${MOCK_BIN}/cargo" <<'SH'
#!/usr/bin/bash
set -euo pipefail
ORDER_FILE="${ORDER_FILE:?}"
if [[ "${1:-}" == "run" && "${2:-}" == "-p" && "${3:-}" == "oberon4-cli" && "${4:-}" == "--" ]]; then
  shift 4
  case "${1:-}" in
    daemon)
      case "${2:-}" in
        start)
          printf 'doctor_daemon_start\n' >>"${ORDER_FILE}"
          echo "running=true endpoint=${OBERON4_LIVE_BASE_URL:-} model=${OBERON4_LIVE_OPENAI_MODEL:-}"
          exit 0
          ;;
        stop)
          printf 'doctor_daemon_stop\n' >>"${ORDER_FILE}"
          echo "running=false"
          exit 0
          ;;
      esac
      ;;
    config)
      printf 'doctor_config_%s\n' "${3:-unknown}" >>"${ORDER_FILE}"
      echo "set ${3:-}=${4:-}"
      exit 0
      ;;
    doctor)
      printf 'doctor_json\n' >>"${ORDER_FILE}"
      if [[ "${MOCK_DOCTOR_FAIL:-0}" == "1" ]]; then
        echo '{"running":false,"ready":false,"endpoint_check":"fail","error":"bad"}'
        exit 0
      fi
      if [[ "${MOCK_DOCTOR_INVALID_JSON:-0}" == "1" ]]; then
        echo 'not-json'
        exit 0
      fi
      printf '{"running":true,"ready":true,"endpoint_base_url":"%s","model":"%s","endpoint_check":"pass","error":null}\n' \
        "${OBERON4_LIVE_BASE_URL:-}" \
        "${OBERON4_LIVE_OPENAI_MODEL:-}"
      exit 0
      ;;
  esac
fi
echo "unexpected cargo invocation: $*" >&2
exit 1
SH
chmod +x "${MOCK_BIN}/cargo"

assert_contains() {
  local file="$1"
  local pattern="$2"
  if ! rg -n --fixed-strings "${pattern}" "${file}" >/dev/null; then
    echo "assertion failed: expected pattern '${pattern}' in ${file}" >&2
    cat "${file}" >&2
    exit 1
  fi
}

summary_value() {
  local summary_path="$1"
  local expr="$2"
  SUMMARY_PATH="${summary_path}" EXPR="${expr}" node <<'NODE'
const fs = require("fs");
const summary = JSON.parse(fs.readFileSync(process.env.SUMMARY_PATH, "utf8"));
const expr = process.env.EXPR;
let value;
switch (expr) {
  case "base_url":
    value = summary.base_url;
    break;
  case "model":
    value = summary.model;
    break;
  case "api_key_env":
    value = summary.api_key_env;
    break;
  case "status":
    value = summary.status;
    break;
  case "stages.live_hotspot_smoke":
    value = summary.stages.find((s) => s.name === "live_hotspot_smoke")?.status;
    break;
  case "stages.live_daemon_smoke":
    value = summary.stages.find((s) => s.name === "live_daemon_smoke")?.status;
    break;
  case "stages.doctor_json":
    value = summary.stages.find((s) => s.name === "doctor_json")?.status;
    break;
  default:
    throw new Error(`unsupported expr: ${expr}`);
}
process.stdout.write(String(value));
NODE
}

latest_run_dir() {
  find "${LOG_ROOT}" -mindepth 1 -maxdepth 1 -type d | sort | tail -n 1
}

run_wrapper() {
  local output_file="$1"
  shift
  : > "${ORDER_FILE}"
  env PATH="${MOCK_BIN}:$PATH" \
    ORDER_FILE="${ORDER_FILE}" \
    LIVE_LOCAL_LOG_DIR="${LOG_ROOT}" \
    FAILURE_BUNDLE_ROOT="${BUNDLE_ROOT}" \
    "$@" bash "${WRAPPER_SCRIPT}" >"${output_file}" 2>&1
}

latest_bundle_dir() {
  find "${BUNDLE_ROOT}" -mindepth 1 -maxdepth 1 -type d | sort | tail -n 1
}

SUCCESS_OUTPUT="${TMP_DIR}/success.out"
run_wrapper "${SUCCESS_OUTPUT}"
SUCCESS_RUN_DIR="$(latest_run_dir)"
SUCCESS_SUMMARY="${SUCCESS_RUN_DIR}/summary.json"

assert_contains "${SUCCESS_OUTPUT}" "[live-local] PASS run_dir=${SUCCESS_RUN_DIR}"
[[ -f "${SUCCESS_SUMMARY}" ]] || { echo "missing summary.json" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/live_hotspot_smoke.log" ]] || { echo "missing hotspot log" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/live_daemon_smoke.log" ]] || { echo "missing daemon log" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/doctor_json.log" ]] || { echo "missing doctor log" >&2; exit 1; }

[[ "$(summary_value "${SUCCESS_SUMMARY}" base_url)" == "http://192.168.50.239:51824/v1" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" model)" == "gpt-5.1-codex-mini" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" api_key_env)" == "OPENAI_API_KEY" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" status)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.live_hotspot_smoke)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.live_daemon_smoke)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.doctor_json)" == "pass" ]] || exit 1
assert_contains "${ORDER_FILE}" "live_hotspot_smoke"
assert_contains "${ORDER_FILE}" "live_daemon_smoke"
assert_contains "${ORDER_FILE}" "doctor_json"

FAIL_OUTPUT="${TMP_DIR}/fail.out"
if run_wrapper "${FAIL_OUTPUT}" \
  MOCK_LIVE_DAEMON_FAIL=1 \
  OBERON4_LIVE_API_KEY_ENV=TEST_LIVE_KEY \
  TEST_LIVE_KEY=super-secret-token; then
  echo "expected wrapper failure when daemon stage fails" >&2
  exit 1
fi
FAIL_RUN_DIR="$(latest_run_dir)"
FAIL_SUMMARY="${FAIL_RUN_DIR}/summary.json"
FAIL_BUNDLE_DIR="$(latest_bundle_dir)"
[[ "$(summary_value "${FAIL_SUMMARY}" status)" == "fail" ]] || exit 1
[[ "$(summary_value "${FAIL_SUMMARY}" stages.live_hotspot_smoke)" == "pass" ]] || exit 1
[[ "$(summary_value "${FAIL_SUMMARY}" stages.live_daemon_smoke)" == "fail" ]] || exit 1
[[ "$(summary_value "${FAIL_SUMMARY}" stages.doctor_json)" == "pending" ]] || exit 1
[[ -f "${FAIL_BUNDLE_DIR}/summary.json" ]] || { echo "missing failure summary" >&2; exit 1; }
[[ -f "${FAIL_BUNDLE_DIR}/commands.txt" ]] || { echo "missing failure commands.txt" >&2; exit 1; }
[[ -f "${FAIL_BUNDLE_DIR}/env.txt" ]] || { echo "missing failure env.txt" >&2; exit 1; }
[[ -f "${FAIL_BUNDLE_DIR}/doctor.json" ]] || { echo "missing fallback doctor.json" >&2; exit 1; }
[[ -f "${FAIL_BUNDLE_DIR}/logs/live_daemon_smoke.log" ]] || { echo "missing daemon log in bundle" >&2; exit 1; }
assert_contains "${FAIL_OUTPUT}" "stage=live_daemon_smoke status=fail bundle=${FAIL_BUNDLE_DIR} log=${FAIL_RUN_DIR}/live_daemon_smoke.log"
assert_contains "${FAIL_BUNDLE_DIR}/commands.txt" "live_daemon_smoke: bash scripts/live-daemon-smoke.sh"
assert_contains "${FAIL_BUNDLE_DIR}/env.txt" "OBERON4_LIVE_API_KEY_ENV=TEST_LIVE_KEY"
assert_contains "${FAIL_BUNDLE_DIR}/env.txt" "OBERON4_LIVE_API_KEY_STATUS=set"
if rg -n --fixed-strings "super-secret-token" "${FAIL_BUNDLE_DIR}/env.txt" >/dev/null; then
  echo "assertion failed: secret value leaked into live bundle env.txt" >&2
  exit 1
fi

HOTSPOT_FAIL_OUTPUT="${TMP_DIR}/hotspot-fail.out"
if run_wrapper "${HOTSPOT_FAIL_OUTPUT}" MOCK_LIVE_HOTSPOT_FAIL=1; then
  echo "expected wrapper failure when hotspot stage fails" >&2
  exit 1
fi
HOTSPOT_FAIL_RUN_DIR="$(latest_run_dir)"
HOTSPOT_FAIL_SUMMARY="${HOTSPOT_FAIL_RUN_DIR}/summary.json"
HOTSPOT_FAIL_BUNDLE_DIR="$(latest_bundle_dir)"
[[ "$(summary_value "${HOTSPOT_FAIL_SUMMARY}" stages.live_hotspot_smoke)" == "fail" ]] || exit 1
[[ "$(summary_value "${HOTSPOT_FAIL_SUMMARY}" stages.live_daemon_smoke)" == "pending" ]] || exit 1
assert_contains "${HOTSPOT_FAIL_OUTPUT}" "stage=live_hotspot_smoke status=fail bundle=${HOTSPOT_FAIL_BUNDLE_DIR} log=${HOTSPOT_FAIL_RUN_DIR}/live_hotspot_smoke.log"
[[ -f "${HOTSPOT_FAIL_BUNDLE_DIR}/logs/live_hotspot_smoke.log" ]] || { echo "missing hotspot log in bundle" >&2; exit 1; }

DOCTOR_FAIL_OUTPUT="${TMP_DIR}/doctor-fail.out"
if run_wrapper "${DOCTOR_FAIL_OUTPUT}" MOCK_DOCTOR_INVALID_JSON=1; then
  echo "expected wrapper failure when doctor JSON is invalid" >&2
  exit 1
fi
DOCTOR_FAIL_RUN_DIR="$(latest_run_dir)"
DOCTOR_FAIL_SUMMARY="${DOCTOR_FAIL_RUN_DIR}/summary.json"
DOCTOR_FAIL_BUNDLE_DIR="$(latest_bundle_dir)"
[[ "$(summary_value "${DOCTOR_FAIL_SUMMARY}" stages.doctor_json)" == "fail" ]] || exit 1
assert_contains "${DOCTOR_FAIL_OUTPUT}" "stage=doctor_json status=fail bundle=${DOCTOR_FAIL_BUNDLE_DIR} log=${DOCTOR_FAIL_RUN_DIR}/doctor_json.log"
[[ -f "${DOCTOR_FAIL_BUNDLE_DIR}/doctor.json" ]] || { echo "missing doctor.json in bundle" >&2; exit 1; }
assert_contains "${DOCTOR_FAIL_BUNDLE_DIR}/doctor.json" "not-json"

echo "test-live-local-smoke.sh: PASS"
