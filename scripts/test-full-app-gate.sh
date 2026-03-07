#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
GATE_SCRIPT="${ROOT_DIR}/scripts/full-app-gate.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

MOCK_BIN="${TMP_DIR}/bin"
LOG_DIR="${TMP_DIR}/logs"
STATE_FILE="${TMP_DIR}/state.json"
mkdir -p "${MOCK_BIN}" "${LOG_DIR}"

cat > "${STATE_FILE}" <<'JSON'
{
  "pipeline": { "completed": true },
  "jobs": {}
}
JSON

cat > "${MOCK_BIN}/curl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${MOCK_CURL_FAIL:-0}" == "1" ]]; then
  echo "curl failed to reach endpoint" >&2
  echo "traceback: synthetic failure for tail readability" >&2
  echo "error: upstream unavailable" >&2
  exit 7
fi
echo "ok"
SH
chmod +x "${MOCK_BIN}/curl"

cat > "${MOCK_BIN}/cargo" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "fmt" ]]; then
  echo "cargo fmt --check"
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
  echo "cargo test $*"
  exit 0
fi
echo "unexpected cargo invocation: $*" >&2
exit 1
SH
chmod +x "${MOCK_BIN}/cargo"

cat > "${MOCK_BIN}/npm" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" != "--prefix" ]]; then
  echo "unexpected npm invocation: $*" >&2
  exit 1
fi
shift 2
if [[ "${1:-}" != "run" ]]; then
  echo "unexpected npm invocation: $*" >&2
  exit 1
fi
case "${2:-}" in
  test:ui:prod)
    echo "npm run test:ui:prod"
    ;;
  test:screenshots:prod)
    echo "npm run test:screenshots:prod"
    ;;
  *)
    echo "unexpected npm script: ${2:-}" >&2
    exit 1
    ;;
esac
SH
chmod +x "${MOCK_BIN}/npm"

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
    STATE_FILE="${STATE_FILE}" \
    GATE_LOG_DIR="${LOG_DIR}" \
    MAIN_APP_URL="http://127.0.0.1:65535" \
    "$@" bash "${GATE_SCRIPT}" >"${output_file}" 2>&1
}

SMOKE_OUTPUT="${TMP_DIR}/smoke.out"
run_gate "${SMOKE_OUTPUT}" GATE_MODE=smoke
assert_contains "${SMOKE_OUTPUT}" "[gate] mode=smoke"
if [[ -f "${LOG_DIR}/cargo_fmt_check.log" ]]; then
  echo "assertion failed: smoke mode should not run cargo fmt by default" >&2
  exit 1
fi
if [[ -f "${LOG_DIR}/website_ui_smoke.log" ]]; then
  echo "assertion failed: smoke mode should not run website UI smoke by default" >&2
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
if [[ ! -f "${LOG_DIR}/website_ui_smoke.log" ]]; then
  echo "assertion failed: strict mode should run website UI smoke" >&2
  exit 1
fi
if [[ ! -f "${LOG_DIR}/website_screenshot_regression.log" ]]; then
  echo "assertion failed: strict mode should run screenshot regression" >&2
  exit 1
fi

FAIL_OUTPUT="${TMP_DIR}/fail-tail.out"
if run_gate "${FAIL_OUTPUT}" GATE_MODE=smoke MOCK_CURL_FAIL=1 GATE_FAILURE_TAIL_LINES=5; then
  echo "assertion failed: failing smoke probe should exit non-zero" >&2
  exit 1
fi
if [[ ! -f "${LOG_DIR}/probe_endpoints.log" ]]; then
  echo "assertion failed: expected stable step log ${LOG_DIR}/probe_endpoints.log" >&2
  exit 1
fi
assert_contains "${FAIL_OUTPUT}" "[gate] failure summary (${LOG_DIR}/probe_endpoints.log):"
assert_contains "${FAIL_OUTPUT}" "[gate] failure summary"
assert_contains "${FAIL_OUTPUT}" "[gate] key error lines:"
assert_contains "${FAIL_OUTPUT}" "[gate] recent tail"
assert_contains "${FAIL_OUTPUT}" "starting at line 1"
assert_contains "${FAIL_OUTPUT}" "synthetic failure for tail readability"

echo "test-full-app-gate.sh: PASS"
