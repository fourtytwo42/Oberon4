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
mkdir -p "${MOCK_BIN}" "${LOG_DIR}" "${BUNDLE_ROOT}"

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
if [[ "${1:-}" == "run" && "${2:-}" == "-p" && "${3:-}" == "oberon4-cli" && "${4:-}" == "--" && "${5:-}" == "doctor" && "${6:-}" == "--json" ]]; then
  echo '{"running":false,"ready":false,"endpoint_check":"fail","error":"mocked"}'
  exit 1
fi
echo "unexpected cargo invocation: $*" >&2
exit 1
SH
chmod +x "${MOCK_BIN}/cargo"

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

FAIL_OUTPUT="${TMP_DIR}/fail-tail.out"
if run_gate "${FAIL_OUTPUT}" \
  GATE_MODE=smoke \
  RUN_WORKSPACE_TESTS=1 \
  MOCK_CARGO_FAIL_TEST=1 \
  GATE_FAILURE_TAIL_LINES=5 \
  OBERON4_LIVE_BASE_URL=http://example.test/v1 \
  OBERON4_LIVE_OPENAI_MODEL=gpt-test \
  OBERON4_LIVE_API_KEY_ENV=TEST_GATE_KEY \
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
[[ -f "${BUNDLE_DIR}/commands.txt" ]] || { echo "missing bundle commands.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/env.txt" ]] || { echo "missing bundle env.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/doctor-output.txt" ]] || { echo "missing bundle doctor-output.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/doctor-status.txt" ]] || { echo "missing bundle doctor-status.txt" >&2; exit 1; }
[[ -f "${BUNDLE_DIR}/logs/cargo_test_workspace.log" ]] || { echo "missing copied workspace log" >&2; exit 1; }
assert_contains "${FAIL_OUTPUT}" "[gate] failure summary (${LOG_DIR}/cargo_test_workspace.log):"
assert_contains "${FAIL_OUTPUT}" "[gate] failure summary"
assert_contains "${FAIL_OUTPUT}" "[gate] key error lines:"
assert_contains "${FAIL_OUTPUT}" "[gate] recent tail"
assert_contains "${FAIL_OUTPUT}" "starting at line 1"
assert_contains "${FAIL_OUTPUT}" "mocked regression"
assert_contains "${FAIL_OUTPUT}" "[gate] failure_bundle stage=cargo_test_workspace bundle=${BUNDLE_DIR} log=${LOG_DIR}/cargo_test_workspace.log"
assert_contains "${BUNDLE_DIR}/summary.json" '"failed_stage": "cargo_test_workspace"'
assert_contains "${BUNDLE_DIR}/commands.txt" "cargo_test_workspace: cargo test --workspace"
assert_contains "${BUNDLE_DIR}/env.txt" "OBERON4_LIVE_BASE_URL=http://example.test/v1"
assert_contains "${BUNDLE_DIR}/env.txt" "OBERON4_LIVE_OPENAI_MODEL=gpt-test"
assert_contains "${BUNDLE_DIR}/env.txt" "OBERON4_LIVE_API_KEY_ENV=TEST_GATE_KEY"
assert_contains "${BUNDLE_DIR}/env.txt" "OBERON4_LIVE_API_KEY_STATUS=set"
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

WRAPPER_OUTPUT="${TMP_DIR}/wrapper.out"
env PATH="${MOCK_BIN}:$PATH" \
  GATE_LOG_DIR="${LOG_DIR}" \
  GATE_MODE=smoke \
  bash "${WRAPPER_SCRIPT}" >"${WRAPPER_OUTPUT}" 2>&1
assert_contains "${WRAPPER_OUTPUT}" "scripts/full-app-gate.sh is deprecated"
assert_contains "${WRAPPER_OUTPUT}" "[gate] mode=smoke"

echo "test-full-app-gate.sh: PASS"
