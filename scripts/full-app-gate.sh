#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
STATE_FILE="${STATE_FILE:-${ROOT_DIR}/.agent-orchestration/state.json}"

: "${MAIN_APP_URL:=http://127.0.0.1:11434}"
: "${GATE_MODE:=strict}"
run_format_check_set=0
run_clippy_check_set=0
run_workspace_tests_set=0
run_starter_plugin_tests_set=0
run_ui_tests_set=0
run_screenshot_regression_set=0
[[ -v RUN_FORMAT_CHECK ]] && run_format_check_set=1
[[ -v RUN_CLIPPY_CHECK ]] && run_clippy_check_set=1
[[ -v RUN_WORKSPACE_TESTS ]] && run_workspace_tests_set=1
[[ -v RUN_STARTER_PLUGIN_TESTS ]] && run_starter_plugin_tests_set=1
[[ -v RUN_UI_TESTS ]] && run_ui_tests_set=1
[[ -v RUN_SCREENSHOT_REGRESSION ]] && run_screenshot_regression_set=1
: "${RUN_FORMAT_CHECK:=1}"
: "${RUN_CLIPPY_CHECK:=1}"
: "${RUN_WORKSPACE_TESTS:=1}"
: "${RUN_STARTER_PLUGIN_TESTS:=1}"
: "${RUN_UI_TESTS:=1}"
: "${RUN_SCREENSHOT_REGRESSION:=1}"
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
    if [[ "${RUN_FORMAT_CHECK}" != "1" || "${RUN_CLIPPY_CHECK}" != "1" || "${RUN_WORKSPACE_TESTS}" != "1" || "${RUN_STARTER_PLUGIN_TESTS}" != "1" || "${RUN_UI_TESTS}" != "1" || "${RUN_SCREENSHOT_REGRESSION}" != "1" ]]; then
      echo "[gate] strict mode requires RUN_FORMAT_CHECK=1 RUN_CLIPPY_CHECK=1 RUN_WORKSPACE_TESTS=1 RUN_STARTER_PLUGIN_TESTS=1 RUN_UI_TESTS=1 RUN_SCREENSHOT_REGRESSION=1" >&2
      echo "[gate] use GATE_MODE=smoke to run a reduced gate profile" >&2
      exit 1
    fi
    ;;
  smoke)
    if [[ "${run_format_check_set}" -eq 0 ]]; then RUN_FORMAT_CHECK=0; fi
    if [[ "${run_clippy_check_set}" -eq 0 ]]; then RUN_CLIPPY_CHECK=0; fi
    if [[ "${run_workspace_tests_set}" -eq 0 ]]; then RUN_WORKSPACE_TESTS=0; fi
    if [[ "${run_starter_plugin_tests_set}" -eq 0 ]]; then RUN_STARTER_PLUGIN_TESTS=0; fi
    if [[ "${run_ui_tests_set}" -eq 0 ]]; then RUN_UI_TESTS=0; fi
    if [[ "${run_screenshot_regression_set}" -eq 0 ]]; then RUN_SCREENSHOT_REGRESSION=0; fi
    ;;
  *)
    echo "[gate] invalid GATE_MODE='${GATE_MODE}' (expected: strict|smoke)" >&2
    exit 1
    ;;
esac

echo "[gate] validating orchestration completion..."
node -e '
const fs=require("fs");
const p=process.argv[1];
if(!fs.existsSync(p)){console.error(`[gate] missing state file: ${p}`); process.exit(1);}
const s=JSON.parse(fs.readFileSync(p,"utf8"));
const completed=Boolean(s?.pipeline?.completed);
const jobs=Object.values(s.jobs||{});
const done=jobs.filter(j=>j.status==="done").length;
if(!completed){console.error("[gate] pipeline not marked completed"); process.exit(1);}
if(done!==jobs.length){console.error(`[gate] not all jobs done: ${done}/${jobs.length}`); process.exit(1);}
console.log(`[gate] orchestration complete: ${done}/${jobs.length}`);
' "${STATE_FILE}"

echo "[gate] probing main app endpoints..."
run_step probe_endpoints curl -fsS "${MAIN_APP_URL}/v1/endpoints"
run_step probe_console curl -fsS -I "${MAIN_APP_URL}/console"
echo "[gate] main app reachable: ${MAIN_APP_URL}"
echo "[gate] mode=${GATE_MODE}"

cd "${ROOT_DIR}"

if [ "${RUN_FORMAT_CHECK}" = "1" ]; then
  run_step cargo_fmt_check cargo fmt --check
fi

if [ "${RUN_CLIPPY_CHECK}" = "1" ]; then
  run_step cargo_clippy cargo clippy --workspace --all-targets -- ${CLIPPY_FLAGS}
fi

if [ "${RUN_WORKSPACE_TESTS}" = "1" ]; then
  run_step cargo_test_workspace cargo test --workspace
fi

if [ "${RUN_STARTER_PLUGIN_TESTS}" = "1" ]; then
  run_step test_llmhotspot_quick_choice_numbers \
    cargo test -p llmhotspot routing_pipeline_telegram_quick_choice_fallback_numbers_are_contiguous_for_valid_options -- --exact
  run_step test_llmhotspot_quick_choice_inline \
    cargo test -p llmhotspot routing_pipeline_surfaces_telegram_inline_quick_choice_with_numbered_fallback -- --exact
  run_step test_llmhotspot_approval_required \
    cargo test -p llmhotspot routing_pipeline_returns_approval_required_without_execution -- --exact
  run_step test_llm_control_api_keepalive \
    cargo test -p llm-control-api --lib keepalive_loop -- --nocapture
fi

if [ "${RUN_UI_TESTS}" = "1" ]; then
  run_step website_ui_smoke npm --prefix "${ROOT_DIR}/apps/website" run test:ui:prod
fi

if [ "${RUN_SCREENSHOT_REGRESSION}" = "1" ]; then
  run_step website_screenshot_regression npm --prefix "${ROOT_DIR}/apps/website" run test:screenshots:prod
fi

echo "[gate] PASS: full-app completion gate satisfied."
