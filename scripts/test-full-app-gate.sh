#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
GATE_SCRIPT="${ROOT_DIR}/scripts/oberon4-gate.sh"
WRAPPER_SCRIPT="${ROOT_DIR}/scripts/full-app-gate.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

MOCK_BIN="${TMP_DIR}/bin"
LOG_DIR="${TMP_DIR}/logs"
BUNDLE_ROOT="${TMP_DIR}/failure-bundles"
DATA_DIR="${TMP_DIR}/data"
mkdir -p "${MOCK_BIN}" "${LOG_DIR}" "${BUNDLE_ROOT}" "${DATA_DIR}"
printf 'daemon stdout line\n' > "${DATA_DIR}/daemon.out"
printf 'daemon stderr line\n' > "${DATA_DIR}/daemon.err"

cat > "${MOCK_BIN}/cargo" <<'SH'
#!/usr/bin/bash
set -euo pipefail
if [[ "${1:-}" == "fmt" ]]; then
  echo "cargo fmt --all -- --check"
  exit 0
fi
if [[ "${1:-}" == "clippy" ]]; then
  if [[ "${MOCK_CARGO_FAIL_CLIPPY:-0}" == "1" ]]; then
    echo "error: clippy synthetic failure" >&2
    exit 1
  fi
  echo "cargo clippy --workspace --all-targets"
  exit 0
fi
if [[ "${1:-}" == "test" ]]; then
  if [[ "${MOCK_CARGO_FAIL_TEST:-0}" == "1" ]]; then
    echo "panic: synthetic cargo test failure" >&2
    echo "caused by: mocked regression" >&2
    exit 1
  fi
  echo "cargo test $*"
  exit 0
fi
if [[ "${1:-}" == "audit" ]]; then
  echo "cargo-audit ok"
  exit 0
fi
if [[ "${1:-}" == "run" && "${2:-}" == "-p" && "${3:-}" == "oberon4-cli" && "${4:-}" == "--" && "${5:-}" == "doctor" && "${6:-}" == "--json" ]]; then
  echo '{"running":false,"ready":false,"endpoint_check":"fail","error":"mocked"}'
  exit 1
fi
if [[ "${1:-}" == "run" && "${2:-}" == "-p" && "${3:-}" == "oberon4-cli" && "${4:-}" == "--" && "${5:-}" == "config" && "${6:-}" == "show" ]]; then
  echo '{"endpoint":{"base_url":"http://example.test","model":"gpt-test","api_key_env":"<redacted>"}}'
  exit 0
fi
if [[ "${1:-}" == "run" && "${2:-}" == "-p" && "${3:-}" == "oberon4-cli" && "${4:-}" == "--" && "${5:-}" == "logs" && "${6:-}" == "tail" ]]; then
  echo '{"level":"info","message":"mocked logs tail"}'
  exit 0
fi
echo "unexpected cargo invocation: $*" >&2
exit 1
SH
chmod +x "${MOCK_BIN}/cargo"

cat > "${MOCK_BIN}/gitleaks" <<'SH'
#!/usr/bin/bash
set -euo pipefail
report_path=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --report-path)
      report_path="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
if [[ -n "${report_path}" ]]; then
  printf '[]\n' > "${report_path}"
fi
if [[ "${MOCK_SECURITY_GATE_FAIL:-0}" == "1" ]]; then
  echo "gitleaks synthetic failure" >&2
  exit 1
fi
echo "gitleaks ok"
SH
chmod +x "${MOCK_BIN}/gitleaks"

cat > "${MOCK_BIN}/semgrep" <<'SH'
#!/usr/bin/bash
set -euo pipefail
output_path=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      output_path="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
if [[ -n "${output_path}" ]]; then
  printf '{"results":[]}\n' > "${output_path}"
fi
echo "semgrep ok"
SH
chmod +x "${MOCK_BIN}/semgrep"

cat > "${MOCK_BIN}/cargo-audit" <<'SH'
#!/usr/bin/bash
set -euo pipefail
printf '{"vulnerabilities":{"found":false}}\n'
SH
chmod +x "${MOCK_BIN}/cargo-audit"

