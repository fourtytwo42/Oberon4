#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
SECURITY_GATE_SCRIPT="${ROOT_DIR}/scripts/oberon4-security-gate.sh"
WRAPPER_SCRIPT="${ROOT_DIR}/scripts/security-gate.sh"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

MOCK_BIN="${TMP_DIR}/bin"
LOG_DIR="${TMP_DIR}/logs"
ORDER_FILE="${TMP_DIR}/order.log"
mkdir -p "${MOCK_BIN}" "${LOG_DIR}"

cat > "${MOCK_BIN}/gitleaks" <<'SH'
#!/usr/bin/bash
set -euo pipefail
printf 'secret_scan\n' >>"${ORDER_FILE:?}"
if [[ "${MOCK_SECRET_SCAN_FAIL:-0}" == "1" ]]; then
  echo "gitleaks synthetic failure" >&2
  exit 1
fi
echo "gitleaks ok"
SH
chmod +x "${MOCK_BIN}/gitleaks"

cat > "${MOCK_BIN}/semgrep" <<'SH'
#!/usr/bin/bash
set -euo pipefail
printf 'sast\n' >>"${ORDER_FILE:?}"
if [[ "${MOCK_SAST_FAIL:-0}" == "1" ]]; then
  echo "semgrep synthetic failure" >&2
  exit 1
fi
echo "semgrep ok"
SH
chmod +x "${MOCK_BIN}/semgrep"

cat > "${MOCK_BIN}/cargo-audit" <<'SH'
#!/usr/bin/bash
set -euo pipefail
printf 'dependency_audit\n' >>"${ORDER_FILE:?}"
if [[ "${MOCK_DEPENDENCY_AUDIT_FAIL:-0}" == "1" ]]; then
  echo "cargo-audit synthetic failure" >&2
  exit 1
fi
echo "cargo-audit ok"
SH
chmod +x "${MOCK_BIN}/cargo-audit"

assert_contains() {
  local file="$1"
  local pattern="$2"
  if ! rg -n --fixed-strings "${pattern}" "${file}" >/dev/null; then
    echo "assertion failed: expected pattern '${pattern}' in ${file}" >&2
    cat "${file}" >&2
    exit 1
  fi
}

run_gate() {
  local output_file="$1"
  shift
  : > "${ORDER_FILE}"
  env PATH="${MOCK_BIN}:$PATH" \
    ORDER_FILE="${ORDER_FILE}" \
    SECURITY_GATE_LOG_DIR="${LOG_DIR}" \
    "$@" bash "${SECURITY_GATE_SCRIPT}" >"${output_file}" 2>&1
}

SUCCESS_OUTPUT="${TMP_DIR}/success.out"
run_gate "${SUCCESS_OUTPUT}"
assert_contains "${SUCCESS_OUTPUT}" "[security-gate] PASS: Oberon4 security verification satisfied."
assert_contains "${ORDER_FILE}" "secret_scan"
assert_contains "${ORDER_FILE}" "sast"
assert_contains "${ORDER_FILE}" "dependency_audit"
[[ -f "${LOG_DIR}/secret_scan.log" ]] || { echo "missing secret_scan.log" >&2; exit 1; }
[[ -f "${LOG_DIR}/sast.log" ]] || { echo "missing sast.log" >&2; exit 1; }
[[ -f "${LOG_DIR}/dependency_audit.log" ]] || { echo "missing dependency_audit.log" >&2; exit 1; }
assert_contains "${LOG_DIR}/secret_scan.log" "gitleaks ok"
assert_contains "${LOG_DIR}/sast.log" "semgrep ok"
assert_contains "${LOG_DIR}/dependency_audit.log" "cargo-audit ok"

MISSING_TOOL_OUTPUT="${TMP_DIR}/missing-tool.out"
if env PATH="/usr/bin:/bin" SECURITY_GATE_LOG_DIR="${LOG_DIR}" bash "${SECURITY_GATE_SCRIPT}" >"${MISSING_TOOL_OUTPUT}" 2>&1; then
  echo "expected missing-tool failure" >&2
  exit 1
fi
assert_contains "${MISSING_TOOL_OUTPUT}" "[security-gate] step=secret_scan status=fail missing_tool=gitleaks"

SKIP_OUTPUT="${TMP_DIR}/skip.out"
run_gate "${SKIP_OUTPUT}" RUN_SECRET_SCAN=0 RUN_SAST=1 RUN_DEPENDENCY_AUDIT=0
if rg -n --fixed-strings "secret_scan" "${ORDER_FILE}" >/dev/null; then
  echo "secret_scan should be skipped" >&2
  exit 1
fi
assert_contains "${ORDER_FILE}" "sast"
if rg -n --fixed-strings "dependency_audit" "${ORDER_FILE}" >/dev/null; then
  echo "dependency_audit should be skipped" >&2
  exit 1
fi

FAIL_OUTPUT="${TMP_DIR}/fail.out"
if run_gate "${FAIL_OUTPUT}" MOCK_SAST_FAIL=1; then
  echo "expected sast failure" >&2
  exit 1
fi
assert_contains "${FAIL_OUTPUT}" "[security-gate] step=sast status=fail log=${LOG_DIR}/sast.log"
assert_contains "${LOG_DIR}/sast.log" "semgrep synthetic failure"

WRAPPER_OUTPUT="${TMP_DIR}/wrapper.out"
env PATH="${MOCK_BIN}:$PATH" \
  ORDER_FILE="${ORDER_FILE}" \
  SECURITY_GATE_LOG_DIR="${LOG_DIR}" \
  bash "${WRAPPER_SCRIPT}" >"${WRAPPER_OUTPUT}" 2>&1
assert_contains "${WRAPPER_OUTPUT}" "scripts/security-gate.sh is deprecated"
assert_contains "${WRAPPER_OUTPUT}" "[security-gate] PASS: Oberon4 security verification satisfied."

echo "test-security-gate.sh: PASS"
