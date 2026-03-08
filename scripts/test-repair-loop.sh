#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPAIR_SCRIPT="${ROOT_DIR}/scripts/repair-loop.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

MOCK_BIN="${TMP_DIR}/bin"
ORDER_FILE="${TMP_DIR}/order.log"
LOG_ROOT="${TMP_DIR}/runs"
mkdir -p "${MOCK_BIN}" "${LOG_ROOT}"

cat > "${MOCK_BIN}/bash" <<'SH'
#!/usr/bin/bash
set -euo pipefail
ORDER_FILE="${ORDER_FILE:?}"
if [[ "${1:-}" == "scripts/oberon4-gate.sh" ]]; then
  printf 'oberon4_gate\n' >>"${ORDER_FILE}"
  if [[ "${MOCK_GATE_FAIL:-0}" == "1" ]]; then
    echo "[gate] failure_bundle stage=cargo_test_workspace bundle=/tmp/mock-gate-bundle log=/tmp/mock-gate.log" >&2
    exit 1
  fi
  echo "[gate] PASS"
  exit 0
fi
if [[ "${1:-}" == "scripts/tui-pty-smoke.sh" ]]; then
  printf 'tui_pty_smoke\n' >>"${ORDER_FILE}"
  if [[ "${MOCK_TUI_FAIL:-0}" == "1" ]]; then
    echo "tui failed" >&2
    exit 1
  fi
  echo "tui ok"
  exit 0
fi
if [[ "${1:-}" == "scripts/live-local-smoke.sh" ]]; then
  printf 'live_local_smoke\n' >>"${ORDER_FILE}"
  if [[ "${MOCK_LIVE_FAIL:-0}" == "1" ]]; then
    echo "[live-local] stage=live_daemon_smoke status=fail bundle=/tmp/mock-live-bundle log=/tmp/mock-live.log" >&2
    exit 1
  fi
  echo "[live-local] PASS"
  exit 0
fi
exec /usr/bin/bash "$@"
SH
chmod +x "${MOCK_BIN}/bash"

cat > "${MOCK_BIN}/cargo" <<'SH'
#!/usr/bin/bash
set -euo pipefail
ORDER_FILE="${ORDER_FILE:?}"
if [[ "${1:-}" == "run" && "${2:-}" == "-p" && "${3:-}" == "oberon4-cli" && "${4:-}" == "--" && "${5:-}" == "doctor" && "${6:-}" == "--json" ]]; then
  printf 'doctor_json\n' >>"${ORDER_FILE}"
  if [[ "${MOCK_DOCTOR_FAIL:-0}" == "1" ]]; then
    echo '{"running":false,"ready":false,"endpoint_check":"fail","error":"mock-doctor-fail"}'
    exit 1
  fi
  echo '{"running":true,"ready":true,"endpoint_check":"pass","error":null,"config_path":"/tmp/config.toml","data_dir":"/tmp/data","socket_path":"/tmp/daemon.sock","api_key_env":"OPENAI_API_KEY","config_exists":true,"daemon_reachable":true}'
  exit 0
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
  case "status":
    value = summary.status;
    break;
  case "run_live_local_smoke":
    value = summary.run_live_local_smoke;
    break;
  case "failure_bundle_path":
    value = summary.failure_bundle_path;
    break;
  case "stages.doctor_json":
    value = summary.stages.find((s) => s.name === "doctor_json")?.status;
    break;
  case "stages.oberon4_gate":
    value = summary.stages.find((s) => s.name === "oberon4_gate")?.status;
    break;
  case "stages.tui_pty_smoke":
    value = summary.stages.find((s) => s.name === "tui_pty_smoke")?.status;
    break;
  case "stages.live_local_smoke":
    value = summary.stages.find((s) => s.name === "live_local_smoke")?.status;
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

run_repair() {
  local output_file="$1"
  shift
  : > "${ORDER_FILE}"
  env PATH="${MOCK_BIN}:$PATH" \
    ORDER_FILE="${ORDER_FILE}" \
    REPAIR_LOOP_LOG_DIR="${LOG_ROOT}" \
    "$@" bash "${REPAIR_SCRIPT}" >"${output_file}" 2>&1
}

SUCCESS_OUTPUT="${TMP_DIR}/success.out"
run_repair "${SUCCESS_OUTPUT}"
SUCCESS_RUN_DIR="$(latest_run_dir)"
SUCCESS_SUMMARY="${SUCCESS_RUN_DIR}/summary.json"
[[ -f "${SUCCESS_SUMMARY}" ]] || { echo "missing summary.json" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/doctor.json" ]] || { echo "missing doctor payload" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/doctor_json.log" ]] || { echo "missing doctor log" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/oberon4_gate.log" ]] || { echo "missing gate log" >&2; exit 1; }
[[ -f "${SUCCESS_RUN_DIR}/tui_pty_smoke.log" ]] || { echo "missing tui log" >&2; exit 1; }
assert_contains "${SUCCESS_OUTPUT}" "[repair-loop] PASS run_dir=${SUCCESS_RUN_DIR}"
assert_contains "${ORDER_FILE}" "doctor_json"
assert_contains "${ORDER_FILE}" "oberon4_gate"
assert_contains "${ORDER_FILE}" "tui_pty_smoke"
[[ "$(summary_value "${SUCCESS_SUMMARY}" status)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" run_live_local_smoke)" == "false" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.doctor_json)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.oberon4_gate)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.tui_pty_smoke)" == "pass" ]] || exit 1
[[ "$(summary_value "${SUCCESS_SUMMARY}" stages.live_local_smoke)" == "skipped" ]] || exit 1

