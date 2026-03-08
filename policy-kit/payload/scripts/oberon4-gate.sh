#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

: "${GATE_MODE:=strict}"
run_format_check_set=0
run_clippy_check_set=0
run_workspace_tests_set=0
run_e2e_tests_set=0
run_live_endpoint_smoke_set=0
run_live_daemon_smoke_set=0
[[ -v RUN_FORMAT_CHECK ]] && run_format_check_set=1
[[ -v RUN_CLIPPY_CHECK ]] && run_clippy_check_set=1
[[ -v RUN_WORKSPACE_TESTS ]] && run_workspace_tests_set=1
[[ -v RUN_E2E_TESTS ]] && run_e2e_tests_set=1
[[ -v RUN_LIVE_ENDPOINT_SMOKE ]] && run_live_endpoint_smoke_set=1
[[ -v RUN_LIVE_DAEMON_SMOKE ]] && run_live_daemon_smoke_set=1
: "${RUN_FORMAT_CHECK:=1}"
: "${RUN_CLIPPY_CHECK:=1}"
: "${RUN_WORKSPACE_TESTS:=1}"
: "${RUN_E2E_TESTS:=1}"
: "${RUN_LIVE_ENDPOINT_SMOKE:=0}"
: "${RUN_LIVE_DAEMON_SMOKE:=0}"
: "${CLIPPY_FLAGS:=-D warnings}"
: "${GATE_LOG_DIR:=${ROOT_DIR}/artifacts/gate-logs}"
: "${GATE_FAILURE_TAIL_LINES:=40}"
: "${GATE_ERROR_SUMMARY_LINES:=12}"

mkdir -p "${GATE_LOG_DIR}"

print_failure_tail() {
  local log_file="$1"
  local tail_lines="$2"
  local summary_lines="$3"
  local pattern='error|failed|failure|panic|exception|traceback|caused by'

  echo "[gate] failure summary (${log_file}):"
  if [[ ! -s "${log_file}" ]]; then
    echo "[gate] log file is empty"
    return
  fi

  echo "[gate] key error lines:"
  if command -v rg >/dev/null 2>&1; then
    rg -i -n "${pattern}" "${log_file}" | tail -n "${summary_lines}" || true
  else
    grep -i -nE "${pattern}" "${log_file}" | tail -n "${summary_lines}" || true
  fi

  local total_lines start_line
  total_lines="$(wc -l <"${log_file}")"
  start_line=$(( total_lines - tail_lines + 1 ))
  if (( start_line < 1 )); then
    start_line=1
  fi
  echo "[gate] recent tail (last ${tail_lines} lines, starting at line ${start_line}):"
  tail -n "${tail_lines}" "${log_file}" | nl -ba -v "${start_line}"
}

run_step() {
  local step_name="$1"
  shift
  local log_file="${GATE_LOG_DIR}/${step_name}.log"
  echo "[gate] step=${step_name}"
  if "$@" >"${log_file}" 2>&1; then
    echo "[gate] step=${step_name} status=pass"
  else
    echo "[gate] step=${step_name} status=fail"
    print_failure_tail "${log_file}" "${GATE_FAILURE_TAIL_LINES}" "${GATE_ERROR_SUMMARY_LINES}"
    exit 1
  fi
}

case "${GATE_MODE}" in
  strict)
    if [[ "${RUN_FORMAT_CHECK}" != "1" || "${RUN_CLIPPY_CHECK}" != "1" || "${RUN_WORKSPACE_TESTS}" != "1" || "${RUN_E2E_TESTS}" != "1" ]]; then
      echo "[gate] strict mode requires RUN_FORMAT_CHECK=1 RUN_CLIPPY_CHECK=1 RUN_WORKSPACE_TESTS=1 RUN_E2E_TESTS=1" >&2
      echo "[gate] use GATE_MODE=smoke to run a reduced gate profile" >&2
      exit 1
    fi
    ;;
  smoke)
    if [[ "${run_format_check_set}" -eq 0 ]]; then RUN_FORMAT_CHECK=0; fi
    if [[ "${run_clippy_check_set}" -eq 0 ]]; then RUN_CLIPPY_CHECK=0; fi
    if [[ "${run_workspace_tests_set}" -eq 0 ]]; then RUN_WORKSPACE_TESTS=0; fi
    if [[ "${run_e2e_tests_set}" -eq 0 ]]; then RUN_E2E_TESTS=0; fi
    if [[ "${run_live_endpoint_smoke_set}" -eq 0 ]]; then RUN_LIVE_ENDPOINT_SMOKE=0; fi
    if [[ "${run_live_daemon_smoke_set}" -eq 0 ]]; then RUN_LIVE_DAEMON_SMOKE=0; fi
    ;;
  *)
    echo "[gate] invalid GATE_MODE='${GATE_MODE}' (expected: strict|smoke)" >&2
    exit 1
    ;;
esac

echo "[gate] mode=${GATE_MODE}"
cd "${ROOT_DIR}"

if [[ "${RUN_FORMAT_CHECK}" == "1" ]]; then
  run_step cargo_fmt_check cargo fmt --all -- --check
fi

if [[ "${RUN_CLIPPY_CHECK}" == "1" ]]; then
  run_step cargo_clippy cargo clippy --workspace --all-targets -- ${CLIPPY_FLAGS}
fi

if [[ "${RUN_WORKSPACE_TESTS}" == "1" ]]; then
  run_step cargo_test_workspace cargo test --workspace
fi

if [[ "${RUN_E2E_TESTS}" == "1" ]]; then
  run_step cargo_test_e2e cargo test -p oberon4-cli --test end_to_end
fi

if [[ "${RUN_LIVE_ENDPOINT_SMOKE}" == "1" ]]; then
  run_step live_endpoint_smoke bash scripts/live-hotspot-smoke.sh
fi

if [[ "${RUN_LIVE_DAEMON_SMOKE}" == "1" ]]; then
  run_step live_daemon_smoke bash scripts/live-daemon-smoke.sh
fi

echo "[gate] PASS: Oberon4 verification gate satisfied."