cat > "${MOCK_BIN}/bash" <<'SH'
#!/usr/bin/bash
set -euo pipefail
if [[ "${1:-}" == "scripts/live-hotspot-smoke.sh" ]]; then
  echo "live hotspot smoke"
  exit 0
fi
if [[ "${1:-}" == "scripts/live-daemon-smoke.sh" ]]; then
  echo "live daemon smoke"
  exit 0
fi
if [[ "${1:-}" == "scripts/tui-pty-smoke.sh" ]]; then
  echo "tui pty smoke"
  exit 0
fi
exec /usr/bin/bash "$@"
SH
chmod +x "${MOCK_BIN}/bash"

assert_contains() {
  local file="$1"
  local pattern="$2"
  if ! rg -n --fixed-strings "${pattern}" "${file}" >/dev/null; then
    echo "assertion failed: expected pattern '${pattern}' in ${file}" >&2
    echo "----- file dump (${file}) -----" >&2
    cat "${file}" >&2
    exit 1
  fi
}

run_gate() {
  local output_file="$1"
  shift
  env PATH="${MOCK_BIN}:$PATH" \
    GATE_LOG_DIR="${LOG_DIR}" \
    FAILURE_BUNDLE_ROOT="${BUNDLE_ROOT}" \
    "$@" bash "${GATE_SCRIPT}" >"${output_file}" 2>&1
}

latest_bundle_dir() {
  find "${BUNDLE_ROOT}" -mindepth 1 -maxdepth 1 -type d | sort | tail -n 1
}

SMOKE_OUTPUT="${TMP_DIR}/smoke.out"
run_gate "${SMOKE_OUTPUT}" GATE_MODE=smoke
assert_contains "${SMOKE_OUTPUT}" "[gate] mode=smoke"
if [[ -f "${LOG_DIR}/cargo_fmt_check.log" ]]; then
  echo "assertion failed: smoke mode should not run cargo fmt by default" >&2
  exit 1
fi
if [[ -f "${LOG_DIR}/cargo_test_workspace.log" ]]; then
  echo "assertion failed: smoke mode should not run workspace tests by default" >&2
  exit 1
fi

STRICT_REJECT_OUTPUT="${TMP_DIR}/strict-reject.out"
if run_gate "${STRICT_REJECT_OUTPUT}" GATE_MODE=strict RUN_FORMAT_CHECK=0; then
  echo "assertion failed: strict mode must reject disabled checks" >&2
  exit 1
fi
assert_contains "${STRICT_REJECT_OUTPUT}" "[gate] strict mode requires RUN_FORMAT_CHECK=1"

STRICT_OUTPUT="${TMP_DIR}/strict.out"
run_gate "${STRICT_OUTPUT}" GATE_MODE=strict
assert_contains "${STRICT_OUTPUT}" "[gate] mode=strict"
if [[ ! -f "${LOG_DIR}/cargo_clippy.log" ]]; then
  echo "assertion failed: strict mode should run cargo clippy" >&2
  exit 1
fi
if [[ ! -f "${LOG_DIR}/cargo_test_workspace.log" ]]; then
  echo "assertion failed: strict mode should run workspace tests" >&2
  exit 1
fi
if [[ ! -f "${LOG_DIR}/cargo_test_e2e.log" ]]; then
  echo "assertion failed: strict mode should run end-to-end tests" >&2
  exit 1
fi
if [[ ! -f "${LOG_DIR}/security_gate.log" ]]; then
  echo "assertion failed: strict mode should run security gate by default" >&2
  exit 1
fi
assert_contains "${LOG_DIR}/security_gate.log" "[security-gate] PASS: Oberon4 security verification satisfied."

FAIL_OUTPUT="${TMP_DIR}/fail-tail.out"
if run_gate "${FAIL_OUTPUT}" \
  GATE_MODE=smoke \
  RUN_WORKSPACE_TESTS=1 \
  MOCK_CARGO_FAIL_TEST=1 \
  GATE_FAILURE_TAIL_LINES=5 \
  OBERON4_LIVE_BASE_URL=http://example.test/v1 \
  OBERON4_LIVE_OPENAI_MODEL=gpt-test \
  OBERON4_LIVE_API_KEY_ENV=TEST_GATE_KEY \
  OBERON4_DATA_DIR="${DATA_DIR}" \
  TEST_GATE_KEY=super-secret-token; then
  echo "assertion failed: failing workspace test step should exit non-zero" >&2
  exit 1