GATE_FAIL_OUTPUT="${TMP_DIR}/gate-fail.out"
if run_repair "${GATE_FAIL_OUTPUT}" MOCK_GATE_FAIL=1; then
  echo "expected repair loop failure when gate fails" >&2
  exit 1
fi
GATE_FAIL_RUN_DIR="$(latest_run_dir)"
GATE_FAIL_SUMMARY="${GATE_FAIL_RUN_DIR}/summary.json"
[[ "$(summary_value "${GATE_FAIL_SUMMARY}" status)" == "fail" ]] || exit 1
[[ "$(summary_value "${GATE_FAIL_SUMMARY}" stages.doctor_json)" == "pass" ]] || exit 1
[[ "$(summary_value "${GATE_FAIL_SUMMARY}" stages.oberon4_gate)" == "fail" ]] || exit 1
[[ "$(summary_value "${GATE_FAIL_SUMMARY}" stages.tui_pty_smoke)" == "pending" ]] || exit 1
[[ "$(summary_value "${GATE_FAIL_SUMMARY}" failure_bundle_path)" == "/tmp/mock-gate-bundle" ]] || exit 1
assert_contains "${GATE_FAIL_OUTPUT}" "[repair-loop] stage=oberon4_gate status=fail bundle=/tmp/mock-gate-bundle log=${GATE_FAIL_RUN_DIR}/oberon4_gate.log"

LIVE_SUCCESS_OUTPUT="${TMP_DIR}/live-success.out"
run_repair "${LIVE_SUCCESS_OUTPUT}" RUN_LIVE_LOCAL_SMOKE=1
LIVE_SUCCESS_RUN_DIR="$(latest_run_dir)"
LIVE_SUCCESS_SUMMARY="${LIVE_SUCCESS_RUN_DIR}/summary.json"
[[ "$(summary_value "${LIVE_SUCCESS_SUMMARY}" run_live_local_smoke)" == "true" ]] || exit 1
[[ "$(summary_value "${LIVE_SUCCESS_SUMMARY}" stages.live_local_smoke)" == "pass" ]] || exit 1
assert_contains "${ORDER_FILE}" "live_local_smoke"

LIVE_FAIL_OUTPUT="${TMP_DIR}/live-fail.out"
if run_repair "${LIVE_FAIL_OUTPUT}" RUN_LIVE_LOCAL_SMOKE=1 MOCK_LIVE_FAIL=1; then
  echo "expected repair loop failure when live wrapper fails" >&2
  exit 1
fi
LIVE_FAIL_RUN_DIR="$(latest_run_dir)"
LIVE_FAIL_SUMMARY="${LIVE_FAIL_RUN_DIR}/summary.json"
[[ "$(summary_value "${LIVE_FAIL_SUMMARY}" stages.live_local_smoke)" == "fail" ]] || exit 1
[[ "$(summary_value "${LIVE_FAIL_SUMMARY}" failure_bundle_path)" == "/tmp/mock-live-bundle" ]] || exit 1
assert_contains "${LIVE_FAIL_OUTPUT}" "[repair-loop] stage=live_local_smoke status=fail bundle=/tmp/mock-live-bundle log=${LIVE_FAIL_RUN_DIR}/live_local_smoke.log"

DOCTOR_FAIL_OUTPUT="${TMP_DIR}/doctor-fail.out"
if run_repair "${DOCTOR_FAIL_OUTPUT}" MOCK_DOCTOR_FAIL=1; then
  echo "expected repair loop failure when doctor fails" >&2
  exit 1
fi
DOCTOR_FAIL_RUN_DIR="$(latest_run_dir)"
DOCTOR_FAIL_SUMMARY="${DOCTOR_FAIL_RUN_DIR}/summary.json"
[[ "$(summary_value "${DOCTOR_FAIL_SUMMARY}" stages.doctor_json)" == "fail" ]] || exit 1
[[ "$(summary_value "${DOCTOR_FAIL_SUMMARY}" stages.oberon4_gate)" == "pending" ]] || exit 1
[[ "$(summary_value "${DOCTOR_FAIL_SUMMARY}" failure_bundle_path)" == "null" ]] || exit 1
assert_contains "${DOCTOR_FAIL_OUTPUT}" "[repair-loop] stage=doctor_json status=fail log=${DOCTOR_FAIL_RUN_DIR}/doctor_json.log"

echo "test-repair-loop.sh: PASS"
