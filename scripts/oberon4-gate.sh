#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
source "${ROOT_DIR}/scripts/lib/failure-bundle.sh"

: "${GATE_MODE:=strict}"
run_format_check_set=0
run_clippy_check_set=0
run_workspace_tests_set=0
run_e2e_tests_set=0
run_live_endpoint_smoke_set=0
run_live_daemon_smoke_set=0
run_tui_pty_smoke_set=0
[[ -v RUN_FORMAT_CHECK ]] && run_format_check_set=1
[[ -v RUN_CLIPPY_CHECK ]] && run_clippy_check_set=1
[[ -v RUN_WORKSPACE_TESTS ]] && run_workspace_tests_set=1
[[ -v RUN_E2E_TESTS ]] && run_e2e_tests_set=1
[[ -v RUN_LIVE_ENDPOINT_SMOKE ]] && run_live_endpoint_smoke_set=1
[[ -v RUN_LIVE_DAEMON_SMOKE ]] && run_live_daemon_smoke_set=1
[[ -v RUN_TUI_PTY_SMOKE ]] && run_tui_pty_smoke_set=1
: "${RUN_FORMAT_CHECK:=1}"
: "${RUN_CLIPPY_CHECK:=1}"
: "${RUN_WORKSPACE_TESTS:=1}"
: "${RUN_E2E_TESTS:=1}"
: "${RUN_LIVE_ENDPOINT_SMOKE:=0}"
: "${RUN_LIVE_DAEMON_SMOKE:=0}"
: "${RUN_TUI_PTY_SMOKE:=0}"
: "${CLIPPY_FLAGS:=-D warnings}"
: "${GATE_LOG_DIR:=${ROOT_DIR}/artifacts/gate-logs}"
: "${GATE_FAILURE_TAIL_LINES:=40}"
: "${GATE_ERROR_SUMMARY_LINES:=12}"

mkdir -p "${GATE_LOG_DIR}"

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

write_gate_env_file() {
  local output_path="$1"
  {
    printf 'GATE_MODE=%s\n' "${GATE_MODE}"
    printf 'RUN_FORMAT_CHECK=%s\n' "${RUN_FORMAT_CHECK}"
    printf 'RUN_CLIPPY_CHECK=%s\n' "${RUN_CLIPPY_CHECK}"
    printf 'RUN_WORKSPACE_TESTS=%s\n' "${RUN_WORKSPACE_TESTS}"
    printf 'RUN_E2E_TESTS=%s\n' "${RUN_E2E_TESTS}"
    printf 'RUN_LIVE_ENDPOINT_SMOKE=%s\n' "${RUN_LIVE_ENDPOINT_SMOKE}"
    printf 'RUN_LIVE_DAEMON_SMOKE=%s\n' "${RUN_LIVE_DAEMON_SMOKE}"
    printf 'RUN_TUI_PTY_SMOKE=%s\n' "${RUN_TUI_PTY_SMOKE}"
    printf 'CLIPPY_FLAGS=%s\n' "${CLIPPY_FLAGS}"
    printf 'GATE_LOG_DIR=%s\n' "${GATE_LOG_DIR}"
    printf 'GATE_FAILURE_TAIL_LINES=%s\n' "${GATE_FAILURE_TAIL_LINES}"
    printf 'GATE_ERROR_SUMMARY_LINES=%s\n' "${GATE_ERROR_SUMMARY_LINES}"
    if [[ -n "${OBERON4_LIVE_BASE_URL:-}" ]]; then
      printf 'OBERON4_LIVE_BASE_URL=%s\n' "${OBERON4_LIVE_BASE_URL}"
    fi
    if [[ -n "${OBERON4_LIVE_OPENAI_MODEL:-}" ]]; then
      printf 'OBERON4_LIVE_OPENAI_MODEL=%s\n' "${OBERON4_LIVE_OPENAI_MODEL}"
    fi
    if [[ -n "${OBERON4_LIVE_API_KEY_ENV:-}" ]]; then
      printf 'OBERON4_LIVE_API_KEY_ENV=%s\n' "${OBERON4_LIVE_API_KEY_ENV}"
      if [[ -n "${!OBERON4_LIVE_API_KEY_ENV:-}" ]]; then
        printf 'OBERON4_LIVE_API_KEY_STATUS=set\n'
      else
        printf 'OBERON4_LIVE_API_KEY_STATUS=unset\n'
      fi
    fi
  } >"${output_path}"
}

capture_gate_failure_bundle() {
  local failed_stage="$1"
  local primary_log_path="$2"
  local bundle_dir
  bundle_dir="$(failure_bundle_create_dir "${ROOT_DIR}" "oberon4-gate")"
  mkdir -p "${bundle_dir}/logs"
  failure_bundle_write_metadata \
    "${bundle_dir}/summary.json" \
    "oberon4-gate" \
    "${failed_stage}" \
    "${primary_log_path}"
  cp "${bundle_dir}/summary.json" "${bundle_dir}/bundle-meta.json"
  write_commands_file "${bundle_dir}/commands.txt"
  write_gate_env_file "${bundle_dir}/env.txt"
  find "${GATE_LOG_DIR}" -maxdepth 1 -type f -name '*.log' -print0 | while IFS= read -r -d '' log_file; do
    failure_bundle_copy_file "${log_file}" "${bundle_dir}/logs"
  done
  failure_bundle_capture_doctor "${bundle_dir}" cargo run -p oberon4-cli -- doctor --json
  failure_bundle_capture_effective_config "${bundle_dir}" cargo run -p oberon4-cli -- config show
  printf '%s\n' "${bundle_dir}"
}

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
  record_command "${step_name}" "$@"
  echo "[gate] step=${step_name}"
  if "$@" >"${log_file}" 2>&1; then
    echo "[gate] step=${step_name} status=pass"
  else
    local bundle_dir
    echo "[gate] step=${step_name} status=fail"
    print_failure_tail "${log_file}" "${GATE_FAILURE_TAIL_LINES}" "${GATE_ERROR_SUMMARY_LINES}"
    bundle_dir="$(capture_gate_failure_bundle "${step_name}" "${log_file}")"
    echo "[gate] failure_bundle stage=${step_name} bundle=${bundle_dir} log=${log_file}" >&2
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
    if [[ "${run_tui_pty_smoke_set}" -eq 0 ]]; then RUN_TUI_PTY_SMOKE=0; fi
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

if [[ "${RUN_TUI_PTY_SMOKE}" == "1" ]]; then
  run_step tui_pty_smoke bash scripts/tui-pty-smoke.sh
fi

echo "[gate] PASS: Oberon4 verification gate satisfied."