fi
BUNDLE_DIR="$(latest_bundle_dir)"
if [[ ! -f "${LOG_DIR}/cargo_test_workspace.log" ]]; then
  echo "assertion failed: expected stable step log ${LOG_DIR}/cargo_test_workspace.log" >&2
  exit 1
fi
[[ -d "${BUNDLE_DIR}" ]] || { echo "missing failure bundle" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/summary.json" ]] || { echo "missing bundle summary.json" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/bundle-meta.json" ]] || { echo "missing bundle bundle-meta.json" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/commands.txt" ]] || { echo "missing bundle commands.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/env.txt" ]] || { echo "missing bundle env.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/doctor-output.txt" ]] || { echo "missing bundle doctor-output.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/doctor-status.txt" ]] || { echo "missing bundle doctor-status.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/effective-config.json" ]] || { echo "missing bundle effective-config.json" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/effective-config-status.txt" ]] || { echo "missing bundle effective-config-status.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/logs-tail.txt" ]] || { echo "missing bundle logs-tail.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/logs-tail-status.txt" ]] || { echo "missing bundle logs-tail-status.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/request-summary.txt" ]] || { echo "missing bundle request-summary.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/evidence-summary.json" ]] || { echo "missing bundle evidence-summary.json" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/daemon-runtime.txt" ]] || { echo "missing bundle daemon-runtime.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/daemon-out.log" ]] || { echo "missing bundle daemon-out.log" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/daemon-err.log" ]] || { echo "missing bundle daemon-err.log" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/logs/cargo_test_workspace.log" ]] || { echo "missing copied workspace log" >&2; exit 1; }
assert_contains "${FAIL_OUTPUT}" "[gate] failure summary (${LOG_DIR}/cargo_test_workspace.log):"
assert_contains "${FAIL_OUTPUT}" "[gate] failure summary"
assert_contains "${FAIL_OUTPUT}" "[gate] key error lines:"
assert_contains "${FAIL_OUTPUT}" "[gate] recent tail"
assert_contains "${FAIL_OUTPUT}" "starting at line 1"
assert_contains "${FAIL_OUTPUT}" "mocked regression"
assert_contains "${FAIL_OUTPUT}" "[gate] failure_bundle stage=cargo_test_workspace bundle=${BUNDLE_DIR} log=${LOG_DIR}/cargo_test_workspace.log"
assert_contains "${BUNDLE_DIR}/summary.json" '"failed_stage": "cargo_test_workspace"'
assert_contains "${BUNDLE_DIR}/bundle-meta.json" '"entrypoint": "oberon4-gate"'
assert_contains "${BUNDLE_DIR}/commands.txt" "cargo_test_workspace: cargo test --workspace"
assert_contains "${BUNDLE_DIR}/env.txt" "OBERON4_LIVE_BASE_URL=http://example.test/v1"
assert_contains "${BUNDLE_DIR}/env.txt" "OBERON4_LIVE_OPENAI_MODEL=gpt-test"
assert_contains "${BUNDLE_DIR}/env.txt" "OBERON4_LIVE_API_KEY_ENV=TEST_GATE_KEY"
assert_contains "${BUNDLE_DIR}/env.txt" "OBERON4_LIVE_API_KEY_STATUS=set"
assert_contains "${BUNDLE_DIR}/daemon-runtime.txt" "daemon_out_status=present"
assert_contains "${BUNDLE_DIR}/daemon-runtime.txt" "daemon_err_status=present"
assert_contains "${BUNDLE_DIR}/daemon-out.log" "daemon stdout line"
assert_contains "${BUNDLE_DIR}/daemon-err.log" "daemon stderr line"
assert_contains "${BUNDLE_DIR}/logs-tail.txt" "mocked logs tail"
assert_contains "${BUNDLE_DIR}/request-summary.txt" "source=${LOG_DIR}/cargo_test_workspace.log"
assert_contains "${BUNDLE_DIR}/request-summary.txt" "mocked regression"
assert_contains "${BUNDLE_DIR}/evidence-summary.json" '"entrypoint": "oberon4-gate"'
assert_contains "${BUNDLE_DIR}/evidence-summary.json" '"failed_stage": "cargo_test_workspace"'
assert_contains "${BUNDLE_DIR}/evidence-summary.json" '"doctor_exit_code": "1"'
assert_contains "${BUNDLE_DIR}/evidence-summary.json" '"effective_config_exit_code": "0"'
assert_contains "${BUNDLE_DIR}/evidence-summary.json" '"daemon_log_available": true'
assert_contains "${BUNDLE_DIR}/evidence-summary.json" '"request_excerpt_available": true'
if rg -n --fixed-strings "super-secret-token" "${BUNDLE_DIR}/env.txt" >/dev/null; then
  echo "assertion failed: secret value leaked into gate bundle env.txt" >&2
  exit 1
fi

LIVE_OUTPUT="${TMP_DIR}/live.out"
run_gate "${LIVE_OUTPUT}" GATE_MODE=smoke RUN_LIVE_ENDPOINT_SMOKE=1 RUN_LIVE_DAEMON_SMOKE=1
if [[ ! -f "${LOG_DIR}/live_endpoint_smoke.log" ]]; then
  echo "assertion failed: live endpoint smoke should be logged when enabled" >&2
  exit 1
fi
if [[ ! -f "${LOG_DIR}/live_daemon_smoke.log" ]]; then
  echo "assertion failed: live daemon smoke should be logged when enabled" >&2
  exit 1
fi

TUI_PTY_OUTPUT="${TMP_DIR}/tui-pty.out"
run_gate "${TUI_PTY_OUTPUT}" GATE_MODE=smoke RUN_TUI_PTY_SMOKE=1
if [[ ! -f "${LOG_DIR}/tui_pty_smoke.log" ]]; then
  echo "assertion failed: tui PTY smoke should be logged when enabled" >&2
  exit 1
fi

SECURITY_OUTPUT="${TMP_DIR}/security.out"
run_gate "${SECURITY_OUTPUT}" GATE_MODE=smoke RUN_SECURITY_GATE=1
if [[ ! -f "${LOG_DIR}/security_gate.log" ]]; then
  echo "assertion failed: security gate should be logged when enabled" >&2
  exit 1
fi
assert_contains "${LOG_DIR}/security_gate.log" "[security-gate] PASS: Oberon4 security verification satisfied."

SECURITY_FAIL_OUTPUT="${TMP_DIR}/security-fail.out"
if run_gate "${SECURITY_FAIL_OUTPUT}" GATE_MODE=smoke RUN_SECURITY_GATE=1 MOCK_SECURITY_GATE_FAIL=1; then
  echo "assertion failed: failing security gate should exit non-zero" >&2
  exit 1
fi
SECURITY_FAIL_BUNDLE_DIR="$(latest_bundle_dir)"
assert_contains "${SECURITY_FAIL_OUTPUT}" "[gate] failure_bundle stage=security_gate bundle=${SECURITY_FAIL_BUNDLE_DIR} log=${LOG_DIR}/security_gate.log"
[[ -f "${SECURITY_FAIL_BUNDLE_DIR}/logs/security_gate.log" ]] || { echo "missing copied security gate log" >&2; exit 1; }
assert_contains "${SECURITY_FAIL_BUNDLE_DIR}/logs/security_gate.log" "[security-gate] stage=secret_scan status=fail"

WRAPPER_OUTPUT="${TMP_DIR}/wrapper.out"
env PATH="${MOCK_BIN}:$PATH" \
  GATE_LOG_DIR="${LOG_DIR}" \
  GATE_MODE=smoke \
  bash "${WRAPPER_SCRIPT}" >"${WRAPPER_OUTPUT}" 2>&1
assert_contains "${WRAPPER_OUTPUT}" "scripts/full-app-gate.sh is deprecated"
assert_contains "${WRAPPER_OUTPUT}" "[gate] mode=smoke"

echo "test-full-app-gate.sh: PASS"
